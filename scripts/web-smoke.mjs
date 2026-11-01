// Load the web build in a running headless Chrome over the DevTools protocol and fail on any GPU
// validation error, shader error, panic, or uncaught exception.
//
//   node scripts/web-smoke.mjs <devtools-port> <url> <wait-ms> <screenshot.png> [expected-renderer]
//
// With expected-renderer (for example "WebGPU"), the app's "FlightCraft renderer: ..." console line
// must name it, so a silent fallback to WebGL 2 fails the check instead of hiding a WebGPU bug.
import { writeFileSync } from "node:fs";

const [port, url, waitMs, out, expected] = process.argv.slice(2);
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const problems = [];
let renderer = null;

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
await send("Page.navigate", { url });
await sleep(Number(waitMs));
const shot = await send("Page.captureScreenshot", { format: "png" });
if (shot.result?.data) writeFileSync(out, Buffer.from(shot.result.data, "base64"));
ws.close();

console.log(`web-smoke: renderer ${renderer ?? "(not reported)"}`);
if (expected && renderer !== expected) problems.push(`expected renderer ${expected}, got ${renderer ?? "none"}`);
if (problems.length) {
  for (const p of problems) console.error(`web-smoke: ${p}`);
  process.exit(1);
}
console.log("web-smoke: ok");
process.exit(0);
