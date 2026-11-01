# FlightCraft in the browser

Install the pinned Rust toolchain, including `wasm32-unknown-unknown`, and **wasm-bindgen-cli
0.2.130**. The CLI must match the `wasm-bindgen` crate version in `Cargo.lock` exactly; the build
checks this before compiling. From the repository root:

```sh
cargo install wasm-bindgen-cli --version 0.2.130 --locked
cargo xtask web
python3 -m http.server 8080 --directory dist
```

Open <http://localhost:8080/>. `cargo xtask web` runs the release wasm build and
`wasm-bindgen --target web`, then writes `index.html`, the JavaScript loader, the wasm module, and
the takeoff example into the repository's `dist/`. No Trunk installation is needed. The CLI is
looked up on `PATH`; set `WASM_BINDGEN` to use a specific executable. If using a build queue, run
the entire `cargo xtask web` command through that queue so its nested build shares the same slot
and target directory.

Upload the contents of `dist/` to any static HTTP(S) host. Relative asset URLs also work under a
subdirectory, with no route rewrites, backend, or cross-origin isolation headers. Serve `.wasm`
as `application/wasm`. The simulator runs locally after the page loads. Opening `index.html`
directly as a `file://` URL is unsupported because browsers restrict module and wasm fetching.
CI uploads the same directory as the `flightcraft-web` artifact.

The canvas fills the browser window. Click it to focus keyboard input, and open **Input bindings**
for controls and editable JSON shortcuts. Settings persist in browser storage. Connect a gamepad
and press a button to let the browser expose it. Use localhost or HTTPS for WebGPU; wgpu falls back
to WebGL2 when WebGPU is unavailable.

## Scripted takeoff

Open <http://localhost:8080/?script=takeoff.jsonl> to run the bundled copy of
[`examples/takeoff.jsonl`](../../examples/takeoff.jsonl). It applies full throttle, rotates at nine
simulated seconds, and eases into the climb. Allow about 15 seconds to see the aircraft airborne.

`?script=<url>` fetches a UTF-8 JSON-lines script before starting the simulation. Relative URLs
resolve against the page; an absolute URL on another origin needs that host to allow CORS.
URL-encode script URLs containing `&`, `#`, or other query delimiters. Fetch and parse errors
appear in the loading message and browser console; command errors appear in the app.

The format is shared with the desktop `--script` option: each line has `at_s`, `command`, and
`input`, and blank lines and `#` comments are allowed. Times are simulated seconds; commands at
the same time keep file order. Scripts are limited to 4 MiB and 10,000 commands. Keyboard and
gamepad flight controls are held while commands remain. See
[`crates/ui/src/script.rs`](../../crates/ui/src/script.rs).

## Trunk development

[Trunk](https://trunkrs.dev/) remains supported. From `apps/web`, run `trunk serve --open` or
`trunk build --release`. Its output goes to `apps/web/dist/`, including the takeoff script.

## Headless browser smoke check

Serve `dist/` as above, then use Chrome with software WebGL2:

```sh
google-chrome --headless=new --enable-unsafe-swiftshader --use-angle=swiftshader \
  --enable-logging=stderr --v=0 --screenshot=web.png --window-size=1280,800 \
  --virtual-time-budget=8000 http://localhost:8080/
```

On Flatpak installations, replace `google-chrome` with `flatpak run com.google.Chrome`.
Inspect the screenshot for the scene and instruments, and inspect stderr for app errors. For a
takeoff capture, add `?script=takeoff.jsonl` and allow more time; GPU initialization and headless
animation scheduling can require wall time in addition to a virtual-time budget.
