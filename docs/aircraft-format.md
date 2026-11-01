# Aircraft files (version 1)

An aircraft is UTF-8 JSON in a `*.aircraft.json` file, with `format_version: 1`.
Start with [Corvid](../aircraft/corvid.aircraft.json). It is also embedded in the binary as the
default. All Corvid data is original; each table carries its own source note.

The required objects are `identity`, `geometry`, `mass`, `aerodynamics`, `propulsion`, and `systems`.
Units are SI, and angles are radians. Body axes are x forward, y right, and z down. Geometry points
and `mass.cg_m` share a datum; the inertia tensor is about the CG. `reference_point_m` identifies the
geometry reference. Version 1 uses a fixed total `mass_kg`, including fuel and occupants, without
fuel burn or payload stations. Drawing selects the original procedural Corvid or Larrow mesh by
`identity.id`; unknown ids use the Corvid outline. The Larrow is an original single-seat, low-wing,
650 kg aerobatic trainer, with 11 m² wing area, 8 m span, and a 150 kW fixed-pitch propeller.
Its mass moments are lumped-mass design estimates for a narrow fuselage and light outboard wings.
Its table source notes describe independent textbook estimates, not measurements of any aircraft.

Every table contains `points` (pairs of breakpoint and ordinate) and a nonempty `source` note.
Tables interpolate linearly and clamp outside their domain. The seven aerodynamic tables use
radian incidence or sideslip; propeller tables use advance ratio, and the engine table maps density
ratio to available-power fraction. Derivatives use nondimensional rates and full normalised
control travel. Flap stages are normalised detents; continuous intermediate travel is supported.

Loading rejects unknown fields at every level, unsupported versions, and files above 1 MiB before
parsing. Errors name a field or array element. All numbers must be finite with magnitude at most
1e12. Mass, dimensions, areas, and other positive parameters have a minimum of 1e-6. Tables require
2–256 strictly increasing breakpoints. The inertia tensor must be positive definite and symmetric
to 1e-9 times its diagonal scale (at least one); accepted off-diagonal pairs are averaged for
integration. Gear contacts must be below the CG, and their weight fractions must sum to one.
Wheel contacts may describe either tricycle or tailwheel gear. Runway initialization aligns the
longitudinal plane through the foremost and aftmost contacts with level ground, then allows the
spring/damper gear to settle. Equal-height contacts keep the original level start. A positive
`steering_rad` turns that wheel right for positive rudder; an aft steerable wheel uses a negative
angle to yaw the nose right. Larrow's mains are ahead of its CG, with the short tailwheel aft.
No fields or meanings change, so both built-ins use version 1.

Gear, hard points, and flap stages are bounded; normalised control ranges and ordered flap stages
are checked. Validation ensures structural and numeric safety, not flightworthiness or trim.

The generated [JSON Schema](../schema/aircraft.json) describes structure and simple bounds;
`Aircraft::validate` additionally enforces cross-field and physical constraints. Regenerate both
aircraft and command schemas with `cargo xtask schemas`; CI checks freshness with
`cargo xtask schemas --check`.

`aircraft.list` takes `{}` and returns the Corvid and Larrow identities. The header picker calls
`aircraft.load` followed by `sim.reset`, starting the selected aircraft running on the runway.
Scripts and browser clients can select a built-in without embedding JSON or reading a file:

```json
{"source":{"kind":"built_in","id":"larrow"}}
```

See [the Larrow takeoff script](../examples/larrow-takeoff.jsonl) for raising the tail, rotating,
and climbing. Both built-ins are embedded and available offline. To stage an aircraft for the next
successful `sim.reset`, call `aircraft.load` with:

```json
{"source":{"kind":"json","text":"<complete aircraft JSON>"}}
```

Native callers may instead supply `{"source":{"kind":"file","path":"trainer.aircraft.json"}}`.
The file read is capped too. Browser builds accept built-in selections and JSON text. Add `"reset": true` to apply
immediately with a paused runway reset, preserving weather and playback rate, and clearing holds
and failures. Loading and resetting are atomic: an error preserves both active and staged aircraft.
A successful selection persists across resets. Load the `corvid` built-in (or its JSON) again to
restore the default.

Rust callers use `Aircraft::from_json`, `Aircraft::corvid`, `Aircraft::larrow`, `Aircraft::builtins`,
`Sim::new_with_aircraft`, or
`Sim::new_with_aircraft_and_weather`. Force, coefficient, and propulsion functions take `&Aircraft`.
`Sim::aircraft` exposes the active definition read-only.

CI runs the unchanged golden trajectory assertion through the normal Rust test harness on
`wasm32-wasip1`, using the pinned official Wasmtime release and `--test-threads=1`:

```sh
CARGO_TARGET_WASM32_WASIP1_RUNNER="wasmtime run" cargo test --locked -p flightcraft-sim --test golden --target wasm32-wasip1 -- --test-threads=1
```

Runtime references: [Rust WASI target](https://doc.rust-lang.org/rustc/platform-support/wasm32-wasip1.html),
[Cargo target runner](https://doc.rust-lang.org/cargo/reference/config.html#targettriplerunner), and
[Wasmtime CLI](https://docs.wasmtime.dev/cli-options.html).
