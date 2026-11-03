# Aircraft files (version 2) and aircraft packs

Aircraft definitions are original UTF-8 JSON documents with `format_version: 2` and a `kind`.
The embedded [Corvid](../aircraft/corvid/) and [Larrow](../aircraft/larrow/) use the same parser,
reference resolver, and physical validation as packs. Units are SI; angles are radians. Body axes
are x forward, y right, and z down. Geometry and load stations share a datum. Inertia is about the
specified body's CG, in kg m².

Each document has an `identity`: `id`, `display_name`, `description`, and `author`. Ids are 1–128
lowercase ASCII letters, digits, hyphens, or underscores. Other identity strings are nonempty and
at most 4096 bytes. References are ids within a known file kind. A pack may reference built-in
parts or parts from packs already installed. It cannot replace an existing id within a kind.

## Documents and references

| `kind` | Fields besides version and identity | Purpose |
|---|---|---|
| `family` | `geometry`, `aerodynamics`, `controls` | Geometry, coefficient tables, gear, surface travel, and ground contact properties shared by a family. |
| `variant` | `family`, `engine`, `systems`, `empty_mass`, `min_mass_kg`, `max_mass_kg`, `cg_envelope`, `flap_stages`, `fuel_capacity_kg`, `stations`, `v_speeds` | References and limits for a particular configuration. |
| `engine` | `propulsion`, `induction`, `fuel_flow_kg_s` | Shaft power, fixed-pitch propeller, induction, and fuel-flow curve. |
| `systems` | `installed`, optional `definition` | Up to 64 distinct system ids and validated operational vacuum, electrical, pitot-static, fuel, and engine parameters. See [systems](systems.md). |
| `panel` | `instruments` | Instrument installation ids, module ids, skin ids, positions, and sizes. |
| `aircraft` | `variant`, `panel`, `load` | A named aircraft with a default panel and station load. |

The [generated schema](../schema/aircraft.json) describes all six document kinds. The
[pack schema](../schema/pack.json) describes manifests. `cargo xtask schemas` regenerates schemas;
`cargo xtask schemas --check` checks freshness, including command schemas.

Geometry contains wing area, span, chord, a reference point, 1–16 gear contacts, and 1–64 hard
points. Gear contacts have position, stiffness, damping, static weight fraction, and steering
angle. Fractions sum to one; each contact is below the loaded CG. Positive steering turns a
forward wheel right; an aft steerable wheel uses a negative angle to yaw the nose right.
Runway initialization aligns the foremost and aftmost contacts with the ground, then lets the
spring/damper gear settle. Drawing currently chooses the procedural Corvid or Larrow mesh by id;
unknown ids use the Corvid outline.

Aerodynamics retains the seven coefficient tables and nondimensional derivatives from version 1.
Each table has 2–256 strictly increasing breakpoint/ordinate pairs and a nonempty `source` note.
Interpolation is linear and clamped at the edges. `controls` contains normalized elevator,
aileron, rudder, throttle, flaps, and brake limits, plus gear travel, contact-force and touchdown
limits, and friction coefficients. Normalized flap detents move to the variant.

A variant's `flap_stages` are 2–32 ordered objects with normalized `position` and positive
`max_speed_mps`, spanning the family's flap travel. `v_speeds` contains `vs_mps`, `vso_mps`,
`vr_mps`, `vx_mps`, `vy_mps`, `vfe_mps`, `vno_mps`, and `vne_mps`. These are limit metadata;
flap speed limits do not automatically retract flaps or change the flight model.

An engine's `propulsion` contains `max_power_w`, `idle_power_w`, `power_density` (power fraction
against density ratio), fixed-pitch propeller `prop_diameter_m`, `prop_ct`, and `prop_cp` tables
against advance ratio, and the RPM and windmilling parameters. `induction` is `carburettor` or
`injection`. `fuel_flow_kg_s` is a table against shaft-power fraction, from 0 to 1, with
nonnegative flow. Induction and fuel flow feed the operational engine and fuel systems when the
systems layer supplies a `definition`; placeholder installations retain legacy behavior.

Each panel instrument has a unique installation `id`, a known `module`, a `skin` id (default
`original`), a nonnegative `position: [x, y]`, and a positive `size: [width, height]`, in panel
grid units. Modules include the six-pack, engine gauges, fuel, electrical, suction, and
annunciators. The UI resolves skin ids against its runtime registry; unknown ids are panel load
errors. See [instruments and skin registration](instruments.md) for the module catalogue,
presentation models, controls, and private pack extension point.

## Loads and weight and balance

A variant defines up to 64 stations, each with `id`, `kind` (`pilot`, `passenger`, `baggage`, or
`fuel`), `position_m: [x, y, z]`, and positive `max_mass_kg`. A load supplies station ids and masses;
omitted stations have zero mass. Unknown stations, repeated stations, negative masses, and
non-finite values are errors. Zero mass is permitted.

```json
{
  "stations": [
    {"station": "pilot", "mass_kg": 80},
    {"station": "passenger", "mass_kg": 80},
    {"station": "fuel", "mass_kg": 40}
  ]
}
```

`empty_mass` contains `mass_kg`, `cg_m`, and a symmetric, positive-definite `inertia_kg_m2` tensor.
Total mass is empty mass plus station masses. CG is the sum of empty-body and station moments
divided by total mass, for all three axes. Weight is total mass times 9.80665 m/s². The loaded
inertia uses the parallel-axis theorem, treating station loads as point masses. Empty inertia
must already include fixed equipment.

`cg_envelope` is a simple polygon of 3–64 `[longitudinal_cg_m, total_mass_kg]` vertices ordered
around its edge. Both orientations and concave polygons are supported; edges count as in limits.
Self-intersections and zero-area polygons are rejected. Limits include this envelope, minimum and
maximum mass, each station's mass limit, and total fuel capacity. The calculation returns all
violations. A structurally invalid definition is always rejected.

For example, a 600 kg empty body at x = 1 m plus 80 kg at x = 2 m, 60 kg at x = 2 m, 20 kg at
x = 4 m, and 40 kg at x = 1 m has 800 kg total mass and longitudinal CG = 1000 / 800 = 1.25 m.

The built-in arms and empty-body inertia are original design estimates chosen so their default
occupants and fuel reproduce the old mass, CG, and inertia exactly. For the Corvid, the 800 kg
empty body at x = -0.0625 m balances two 80 kg occupants at x = 0.3125 m, with 40 kg fuel at the
datum. The occupants' lateral arms are -0.25 m and +0.25 m; baggage is aft. For the Larrow, the
80 kg pilot at x = 0.25 m balances 40 kg fuel at x = -0.5 m. Other loads change CG and inertia.
These remain fictional estimates, with no claim of correspondence to a real aircraft.

## Commands and runtime API

`aircraft.list` takes `{}` and lists each named aircraft's identity, default `variant`, `panel`,
and `source`: `{"kind":"built_in"}` or `{"kind":"pack","id":"demo-pack"}`.
Select a named aircraft using `{"source":{"kind":"installed","id":"corvid-tourer"}}`.
The existing `built_in` source remains available for the embedded Corvid and Larrow.

`aircraft.load` accepts an explicit composition:

```json
{
  "variant": "corvid-standard",
  "panel": "corvid-six",
  "load": {"stations": [{"station": "pilot", "mass_kg": 80}, {"station": "fuel", "mass_kg": 40}]},
  "reset": true,
  "allow_out_of_limits": false
}
```

A load outside its envelope or other limits returns an error unless `allow_out_of_limits: true`.
With that explicit override, loading succeeds and returns `warnings` and `weight_balance` with
`in_limits: false`. The override never bypasses numeric, structural, or reference validation.
`weight_balance.compute` takes `variant` and `load`, and returns `mass_kg`, `weight_n`, `cg_m`,
`in_limits`, and `violations`, without changing the flight.

Omitting `reset`, or setting it false, stages selection until the next successful `sim.reset`.
`reset: true` applies a paused runway reset immediately, preserving weather, world, and playback
rate, and clearing holds and failures. Aircraft selection and resets are atomic. A failed command
preserves both active and staged aircraft; successful selections persist across later resets,
including an explicitly allowed load outside limits.

A `source` selection and explicit `variant`/`panel`/`load` are mutually exclusive. A JSON source
`{"source":{"kind":"json","text":"<aircraft document>"}}` resolves version 2 references
against the session catalogue. Native callers can use `{"source":{"kind":"file","path":"aircraft.json"}}`.
These sources also accept complete version 1 files. Browser calls never open dialogs or read files.

Rust callers use `LayerFile::from_json`, `AircraftCatalog::builtins`, `install_pack`, and `resolve`
or `resolve_aircraft`. Resolution returns a `ResolvedAircraft` with the physics `aircraft`, variant,
engine, installed systems, panel, load, and balance. `Session::aircraft_configuration` exposes the
active flight's resolved metadata; it is absent for a legacy fixed-mass source. `Sim` continues to
consume the resolved `Aircraft` through its existing constructors; `Sim::configure_systems` installs
the resolved systems, engine metadata, and station fuel into a new flight. The resolved runtime structure
retains the version 1 representation for compatibility; authorable version 2 files use `LayerFile`.

## Packs and loading

A pack is a directory with `pack.json` and manifest-listed version 2 JSON files. Zip transport is
reserved for later. The [demo pack](../examples/packs/demo-pack/pack.json) adds an original Corvid
touring variant using the built-in airframe and systems, a different injected engine, and a
three-instrument panel:

```json
{
  "id": "demo-pack",
  "name": "Corvid touring demonstration",
  "version": "1.0.0",
  "licence": "Apache-2.0",
  "authors": ["FlightCraft contributors"],
  "affiliation": "An original fictional design; not affiliated with or endorsed by any aircraft or equipment manufacturer.",
  "files": [
    {"path": "variant.json", "kind": "variant"},
    {"path": "engine.json", "kind": "engine"},
    {"path": "panel.json", "kind": "panel"},
    {"path": "aircraft.json", "kind": "aircraft"}
  ]
}
```

Every manifest field is required. `affiliation` must explicitly state the author's relationship
or non-affiliation; real aircraft packs belong outside this brand-neutral repository. The loader
requires a nonempty statement, and does not attempt to interpret legal language. A pack supplies
definition data only; scripts, native code, arbitrary asset kinds, and unknown instrument modules
are rejected.

On native, `flightcraft --packs-dir examples/packs` scans immediate pack subdirectories. The
selected directory is saved as the `flightcraft.packs_directory.v1` preference and reused next
launch; the CLI overrides it. The default directory is `packs`; a downloaded build started elsewhere falls back to the `packs` folder beside the executable. Desktop builds bundle the packs pinned in `bundled-packs.json` (each fetched at its pinned commit by `scripts/bundle-packs.py`, copying only the manifest's files plus licence, readme, and attribution). Directory entries are processed
in sorted order, so packs referencing other external packs must follow their dependencies.

On the web, repeat the `pack` query parameter with manifest URLs:
`?pack=packs/demo-pack/pack.json&pack=packs/another-pack/pack.json`.
URLs may be relative to the page or absolute HTTP(S) URLs with CORS enabled. Files resolve relative
to their manifest URL. The app streams bounded responses, fetches only listed files, and installs
packs in URL-list order. It remains usable offline with the embedded definitions.

A bad pack produces an app error and is skipped as a whole. Other packs still load, and current
flights continue. Duplicate pack ids and attempts to replace installed definitions are errors.
Native callers can use `read_pack` and `Session::load_packs_directory`; browser transports supply
`PackContents` to `Session::install_pack`, which shares the same catalogue validation.

## Hostile-input limits

Unknown fields are rejected throughout. Every numeric value must be finite and have magnitude at
most 1e12. Positive engineering parameters are at least 1e-6. The original table, geometry,
control-travel, gear, inertia, and coefficient checks also apply to every resolved model and unused
family or engine. Physical validation is additional to JSON Schema validation.

| Resource | Cap |
|---|---|
| Manifest | 64 KiB |
| Each JSON document | 1 MiB |
| Whole pack, including manifest | 8 MiB |
| Files per pack | 64 |
| Packs per catalogue / directory entries / web URLs | 64 |
| Manifest authors | 1–32 |
| Manifest name, version, licence, affiliation, each author | 1–4096 bytes each |
| Relative file path | 256 bytes |

File paths contain portable ASCII components, ending in `.json`. Absolute paths, URL syntax,
backslashes, empty or hidden components, traversal, case-insensitive duplicates, and `pack.json`
as a provided layer are rejected. Native packs reject symlink files and subdirectories and check
that resolved files remain under the pack directory. Reads are capped independently of metadata.
Unlisted disk files are ignored; transport payloads containing unlisted files are rejected.

## Migration from version 1

Version 1 files remain readable; fixtures live in `crates/aero/tests/fixtures/`. To create six
version 2 documents without changing the original model:

```sh
cargo xtask aircraft migrate old.aircraft.json new-aircraft-directory
```

The destination must not already exist. `migrate_v1` is the equivalent Rust API. The original fixed
total mass becomes empty mass, with an empty default load: version 1 contained no reliable payload,
fuel, or station data. Migration generates explicit placeholder limits, V-speeds, installed
systems, induction, and fuel-flow metadata, and a six-instrument panel. Review those estimates
before authoring new loads. Each layer is serialized and validated on reload; resolving the
migrated default gives the exact original physics definition.

The built-ins were authored with explicit default occupants and fuel while preserving their
original reset mass, CG, inertia, geometry, controls, and tables. The Corvid golden trajectory
with fuel-driven mass is `0xf228fce1a4055061`. CI also checks that trajectory on `wasm32-wasip1`
with Wasmtime; the browser
applications build for `wasm32-unknown-unknown`.

## Electronic equipment extension

Engine documents optionally declare `control` (`magnetos` or `electronic_dual_lane`) and
`gearbox_ratio`. Propulsion RPM limits describe propeller RPM; the ratio converts this to
engine RPM for indications. Systems definitions optionally declare `modern`, containing
selector positions, master-annunciation sources, and recovery parameters. Panels optionally
declare `equipment`, including a bounded background and individually placed equipment faces;
these panels may omit conventional instruments with an empty `instruments` list.

The [equipment specification](equipment.md) documents fields, load references, controls, and
limitations. The [fictional equipment pack](../examples/packs/equipment-demo/pack.json) is a
complete composable example. Optional fields default to the existing behaviour; built-in
engine definitions and default flight dynamics are unchanged.

Emergency procedures are optional systems-layer pack data. See [emergencies](emergencies.md#aircraft-procedures)
for ordered memory/checklist items, glide parameters, validation, and fictional built-in guidance.
