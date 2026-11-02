# Contributing

Thanks for helping. Start with [docs/SPEC.md](docs/SPEC.md) and [ROADMAP.md](ROADMAP.md).

## Rules

1. **Original work.** Never copy code, data, aircraft, panels, scenery, names, or text from another
   simulator, and never take data from a commercial one. Do not read GPL, LGPL, or AGPL flight
   simulation code (for example FlightGear or JSBSim) while working here.
2. **Licences.** Dependencies must be permissively licensed; `cargo deny check licenses` enforces the
   list in `deny.toml`. A new asset or data source needs a row in ATTRIBUTION.md.
3. **Never crash.** No `unwrap`, `expect`, `panic`, indexing that can go out of bounds, or `unsafe` in
   non-test code. Errors are `Result`s; every input is treated as hostile.
4. **Layers.** A crate depends only on lower layers (see the spec). The physics crates use no clock,
   threads, randomness, or `std` transcendental maths (`libm` only), so results are bit-identical
   everywhere.
5. **Commands.** Anything the app can do is a command with input and output schemas, and calling one
   programmatically never opens a dialog.
6. **UI.** egui only; colours from theme tokens; shortcuts configurable.
7. **Tests.** A change that alters the golden trajectory updates its hash in the same pull request
   and says why.

## Before you push

```
cargo fmt --all
cargo xtask ci
```

One topic per pull request. Describe what changed and how you checked it; show visible changes with a
screenshot.

## Region data

Build or refresh geographic data with `cargo xtask region build`; do not hand-edit generated
sidecars or add terrain/chart binaries to git. See [Build a region](docs/terrain.md#build-a-region)
for the online and offline layouts, national navigation, cells, validation, and release procedure.
Run `cargo xtask data fetch` before local region-dependent tests or packaging. Use
`cargo xtask data fetch --offline` to restore a populated cache without network access. CI requires
all locked showcase tiles. Review `validation.json`, `validation.md`, source receipts, attribution,
and the release manifest before publishing data. The first data release must exist before the
binary-removal change is merged.
