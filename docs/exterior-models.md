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

The Sling pack carries its own generator and real-aircraft geometry source. From that pack, run
`python3 scripts/generate-exterior.py --sling-out .`. The engine repository does
not embed a real aircraft. Sling dimensions and construction facts come from the external POH
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

The initial profile supports triangle primitives, float POSITION/NORMAL attributes, indices, and
material base colors. Texture count is capped at **zero**; image, external URI, sparse accessor,
skin, morph-target, and embedded-animation assets are rejected. Registration is geometry lettering,
so it needs no font or texture. Keep GLBs below 4 MiB, 128 nodes, 90,000 expanded vertices, and 30,000
triangles each. LOD1 must reduce the triangle count. The fictional models use about 17,300 triangles
at LOD0 and 3,000 at LOD1; Sling uses about 22,100 and 3,700, respectively. LOD1 is selected beyond 150 meters from the camera.

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

Use the `fc-exterior` target cache and four build jobs, as required by the working rules:

```sh
export CARGO_TARGET_DIR="$HOME/.cache/buildq/target/flightcraft/fc-exterior"
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

## Forbidden sources

Never download or copy aircraft meshes, trace photographs, distribute POH figures or PDF pages, use
manufacturer logos, read GPL/LGPL/AGPL simulator code, or extract anything from a commercial simulator.
Record every new asset and factual source in ATTRIBUTION.md. Real aircraft remain external packs;
fictional aircraft in this repository must remain original designs.
