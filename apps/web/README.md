# FlightCraft in the browser

Install [Trunk](https://trunkrs.dev/) and the pinned Rust toolchain, including
`wasm32-unknown-unknown`. From this directory, run:

```sh
trunk serve --open
```

The canvas fills the browser window. Click it to focus keyboard input, and open **Input bindings**
for controls and editable JSON shortcuts. Settings persist in browser storage. Connect a gamepad
and press a button to let the browser expose it. Use localhost or HTTPS for WebGPU; wgpu falls back
to WebGL2 when WebGPU is unavailable. No cross-origin isolation headers are needed.

`trunk build --release` produces static files in `dist/`; serve that directory on any static host.
The simulator runs locally after the page loads.
