# External hardware, multiple screens, and home-built panels

Status: specification, not yet implemented.

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
  - Every change goes through `input.bind`, `input.unbind`, and `input.calibrate`, so agents and
    scripts can set up hardware too.

## 3. Device protocol for custom hardware (microcontroller boards, a Raspberry Pi, or anything else)

### Transports, all optional and all speaking the same messages

| Transport | For | Notes |
|---|---|---|
| WebSocket on localhost (`ws://127.0.0.1:<port>/devices`) | programs on the same computer | default |
| WebSocket or UDP on the LAN | Wi-Fi boards and remote panels | off by default; pairing code required |
| USB serial (CDC) | wired microcontroller boards | the desktop app opens ports the user selects; the browser uses Web Serial |

### Messages

JSON lines for every transport. Serial can also use a compact text form that a small Arduino can
produce with `Serial.print`.

1. **Hello:** the device declares an id, a name, a protocol version, and its controls.
   - **Inputs:** axes (range), buttons, switches (positions), encoders, and analogue values.
   - **Outputs:** lights, segment displays, character LCDs, servo gauges (angle range), steppers,
     PWM, and small pixel displays (resolution).
   - Hello comes with an optional default binding set, so a board can arrive pre-mapped.
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
5. **Commands:** a trusted driver may also send any control command (the same JSON as the MCP
   server), subject to the permission the user granted in the connection prompt.

### The variable catalogue (`schema/variables.json`)

Every readable value has a stable dotted name, a type, a unit, a range, and a description. This
covers flight state, air data, engine, electrical, fuel, annunciators, avionics display values,
radios, and autopilot modes. The catalogue is generated from code, checked for freshness in CI
like the other schemas, and drives subscriptions, output bindings, and the docs. Names never
change once published; a renamed variable is a new name plus a deprecated alias.

### Reference drivers and firmware (Apache-2.0, in an `examples/hardware/` folder)

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

### Windows

- **Native desktop:** the app can open any number of extra windows. Each has a monitor,
  fullscreen or borderless, a size, and a layout.
- **Browser:** the app can open extra browser windows that stay in sync.
- **Remote panel:** a second device (a tablet, an old laptop, or a Raspberry Pi in kiosk mode)
  runs the web app in panel mode and connects to a running desktop session over the device
  protocol. It shows a layout and sends knob and touch input back. This is the easy path for a
  screen behind a wooden dashboard: plug in any computer with a browser.

### Layouts (`schema/layout.json`)

- **What a layout is:** a set of elements on a canvas.
- **What an element can be:**
  - any instrument module with its skin;
  - a glass display (PFD or MFD);
  - the outside view, with its own camera (for example a side window on a third monitor);
  - the moving map;
  - a switch panel;
  - a text readout of any variable;
  - an image (a background or a placard).
- **Each element has:**
  - a position and size, in pixels or in millimetres;
  - rotation in 90° steps, z-order, and an optional circular or rectangular mask;
  - bezel on or off, and a brightness and night-lighting group;
  - "accepts input" on or off (for touch screens).
- **Layouts are files:** the app watches them, so a hand edit appears at once.
- **Edit mode, from a toggle or a shortcut:**
  - drag to move and handles to resize;
  - snap to a grid or to other elements;
  - align and distribute, and lock elements;
  - a properties panel;
  - undo and redo.
- **Edits are commands:** every edit is a `layout.*` command, so the same layout can be built by
  dragging, by writing JSON, or by an agent ("put the engine gauges along the bottom of
  monitor 2").
- **Starting layouts:** layouts ship for each aircraft's panel and a few common setups. Examples
  are "second monitor: full panel", "laptop: compact", and "two glass displays".

### Physical panels (a CNC-cut board in front of a screen)

1. **Calibrate the screen.**
   - The monitor's physical size comes from the system when it reports it.
   - Otherwise, the app shows a ruler you match to a credit card, or you type the measured width.
   - After that, millimetre layouts are true to size.
2. **Size instruments to real panel standards.**
   - Round instruments default to the common 3⅛-inch and 2¼-inch openings.
   - Faces are drawn so the visible dial fits the opening.
3. **Export the cut file.** "Export cut template" writes SVG and DXF at 1:1, ready for a CNC
   router or laser. It includes:
   - every opening;
   - mounting-hole positions for physical switches and encoders placed on the layout as
     "hardware" elements;
   - label engraving paths;
   - the screen outline and alignment marks.
4. **Align the board to the screen.**
   - An alignment screen draws crosshairs at every opening's centre and outlines each cut-out.
   - Per-element nudges fix small cutting errors without moving anything else.
5. **Keep the panel tidy.**
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

Each action or command has at most one active binding per input. Each output has exactly one
source variable. The binding loader refuses conflicts and names both claimants, instead of letting
two devices fight over the throttle or one light.

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

### Versioning

- **Version numbers:** the device protocol carries a major version, negotiated in hello; every
  manifest, profile, binding file, and layout carries its own version.
- **Changes:** schema changes are additive first, and old files are upgraded on load with a
  notice. A breaking change takes two or more releases, with the old form accepted in between.
- **Checks:** CI checks that a changed schema bumps its version, as it already checks schema
  freshness.

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
   - the shared device core and its families;
   - virtual devices for tests;
   - declarative serial manifests;
   - the variable catalogue and device profiles;
   - bindings with bind-by-moving and per-aircraft sets;
   - outputs to quadrant lights;
   - test fixtures for raw HID reports.
2. **Device protocol:**
   - localhost WebSocket and USB serial transports;
   - the protocol test harness;
   - the Arduino, ESP32, Raspberry Pi, and Rust reference drivers.
3. **Layouts and windows:**
   - extra native windows with layouts and edit mode;
   - layout files with hot reload;
   - starting layouts.
4. **Physical panels:** screen calibration, millimetre layouts, cut-template export, and the
   alignment screen.
5. **Remote panels:** web panel mode over the device protocol, browser multi-window, and LAN
   pairing.

Each phase ships with docs, a contributor guide ("write a driver", "add a device profile",
"build a panel"), and an agent skill. A coding agent asked to "make my Arduino panel work with
FlightCraft" should then know exactly what to do.

## 8. Open questions

1. Device profiles that name commercial controllers: keep them in a separate community profile
   pack, as with real aircraft (current default), or allow them in the engine repo as plain
   compatibility data?
2. In-process driver plugins (sandboxed WebAssembly components) for drivers that need lower
   latency than a local socket: worth it later, or never?
3. A compact binary form of the protocol for the smallest boards, or is the text form enough?
