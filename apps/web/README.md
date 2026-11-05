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

For an unoptimized local measurement build, use `cargo xtask web --debug`. Its output also
goes to `dist/`; record the build profile alongside any timing results.

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

## Choosing the GPU path

The page uses WebGPU where the browser has it and WebGL 2 otherwise. Add `?gpu=webgl2` to force
WebGL 2 (a fallback if WebGPU misbehaves on a machine), or `?gpu=webgpu` to allow WebGPU only. The
status bar shows which one is drawing. `scripts/web-smoke.sh` loads the page on WebGPU and on WebGL 2
and fails on any shader or GPU error; CI runs it before publishing. Each pass builds a new 3D view
mid-session (`?late_view=<seconds>`), and a second WebGPU pass destroys the GPU device first
(`?lose_device=<seconds>`): when a browser loses the device (a GPU reset), the 3D views stop, a banner
offers a reload on WebGL 2, and nothing panics. The WebGPU passes need a headed Chrome on a virtual
display (`xvfb-run`): headless Chrome cannot present a WebGPU canvas and loses the device at start.

## Aircraft packs

Aircraft packs can be loaded from static hosts by repeating the `pack` query parameter with
manifest URLs, for example `?pack=packs/demo-pack/pack.json`. Relative file paths resolve against
the manifest URL; cross-origin hosts need CORS. Bad packs are reported and skipped. See
[aircraft format and packs](../../docs/aircraft-format.md) for limits and the example manifest.

## Geographic regions

Open [New York area](https://flightcraft-sim.github.io/flightcraft/?region=terrain/new-york/region.json)
to start on Blairstown runway 07, then use **Start** to search airports and choose another runway.
Also fly [Eastern Pennsylvania and New Jersey](https://flightcraft-sim.github.io/flightcraft/?region=terrain/eastern-pa/region.json), with the same Blairstown origin.
The static build includes both region directories, with public-domain elevation and FAA airports.
The same query works on a local static host.

### Fly NYC (the default)

With no `?region=`, the site opens **Fly NYC**: the New York region, southbound at about 1,000 ft
over the Hudson River at the George Washington Bridge, with the Manhattan skyline built from public
USGS 3DEP lidar, NAIP imagery and NHD water (`terrain/new-york/scenery/hudson-corridor/`, see
[docs/scenery.md](../../docs/scenery.md#hudson-corridor-fly-nyc)). Query options:

- `?start=kteb` departs Teterboro runway 06 instead (about 9 km west of the river, outside the
  lidar building envelope, so the skyline appears only after the climb-out).
- `?start=runway` starts on the region's Blairstown runway like the other regions.
- `?region=none` opens the original flat Training Valley world.
- `?region=terrain/eastern-pa/region.json` (repeatable) selects any other region, as before.
