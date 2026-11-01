# Height tiles and the training valley

`cargo xtask terrain` regenerates the original landscape in
`assets/terrain/training-valley/` using `world::TRAINING_SEED`. There are 36 tiles covering
12 km by 12 km, with 65 by 65 samples per tile at 31.25 m spacing. Total tile data is 757,008
bytes. Adjacent edges share identical samples. An apron around Corvid Field is flattened to
zero MSL, and the 900 m by 23 m runway remains at the NED origin. The ridge rises to about
600 m. The lake is a flat 35 m MSL surface. No external elevation data is used.

The version 1 `.fcht` format is little-endian:

| Offset | Type | Meaning |
|---|---|---|
| 0 | 4 bytes | ASCII `FCHT` |
| 4 | u16 | Version, currently 1 |
| 6 | u16 | Reserved, must be zero |
| 8 | f64 | Origin north, local metres |
| 16 | f64 | Origin east, local metres |
| 24 | f32 | Sample spacing in metres |
| 28 | u32 | Square grid width in samples |
| 32 | width² f32 | Heights MSL in metres; north varies fastest |
| following heights | (width−1)² u8 | Cell surfaces; north varies fastest: 0 grass, 1 pavement, 2 water |

Width is capped at 257, spacing is in 0.1–10,000 m, origins are within ±10,000 km,
and heights are within ±20 km. All floating values must be finite, and water cells must have four equal corner heights. The reader rejects unknown
versions, reserved bits, surfaces, truncated arrays, and trailing bytes before constructing a tile.
Tiles are immutable after validation. Height interpolation is bilinear, and its analytic gradient
gives an upward NED unit normal, `[-dh/dnorth, -dh/deast, -1]`, normalised with `libm`.
Queries outside the bundled region fall back to sea-level grass. The runway rectangle overrides
the surface and height. At a shared tile edge, the first tile in storage order owns the query;
heights agree, while the one-sided gradient can differ.

`sim.reset` accepts `"world": "flat"` (the default) or `"world": "training-valley"`.
`sim.state` reports the world choice. `world.height` accepts `north_m` and `east_m` and returns
`height_msl_m`, `normal_ned`, and `surface`. Invalid resets leave the current simulation intact.
The bundled bytes are included in native and browser binaries, so both work offline.

`Sim::new_with_world` and `Sim::from_state_in_world` support terrain experiments; existing
constructors retain the flat world. Terrain content contributes to replay identity when tiles
are present. The original flat-world golden trajectory and hash remain unchanged. Rust `Reset` literals now
supply `world`; JSON callers may omit it. The UI passes the immutable world through
`Scene::set_world`, and `Palette` adds `earth` and `water` colour tokens.
Gear damping uses velocity into the surface normal, spring penetration is measured along that
normal, and friction lies in the local tangent plane. AGL remains the lowest wheel's vertical
clearance above terrain. Airframe hard points are checked against terrain independently of gear.
Water currently uses the same collision model as other ground; buoyancy is outside this task.

Rendering uploads two meshes per tile once per world content change. The near mesh uses every
sample, and the far mesh uses every fourth sample (125 m spacing); a 3 km camera-to-tile-centre
threshold selects the level. Vertical skirts cover cracks between neighbouring levels. Buffers
use ordinary vertices, uniforms, and depth testing, within WebGL 2 limits. Surface normals shade
the ground, water uses the theme's blue token, and the runway remains a separate overlay.
