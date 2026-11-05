// Load the web build in a running Chrome over the DevTools protocol and fail on any GPU
// validation error, shader error, panic, uncaught exception, or lost GPU device. Pages can ask for
// a new 3D view mid-session (?late_view=<s>) and for the device to be destroyed (?lose_device=<s>).
//
//   node scripts/web-smoke.mjs <devtools-port> <url> <wait-ms> <screenshot.png> [expected-renderer]
//
// With expected-renderer (for example "WebGPU"), the app's "FlightCraft renderer: ..." console line
// must name it, so a silent fallback to WebGL 2 fails the check instead of hiding a WebGPU bug.
import { writeFileSync } from "node:fs";

const [port, url, waitMs, out, expected] = process.argv.slice(2);
// ?lose_device=<s> destroys the GPU device on purpose; any other loss of it is a failure.
const wantsLoss = /[?&]lose_device=/.test(url);
// SMOKE_LOSS_OK=1: the browser may lose the device on its own; that is not a failure.
const lossOk = Boolean(process.env.SMOKE_LOSS_OK);
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const problems = [];
let renderer = null;
let lateView = false;
let deviceLost = false;

let target;
for (let i = 0; i < 100 && !target; i++) {
  try {
    const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
    target = list.find((t) => t.type === "page");
  } catch {}
  if (!target) await sleep(200);
}
if (!target) {
  console.error("web-smoke: no page target in Chrome");
  process.exit(2);
}

const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((resolve) => ws.addEventListener("open", resolve));
let id = 0;
const pending = new Map();
const bad = /WGSL|ShaderModule|Invalid|validation|panicked|could not start|RuntimeError|unreachable/i;
ws.addEventListener("message", (event) => {
  const m = JSON.parse(event.data);
  if (m.id && pending.has(m.id)) {
    pending.get(m.id)(m);
    pending.delete(m.id);
  }
  if (m.method === "Runtime.consoleAPICalled") {
    const text = m.params.args.map((a) => a.value ?? a.description ?? "").join(" ");
    const found = text.match(/FlightCraft renderer: (.+)/);
    if (found) renderer = found[1].trim();
    if (process.env.SMOKE_DIAG) console.log("console." + m.params.type + ": " + text.slice(0, 300));
    if (/FlightCraft late view: built/.test(text)) lateView = true;
    if (/GPU device lost/.test(text)) deviceLost = true;
    if (/GPU device lost/.test(text) && (wantsLoss || lossOk)) return;
    if (m.params.type === "error" || bad.test(text)) problems.push(`console.${m.params.type}: ${text}`);
  }
  if (m.method === "Log.entryAdded") {
    const { level, text } = m.params.entry;
    if (level === "error" || bad.test(text)) problems.push(`log ${level}: ${text}`);
  }
  if (m.method === "Runtime.exceptionThrown") {
    const d = m.params.exceptionDetails;
    problems.push(`exception: ${d.text} ${d.exception?.description ?? ""}`);
  }
});
const send = (method, params = {}) =>
  new Promise((resolve) => {
    const i = ++id;
    pending.set(i, resolve);
    ws.send(JSON.stringify({ id: i, method, params }));
  });

await send("Runtime.enable");
await send("Log.enable");
await send("Emulation.setDeviceMetricsOverride", { width: 1280, height: 800, deviceScaleFactor: 1, mobile: false });
await send("Page.enable");
// Count the buffers the page asks the GPU for, so the run can tell a 3D view really built its
// resources from one that never reached the GPU. With SMOKE_DIAG, also log GPU calls and device loss.
await send("Page.addScriptToEvaluateOnNewDocument", {
  source: `(() => {
    window.__gpuBuffers = 0;
    window.__viewBuffers = 0;
    const diag = ${process.env.SMOKE_DIAG ? "true" : "false"};
    const log = (what, extra) => console.log("smoke-diag " + what + " " + JSON.stringify({ buffers: window.__gpuBuffers, t: Math.round(performance.now()), ...extra }));
    if (!window.GPUDevice) return;
    const create = GPUDevice.prototype.createBuffer;
    GPUDevice.prototype.createBuffer = function (d) {
      window.__gpuBuffers++;
      // Every 3D view builds one of these (its sun and atmosphere uniform).
      if (d.label === "physical atmosphere") window.__viewBuffers++;
      try { return create.call(this, d); } catch (e) { log("createBuffer threw", { size: d.size, label: d.label, mapped: d.mappedAtCreation, message: String(e) }); throw e; }
    };
    if (diag) {
      const rd = GPUAdapter.prototype.requestDevice;
      GPUAdapter.prototype.requestDevice = function (...a) {
        return rd.apply(this, a).then((dev) => { dev.lost.then((i) => log("device lost", { reason: i.reason, message: i.message })); return dev; });
      };
      setInterval(() => log("tick"), 2000);
    }
  })();`,
});
const views = async () => (await send("Runtime.evaluate", { expression: "window.__viewBuffers" })).result?.result?.value ?? 0;
await send("Page.navigate", { url });
// The first views are built within the first seconds; the late view comes after this sample.
await sleep(Math.min(4000, Number(waitMs)));
const viewsBefore = await views();
await sleep(Number(waitMs) - Math.min(4000, Number(waitMs)));
const viewsAfter = await views();
const shot = await send("Page.captureScreenshot", { format: "png" });
if (shot.result?.data) writeFileSync(out, Buffer.from(shot.result.data, "base64"));
ws.close();

console.log(`web-smoke: renderer ${renderer ?? "(not reported)"}`);
if (/[?&]late_view=/.test(url) && !lateView) problems.push("the late 3D view was never built");
if (wantsLoss && !deviceLost) problems.push("the GPU device was never lost");
if (!wantsLoss && !lossOk && deviceLost) problems.push("the GPU device was lost");
// A view built on a live WebGPU device must have created its GPU resources.
if (lateView && renderer === "WebGPU" && !deviceLost && !(viewsAfter > viewsBefore)) {
  problems.push(`the late 3D view created no GPU resources (${viewsBefore} views at 4 s, ${viewsAfter} at the end)`);
}
if (expected && renderer !== expected) problems.push(`expected renderer ${expected}, got ${renderer ?? "none"}`);
if (problems.length) {
  for (const p of problems) console.error(`web-smoke: ${p}`);
  process.exit(1);
}
console.log("web-smoke: ok");
process.exit(0);
