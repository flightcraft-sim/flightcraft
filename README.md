# FlightCraft

**The Art of Flying.**

An open-source flight simulator in pure Rust, for the desktop and the browser, with a deterministic
core that people, scripts, and AI agents fly the same way.

**Fly it in your browser:** https://flightcraft-sim.github.io/flightcraft/ (add `?script=takeoff.jsonl`
to watch a scripted takeoff).

## What works today

- The Corvid, an original two-seat trainer, flies on a 6-DOF model from textbook aerodynamics:
  takeoff, climb, turns, and landing on one runway in a standard atmosphere with no wind.
- The desktop app: a 3D chase view, sensed flight and engine instruments, and keyboard and gamepad controls you can rebind.
  `cargo run -p flightcraft-desktop -- --script examples/takeoff.jsonl` flies a scripted takeoff.
- The same app in the browser: `cargo xtask web` writes static files to `dist/` that any static
  host serves (WebGPU, or WebGL 2 where there is none). `?script=takeoff.jsonl` flies the takeoff.
- An MCP server (`flightcraft-mcp`, see [docs/mcp.md](docs/mcp.md)): an agent resets, sets controls,
  steps, and reads the state. With the autopilot holds and `sim.run_until`, a test flies the full
  traffic pattern in 32 calls.
- [Multiple screens](docs/multiple-screens.md): native windows, individual panel pieces, touch controls, shared browser pages, and opt-in paired LAN panels.
- Bridged MCP: `flightcraft --control 7890` and `flightcraft-mcp --connect 7890` let an agent fly the
  app a person is watching, and take screenshots of it.
- Wind, shear, and repeatable turbulence; engine failure, partial power, and jammed controls.
- A cockpit view from the left seat, with glances left, right, and up.
- Aircraft are files: the Corvid is composed from `aircraft/corvid/`, in a versioned format
  ([docs/aircraft-format.md](docs/aircraft-format.md)) checked field by field on load.
- The trajectory is deterministic: a scripted flight gives the same hash every run.

- Two aircraft, the Corvid trainer and the Larrow aerobatic taildragger, picked in the header.
- Terrain from height tiles: the original Training Valley world, with ground contact on slopes.

- Real public-domain elevation, FAA airports, and selectable runway starts in the
  [New York area](https://flightcraft-sim.github.io/flightcraft/?region=terrain/new-york/region.json) and
  [Eastern Pennsylvania and New Jersey](https://flightcraft-sim.github.io/flightcraft/?region=terrain/eastern-pa/region.json).

Instrument practice includes weather, IMC, the hood, and [IFR lessons with ACS grading](docs/ifr-training.md).

### Make it your plane

Set your tail number and spoken aircraft type in **Radios**. FlightCraft remembers them for each
aircraft variant and paints your number on both sides of the Corvid, Larrow, and installed Sling
exteriors on desktop and web. US N-numbers and foreign prefix-dash registrations are supported;
**Use pack** restores the original paint and radio identity. See [callsigns](docs/radio.md#radio-identity).

The plan is in
[ROADMAP.md](ROADMAP.md) and the design in [docs/SPEC.md](docs/SPEC.md).

## Building

```
cargo run -p flightcraft-desktop     # fly
cargo xtask ci                       # the checks CI runs
```

On Linux the gamepad support needs the udev development files (`libudev-dev` or `systemd-devel`).

## Ground rules

- Original work only: no code, data, aircraft, or text from other simulators, and no copyleft code.
- Fictional aircraft of our own design, with flight models from physics and public data.
- Every asset is original or openly licensed, listed in [ATTRIBUTION.md](ATTRIBUTION.md).

## Licence

Licensed under the [Apache License, Version 2.0](LICENSE). Unless you state otherwise, any
contribution you submit for inclusion is licensed the same way, without additional terms or
conditions.
