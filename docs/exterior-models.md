# Making an aircraft exterior

The renderer accepts bounded glTF 2.0 binary models (`.glb`) from pack format 2.
Corvid and Larrow embed the same model profile, without filesystem access, on desktop and web.
An unknown aircraft or a missing exterior uses the existing procedural outline. Invalid external
packs fail transactionally, leaving the active flight and installed catalog intact.

Run `python3 scripts/generate-exteriors.py` to regenerate the fictional aircraft. Both the generator
and its two outputs per aircraft are checked in. It uses only Python's standard library. There are
no downloaded meshes, imported fonts, source photographs, or traced outlines. Lofted elliptical
fuselage sections, closed analytic wing sections, swept tail surfaces, smooth wheels, gear struts,
and geometry lettering are original artwork. Normals are accumulated at shared surface vertices.
The material palette is an original white, navy, and teal or orange livery, without logos.

The Sling pack carries its own Blender source, `scripts/blender-exterior.py`; its
`docs/blender-exterior.md` records the confined Blender 5.2.0 LTS regeneration workflow.
The retained `scripts/generate-exterior.py --sling-out .` reproduces older pack revisions.
The engine repository does not embed a real aircraft. Sling dimensions and construction facts come from the external POH
and visual inspection of Section 1.3 of the local POH PDF. The swept fin and conventional tail,
curved tips, canopy rails, rear windows, wheel pants, pitot, and antenna were independently
constructed after that inspection. No source pages or images are distributed. Fine contours
remain modeling estimates; see the pack README.

## Authoring contract

Use meters and pack body axes: x forward, y right, z down. This is an explicit FlightCraft asset
coordinate convention; do not apply a second glTF Y-up conversion. The renderer subtracts the loaded
CG and then applies the aircraft attitude. Local node origins are hinge or axle pivots. Parent nodes
allow wheels to inherit steering and spring compression. Each node needs a unique portable name.
Every motion, light, and registration binding must name a node. Wheels under a steering group inherit
that group's movement. Runtime glTF animations and skins are unsupported: the simulator owns poses.

The profile supports triangle primitives, POSITION/NORMAL, indices, TEXCOORD_0, tangents, and
metallic-roughness PBR materials. Base color and emissive maps use sRGB; normal, AO, and packed
metallic-roughness maps remain linear. Normal-mapped primitives require UVs and tangents.
Clearcoat factors are supported. Original color-only models continue to load.

Each GLB is capped at 64 MiB, 128 nodes, 32 materials (including the default material), 120,000
triangles, and 360,000 expanded vertices. It may contain at most 8 embedded or manifest-listed PNG
images and 64 texture references. Images are limited to 4096² and 320 MiB total decoded RGBA storage per
model. Every PNG header and binary view is checked before decompression; the decoder also has
allocation limits. Unlisted image URIs, external buffers, sparse accessors, skins, morph targets, and
embedded animations are rejected. Shared PNG inventories and pilot-seat metadata are described in
[Pilot-seat interiors](cockpit-interior.md). Packs have a separate 128 MiB encoded limit. GPU uploads
deduplicate shared images
and build color-correct and normal-correct mipmaps. Devices limited to 2048 textures upload the
appropriate filtered mip instead of failing or allocating an oversized texture.

LOD1 must reduce the count and remain below 25,000 triangles; optional `lod2` must reduce it again
and remain below 5,000. LOD1 is selected beyond 150 meters and LOD2 beyond 500 meters. Bounds
must match the stated dimensions within 1%. Existing named hinges, axles, lights, and registration
markers remain stable. Sling's Blender source uses POH numerical text and original procedural
construction; it is separate from the retained legacy generator.

Transparent primitives use straight alpha, depth testing without depth writes, and back-to-front
triangle sorting along the camera axis. Opaque geometry is drawn before glass. Exterior lighting
uses GGX metallic-roughness shading, normal maps, AO, emissive light gates, clearcoat, and sky
reflections. `Scene::set_exterior_environment` accepts scenery-owned linear lighting and an optional
GGX-prefiltered cubemap through `exterior::ExteriorEnvironment` / `EnvironmentProbe`. Without a
probe, the shader reflects a procedural sky/ground environment. Scenery retains ownership of
terrain, sky drawing, and shadows.

Registration may remain geometry lettering. For baked lettering, `registration.surface` declares
the body-space X/Z patch, mesh node, letter height, linear background color hint, and ink color. The
`Scene::set_registration` uses the atlas painter when this surface is declared, through the
same API and cache/reset path as main f066ed5. It rasterizes the original stroke glyphs through
the model's UVs over the baked default at all three LODs. Normal, AO, roughness, and emissive
images stay intact; clearing an override restores the immutable original asset. The painter
samples clean paint within the declared patch to match the actual baked colour, falling back
to the declared hint; this avoids a rectangular colour change around replacement letters.
Main's existing UI supplies the saved callsign override; preserve that plumbing during the flight lead's rebase.

Deflection bindings specify a unit local axis and travel at negative and positive control demand.
Positive elevator demand is nose up; choose hinge signs to match that convention. Flaps interpolate
from the first angle at zero demand to the second at full demand. Wheels use rolling distance divided
by their declared radius. Springs use penetration at the same gear contact and terrain height as
the physics, capped at gear travel. They do not feed forces back into flight. Propeller phase and
rolling distance advance with simulation time. Below 650 propeller RPM, blades are visible; above
that speed an alpha-blended disc replaces them. Pause freezes the phases.

Navigation lamps are red on the left, green on the right, and white aft. Strobes flash for 90 ms
per 1.2 seconds; the red beacon flashes for 160 ms per second. They use simulation time. Switches,
breakers, and bus power gate the lamps. A fixture may name its installed `power_load`, including
shared circuits such as the Sling nav/strobe installation. The landing light adds a forward, downward cone of illumination
to terrain and runway fragments; time of day controls its contrast. The renderer has no shadow maps;
its existing soft ground contact shadow remains in use. Packs with `interior.pilot_eye_m` use their authored cabin in cockpit view, including any moving frame and glass.
Other packs keep the existing procedural cockpit geometry and panel. Wing and cockpit views use a 0.015-meter near plane.

## Verification and captures

Use the `fc-plane` target cache and four build jobs, as required by the working rules:

```sh
export CARGO_TARGET_DIR="$HOME/builds/cargo-target/flightcraft/fc-plane"
export PKG_CONFIG_PATH="$HOME/builds/pkgconfig-shim"
export CARGO_BUILD_JOBS=4
export FLIGHTCRAFT_SLING_PACK="$HOME/builds/worktrees/sling-exterior"
cargo test -j 4 -p flightcraft-aero -p flightcraft-render
cargo test -j 4 -p flightcraft-ui render_exterior_director_shots -- --ignored --nocapture
```

The optional Sling environment variable adds the external aircraft to bounds, animation, and GPU
checks. Offscreen tests use the actual renderer with WebGL 2 device limits and save the four director
views plus night lighting under `out/exterior-after/`. They require no downloaded terrain or real
window. Flight lead should capture each aircraft from the packaged build at cockpit takeoff, wing
climb, flyby cruise, tower final, and a night chase view with the exterior lights enabled. Check
registration from both sides, moving surfaces, and wheel steering at taxi speed.

The accepted V8 Blender exterior is installed in the Sling pack. Repeat its matched-camera
proof against the installed GLBs with the seven-view manifest:

```sh
export FLIGHTCRAFT_CYCLES_OUTPUT="$HOME/builds/codex-jobs/blender-queue/fc-plane-sling2-exterior-v8.out"
export FLIGHTCRAFT_EXTERIOR_PROOF="$HOME/builds/codex-jobs/blender-queue/fc-plane-engine-v8"
cargo +1.98.0 test --offline -j 4 -p flightcraft-render matched_sling_exterior_cameras -- --ignored --nocapture
```

This test reads the exact perspective and orthographic cameras and sun direction from the
Blender receipts, loads the actual pack GLBs, and draws them with the exterior PBR path. It
records the five original views, pure side elevation, and pure top plan. The pack's
`scripts/compare-exterior.py` combines the unmodified PNGs into a self-contained comparison page.
V8 completed in confined Blender 5.2.0 LTS and was accepted for integration. Installed
triangle counts are 101,452 / 18,583 / 4,204. The executed V8 source remains in the queue as `fc-plane-sling2-exterior-v8.py`; the pack source
`scripts/blender-exterior.py` now carries queued V9. The unchanged old generator reproduces older pack revisions.
Installed receipts include the Blender version, source hash, model hashes, bounds, containment,
and camera metadata. The remaining chunky side-on wing root is accepted for now; smoothing
and thinning that fillet is later polish, without changing this accepted revision.

All seven V8 engine captures passed on Vulkan llvmpipe with WebGL2 device limits against the
installed pack, with the exact Cycles cameras, neutral controls, and stopped propeller.
`~/builds/codex-jobs/blender-queue/fc-plane-v8-comparison.html` embeds the seven unmodified
Cycles/engine image pairs side by side. Individual engine PNGs and the byte-identical camera
manifest are in `fc-plane-engine-v8/` beside the Cycles output folder.

The N824RX run is in `fc-plane-engine-v8-registration/`. The actual-GLB registration test
passed on both sides at all three LODs: old baked N371FC ink is erased, new ink appears,
geometry and the other PBR channels stay unchanged, and reset restores the immutable original.
The left elevation and right rear-quarter images were inspected for readable N824RX, with no
overlapping N371FC or colour rectangle. Bounds and moving-part tests also include LOD2 and
follow nested gear groups through to visible descendants.

The direction test probes real aileron trailing-edge vertices, classifies left/right by neutral
body Y, and checks both positive and negative roll at every available LOD of Corvid, Larrow,
and the external Sling. Positive right roll must raise the right edge and lower the left edge.
Both sides must be present in each LOD. The installed bindings already have the correct signs.

Generate the accepted V8 exterior motion proof with the same pack environment:

```sh
export XDG_RUNTIME_DIR=/tmp/fc-plane-runtime
export VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json
cargo +1.98.0 test --offline -j 4 -p flightcraft-render sling_exterior_motion_proof -- --ignored --nocapture
```

`out/plane/motion/` contains 60 unmodified 1280×800 GPU readbacks and a labelled
`contact-sheet.png`. Its first four rows have twelve frames each: aileron, elevator, and rudder
sweep from -100% to +100%; flaps extend from 0% to 100%. Other controls remain neutral.
The last row has two identical stopped-prop frames, eight 300 RPM blade orientations at 1/120 s
intervals, and two 2,000 RPM blur-disc frames. This uses the viewport's actual PBR primitives,
motion transforms, and RPM visibility gate, with fixed lighting. The camera uses Director Wing's
body-space eye (-1, -7, -1.4 m); its aim is centred on the aircraft to retain the tail.
`manifest.json` records the fixed camera, adapter, demands, timestamps, phase, vertex counts,
pixel changes, and actual trailing-edge positions for every frame. Assertions check visible
control movement, a stationary zero-RPM prop, distinct 300 RPM images, and replacement of
the solid blades by the blur mesh. Simulation-to-pose wiring and paused phases have separate
unit coverage; the image sequence supplies explicit poses rather than flying a manoeuvre.

Visible comparison differences: geometry and framing match, including the accepted chunky
wing root. Cycles has cast shadows, a textured blue apron, and a multiple-scattering sky;
the engine proof has an unshadowed flat-grey apron/background and procedural reflection
fallback. Tone mapping and reflected lighting produce different clearcoat highlights and
darker side surfaces. Engine glass uses sorted alpha/Fresnel instead of Cycles transmission
and refraction; cabin contrast differs. Single-sample engine edges show more aliasing.

Run `sling_baked_registration_replaced_on_both_sides_at_every_lod` with `--ignored` and the same
pack environment to repeat the CPU registration check. To capture a custom number, set
`FLIGHTCRAFT_PROOF_REGISTRATION=N824RX` and select a separate proof output directory. Leave
that variable unset for default-livery comparisons.

The engine worktree is now based on main **8995d43**, including merged engine work **82e9ecb**.
The earlier rebase handoff is complete. Canopy changes remain uncommitted in the engine and external
pack worktrees. The accepted V10 installation and proofs are described below. Full workspace CI
remains a pre-commit requirement; hardware-GPU performance has not been measured.

## Forbidden sources

Never download or copy aircraft meshes, trace photographs, distribute POH figures or PDF pages, use
manufacturer logos, read GPL/LGPL/AGPL simulator code, or extract anything from a commercial simulator.
Record every new asset and factual source in ATTRIBUTION.md. Real aircraft remain external packs;
fictional aircraft in this repository must remain original designs.

## V9 sliding-canopy implementation (superseded by V10)

Queue job `fc-plane-sling2-exterior-v9.py` derived from accepted V8. It introduced the split into
fixed windshield and frame, fixed roll-over hoop with inside latch, and moving
`canopy_slide` with glass, bows, side sills and centre exterior handle. Straight rails sit on the
cockpit side skins. Full open is 1.10 m aft; the POH gives no travel, so the pack estimate register
records this and the original rail/bow/handle dimensions.

Validation bounds the continuous swept moving geometry above all relevant fuselage triangles,
including its rear edge, by >3 mm at every LOD. It also requires the open front bow to clear
the seat backs. It writes per-LOD clearance receipts and renders the seven closed views plus
`beauty-open` and `elevation-side-open` with explicit pose in the exact camera manifest.
The installer validates everything before replacing assets and adding the format-3 slide binding
and systems capability. Flight lead ran V9 and requested the two V10 corrections described below;
the installed assets and current proofs use accepted V10.

To repeat the current proofs, set `FLIGHTCRAFT_CYCLES_OUTPUT` and
`FLIGHTCRAFT_EXTERIOR_PROOF` to the V10 folders and run `matched_sling_exterior_cameras` as above.
It captures nine matched engine views, including the two open poses. The existing pack comparison
script places the unmodified open engine and Cycles images alongside each other.
`sling_exterior_motion_proof` adds eight canopy frames to a sixth contact-sheet row (68 frames
total) when the installed binding is present. `sling_canopy_moves_only_its_descendants_at_every_lod`
checks the actual installed GLB descendants at all eight fractions and all LODs. These ignored tests require
the external pack and, for captures, an offscreen GPU.

Engine-only tests cover pose/state wiring, interpolation and stationary fixed nodes without external
assets. Cockpit egui controls use `aircraft.canopy`; a projected physical inside-latch hit target
shares the same command with the overhead control, and dragging controls the open fraction.

## V10 canopy correction and proof refresh

V9 completed cleanly in confined Blender 5.2.0 LTS at 101,041 / 18,631 / 4,826 triangles.
Flight lead accepted the sliding mechanism and requested correction of the hovering exterior
handle and the flat-looking crown. V10 uses one original cubic Bezier meridian for windshield
and slider; the two pieces share their exact seam and tangent. A flush outside handle at
centre-top connects to a narrow moving crown frame strip. The 1.10 m aft travel is unchanged.

The pack source fits every vertex of the whole exterior handle, crown strip, moving bows/sills
and windshield frame to its actual glazing at each LOD. The GLB validator measures every such
vertex against the exported glass triangles with a 3 mm limit and rejects unchecked moving
hardware. Fixed roll-over-bar hardware and slider rails/runners retain their mounting roles.
The existing continuous turtledeck sweep gate remains in force.

Flight lead accepted the completed confined Blender 5.2 run. V10 is now installed in the external
pack as format **3**, version **0.3.0**, with **102,411 / 19,023 / 4,837 triangles**. Its source SHA-256
is `1a0747181cf824794f5c54c9d3afbef73bd1cb672254b7cb5c5dd748bd4c74ee`.
All exported hardware vertices pass the 3 mm gate (maximum 2.507 mm); continuous turtledeck
clearances are 23.648 / 23.097 / 24.575 mm at LOD0/1/2. Travel remains 1.10 m aft.

`scripts/finalize-sling-exterior.py` completed the install and proof refresh in the foreground.
All three actual-pack proof tests passed: canopy descendants move aft at every LOD while other
nodes remain fixed; all nine exact camera/pose pairs were captured; and the motion proof contains
68 frames with eight closed-to-open canopy poses in its sixth row. Captures used the actual PBR
renderer on Vulkan llvmpipe (Mesa 26.0.8); this is a rendering proof, with no hardware-GPU
performance measurement. The open beauty/side views and contact sheet were visually inspected.

Current artifacts in `~/builds/codex-jobs/blender-queue/`:

- `fc-plane-engine-v10/`: nine engine PNGs and the exact camera manifest.
- `fc-plane-v10-comparison.html`: all nine unmodified Cycles/engine pairs.
- `fc-plane-v10-proof.json`: completed phases, source/asset/image hashes, hardware/clearance
  receipts and both worktrees' changed files; status `installed_and_captured`, pending list empty.
- `fc-plane-v10-engine-changed-files.txt`, `fc-plane-v10-pack-changed-files.txt`, and combined
  `fc-plane-v10-changed-files.txt`: final uncommitted file lists.

The motion PNGs, pose/pixel manifest and `contact-sheet.png` are in engine `out/plane/motion/`.
To repeat the complete synchronous pipeline, run queue `fc-plane-v10-engine-proof.sh` against
the completed accepted output. Both repositories remain uncommitted for flight lead's merge.
