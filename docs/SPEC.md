# FlightCraft specification

Status: draft, 2026-11-01. This document says what FlightCraft is meant to be. What works today is in
the README; the order of work is in ROADMAP.md.

## 1. What it is

FlightCraft is an open-source flight simulator written entirely in Rust. One codebase builds a
desktop app and a browser app that needs no server. Every action is a command with a typed input and
output, so a person at the keyboard, a script, a test, and an AI agent over MCP all fly the same way.

The core is a deterministic, headless simulation: given the same aircraft, start, and control
inputs, it produces the same trajectory, bit for bit, on every platform.

## 2. What it is not

- Not a recreation of any existing simulator. No aircraft, flight model, livery, panel, scenery,
  name, or text is taken from one, and no commercial simulator is ever driven or mined for data.
- Not a certified training device. Flight models aim to be physically plausible and internally
  consistent, not type-accurate to a real aircraft.
- Not a replica of real aircraft. Every aircraft is a fictional design of our own, with no real
  manufacturer's name or logo.
- Not a game engine. The renderer draws what the simulation needs, nothing more.

## 3. Audience

1. People who want a light, free simulator that runs in a browser tab.
2. Developers and researchers who want a deterministic flight model they can script, test, and
   drive from code or an agent (control design, autopilot experiments, reinforcement learning).
3. Designers who model an aircraft in SolidCraft and want to fly it (see section 10).
4. Contributors who want a small, readable Rust codebase with strict rules.

## 4. Principles

1. Pure Rust; every application crate builds for `wasm32-unknown-unknown` and runs offline in a
   browser.
2. Everything is a command with input and output schemas. Programmatic calls never open a dialog.
3. A deterministic core that runs headless. Crates are layered and the physics knows nothing of the
   UI.
4. Never crash: no `unwrap`, `expect`, `panic`, or `unsafe` outside tests. Hostile input (a bad
   aircraft file, a NaN control, a huge step count) is an error, never a crash.
5. One UI toolkit (egui on wgpu). Colours come from theme tokens; shortcuts are configurable, never
   hard-coded.
6. Everything is checked by CI, and `cargo xtask ci` runs the same checks locally.

## 5. Architecture

| Layer | Crate | Role |
|---|---|---|
| L0 | `world` | ISA atmosphere, wind, ground height and surface, airfields and runways |
| L0 | `aero` | aircraft definitions; coefficient tables; forces and moments from an airflow state; propulsion and mass |
| L1 | `sim` | 6-DOF rigid body, fixed-step integrator, ground contact, the trajectory hash |
| L2 | `control` | the session, the command registry with schemas, the MCP protocol |
| L2 | `render` | scene description and wgpu drawing: terrain, runway, aircraft, chase and cockpit cameras |
| L3 | `ui` | egui front end: instruments, panels, input mapping, theme |
| apps | `desktop`, `web`, `mcp` | native window; browser build; headless MCP server over stdio |

Rules, checked by `cargo xtask layers`:

- A crate depends only on crates in lower layers. `render` reads simulation state but never changes
  it; it never depends on `control` or `ui`.
- `world`, `aero`, and `sim` depend on no windowing, GPU, clock, thread, or random-number crate.
- `control` is the only way the UI changes the simulation. A button and an MCP call reach the same
  command.

### Frames and units

SI units throughout. World frame: local north-east-down with its origin at the runway threshold.
Body frame: x forward, y right, z down. Knots, feet, and degrees appear only in the UI and in command
outputs that ask for them.

### Time

The simulation steps at a fixed 120 Hz. The app runs as many steps as real time needs per frame
(capped, so a stalled tab does not spiral) and can pause or run faster. Headless callers step
explicitly.

## 6. Flight model

Approach: a component build-up model driven by coefficient tables, the textbook method of
Etkin, Roskam, Stevens and Lewis, and the NACA reports.

- Forces and moments from non-dimensional coefficients (CL, CD, CY, Cl, Cm, Cn) as functions of
  angle of attack, sideslip, control deflections, flap setting, and non-dimensional rates.
- Tables are linear-interpolated, clamped at their edges, and checked when an aircraft loads
  (monotone breakpoints, finite values, sane sizes).
- Post-stall: lift falls off past the stall angle and blends to a flat-plate model at high angle
  of attack, so spins and deep stalls stay bounded and recoverable in the model.
- Propulsion: a piston engine with a power curve against density altitude and a fixed-pitch
  propeller from advance-ratio thrust and power coefficient tables. Torque and p-factor come later.
- Ground: each gear leg is a spring and damper with rolling friction, braking, and nose-wheel
  steering; a contact off the gear (a wingtip, the fuselage) counts as a crash.
- Integration: semi-implicit or fourth-order Runge-Kutta at 120 Hz, quaternion attitude,
  renormalised each step.

Determinism: no fused multiply-add assumptions, no `std` transcendental functions in L0 and L1
(`libm` only, which is pure Rust and gives the same bits everywhere), no hash-map iteration, no
threads, no clock. The golden test checks a trajectory hash.

Sources: public NACA reports (for example Report 824 for airfoil section data), and textbook
aerodynamics. Each table in an aircraft file cites where its numbers came from.

### Accuracy targets

Plausibility and consistency, checked by tests rather than by feel:

1. Stall speed within 5% of the value computed from the aircraft's CLmax, weight, and wing area.
2. Steady level flight trims at the airspeed the drag polar and power curve predict, within 5%.
3. Phugoid period within 15% of the Lanchester approximation (π√2·V/g) for the trim speed.
4. Short-period and Dutch-roll modes are stable and damped for the default aircraft.
5. Takeoff ground roll and climb rate fall in the range of a light single of that size and power.
6. Energy is conserved within tolerance in a glide with no drag and no engine (a sanity test of the
   integrator).

## 7. World and terrain

First slice: flat ground at sea level with one paved runway (900 m by 23 m, heading 360) on the
standard atmosphere, no wind.

The original training valley now provides bundled height tiles, slope contact, and two renderer
levels of detail; see [terrain format and commands](terrain.md). The flat world remains the
default and retains the original golden trajectory.

Later, from open data only, each with a row in ATTRIBUTION.md:

| Data | Source | Licence | Status |
|---|---|---|---|
| Elevation | SRTM (NASA/USGS) | Public domain | Planned |
| Elevation | Copernicus DEM GLO-30 | Copernicus DEM licence, free with attribution; terms to be checked before use | Open question |
| Airfields, roads, water, land use | OpenStreetMap | ODbL 1.0, with "© OpenStreetMap contributors" shown in the app | Planned |

Terrain streams in tiles. The browser build fetches tiles from a static host or works from tiles
bundled with the page; no tile server we run is required. The simulation reads ground height from
`world` only, so terrain changes nothing in `sim`.

Weather, in order: constant wind, gusts and turbulence (a Dryden-style model from its published
spectra), then cloud layers and visibility for the renderer.

## 8. Aircraft format

An aircraft is a JSON file (`*.aircraft.json`) with a `format_version`, read by `aero`:

- identity: an id, a display name, a description, and the author;
- geometry: wing area, span, mean chord, reference point, gear contact points, and outline meshes
  for drawing;
- mass: empty mass, fuel, payload stations, centre of gravity, inertia tensor;
- aerodynamics: coefficient tables with breakpoints and a source note for each;
- propulsion: engine power curve and propeller tables;
- systems: control surface limits and rates, flap stages, brakes.

Loading is hostile-input safe: every value is checked, sizes are capped, and an error names the
field. The first aircraft is the Corvid, a fictional two-seat, high-wing, fixed-gear trainer of
about 1000 kg with a 120 kW engine, built into the binary and also shipped as a file.

## 9. Commands and MCP

Every command has a name, a JSON input schema, and a JSON output schema (generated by `schemars`).
The registry lists them, and both the UI and MCP call through it. First set:

| Command | Does |
|---|---|
| `sim.reset` | start a flight: aircraft, start (runway or in the air), paused or running |
| `sim.set_controls` | set any of elevator, aileron, rudder, throttle, flaps, brake |
| `sim.step` | advance a number of fixed steps (capped per call) and return the readout |
| `sim.state` | the full state and the readout |
| `sim.pause`, `sim.resume`, `sim.set_rate` | real-time control in the app |
| `aircraft.list`, `aircraft.load` | the aircraft available and loading one from a file or JSON |
| `view.set_camera` | chase or cockpit, and camera offsets |
| `app.screenshot` | a PNG of the window (app only) |

The MCP server (`apps/mcp`) speaks JSON-RPC over stdio, headless. A bridged mode connects it to a
running app so an agent can fly what a person sees. An agent's loop is: reset, then set controls,
step, and read the state, repeated. A test flies a full traffic pattern this way.

## 10. SolidCraft aircraft

The distinctive goal: model an aircraft in SolidCraft and fly it here.

1. A SolidCraft design names its parts by role (wing, horizontal tail, vertical tail, fuselage,
   control surfaces, gear contacts) and exports geometry and mass properties.
2. FlightCraft reads that export, builds lifting surfaces from the geometry, and computes the
   coefficient tables with a vortex-lattice method for the linear range plus section data for
   stall and drag.
3. The result is an ordinary aircraft file, so it can be inspected, edited, and committed.

The exchange format is agreed with SolidCraft before either side builds it (an open question).

## 11. Web build

- Target `wasm32-unknown-unknown`, built with `trunk`; WebGPU where the browser has it, WebGL 2
  otherwise.
- Single-threaded, so it needs no cross-origin isolation headers and the simulation stays
  deterministic.
- No server: the page and its assets are static files. Settings persist in browser storage.
- Keyboard and the browser gamepad API; touch controls later.
- The same golden trajectory hash in the browser as on the desktop, checked in CI under a wasm
  runtime.

## 12. Tests

1. Unit tests in each crate: atmosphere against the ISA tables, interpolation, quaternion maths,
   loading hostile aircraft files.
2. Model tests for the accuracy targets in section 6.
3. Golden tests: a scripted takeoff, climb, and landing whose trajectory hash is fixed, run on Linux,
   macOS, Windows, and wasm.
4. Property tests: random and hostile control inputs and commands never panic and never produce
   non-finite state.
5. Agent test: a scripted MCP client flies a traffic pattern and lands within the runway.
6. App smoke tests: the desktop app starts and draws a frame headless; the web build compiles and
   loads.

## 13. Milestones

| Milestone | Done when |
|---|---|
| M0 Foundation | workspace, licences, CI with licence and layer checks |
| M1 First flight | the Corvid takes off, climbs, and lands in the desktop app and the browser, with basic instruments and a chase view; golden test passes on every platform |
| M2 Agent pilot | the MCP server flies the pattern in a test; the app can be driven over MCP |
| M3 Real world | elevation tiles for one region, wind and turbulence, engine failure |
| M4 Second aircraft | a second fictional aircraft and the aircraft file format stable |
| M5 Designed in SolidCraft | an aircraft modelled in SolidCraft flies |

## 14. Open questions

1. The GitHub organisation and where the repository finally lives.
2. Copernicus DEM licence terms for redistributing derived tiles.
3. Hosting for the browser build (GitHub Pages is the default) and for terrain tiles.
4. The SolidCraft export format for aircraft geometry and roles.
5. Contribution terms: the Developer Certificate of Origin or no sign-off.
