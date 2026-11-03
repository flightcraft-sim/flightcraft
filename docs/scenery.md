# Public-data scenery and the local 3D Tiles provider

This implements the first acquisition and rendering gate of
[the source plan on main](https://github.com/flightcraft-sim/flightcraft/blob/377c5c026576d2c27f222c2dfaa4b5825992e423/docs/scenery-sources.md).
It is **not yet the completed photoreal golden sample**. Generated imagery and terrain, imported
city models, and future streaming adapters all use OGC 3D Tiles 1.1 with glTF 2.0 content.
There is no second proprietary scenery tile format.

Implemented: bounded/resumable acquisition; receipt-verified offline imagery pyramids; windowed
float GeoTIFF/COG elevation sampling; explicit 3D Tiles quadtrees; embedded lossless WebP GLBs;
worker decoding; geometric-error selection; CPU and GPU parent fallback; bounded uploads; local
provider preferences; and offscreen reference previews. Existing simulation ground, synthetic
vision, planning maps, charts, and clouds retain their paths. Imagery is presentation data and does
not change replay identity or collision elevations.

Not accepted yet: independent painted-runway alignment, vertical-datum conversion, measured lidar
trees and roofs, NHD water geometry, physical sun/sky, cascaded shadows, exposure/bloom, the Water
Gap proof, automatic installed-region selection, browser scenery fetching, or the 60 fps integrated-GPU acceptance run. Publication rejects
scenery without the explicit vertical-datum, runway-alignment, and lidar acceptance flags.

## First fetch: 1N7 test area

Run from this checkout on the networked flight-lead machine. Each command uses the external target,
four build jobs, an 8 GB memory scope, and the UI's libudev shim; run one command at a time. `cargo xtask` is the repository
alias for `cargo run --quiet -p xtask --`.

```sh
CARGO_TARGET_DIR=/home/ray/builds/cargo-target/flightcraft/fc-scenery CARGO_BUILD_JOBS=4 PKG_CONFIG_PATH=$HOME/builds/pkgconfig-shim capped 8G -- cargo xtask scenery fetch --region eastern-pa --area 1n7-test
```

Default files land on the build SSD, outside the checkout:

```
/var/mnt/inland/builds/codex-jobs/scenery/eastern-pa/1n7-test/cache/
  sources.json
  objects/<sha256-of-url>
  objects/<sha256-of-url>.json
  objects/<sha256-of-url>.part          # only during interrupted downloads
  objects/<sha256-of-url>.partial.json
```

Please return this directory and `sources.json` after fetching. No account, key, or payment is used.
An interrupted command is rerun unchanged. Completed objects are hashed against receipts; interrupted
objects resume only with an ETag or Last-Modified validator and a matching Content-Range. A server
returning 200 restarts the object. Corrupt completed objects fail instead of silently replacing a
reviewed source. To refetch one rejected response, remove its URL-keyed object and receipt together.
Each HTTP request starts at least one second after the previous request started. Source objects are
streamed to disk, capped at 2 GB apiece; an oversized USGS product needs a smaller upstream selection,
not an increased in-memory allocation.

Fetch checks NAIP `maxImageWidth` and `maxImageHeight`, requests 64 separate 512×512 JPEGs, and uses
fixed EPSG:3857 pixel-area bounds with `adjustAspectRatio=false`. TNM discovery is paginated and
receipted, first for 1 m DEM GeoTIFFs, then classified LPC LAZ files. It downloads intersecting products,
including overlapping editions; review the discovery receipts before expanding the area. A missing
1 m DEM is an error. Missing lidar remains visible in the receipts and cannot pass release acceptance.
The initial importer reads geographic WGS84/NAD83 (including NAD83(2011)) or UTM zone 18 north, meter units, float32, north-up
GeoTIFFs; unsupported CRS, sample types, compression, missing coverage, and nodata fail explicitly.
NAD83 frame transformations still need the independent alignment gate. LAZ acquisition is implemented;
LAZ decoding, EPT hierarchy selection, classification filtering, canopy/roof reconstruction, and geoid
conversion remain next steps once these local inputs are available. No footprint fallback is included.

## Offline build

```sh
CARGO_TARGET_DIR=/home/ray/builds/cargo-target/flightcraft/fc-scenery CARGO_BUILD_JOBS=4 PKG_CONFIG_PATH=$HOME/builds/pkgconfig-shim capped 8G -- cargo xtask scenery build --region eastern-pa --area 1n7-test
```

Use `--cache <directory>` if flight lead fetched elsewhere. `--out <new-directory>` selects output;
otherwise it is the area's adjacent `built/`. Inputs are never fetched by build. Output is staged in
`<out>.building`, and the completed directory is renamed into place. Existing outputs are refused.
A failed build removes its staging directory. A process killed during build may leave `.building`;
remove that directory before rebuilding. Download resumption is independent of build staging.

Output contains `tileset.json`, `tiles/<level>/<x>/<y>.glb`, `sources.json`, and `build-report.json`.
The 1N7 test has 85 content nodes across four levels. GLBs contain original terrain triangles,
UVs, and embedded lossless WebP via `EXT_texture_webp`. Worker decoding creates the complete mip
chain before GPU upload. Parent imagery is filtered from its four children. A mild global levels
adjustment lifts dark tones; it does not recover pixels hidden by baked shadows or remove all haze.

Elevation is sampled from bounded TIFF chunks (at most four cached chunks per DEM, with a global cache reset above 64 MiB). Bilinear sampling
honors pixel-area versus pixel-point registration and rejects nodata. Mesh decimation is independent
of the 1 m source spacing. Current preview normals are reconstructed per triangle in the content loader.
The existing simulation DEM stays unchanged. **Orthometric DEM heights are currently previewed as
ellipsoidal heights in GLB; `verticalDatumVerified` remains false.** This keeps MSL height readable in
the local preview but is not an accepted geodetic export. Resolve the NAVD88/WGS84 conversion before
publishing or combining these tiles with external georeferenced content.
The current footprint mask suppresses the procedural runway only. Real-DEM validation must also
resolve fallback-terrain occlusion wherever the new surface lies below the coarse base mesh; the
level reference preview does not exercise that case.

The larger presets exist for later acquisition: `1n7` (0.3 m), `water-gap` (0.6 m), and `surroundings`
(1.2 m). Square power-of-two roots include padding: respectively about 19.66, 39.32, and 78.64 km
across. They cover the requested airport, corridor, and surrounding envelopes, with substantial
extra fetch outside them. They are not yet corridor-clipped or validated for the big fetch. Prove
`1n7-test` first. One `new-york` or `eastern-pa` region/area is built per command.

## Local provider and the follow-up provider lane

`crates/render/src/scenery/` is the single content/streaming path. The provider descriptor has
`location: {kind: local-path|url, value: ...}`, `enabled` (false by default), and optional
`token_config_key`. That key names a secret in user configuration; it is never the token itself.
Do not put credential values in descriptor URLs, tilesets, GLBs, source receipts, logs, or git.

`TileProvider: Send` supplies bounded tileset bytes and content. `Content` carries bytes, attribution,
cache policy (`LocalData` or `MemoryOnly`), and an optional UTC expiration in milliseconds. `LocalProvider` is the
only implementation. It resolves files beneath the tileset directory, checks canonical containment,
and rejects traversal, absolute content paths, URL content, and oversized files. The runtime has no
HTTP client and no network cache. The initial stream **fails closed** on memory-only content,
expiration, or additional per-content attribution; the follow-up lane must implement expiry and
attribution propagation through CPU and GPU residency before enabling such a provider. This prevents
the local cache behavior from accidentally applying to restricted streaming sources.

An online adapter should own token lookup, request authorization, source-specific endpoint resolution,
quotas, retries, and attribution. It should normalize external content URLs to opaque relative GLB identifiers, resolving the
authorized URLs privately, and pass the same validated tileset and GLB payloads to the renderer, replace expired tiles with valid parent coverage, and purge expired GPU content. It must
not call the acquisition/release cache for streamed data. No Cesium, Google, Esri, TUM, or Geopipe
adapter, token storage implementation, key UI, or source redistribution is included in this lane.

Native adapters can implement `TileProvider` on the decoding worker. Event-loop adapters can
construct `Stream::new`, drain the bounded `Selection.requests` list, asynchronously obtain/decode
the requested content using its inherited transform, and call `insert` or `reject`. `content_request`
also exposes read-only metadata by request ID. These are the same selection and GLB paths, with no
browser transport implemented here. Expiration is expressed as UTC milliseconds rather than a
platform-specific clock type. Remote adapters must still implement the policy gates above.

Initial supported content profile: explicit 1.1 trees; region, box, or sphere volumes; inherited
ADD/REPLACE refinement and affine transforms; embedded GLB 2.0 indexed triangle geometry; node
transforms; one opaque material and one embedded PNG/JPEG/WebP base-color image per GLB; no sparse
accessors, animation, skins, Draco, external image resources, b3dm, external nested tilesets, or
implicit subtrees yet. Unsupported content fails with base terrain still available. Imported city
and photogrammetry datasets need conversion to this profile or extensions to the same loader.
Transforms use doubles through ECEF and the existing local transverse Mercator, then convert to GPU
floats. glTF Y-up to 3D Tiles Z-up is explicit; no ECEF coordinates are uploaded as single-precision
positions. Provider selection does not alter simulation state.

Preferences → Scenery selects Low, Medium, or High and a local `tileset.json`; choose Apply scenery
after editing the path. Settings stay in the user's normal eframe preferences. CPU residency budgets
are 48/96/192 MiB; at most two decode requests are pending, and one/two/two GPU tiles upload per frame.
Replacement parents stay visible until children are decoded **and** uploaded. The browser clamps
High to Medium; fetching a smaller static showcase tileset in the browser is still pending. Quality
budgets are resource limits, not measured guarantees of 60 fps. Local attribution is drawn over the
view. Generated terrain suppresses the procedural runway overlay within its rendered footprint;
synthetic vision keeps its independent runway geometry.

## Proof and handoff

A reference-only build can be reproduced without any downloads:

```sh
CARGO_TARGET_DIR=/home/ray/builds/cargo-target/flightcraft/fc-scenery CARGO_BUILD_JOBS=4 PKG_CONFIG_PATH=$HOME/builds/pkgconfig-shim capped 8G -- cargo xtask scenery build --region eastern-pa --area 1n7-test --sample /home/ray/builds/codex-jobs/1n7-naip-sample.jpg --out /var/mnt/inland/builds/codex-jobs/scenery/eastern-pa/1n7-test/reference-built
CARGO_TARGET_DIR=/home/ray/builds/cargo-target/flightcraft/fc-scenery CARGO_BUILD_JOBS=4 PKG_CONFIG_PATH=$HOME/builds/pkgconfig-shim capped 8G -- cargo test -j 4 -p flightcraft-ui render_1n7_scenery_reference_shots -- --ignored --nocapture
```

`--sample` is restricted to the supplied 1600×860 reference and documented geographic bbox. It is
resampled, coarser than 0.3 m, and uses a level 112 m surface; `previewOnly` is true. Its receipt
identifies the supplied reference and unverified retrieval date. `out/scenery/` contains Runway 07,
short-final Runway 25, and hangar flyby captures, plus side-by-side NAIP references. The right panel
is overhead NAIP, not an independent ground-level photograph. No Water Gap proof is fabricated from
this airport-only sample. Set `FLIGHTCRAFT_SCENERY_TILESET` to compare another built test tileset.

The registration test checks independently sourced FAA endpoints through Mercator pixels, ECEF, and
local TM to millimeter tolerances. **It is a numerical registration test, not proof that painted runway
pixels align within 2 m.** Independently pick both painted runway ends in the fetched full-resolution
imagery and record the residuals before setting `runwayAlignmentVerified`. The sample's export aspect
ratio and original image uncertainty prevent it from satisfying that acceptance gate.

The ignored `scenery_1080p_frame_budget` test warms a Low scene, measures 120 synchronized GPU frames,
writes adapter and p95 timing to `out/scenery/frame-budget.json`, and requires p95 ≤ 16.67 ms. Run it
on the target integrated GPU, with no competing GPU workloads. Software rendering is not evidence
for the integrated-GPU requirement. Dense final lidar content requires another acceptance run.

The current reference proof passed all three image comparisons and GPU validation. The warmed
1080p Low preview measured **4.10 ms p95 on an NVIDIA GeForce GTX 1660 Ti**. This is a discrete GPU
and a level, reference-only airport surface; it does **not** satisfy the integrated-GPU or final
scenery acceptance criteria. Run GPU tests serially (`--test-threads=1`) to avoid timing interference.

Exact real-window shots requested from flight lead after fetched geometry is installed:

1. Pilot eye on Runway 07 at 1N7, 25 m beyond its FAA threshold, looking along true heading 60.7666°.
2. Short final Runway 25, 100 m outside its threshold, 75 ft AGL, looking along true heading 240.7730°.
3. Camera 40.9855956° N, 75.1221222° W, 2,500 ft MSL, true heading 180°, looking 8° down at the
   GNIS gap point (40.9675956° N, 75.1221222° W), 60° vertical FOV (pending corridor data).
4. Director view over the 1N7 hangars: aircraft NED [230, 320, −145] m, observer NED [150, 500, −195] m
   relative to the FAA Runway 07 origin, 51.57° vertical FOV. This is about 108 ft aircraft AGL and
   272 ft observer AGL on the level preview. Also take an unobstructed aircraft frame for the exterior lane.

Use the same date, time, visibility, FOV, and quality for comparisons. The pipeline preview currently
uses the existing day lighting; physical date/time lighting is a remaining gate. Aircraft realism is
owned by the exterior lane, and no aircraft assets change here.

## Releases and checks

Install accepted output under `assets/terrain/<region>/scenery/<area>/`, then use the existing
`cargo xtask data publish` flow. Scenery binaries are ignored by git. Publish groups sorted file/tile
ranges into at most 128 MiB raw archives with individual manifests; archives remain below the existing
256 MiB verified transport cap and GitHub's 2 GB limit. `data.lock` gets dataset `scenery` entries with
per-file and per-archive SHA-256. No lock entry is invented for data that has not passed acceptance.
Web packaging's existing 1 GB total-site check remains in place; a reduced web scenery selection is
required before installing the large presets into a Pages showcase.

Generated schemas: `tileset.json`, `scenery-request.json`, `scenery-sources.json`, `scenery-report.json`,
and `scenery-provider.json`. Tests cover hostile tilesets/GLBs, bounded image decoding, resume-range
validation, DEM registration/nodata, quadtree completeness, geometry registration, and parent coverage.
Shader tests translate both scene shaders to WebGL 2 GLSL. Direct render dependencies on `image`,
`gltf`, `serde`, and `schemars` reuse existing workspace versions; no new third-party package is added.
