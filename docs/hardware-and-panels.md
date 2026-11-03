# External hardware, multiple screens, and home-built panels

Status: hardware and physical-panel specification; screens and layout runtime are implemented as described in section 4.

FlightCraft should drive any physical cockpit. That covers a USB yoke and throttle quadrant
plugged in today, and a hobbyist's panel built from microcontroller boards and switches. It also
covers a second monitor full of gauges, and a CNC-cut wooden dashboard with a screen behind it.
Each of these should work without changing the simulator, and anyone should be able to write a
driver in any language.

## 1. Principles

1. **One way in, one way out.** Hardware changes the simulation only through the existing command
   layer, and it reads the simulation only through a published catalogue of variables. Drivers get
   no private path. The same commands and variables serve the MCP server, scripts, the UI, and
   hardware.
2. **Drivers live outside the simulator.** A driver is any program that speaks the device protocol
   (section 3). It can be written in Rust, C++ for Arduino, MicroPython, Python on a Raspberry Pi,
   Node, or anything else. A crashing or misbehaving driver cannot crash the simulator: its input
   is bounded, validated, and dropped when invalid.
3. **Built-in support covers common devices.** USB joysticks, yokes, rudder pedals, and throttle
   quadrants work through built-in HID support, with no extra program to run. Device profiles are
   data, not code.
4. **Everything is a file and a command.** Bindings, device profiles, and screen layouts are JSON
   files with published schemas. Every UI edit is a command, so a layout made by dragging and a
   layout written by hand are the same thing.
5. **Physical units for physical panels.** A layout can be specified in millimetres on a
   calibrated monitor. The app can also export the cut-out template for the board that sits in
   front of the screen.
6. **Determinism holds.** Device input enters as timestamped controls and commands at fixed-step
   boundaries, exactly like keyboard input. A recorded flight replays identically without the
   hardware.

## 2. Built-in HID devices (yokes, quadrants, pedals, panels)

- **Input:** every axis, button, hat, and encoder of any USB HID game controller.
  - Native support goes beyond gamepads: the current gamepad path drops axes and buttons that a
    standard gamepad lacks, and the HID path will not.
  - The browser gets the same through the Gamepad API, plus WebHID where the browser allows it.
- **Output:** HID output reports for devices with lights and displays, for example landing-gear
  lights and annunciator panels on throttle quadrants. Outputs map to variables in the same
  profile.
- **Device profile** (`schema/device-profile.json`):
  - Matching: USB vendor and product id, plus optional name matching.
  - Failsafe per control kind, stated in the profile: levers hold their last value, sprung axes
    (yoke, stick, pedals) return to centre, and momentary buttons release.
  - Inputs: each raw input gets a name, such as "yoke roll", "lever 3", or "switch: master battery".
  - Calibration defaults: range, centre, dead zone, curve, and detents, such as a reverse or idle
    detent on a lever.
  - Lights: each output report's light bits, with their default variable bindings.
- **Where profiles live:** in packs, so the community adds devices without touching the engine.
  The engine ships generic profiles only: "any joystick" and "any gamepad".
- **Bindings** (`schema/bindings.json`) map a device input to one of three things:
  - an **action** (pitch, roll, throttle 1, flaps up, toggle master);
  - a **command** with fixed arguments, such as `systems.set` with the magnetos at both;
  - an **axis-to-command** mapping with scaling, such as a lever to mixture 0–1.

  A binding carries a dead zone, a response curve, a reversal, and a detent-to-action. Lever
  positions are absolute, while a hat or encoder steps by an amount. Bindings are per aircraft,
  with a shared fallback, so one lever can be mixture in one aircraft and propeller in another.
- **Bind by moving:** the Input bindings screen shows every connected device's live inputs.
  - "Press or move the control" captures it.
  - Conflicts are shown, and calibration happens in the same screen.
  - Bind-by-moving is a UI feature only. Every change goes through `input.bind`, `input.unbind`,
    and `input.calibrate`, which take the device and input explicitly and never open a dialog, so
    agents and scripts can set up hardware too.
- **Telling identical devices apart:** two identical devices share a vendor and product id.
  Bindings key on the USB serial number when there is one, then on the port path. The bindings
  screen shows which physical unit is which (wiggle a control to highlight it), so two identical
  throttle units never swap.
- **Platform notes, handled in phase 1:**
  - Linux reads raw HID through hidraw, which needs a udev rule; the app detects a missing rule
    and shows the exact rule to install.
  - macOS needs the Input Monitoring permission for some devices; the app explains how to grant it.
  - A device seen by both the gamepad path and the HID path is claimed by one of them, never
    reported twice.
  - WebHID and Web Serial exist only in Chromium-based browsers, only on HTTPS or localhost, and
    only after a user gesture (a "Connect device" button). Other browsers fall back to the Gamepad
    API.

## 3. Device protocol for custom hardware (microcontroller boards, a Raspberry Pi, or anything else)

### Transports, all optional and all speaking the same messages

| Transport | For | Notes |
|---|---|---|
| WebSocket on localhost (`ws://127.0.0.1:<port>/devices`) | programs on the same computer | default; token required (below) |
| WebSocket on the LAN | Wi-Fi boards and remote panels | off by default; pairing code required |
| USB serial (CDC) | wired microcontroller boards | the desktop app opens ports the user selects; the browser uses Web Serial |

### Messages

JSON lines for every transport. Serial can also use a compact text form that a small Arduino can
produce with `Serial.print`.

1. **Hello:** the device declares an id, a name, a protocol version, and its controls.
   - **Inputs:** axes (range), buttons, switches (positions), encoders, and analogue values.
   - **Outputs:** lights, segment displays, character LCDs, servo gauges (angle range), steppers,
     PWM, and small pixel displays (resolution).
   - Hello may suggest a default binding set. It is shown to the user as a proposal and applies
     only when they accept it; a device naming itself never binds controls or gains commands.
   - Everything in hello is untrusted. Names and labels are length-limited and shown as plain
     text, never interpreted.
2. **Input events:**
   - Values: `{"in":"lever3","v":0.53}`, or `in lever3 0.53` in the compact form.
   - Edges: `in mags start`.
   - Encoder steps: `in hdg_knob +2`.
   - The simulator turns these into controls and commands through the user's bindings, never
     directly.
3. **Subscriptions:** the device asks for variables at a rate or on change:
   `{"sub":"systems.electrical.main_bus_v","hz":10}` or `{"sub":"annunciator.low_volts","on":"change"}`.
   The simulator pushes `{"var":"...","v":...}`.
4. **Output bindings in the user's file** can drive outputs without the device knowing any
   variable names. For example: light `gear_left_green` ← `gear.left.down_and_locked`; a servo
   gauge ← airspeed in knots with a calibration table; a 7-segment display ← the COM frequency.
   "Dumb" boards that only report inputs and accept output values therefore need no flight
   knowledge in their firmware.
5. **Commands:** a driver may also send control commands (the same JSON as the MCP server), but
   only after the user grants that device command access. Commands are off by default for every
   device.
6. **Heartbeat for moving outputs:** a device with any moving output (servo or stepper drives
   beyond a small gauge needle, motors, motion, or force feedback) must declare a heartbeat
   timeout in hello. The simulator sends a heartbeat at least twice per timeout, and the firmware
   zeroes its moving outputs itself when a heartbeat is missed. The host cannot promise a stop if
   it hangs or the cable is pulled, so the device must. The reference firmware implements this,
   and the simulator refuses to enable a moving output that declares no heartbeat.

### Timing

- **Axes and values** are latest-wins within a fixed step.
- **Edges never coalesce:** a press and a release inside one step both arrive, in order, on
  consecutive steps if necessary.
- **Outputs** are latest-wins per subscription. A slow consumer gets fewer, fresher updates;
  nothing queues up behind it.
- **Recording** happens after bindings are applied, at the step boundary, stamped on the
  simulation thread. A replay needs neither the hardware nor the binding file.

### The variable catalogue (`schema/variables.json`)

Every readable value has a stable dotted name, a type, a unit, a range, and a description. Each
quantity has exactly one unit, and the unit is part of the name (`air_data.indicated_airspeed_kt`,
`electrical.main_bus_v`), because a rename is the expensive mistake. Each new command and variable
has input and output schemas from the start. This
covers flight state, air data, engine, electrical, fuel, annunciators, avionics display values,
radios, and autopilot modes. The catalogue is generated from code, checked for freshness in CI
like the other schemas, and drives subscriptions, output bindings, and the docs. Names never
change once published; a renamed variable is a new name plus a deprecated alias.

### Security

- **Localhost is not trusted by itself.** Any web page in the user's browser can try to open a
  WebSocket to 127.0.0.1. The device server checks the `Origin` header and refuses browser origins
  other than the app's own, and every connection must present a per-session token. The app shows
  the token, writes it to a file only the user can read, and passes it to drivers it starts.
- **Per-device grants:** a new device is listed and can send input only after the user allows it,
  and command access is a separate grant (section 3, message 5). Grants are remembered per device
  identity and can be revoked.
- **Serial ports are physical.** Opening a port can reset a board through DTR, and the port may
  belong to a 3D printer or CNC controller. The app never probes or writes to a port the user has
  not chosen. After opening a port, it reads the device's hello before sending anything, and it
  closes the port when no valid hello arrives. Managed panel computers follow the same rule for
  their local ports.

### Reference drivers and firmware (Apache-2.0, in an `examples/hardware/` folder)

The Arduino reference sketches use the Arduino core, which is LGPL. They link to it as any sketch
does, copy nothing from it, and their README says so. Everything in the folder is Apache-2.0,
like the rest of the repository.


- **Arduino library and sketches** (C++):
  - buttons, switches, and an encoder;
  - an annunciator panel of LEDs;
  - a servo-driven airspeed gauge;
  - a 7-segment radio frequency display.
- **ESP32 sketch over Wi-Fi WebSocket.**
- **Raspberry Pi Python driver** using GPIO, I2C expanders, and an OLED.
- **Rust driver crate** (`flightcraft-device`), which others can reuse for native drivers.
- **Protocol test harness:** a CLI that plays a device against a running simulator and prints
  what it sees. A driver author can test without flying.

### Safety and robustness

- Limits apply to every connection: message size, message rate, number of controls, and
  number of subscriptions.
- A malformed message is dropped with a counter in the device panel. It never causes an error
  dialog storm or a crash.
- A device that disconnects releases its axes to neutral or holds them, per binding, so a
  pulled cable cannot slam the throttle.
- LAN connections are off by default. Turning them on needs a one-time pairing code shown on
  screen.

## 4. Screens, windows, and layouts

### Implemented screens and layout runtime

See the [Multiple screens guide](multiple-screens.md) for setup, touch controls, pairing, command
examples, starter layouts, and current limits. Layout and window-set contracts are generated as
`schema/layout.json` and `schema/screens.json`.

- The native session has one fixed-step owner thread, independent of repaint. Windows read
  presentation copies and route validated commands to that owner. The existing MCP bridge is
  serviced there too; minimizing a window does not stop waits or commands.
- Local browser screens use one named SharedWorker. The Worker owns the session, scripts, and
  fixed-step accumulator. UI frames never step the simulation. Same-origin browser screens share
  that Worker; paired LAN screens connect to the desktop owner instead.
- Native egui viewports have independent layouts, cameras, positions, sizes, fullscreen, and
  borderless settings. Saved window sets restore through app storage or `--screens`.
  `screen.open`, `screen.close`, `screen.list`, `screen.set_page`, `screen.configure`, and
  `screen.monitor` have input and output schemas and never open dialogs.
- The shared page canvas hosts outside views, panels and individual pieces, PFD/MFD, navigator,
  map, regional charts, instructor, radio/kneeboard, and debrief. Only visible pages render.
  Outside pages have independent FOV and heading offset, with forward, left, right, chase, tower,
  and top-down cameras. Each scene owns its GPU target and uniform buffer.
- **Split aircraft panel into pieces** creates independently movable instruments and equipment
  installations, including switches, knobs, breakers, touch displays, and engine controls.
  Edit mode supports dragging, resize handles, grid snapping, alignment guides, context-menu
  properties, direct dragging across windows with a target drop outline, a touch-friendly
  **Move to screen** menu, and undo/redo. Global pointer and viewport rectangles locate the drop;
  explicit desktop origins provide a fallback where the compositor hides window positions.
  Aircraft definitions select the touch control styles and detents. Levers capture contacts
  separately with drag slop, and conventional glass pages add bottom softkeys, rotary knobs,
  concentric FMS controls, and magnified knobs. ATC is available as a separate page or element.
- `layout.set`, `layout.add`, `layout.update`, `layout.remove`, `layout.move`, `layout.undo`,
  `layout.redo`, and `layout.save` validate edits. Transfers convert between calibrated pixel and
  millimetre units. `--layouts-dir` watches the folder with debounce, supports rename saves, and
  preserves last-good layouts without destroying invalid editor files. Browser pages import and
  export layout JSON.
- Starter setups live in `examples/layouts/`: single screen, three outside monitors plus panel,
  instructor and map, five-screen home cockpit, a C172-style six-pack plus controls tablet using
  the fictional Corvid, and a glass touchscreen.
- LAN hosting requires `--panels <private-address:port> --web-dir <packaged-site>`. It serves the
  browser app and same-origin WebSocket, checks Origin and a random host-displayed pairing token,
  and starts every paired screen read-only. The host grants or revokes control per screen and can
  disconnect it from **Screens**. Message, rate, and connection bounds apply. Remote control
  cannot invoke host file commands. The localhost MCP line transport retains its separate policy.

The physical-panel workflow below remains a specification. Rotation/lighting metadata, complete align/distribute tools, cut-template export, and portable
monitor autodetection are unfinished. Explicit monitor profiles carry measured X/Y calibration
and desktop origins. Browser suspension and OS sleep are outside the fixed-step scheduler's
real-time guarantee; deterministic explicit stepping remains available.

### Physical panels (a CNC-cut board in front of a screen)

1. **Calibrate the screen.**
   - Reported monitor sizes are often wrong and break under fractional scaling, so every monitor
     is calibrated: the app shows horizontal and vertical rulers that you match to a credit card
     or a measured length, and stores X and Y scale separately for that monitor.
   - The browser's CSS millimetre is not a real millimetre, so the same calibration applies there.
   - After that, millimetre layouts are true to size.
2. **Size instruments to real panel standards.**
   - Round instruments default to the common 3⅛-inch and 2¼-inch openings.
   - Faces are drawn so the visible dial fits the opening.
3. **Allow for parallax.** The board's thickness and the viewing distance shift what shows
   through an opening, differently for each eye, so a nudge cannot fix it. Openings are cut
   slightly larger than the drawn face by a stated tolerance computed from board thickness and a
   nominal eye distance, both set in the layout.
4. **Export the cut file.** "Export cut template" writes SVG and DXF at 1:1, ready for a CNC
   router or laser. It includes:
   - every opening;
   - mounting-hole positions for physical switches and encoders placed on the layout as
     "hardware" elements;
   - label engraving paths;
   - the screen outline and alignment marks.

   File details that matter to CAM and laser software:
   - **DXF:** millimetre units in the header (`$INSUNITS` 4); real `CIRCLE` and closed
     `LWPOLYLINE` entities, never splines or chains of tiny segments; layers named `CUT`, `HOLE`,
     `ENGRAVE`, `SCREEN`, and `ALIGN`. Rectangular openings honour a declared minimum inside-corner
     radius for router bits. Kerf compensation is left to the CAM software.
   - **SVG:** `width` and `height` in millimetres with a `viewBox` in millimetres, because laser
     software misreads pixels.
   - **Engraved text** is exported as outlines from an openly licensed font.
5. **Open it in SolidCraft (optional).** The layered DXF opens in SolidCraft today through its
   Insert DXF. For a parametric board, the app can also export a SolidCraft command script that
   draws the board with named parameters (size, thickness, and each opening), to run with
   `solidcraft-cli run`. That turns the panel into a part for a printed bezel, edge profiles, or
   CAM. The script is the hand-off, not a saved design file, because commands are SolidCraft's
   stable surface. Its exact shape is agreed with SolidCraft's maintainers before release.
6. **Align the board to the screen.**
   - An alignment screen draws crosshairs at every opening's centre and outlines each cut-out.
   - Per-element nudges fix small cutting errors without moving anything else.
7. **Keep the panel tidy.**
   - Everything outside an opening can be masked black, to avoid light leaks.
   - Instrument lighting dims with a panel-lights variable, so the board can be lit at night.

## 5. Driver tiers and the shared device core

Hardware support comes in three tiers. The cheapest tier is tried first, and every tier presents
the same internal device interface to the simulator: named inputs, named outputs, declared limits,
and a connection state.

1. **Declarative device manifests (data only, no program).**
   - **USB devices:** a HID device profile (section 2) is one kind of manifest.
   - **Serial boards:** a serial manifest is the other. It gives a port match (USB vendor and
     product id, or a name), the baud rate, line patterns that map to inputs (for example
     `B(\d+) ([01])` to "button N"), and output formats (for example `L{index} {0|1}`).
   - **Result:** a board running a ten-line sketch that prints its switch states works with no
     driver program at all.
   - **Where they come from:** manifests install from packs or from a file at runtime, and are
     validated against their schema before use.
2. **Device protocol (section 3).** For boards and programs that want subscriptions, displays, or
   several kinds of control: any language, any transport listed above.
3. **Built-in compiled drivers.** These cover only what data cannot express, such as a protocol
   with binary framing or vendor-specific output reports. They also serve as reference
   implementations of the internal interface.

### Device families

Device families sit on one shared device core. Each family is a data-only description of the
inputs and outputs its devices usually have, plus its default bindings:

- flight controls (yoke, stick, pedals, toe brakes);
- throttle quadrants;
- switch and annunciator panels;
- radio and autopilot panels;
- instrument outputs (servo and stepper gauges, segment and character displays);
- screens and remote panels;
- motion and force outputs.

The shared core owns everything families have in common:

- connection handling and discovery;
- bounded queues and per-device fault isolation;
- the binding engine and the variable catalogue;
- permissions.

Families never depend on each other. A new family adds no code to the core, and the simulator
works with any subset of families present.

### Capabilities are declared limits

A device declares what it can do, not what it currently holds. For example:

- an axis declares its raw range and resolution;
- a display declares its digits or its pixel size;
- a servo gauge declares its angular range and speed.

The binding screen offers only bindings that fit those limits. For example, a 4-digit display is
never offered a 6-character frequency without a declared format.

### One owner per control and per output

Each action or command has at most one active hardware binding. Each output has exactly one
source variable. The binding loader refuses conflicts between hardware devices and names both
claimants, instead of letting two devices fight over the throttle or one light.

The keyboard is not a competing owner. It may always nudge a bound control (for example trim or
throttle from keys). The rule is defined per control kind: for a sprung axis the keyboard adds an
offset, and for a lever or switch the most recent change wins until the hardware moves again.

### Isolation

- **Per connection:** each device runs on its own task with its own limits and counters.
- **One bad device stays contained:** a stalled, flooding, or malformed device is throttled,
  then disconnected, and never delays the fixed step or another device.
- **Devices are independent:** they never talk to each other. They compose only through
  commands and variables.

### Moving hardware needs consent, and stopping never waits

- **Consent:** outputs that move something physical (motion platforms, force-feedback yokes,
  motorised throttles, large servo gauges) are off until the user enables each device. Enabling
  is tied to that device and that session.
- **Stopping:** pause, a lost connection, a crash dialog, or the stop shortcut zeroes those
  outputs at once. Stopping never requires a confirmation.
- **The device stops itself too:** because a hung host or a pulled cable cannot send a stop, every
  moving output relies on the device-side heartbeat in section 3. Motion and force outputs are not
  offered at all until the heartbeat is implemented and tested.

### Versioning

- **Version numbers:** the device protocol carries a major version, negotiated in hello; every
  manifest, profile, binding file, and layout carries its own version.
- **Changes:** schema changes are additive first, and old files are upgraded on load with a
  notice. A breaking change takes two or more releases, with the old form accepted in between.
- **Checks:** CI checks that a changed schema bumps its version, as it already checks schema
  freshness.
- **No merge-conflict magnets:** shared lists every change touches (the variable catalogue
  sources, format versions, and translations) are built from a folder of one small file per
  addition, so parallel pull requests do not collide on one sorted list.

### Managed panel computers

A Raspberry Pi or tablet behind a panel pairs once with the main simulator. After that it
receives its layout, device manifests, and bindings from the main simulator, so it needs no
hand-edited configuration. Its own local inputs (GPIO, I2C, serial) are declared by manifests
that the main simulator pushes and the panel computer validates.

### Testing without hardware

- **Virtual devices:** a virtual device replays recorded or scripted input at a chosen pace.
  Binding, calibration, output, and disconnect behaviour can therefore be tested in CI in
  seconds.
- **Device manifest checks:** every device manifest in a pack must come with a recorded sample
  session that its tests replay.

## 6. Commands and MCP additions

- **Input:**
  - `input.devices` lists connected devices with live values.
  - `input.bind`, `input.unbind`, and `input.calibrate` manage bindings.
  - `input.profiles` lists device profiles.
- **Devices:**
  - `devices.list`, `devices.allow`, and `devices.disconnect` manage device-protocol connections.
  - `devices.serial_ports` and `devices.open_serial` handle wired boards.
- **Variables:** `variables.list` returns the catalogue; `variables.get` reads one or more values.
- **Layouts and windows:**
  - `layout.list`, `layout.load`, and `layout.save`;
  - `layout.add`, `layout.move`, `layout.set`, `layout.remove`, and `layout.export_cut_template`;
  - `window.open`, `window.close`, and `window.set`.

## 7. Phases

1. **HID and bindings:**
   - the simulation on its own thread (a Worker on the web);
   - the shared device core and its families;
   - virtual devices for tests;
   - declarative serial manifests;
   - the variable catalogue and device profiles;
   - bindings with bind-by-moving and per-aircraft sets;
   - outputs to quadrant lights (lights only, no moving outputs);
   - the platform notes in section 2 (udev rule, Input Monitoring, double-report guard);
   - test fixtures for raw HID reports.
2. **Device protocol:**
   - localhost WebSocket with Origin checks and the session token, and USB serial without
     probing;
   - per-device grants and the heartbeat;
   - the protocol test harness;
   - the Arduino, ESP32, Raspberry Pi, and Rust reference drivers.
3. **Layouts and windows:**
   - extra native windows with layouts and edit mode;
   - layout files with hot reload;
   - starting layouts.
4. **Physical panels:** per-monitor calibration, millimetre layouts, parallax tolerance,
   cut-template export in DXF and SVG, the SolidCraft script export, and the alignment screen.
5. **Remote panels:** panel mode served by the desktop app on the LAN, LAN pairing, and
   browser multi-window with one simulation owner.
6. **Moving outputs:** motorised controls, motion, and force feedback, only after the heartbeat
   is proven in the reference firmware.

Each phase ships with docs, a contributor guide ("write a driver", "add a device profile",
"build a panel"), and an agent skill. A coding agent asked to "make my Arduino panel work with
FlightCraft" should then know exactly what to do.

## 8. Decisions

1. **Named commercial controllers:** their device profiles live in a separate community profile
   pack, as with real aircraft. The engine ships generic profiles only.
2. **In-process WebAssembly drivers:** not now. They will be considered only if a measured latency
   need appears that a local socket cannot meet.
3. **Binary protocol:** no. Text at serial speeds is enough, and a terminal can debug it.
4. **UDP:** dropped. It would add an unauthenticated path, and WebSocket covers the same use.

Radio bindings are available now through the existing keymap/gamepad action path: `ComPtt`,
`IntercomPtt`, `RadioPanel`, and `Kneeboard`. Their defaults are T, Y, V, and K, respectively.
Hardware adapters can set `ptt` and `intercom_ptt` with `comms.audio_panel`, and submit recognized or
typed text using `comms.transmit`. Speech recognition and the broader device protocol above remain
future work. See [radios](radio.md).
