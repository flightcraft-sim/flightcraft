// Measure actual browser WebGL 2 callback costs and presented frame intervals over CDP.
// Serve a debug or release dist, start Chrome with a debugging port, then run:
// node scripts/weather-bench.mjs <port> <origin> <output-directory> <profile>
import { mkdirSync, writeFileSync } from 'node:fs';
const [port, origin, output, profile = 'debug'] = process.argv.slice(2);
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
mkdirSync(output, { recursive: true });
let target;
for (let i = 0; i < 100 && !target; i++) {
  try { target = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find(t => t.type === 'page'); } catch {}
  if (!target) await sleep(200);
}
if (!target) throw new Error('No browser page target');
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise(resolve => ws.addEventListener('open', resolve));
let sequence = 0;
const pending = new Map();
const problems = [];
let renderer;
ws.addEventListener('message', e => {
  const m = JSON.parse(e.data);
  if (m.id) { pending.get(m.id)?.(m); pending.delete(m.id); }
  if (m.method === 'Runtime.exceptionThrown') problems.push(JSON.stringify(m.params));
  if (m.method === 'Runtime.consoleAPICalled') {
    const text = m.params.args.map(a => a.value ?? a.description ?? '').join(' ');
    if (/FlightCraft renderer:/.test(text)) renderer = text;
    if (m.params.type === 'error' || /panicked|ShaderModule|validation error/i.test(text)) problems.push(text);
  }
});
const send = (method, params = {}) => new Promise(resolve => {
  const id = ++sequence; pending.set(id, resolve); ws.send(JSON.stringify({ id, method, params }));
});
await send('Page.enable');
await send('Runtime.enable');
await send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 800, deviceScaleFactor: 1, mobile: false });
await send('Page.addScriptToEvaluateOnNewDocument', { source: `
  globalThis.weatherCosts = []; globalThis.weatherIntervals = []; globalThis.weatherMeasure = false;
  let gl; const get = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = function(...args) { const result = get.apply(this,args); if (args[0] === 'webgl2') gl = result; return result; };
  const raf = requestAnimationFrame.bind(globalThis); let last;
  globalThis.requestAnimationFrame = callback => raf(time => {
    const start = performance.now(); callback(time);
    if (globalThis.weatherMeasure) {
      if (gl) gl.finish();
      globalThis.weatherCosts.push(performance.now()-start);
      if (last !== undefined) globalThis.weatherIntervals.push(time-last);
      last = time;
    }
  });` });
const rows = ['profile,renderer,scene,mean_cost_ms,p95_cost_ms,mean_interval_ms,p95_interval_ms,presented_fps'];
for (const preset of ['vfr','ifr','night']) {
  const script = [
    { at_s: 0, command: 'sim.reset', input: { start: { kind: 'air', height_m: 100, airspeed_mps: 40, heading_rad: 0 }, paused: true } },
    { at_s: 0, command: 'weather.set', input: { preset } },
  ].map(v => JSON.stringify(v)).join('\n');
  writeFileSync(`${output}/browser-${preset}.jsonl`,script);
  renderer = undefined;
  const url = `${origin}/?gpu=webgl2&script=${encodeURIComponent(`/weather/browser-${preset}.jsonl`)}`;
  await send('Page.navigate',{ url });
  for (let i=0;i<120&&!renderer;i++) await sleep(500);
  if (!renderer) throw new Error(`App did not start for ${preset}: ${problems.join('; ')}`);
  await sleep(3000);
  await send('Runtime.evaluate',{ expression: 'weatherMeasure = true' });
  await sleep(8000);
  const result = await send('Runtime.evaluate',{ expression: 'weatherMeasure = false; ({costs:weatherCosts,intervals:weatherIntervals})', returnByValue: true });
  const data = result.result?.result?.value;
  if (!data || data.costs.length < 30) throw new Error(`Insufficient frames: ${JSON.stringify(result)}`);
  const stats = values => { const sorted = [...values].sort((a,b)=>a-b); return [values.reduce((a,b)=>a+b,0)/values.length,sorted[Math.floor(sorted.length*.95)]]; };
  const [cost,p95cost]=stats(data.costs),[interval,p95interval]=stats(data.intervals);
  rows.push([profile,renderer.replace('FlightCraft renderer: ',''),preset,cost.toFixed(3),p95cost.toFixed(3),interval.toFixed(3),p95interval.toFixed(3),(1000/interval).toFixed(1)].join(','));
  const shot = await send('Page.captureScreenshot',{ format:'png' });
  if (shot.result?.data) writeFileSync(`${output}/browser-${preset}.png`,Buffer.from(shot.result.data,'base64'));
}
ws.close();
writeFileSync(`${output}/frame-cost-browser.csv`,rows.join('\n')+'\n');
writeFileSync(`${output}/browser-errors.json`,JSON.stringify(problems,null,2));
console.log(rows.join('\n'));
if (problems.length) throw new Error(problems.join('\n'));
