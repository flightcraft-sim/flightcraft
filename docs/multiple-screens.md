# Multiple screens

One flight can drive several desktop windows and paired browser screens. Every screen uses the
same page canvas and reads a snapshot. The native session runs on a separate thread at 120 fixed
steps per second. Local browser pages share one named SharedWorker; only that Worker steps the
flight. Closing, covering, or minimizing a presentation does not stop the native clock.

Open **Screens** to add windows, change their pages, enter layout edit mode, grant a paired screen
control, or disconnect it. Window 0 is the main window. There are practical resource limits:
64 native windows and 128 elements per layout. Rendering additional outside views costs more
than rendering instruments; a panel-only window does not draw an outside scene.

Start from a saved setup:

```sh
flightcraft-desktop --screens examples/layouts/home-cockpit-five.json --layouts-dir my-layouts
```

The examples include `single-screen.json`, `three-monitor-wrap.json`, `instructor-setup.json`,
`home-cockpit-five.json`, `c172-two-screens.json`, and `glass-touchscreen.json`. The C172-style
example means a familiar six-pack and engine-control arrangement, using the original fictional
Corvid. It is not a C172 aircraft, flight model, or manufacturer panel. Load `corvid-glass` for
integrated glass behaviour. Panels and touch displays from installed aircraft packs can also be
split into their individual installed pieces.

Portable desktop builds discover `packs/` and `assets/terrain` beside the executable when the
working directory has no corresponding default folder. Explicit directory options take priority.
Every new map/chart canvas inherits the configured terrain root; opening windows never changes
the process working directory or requires a second copy of the packs.

Pages include the shared tower/approach ATC controls, outside views, the aircraft panel or selected instruments, PFD, MFD, navigator,
planning map, regional EFB charts, instructor station, radio/kneeboard, and debrief. Each outside
page has its own forward, left, right, chase, tower, or top-down view, vertical field of view, and
heading offset. Adjacent wrap-around views should use matching FOV and suitable heading offsets;
the example uses −60°, 0°, and +60°. There is no automatic projector edge blending.
The main combined-screen examples set `follow_main_camera` so cockpit-look shortcuts and the
chase-camera command still work; independent wrap-around views leave it disabled.

## Arrange panel pieces

Choose **Screens → a window → Split aircraft panel into pieces**. This creates one layout element
per instrument or installed equipment item. Equipment items include individual touch displays,
switches, knobs, breakers, and engine controls. The engine-control tablet example uses independent
throttle, mixture, carb heat, flaps, trim, and fuel-selector elements.
Legacy panels also split out magnetos, primer, starter, and each installed electrical breaker.

Enable **Edit layouts**. Drag an element to move it; drag its lower-right handle to resize it.
Position snaps to an eight-unit grid, with alignment guides near other elements. Open the element's
context menu to edit position, size, page, equipment binding, camera FOV, and input permission.
**Move to screen…** transfers the whole element to another window, including its control binding.
In edit mode, drag past the source window edge and release over another FlightCraft window.
A target outline follows the global pointer and uses the target canvas calibration. The move is
one undoable `layout.move`. Native viewport outer rectangles locate the windows; configured window
positions provide a fallback. The menu remains available for touch and keyboard access. **Undo layout** and
**Redo** act across windows. A move is atomic: invalid bounds or duplicate element ids leave both
screens unchanged. An empty source canvas is allowed.

Desktop settings retain the window set. With `--layouts-dir`, edits also save atomically to
`screen-<id>.json` in that folder. The app polls the folder, debounces changes for 250 ms, and
handles editors that save by renaming. Invalid JSON or invalid geometry keeps the last good
layout visible and reports the error; it does not overwrite the invalid editor file.
`layout.save` writes an explicit JSON path without a dialog. Browser pages can paste and copy
layout JSON through **Import / export layout**.

`schema/layout.json` describes one canvas; `schema/screens.json` describes a window set. Pixel
canvases fit uniformly inside their window. Millimetre canvases use independent X/Y
`pixels_per_mm`, divided by the viewport's pixel scale; they retain physical size and clip rather
than shrink when the window is too small. Do not trust CSS millimetres. Measure both axes and set
the calibration for the actual monitor. `screen.monitor` defines monitor origin and size in
desktop points and its physical calibration. `screen.configure` assigns that profile to a window.
Origins are explicit because monitor enumeration and positions are not portable through egui,
particularly under Wayland.

```json
{"index":0,"monitor":{"name":"Panel","origin":[1920,0],"size":[1920,1080],"pixels_per_mm":[3.8,3.9]}}
```

## Touch controls

The aircraft panel definition's `touch_controls` declares labels, styles, and normalized detents.
The Corvid's C172-style console has a black throttle plunger, a red mixture plunger with a central
release button, two-position carb heat, a draggable/flickable trim wheel with nose-up/down marks,
0/10/20/30° flap detents, and a rotary LEFT/BOTH/RIGHT/OFF selector. Lift the 48-pixel OFF guard
before selecting OFF. Mixture drags start on its release handle; a six-pixel slop prevents tap jumps.
The pinned Sling 2 uses a fuel-injected Rotax 912 iS, so no choke or manual mixture is presented.
Its engine definition selects a throttle quadrant; selector, guarded lanes, pumps, and other pieces
retain the pack's declared artwork and command bindings.

Levers have large vertical travel areas. **Drag** a lever; tapping it does not jump its value.
Different contact ids own different levers, so two fingers can move throttle and mixture together.
Optional normalized detents and a friction lock are stored in each control element. A local
context-menu friction lock is also available. Touch uses egui's native/browser touch events;
mouse dragging works too. Throttle, mixture, flaps, trim, carb heat, fuel selector, and brake use
the same validated control commands as other input sources. Trim ranges from −0.3 to +0.3 elevator
bias. Carb heat uses the installed model's on/off control. A prop control requires an installed
panel binding; the supplied fixed-pitch aircraft have no variable-pitch propeller lever.

Glass pages have finger-sized MAP, NRST, and FPL keys. Drag the heading, altitude, range, or FMS
knob, or rotate two fingers over it. Tap a knob to push it. The dark glass bezel has bottom
softkeys and concentric FMS rings: the outer ring changes five legs per detent, the inner one a
single leg. The MFD uses the existing regional moving map with terrain and airspace. Touching a knob opens a larger control with single-detent
buttons and a push action. Heading push synchronizes heading; altitude push selects current
altitude. Existing aircraft touch displays retain their touch menus and shared navigator.
The conventional glass FMS knob selects an active flight-plan leg, with bounds at the first and
last leg; pushing it opens the flight-plan page. With an empty plan, turning it opens that page.
Installed touch navigators retain their own radio-tuning knobs and Home action.
There is no claim of manufacturer-specific G1000 or HDX operating fidelity.

## Pair a tablet or another computer

Package the browser app with `cargo xtask web`, then explicitly enable a private LAN listener:

```sh
flightcraft-desktop --panels 192.168.1.20:7891 --web-dir dist
```

LAN hosting is off unless `--panels` is supplied. Bind to the host's actual private address, not
`0.0.0.0`. On the tablet, open `http://192.168.1.20:7891/?panel=remote&page=instructor`.
Enter a screen name and the fresh token shown in the host's **Screens** menu. Each paired screen
starts **read-only**. Grant **Control** beside that screen on the host to enable instructor and
touch controls. Revoking control takes effect on the next command. **Disconnect** closes its
session; it must pair again. Tokens change when the listener restarts and are never put in URLs.

The desktop serves the actual packaged egui browser app. HTTP assets and the WebSocket share an
origin; the public HTTPS site is not a remote-panel host. The server checks the exact browser
Origin, the token, the peer address, command permissions, message size, client count, and rate.
Control permission admits flight/panel/layout commands, not arbitrary host file operations.
HTTP/WebSocket traffic is unencrypted, so use a trusted private LAN. Public addresses, wildcard
binds, arbitrary Origins, and automatic grants are refused.

For more local browser windows, choose **Open another screen**. They share the named Worker and
its flight, rather than creating another simulation. The first page initializes the flight and
script; later pages observe it. The packaged site requires SharedWorker support. Browser process
suspension, device sleep, and OS timer scheduling can still interrupt real-time operation.

## Validation and current limits

`out/screens/` contains reproducible captures of the shared canvas and a timing report when the
ignored `render_screen_previews` GPU test is run. Its five-canvas benchmark includes CPU submit
and GPU completion; it does not measure the OS compositor or five physical monitor swapchains.
Remote pairing/Origin/permission tests exercise an actual local WebSocket connection, and layout
and touch tests cover validation, last-good files, moves, restart, undo, and independent contacts.
The browser check also connects two ports to one Worker and verifies that its clock advances
without presentation repaint callbacks and that a pause reaches both ports.

Fetch the locked region data and prepare the pinned Sling fixture with
`python3 scripts/bundle-packs.py bundled-packs.json out/screens/packs`.
Recreate the native captures with `cargo test -p flightcraft-ui render_screen_previews -- --ignored`.
For the browser capture, put a packaged browser build in `out/screens/web`, set
`FLIGHTCRAFT_CHROME` to a Chromium executable, and run
`cargo test -p flightcraft-ui remote_instructor_browser_preview -- --ignored --nocapture`.
That test creates an ephemeral loopback host, pairs its browser, and grants control only for the test.

Layout rotation and lighting-group metadata are reserved; arbitrary rotated interactive pages,
night-lighting group linkage, cut-template export, and a full alignment /
distribution editor remain unfinished. Native monitor profiles use explicit origins. Navigator
pages expose the existing offline search; EFB pages expose the existing regional map/chart host,
not a separate third-party EFB. The current shared snapshots copy flight presentation data;
large archives and many complex windows need further profiling.
