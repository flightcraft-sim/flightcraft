# Making an aircraft exterior

The renderer accepts bounded, self-contained glTF 2.0 binary models (`.glb`) from pack format 2.
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
triangles, and 360,000 expanded vertices. It may contain at most 8 embedded PNG images and
64 texture references. Images are limited to 4096² and 320 MiB total decoded RGBA storage per
model. Every PNG header and binary view is checked before decompression; the decoder also has
allocation limits. External images, sparse accessors, skins, morph targets, and embedded animations
are rejected. Packs have a separate 128 MiB encoded limit. GPU uploads deduplicate shared images
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
its existing soft ground contact shadow remains in use. Exterior models are omitted in cockpit view,
which keeps the existing cockpit geometry and panel. Wing view uses a 0.015-meter near plane.

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
triangle counts are 101,452 / 18,583 / 4,204. The source remains the pack's
`scripts/blender-exterior.py`; the unchanged old generator reproduces older pack revisions.
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

Flight lead owns the rebase onto main **460784d** (Rust 1.98, scenery/city providers, and web
packaging). Git metadata is read-only here; no rebase or commit was performed. Keep main's
callsign persistence and UI calls to `Scene::set_registration`. Merge the atlas branch and
optional LOD2 into main's existing `registration.rs`, including the PBR vertex fields in its
legacy geometry path and glyph visibility for the sibling atlas painter. Preserve main's
scene/scenery fields while adding the exterior environment field, PBR draws, and lamp gates.
The small `mesh.rs` lint change is byte-identical to main 01745d1 and needs no further change
on main. Do not replace main's Cargo manifests or lock wholesale; merge the GLTF extension
feature and image dependency additions.

Unverified here: the combined main-460784d/scenery build after rebase, packaged desktop and
browser real-window captures, and hardware-GPU performance. The wasm check covers compilation,
not browser visuals. The stale baseline UI float-literal warnings are fixed on main and left
for the rebase. Full workspace CI remains a pre-commit check for flight lead after rebasing.
Render/control wasm compilation, engine aero/render Clippy, and pack-harness Clippy passed
with Rust 1.98; Clippy used `-D warnings`. The seven engine stills remained byte-identical
after sharing the capture helper with the motion proof. `cargo deny check licenses` could
not run because this environment has no `cargo-deny` subcommand installed.

## Forbidden sources

Never download or copy aircraft meshes, trace photographs, distribute POH figures or PDF pages, use
manufacturer logos, read GPL/LGPL/AGPL simulator code, or extract anything from a commercial simulator.
Record every new asset and factual source in ATTRIBUTION.md. Real aircraft remain external packs;
fictional aircraft in this repository must remain original designs.
