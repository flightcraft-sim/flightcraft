#!/usr/bin/env python3
"""Headless egui screen capture using Chromium's DevTools protocol and only the standard library.

The ignored Rust preview test supplies a private, short-lived host and pairing data on stdin.
No token is put in a URL, an argument, or a saved report.
"""
import base64
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import tempfile
import time
import urllib.request


class DevTools:
    def __init__(self, url):
        from urllib.parse import urlsplit
        parts = urlsplit(url)
        self.socket = socket.create_connection((parts.hostname, parts.port), timeout=30)
        key = base64.b64encode(os.urandom(16)).decode()
        self.socket.sendall((f"GET {parts.path} HTTP/1.1\r\nHost: {parts.netloc}\r\n"
                             f"Upgrade: websocket\r\nConnection: Upgrade\r\n"
                             f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n").encode())
        header = b""
        while not header.endswith(b"\r\n\r\n"):
            header += self.exact(1)
        if not header.startswith(b"HTTP/1.1 101"):
            raise RuntimeError("DevTools handshake failed")
        self.next = 0
        self.errors = []
        self.console = []

    def exact(self, count):
        result = bytearray()
        while len(result) < count:
            chunk = self.socket.recv(count - len(result))
            if not chunk:
                raise RuntimeError("DevTools connection closed")
            result.extend(chunk)
        return bytes(result)

    def send(self, value, opcode=1):
        payload = value.encode() if isinstance(value, str) else value
        size = len(payload)
        header = bytes([0x80 | opcode])
        if size < 126:
            header += bytes([0x80 | size])
        elif size < 65536:
            header += bytes([0x80 | 126]) + struct.pack("!H", size)
        else:
            header += bytes([0x80 | 127]) + struct.pack("!Q", size)
        mask = os.urandom(4)
        payload = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        self.socket.sendall(header + mask + payload)

    def receive(self):
        result = bytearray()
        while True:
            a, b = self.exact(2)
            size = b & 127
            if size == 126:
                size = struct.unpack("!H", self.exact(2))[0]
            elif size == 127:
                size = struct.unpack("!Q", self.exact(8))[0]
            if size > 32 * 1024 * 1024:
                raise RuntimeError("DevTools frame too large")
            mask = self.exact(4) if b & 128 else None
            payload = self.exact(size)
            if mask:
                payload = bytes(v ^ mask[i % 4] for i, v in enumerate(payload))
            if a & 15 == 9:
                self.send(payload, 10)
                continue
            if a & 15 == 8:
                raise RuntimeError("DevTools socket closed")
            result.extend(payload)
            if a & 128:
                return json.loads(result)

    def call(self, method, params=None):
        self.next += 1
        identity = self.next
        self.send(json.dumps(dict(id=identity, method=method, params=params or {})))
        while True:
            response = self.receive()
            if response.get("method") == "Runtime.exceptionThrown":
                self.errors.append(response["params"])
            if response.get("method") == "Runtime.consoleAPICalled":
                self.console.append(response["params"])
            if response.get("id") == identity:
                if "error" in response:
                    raise RuntimeError(str(response["error"]))
                return response.get("result", {})

    def evaluate(self, expression):
        result = self.call("Runtime.evaluate", dict(expression=expression, returnByValue=True,
                                                   awaitPromise=True, userGesture=True))
        if "exceptionDetails" in result:
            raise RuntimeError(str(result["exceptionDetails"]))
        return result.get("result", {}).get("value")

    def click(self, x, y):
        self.call("Input.dispatchMouseEvent", dict(type="mouseMoved", x=x, y=y))
        for kind in ["mousePressed", "mouseReleased"]:
            self.call("Input.dispatchMouseEvent", dict(type=kind, x=x, y=y, button="left", clickCount=1))
        time.sleep(0.2)

    def screenshot(self, path):
        shot = self.call("Page.captureScreenshot", dict(format="png"))
        Path(path).write_bytes(base64.b64decode(shot["data"]))


def run(config):
    output = Path(config["out"])
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="flightcraft-screen-browser-") as profile:
        with open(output / "browser.log", "w") as log:
            browser = subprocess.Popen([config["chrome"], "--headless", "--no-sandbox",
                "--no-first-run", "--no-default-browser-check", "--use-angle=swiftshader",
                "--enable-unsafe-swiftshader", "--remote-debugging-port=0",
                "--user-data-dir=" + profile, "about:blank"], stdout=log, stderr=log)
            try:
                port_file = Path(profile) / "DevToolsActivePort"
                deadline = time.monotonic() + 20
                while not port_file.exists():
                    if time.monotonic() > deadline:
                        raise RuntimeError("Chromium did not expose DevTools")
                    time.sleep(0.1)
                port = port_file.read_text().splitlines()[0]
                targets = json.load(urllib.request.urlopen(f"http://127.0.0.1:{port}/json"))
                page = next(t for t in targets if t["type"] == "page")
                cdp = DevTools(page["webSocketDebuggerUrl"])
                cdp.call("Runtime.enable")
                cdp.call("Page.enable")
                cdp.call("Page.addScriptToEvaluateOnNewDocument", dict(source="""
                    window.__screenTransport = [];
                    const original = WebSocket.prototype.send;
                    WebSocket.prototype.send = function(data) {
                        try { const value=JSON.parse(data); if('token' in value) window.__screenTransport.push({hello_token_length:value.token.length}); } catch(_) {}
                        return original.call(this,data);
                    };
                """))
                cdp.call("Emulation.setDeviceMetricsOverride", dict(width=1280, height=900,
                    deviceScaleFactor=1, mobile=False))
                cdp.call("Page.navigate", dict(url=config["url"]))
                deadline = time.monotonic() + 60
                while cdp.evaluate("document.getElementById('loading') !== null"):
                    if time.monotonic() > deadline:
                        cdp.screenshot(output / "remote-pairing.png")
                        (output / "browser-console.json").write_text(json.dumps(cdp.console, indent=2))
                        raise RuntimeError("Browser app did not finish starting")
                    time.sleep(1)
                time.sleep(2)
                cdp.screenshot(output / "remote-pairing.png")
                # Eguis's first row is the toolbar, followed by instructions and pairing fields.
                # Tab through the two text fields to avoid depending on their horizontal widths.
                cdp.click(550, 72)
                time.sleep(0.5)
                cdp.call("Input.insertText", dict(text=config["token"]))
                time.sleep(0.5)
                cdp.click(760, 76)
                time.sleep(5)
                cdp.screenshot(output / "remote-instructor.png")
                (output / "transport-report.json").write_text(json.dumps(cdp.evaluate("window.__screenTransport"), indent=2))
                # Connect two additional ports to the local screen's named owner, then stop
                # presentation RAF callbacks while checking that its fixed-step clock continues.
                from urllib.parse import urlsplit
                address = urlsplit(config["url"])
                cdp.call("Page.navigate", dict(url=f"{address.scheme}://{address.netloc}/?page=outside&gpu=webgl2&region=assets/terrain/new-york/region.json&script=preview.jsonl"))
                deadline = time.monotonic() + 60
                while cdp.evaluate("document.getElementById('loading') !== null"):
                    if time.monotonic() > deadline:
                        raise RuntimeError("Local Worker page did not finish starting")
                    time.sleep(1)
                time.sleep(2)
                cdp.screenshot(output / "local-worker.png")
                worker = cdp.evaluate("""(async () => {
                    const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
                    const snapshots = [null, null];
                    const errors = [];
                    const workers = [0, 1].map(index => {
                        const worker = new SharedWorker('./screen-worker.js', {type:'module', name:'flightcraft.session.v1'});
                        worker.port.onmessage = event => { const value = JSON.parse(event.data); if(value.snapshot) snapshots[index] = value.snapshot; if(value.error) errors.push(value.error); };
                        worker.port.start();
                        return worker;
                    });
                    for(let i=0; i<150 && snapshots.some(value => !value); ++i) await sleep(100);
                    if(snapshots.some(value => !value)) throw Error('SharedWorker did not publish to both ports');
                    if(snapshots.some(value => value.world !== 'new-york')) throw Error('Worker did not load the New York preview region');
                    window.requestAnimationFrame = () => 0;
                    const before = snapshots[0].state.time_s;
                    await sleep(1000);
                    const advanced = snapshots[0].state.time_s - before;
                    if(advanced < 0.5) throw Error('Worker clock diagnostic: '+JSON.stringify({advanced, before, after:snapshots[0].state.time_s, paused:snapshots[0].paused, rate:snapshots[0].rate, errors}));
                    workers[0].port.postMessage(JSON.stringify({type:'command', request:{id:777, command:'sim.pause', input:{}}}));
                    await sleep(300);
                    if(snapshots.some(value => !value.paused)) throw Error('Pause did not reach both ports');
                    const time = snapshots[0].state.time_s;
                    await sleep(300);
                    if(snapshots.some(value => value.state.time_s !== time)) throw Error('Paused ports disagree');
                    for(const worker of workers) worker.port.postMessage(JSON.stringify({type:'disconnect'}));
                    return {ports:2, world:snapshots[0].world, advanced_without_repaints_s:advanced, shared_pause_time_s:time};
                })()""")
                cdp.screenshot(output / "local-worker.png")
                if cdp.errors:
                    raise RuntimeError("Browser exceptions: " + json.dumps(cdp.errors))
                (output / "browser-report.json").write_text(json.dumps(dict(worker=worker,
                    viewport=[1280, 900], renderer="software WebGL 2", exceptions=cdp.errors,
                    method="Browser build on a private test host; Rust test separately verifies pairing"), indent=2))
            finally:
                browser.terminate()
                try:
                    browser.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    browser.kill()
                    browser.wait()


if __name__ == "__main__":
    run(json.load(sys.stdin))
