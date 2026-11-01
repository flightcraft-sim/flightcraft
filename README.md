# FlightCraft

An open-source flight simulator in pure Rust, for the desktop and the browser, with a deterministic
core that people, scripts, and AI agents fly the same way.

## What works today

- The Corvid, an original two-seat trainer, flies on a 6-DOF model from textbook aerodynamics:
  takeoff, climb, turns, and landing on one runway in a standard atmosphere with no wind.
- The desktop app: a 3D chase view, six instruments, keyboard and gamepad controls you can rebind.
  `cargo run -p flightcraft-desktop -- --script examples/takeoff.jsonl` flies a scripted takeoff.
- The same app in the browser: `cargo xtask web` writes static files to `dist/` that any static
  host serves (WebGPU, or WebGL 2 where there is none). `?script=takeoff.jsonl` flies the takeoff.
- An MCP server (`flightcraft-mcp`, see [docs/mcp.md](docs/mcp.md)): an agent resets, sets controls,
  steps, and reads the state. With the autopilot holds and `sim.run_until`, a test flies the full
  traffic pattern in 32 calls.
- The trajectory is deterministic: a scripted flight gives the same hash every run.

Not yet: terrain, weather, failures, a second aircraft, or a cockpit view. The plan is in
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

Dual-licensed under either of the [Apache License, Version 2.0](LICENSE-APACHE) or the
[MIT licence](LICENSE-MIT), at your option. Unless you state otherwise, any contribution you submit
for inclusion is dual-licensed the same way, without additional terms or conditions.
