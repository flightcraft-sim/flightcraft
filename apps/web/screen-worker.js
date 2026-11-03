// One named SharedWorker owns the flight for every same-origin screen.
const moduleUrl = new URL(self.location.href).searchParams.get('module') || './flightcraft_web.js';
// Register onconnect synchronously: the first port can arrive while wasm loads.
let api;
const ready = import(moduleUrl).then(async module => { await module.default(); api = module; });
const ports = new Set();
let started = false;
self.onconnect = ({ports: [port]}) => {
  ports.add(port);
  port.onmessage = async ({data}) => {
    try {
      await ready;
      const message = JSON.parse(data);
      if (message.type === 'start') {
        if (!started) { api.screen_worker_start(JSON.stringify(message.seed), message.script || ''); started = true; }
      } else if (started && message.type === 'command') {
        port.postMessage(api.screen_worker_command(JSON.stringify(message.request)));
      } else if (message.type === 'disconnect') { ports.delete(port); port.close(); }
    } catch (error) { port.postMessage(JSON.stringify({error: String(error)})); }
  };
  port.start();
};
setInterval(() => { if (started) { try { api.screen_worker_tick(); } catch (error) { for (const p of ports) p.postMessage(JSON.stringify({error: String(error)})); } } }, 4);
setInterval(() => {
  if (!started) return;
  const snapshot = api.screen_worker_snapshot();
  for (const port of ports) { try { port.postMessage(snapshot); } catch (_) { ports.delete(port); } }
}, 33);
