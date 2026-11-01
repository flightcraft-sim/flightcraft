# FlightCraft

An open-source flight simulator in pure Rust, for the desktop and the browser, with a deterministic
core that people, scripts, and AI agents fly the same way.

## What works today

Not much yet: the workspace, the licences, CI, and the simulation's public interface. Nothing flies.
This line changes when the first aircraft takes off. The plan is in [ROADMAP.md](ROADMAP.md) and the
design in [docs/SPEC.md](docs/SPEC.md).

## Building

```
cargo build
cargo xtask ci     # the checks CI runs
```

## Ground rules

- Original work only: no code, data, aircraft, or text from other simulators, and no copyleft code.
- Fictional aircraft of our own design, with flight models from physics and public data.
- Every asset is original or openly licensed, listed in [ATTRIBUTION.md](ATTRIBUTION.md).

## Licence

Dual-licensed under either of the [Apache License, Version 2.0](LICENSE-APACHE) or the
[MIT licence](LICENSE-MIT), at your option. Unless you state otherwise, any contribution you submit
for inclusion is dual-licensed the same way, without additional terms or conditions.
