# FlightCraft: instructions for agents

A clean-room, pure-Rust flight simulator: a deterministic 6-DOF core, an egui and wgpu app for the
desktop and the browser, and an MCP server. Design: `docs/SPEC.md`. Order of work: `ROADMAP.md`.
Human rules: `CONTRIBUTING.md`; they apply to you too.

## Hard rules
- Toolchain: Rust 1.98.0, with rustfmt, clippy, and `wasm32-unknown-unknown`;
  golden trajectory CI also checks `wasm32-wasip1` on the same toolchain.
- Original work only. Never copy another simulator's code, data, aircraft, or text; never drive or
  take data from a commercial simulator. Do not read GPL, LGPL, or AGPL code (FlightGear, JSBSim).
- Licences: `cargo deny check licenses`. New assets get a row in ATTRIBUTION.md.
- Never crash: no unwrap, expect, panic, out-of-bounds indexing, or unsafe in non-test code
  (clippy enforces it workspace-wide).
- Layers (`cargo xtask layers`): world and aero, then sim, then control and render, then ui, then
  apps. render never changes the simulation; the UI changes it only through control's commands.
- Determinism in world, aero, and sim: fixed step, `libm` for all transcendental maths, no clock,
  threads, randomness, or hash-map iteration.
- Every command has input and output schemas; programmatic calls never open dialogs.
- UI: egui only, colours from theme tokens, shortcuts from the keymap, never hard-coded.
- Every application crate builds for `wasm32-unknown-unknown`.
- Run `cargo xtask ci` before every commit.
