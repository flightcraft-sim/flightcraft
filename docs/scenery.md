# Public-data scenery and the local 3D Tiles provider

This implements the first acquisition and rendering gate of
[the source plan on main](https://github.com/flightcraft-sim/flightcraft/blob/377c5c026576d2c27f222c2dfaa4b5825992e423/docs/scenery-sources.md).
It is **not yet the completed photoreal golden sample**. Generated imagery and terrain, imported
city models, lidar-derived buildings, and streaming adapters all use OGC 3D Tiles 1.1 with glTF 2.0 content.
There is no second proprietary scenery tile format.

Implemented: bounded/resumable acquisition; receipt-verified offline imagery pyramids; windowed
float GeoTIFF/COG elevation sampling; explicit 3D Tiles quadtrees; embedded lossless WebP GLBs;
worker decoding; geometric-error selection; CPU and GPU parent fallback; bounded uploads; local
provider preferences; and offscreen reference previews. Existing simulation ground, synthetic
vision, planning maps, charts, and clouds retain their paths. Imagery is presentation data and does
not change replay identity or collision elevations.

Not accepted yet: independent painted-runway alignment, vertical-datum conversion, measured lidar
trees and roofs, NHD water geometry, physical sun/sky, cascaded shadows, exposure/bloom, the Water
Gap proof, automatic installed-region selection, or the 60 fps integrated-GPU acceptance run.
Static browser 3D Tiles fetching is implemented; it still needs real NYC package acceptance.
Publication requires datum and runway acceptance, and measured lidar reconstruction. Fly NYC additionally requires the reviewed 3DEP-only source.

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

## Providers

`crates/render/src/scenery/` remains the single OGC 3D Tiles 1.1 / embedded GLB path.
The unchanged provider descriptor uses `location: {kind: local-path|url, value: ...}`, an
explicit `enabled` flag (false by default), and an optional `token_config_key` naming a secret
in user preferences. Descriptor URLs never contain keys. `LocalProvider` accepts installed
or user-fetched tilesets beneath a canonical local directory; the Esri adapter accepts only
its fixed World Imagery endpoint and the config-key name `esri-imagery`.

`TileProvider: Send` returns bounded tileset bytes and `Content` containing GLB bytes,
attribution, cache policy, and optional UTC expiration. Both `LocalData` and `MemoryOnly` now
pass through the same decoder. The worker requires attribution for memory-only content,
propagates content credits and expiration into CPU residency, and reports provider failures
with the selection. CPU eviction uses last-selected LRU order within the existing quality
budgets. GPU residency follows valid CPU payload identities; switching providers, eviction,
and expiration purge obsolete GPU tiles. Valid parents cover children while replacements load;
when all coverage expires, the base scene remains available. Expired credits leave with their
content. No HTTP work runs inside the frame callback.

### Fly NYC: lidar-only shipped building provider

**NYC Open Data is not an approved open license.** The earlier statement that its terms permitted
redistribution based on flight lead's review was wrong and has been removed. The
[portal terms](https://www.nyc.gov/html/data/terms.html) bind users to NYC.gov's terms and additional
supplying-agency terms. Flight lead reports historical terms reserving the City's right to stop
"any and all display, distribution or other use" for any reason, and OpenStreetMap's rejection of
NYC data in November 2011 on those grounds. We have no clean redistribution permission for the
portal footprints. They and TUM are **local, user-fetched only**, never website or release data.

**Shipped Fly NYC buildings come only from USGS 3DEP classified lidar.** The public-domain source is
<https://s3-us-west-2.amazonaws.com/usgs-lidar-public/NY_NewYorkCity/ept.json>.
Flight lead supplied its metadata: 4,755,025,996 points, EPSG:3857, with Classification, and cube
bounds `[-8267099, 4937860, -31153, -8204119, 5000840, 31827]` (xmin, ymin, zmin, xmax, ymax, zmax).
The padded cube is not a ground-elevation or vertical-datum measurement. No local source download,
reconstruction, or real-data visual proof is claimed by this change.

The machine-readable [area definition](../assets/scenery/fly-nyc.json) fixes the selection to
**40.66–40.84° N, 74.04–73.90° W**, building classification **6**, a **250,000,000-byte** building
allocation, and the shared **1,000,000,000-byte** total website ceiling. It also records the exact
EPT project and source metadata above. A regression test keeps this file equal to the provider's
constants. The envelope covers Manhattan and NYC shoreline visible from the Hudson/VFR corridor;
it does not promise New Jersey or citywide airport buildings.

**Ownership:** fc-scenery acquires selected EPT nodes, decodes LAZ, clusters classified building
points into footprints, fits roof planes, and reconstructs measured meshes. This lane consumes
that output, selects the NYC area, generates area tiles and LODs, enforces the size allocation,
and supplies native/browser providers. It does not duplicate EPT acquisition or reconstruction,
and it never substitutes Socrata/TUM footprints or height extrusions into the shipped layer.

#### Reconstruction handoff through the provider contract

Use the existing [provider schema](../schema/scenery-provider.json) and OGC 3D Tiles/GLBs. The
additional [lidar provenance schema](../schema/scenery-nyc-lidar.json) describes
`tileset.extras.flightcraft.nycLidar`; it is metadata, not another scenery mesh format.
The [area schema](../schema/scenery-nyc-area.json) describes the selection file.

fc-scenery supplies a bounded local 3D Tiles tree with an explicit ENU-to-ECEF root transform.
Each content leaf contains **one complete reconstructed building GLB**; grouping nodes have no
content. This lane generates LODs. The source meshes must already include the reviewed source
CRS, units, vertical correction, and reconstruction; raw EPSG:3857 point positions are not accepted
as glTF coordinates. Roof geometry is preserved at the finest output LOD, and source materials
are replaced by original PBR wall/roof colors.

`extras.flightcraft` must contain `region: "new-york"`, `area: "nyc-lidar"`,
`sourceKind: "usgs-3dep-lidar"`, `roofPolicy: "lidar-only"`, `localOnly: false`,
`redistributionAllowed: true`, and `lidarBuilt: true`. `nycLidar` contains:

- `selection`: the complete object from `assets/scenery/fly-nyc.json`.
- `reconstructionBuildings`: the number of upstream building leaves, checked against the input.
- `reconstructionLeavesAreBuildings: true`: describes the upstream reconstruction, not the final
  batched LOD tiles. Output may combine multiple complete buildings in one area tile.
- `usesExternalFootprints: false`: no NYC portal, TUM, or other footprint dependency.
- `receipts`: acquisition URL, edition, retrieval date, byte count, and SHA-256 records. At least
  the EPT metadata and classified LAZ node receipts are required. Every URL must be beneath the
  exact `https://s3-us-west-2.amazonaws.com/usgs-lidar-public/NY_NewYorkCity/` prefix. Portal/TUM
  receipts, missing provenance, and external footprint declarations fail closed.

`verticalDatumVerified`, `runwayAlignmentVerified`, and `previewOnly` carry fc-scenery's acceptance
state. Unaccepted input produces a preview, never a release. This lane does not turn reconstruction
or source rights into a claim of FAA/datum accuracy. The browser and release validator enforce the
same permanent source restrictions; even older portal builds with stale accepted flags are refused.

Create the standard provider descriptor at
`/var/mnt/inland/builds/codex-jobs/scenery/new-york/nyc-lidar/cache/reconstruction-provider.json`:

```json
{
  "enabled": true,
  "location": {"kind": "local-path", "value": "/path/to/fc-scenery/nyc-buildings/tileset.json"}
}
```

Replace the example path with fc-scenery's real reconstructed provider. Relative paths resolve
from the descriptor directory. Credentials and URL providers are refused by this offline consumer.
Once that provider exists, run one command at a time:

```sh
export CARGO_TARGET_DIR=$HOME/builds/cargo-target/flightcraft/fc-cities
export CARGO_BUILD_JOBS=4
export PKG_CONFIG_PATH=$HOME/builds/pkgconfig-shim
export CCACHE_DIR=/tmp/fc-cities-ccache
capped 8G -- cargo xtask scenery build --region new-york --area nyc-lidar
```

There is no EPT fetch command in this lane: `scenery fetch --area nyc-lidar` explains the handoff
and refuses to pretend to acquire or reconstruct data. Use fc-scenery's acquisition/reconstruction
workflow when available; no unimplemented fetch command is presented as working.

The consumer includes whole buildings whose centers fall inside the fixed envelope, reports
excluded buildings, rejects duplicate content/count mismatches, and converts input GLBs into the
new-york region frame in double precision. It spools bounded batches to temporary scratch and
processes at most 20,000 triangles per area batch. Scratch is removed before success. Fine and
2 m clustered geometry form replacement LODs. Coarse skyline proxies at parent levels preserve
coverage while detail arrives. These distant proxies are approximations derived from the measured
mesh; they are not portal footprint or invented-height fallbacks. Shared position/UV vertices and
16-bit indices where possible reduce download size without changing fine geometry or flat normals.

`build-report.json` counts buildings, excluded buildings, tiles, and **all package bytes**, including
LODs, provenance, area definition, and the report. Build fails above 250 MB; geometry is never
silently dropped to fit. The remainder of the 1 GB allocation is shared with imagery, terrain,
charts, and application files; actual full-site packaging must still fit. No budget compliance is
claimed for the real data until flight lead runs the pipeline.

Use `built/tileset.json` locally for the offscreen/real-window checks below. Once fc-scenery's input
has passed datum and FAA acceptance, rebuild to a new directory with `--out`, install only that
accepted output, and run the website check:

```sh
capped 8G -- cargo xtask scenery build --region new-york --area nyc-lidar --out /var/mnt/inland/builds/codex-jobs/scenery/new-york/nyc-lidar/accepted
mkdir -p assets/terrain/new-york/scenery
cp -a /var/mnt/inland/builds/codex-jobs/scenery/new-york/nyc-lidar/accepted assets/terrain/new-york/scenery/nyc-lidar
capped 8G -- cargo xtask web --check
```

The install destination must be absent. Keep local Socrata and TUM providers outside the website
source tree. Release publication and website packaging refuse both, irrespective of geometric
acceptance flags. No data release lock entries are invented here; flight lead owns publishing.

The packaged browser URL is `?region=terrain/new-york/region.json&scenery=nyc`. It selects
`terrain/new-york/scenery/nyc-lidar/tileset.json`, leaving Esri off. The same-origin adapter refuses
credentials, redirects, path traversal, local-only sources, and incorrect NYC lidar provenance.
It fetches at most two selected GLBs concurrently and uses the existing decoder, CPU/GPU budgets,
and WebGPU/WebGL 2 renderer. Website packaging includes scenery in the complete-site size check.

### NYC portal footprints: local user-fetched provider only

The Socrata fetch and footprint/height converter remain available under `--area nyc-footprints`
for a user who independently accepts applicable [NYC.gov and agency terms](https://www.nyc.gov/html/data/terms.html).
**Do not run that fetch for the shipped Fly NYC workflow.** Local-only classification does not
assert that every local use is permitted. No portal geometry, building heights, roof shapes, or
derived building footprints may enter the shipped lidar reconstruction.

An optional local fetch still validates dataset `nqwf-w8eh`, discovers geometry/roof/ground fields,
requests bounded 1,000-row pages in stable ID order, detects mid-fetch revision changes, and retains
resumable SHA-256 receipts. Build still requires `footprint-review.json` based on the fetched
`footprint-metadata.json` and example review: documented `height_unit` (`feet`, `us-survey-feet`, or
`meters`), `roof_reference` (`absolute` or `above-ground`), NAVD88 evidence, and a reviewed
`presentation_height_offset_m`. No units or heights are guessed. It preserves concavity/courtyards,
uses flat-roof extrusions as the explicit local fallback, and records missing/invalid-height rows.
Any measured input combined with portal footprints remains portal-derived and local-only.

Every fetch receipt manifest and build report has `local_only: true` and
`redistribution_allowed: false`; tilesets have `localOnly: true`, `redistributionAllowed: false`,
and `previewOnly: true`. `release_ready` is always false, even if datum/FAA review flags pass.
Publication, website packaging, and the static browser adapter reject this source. The permanent
area restriction also rejects older outputs created before this licensing correction.

### NYC LoD2 local provider (secondary)

Flight lead verified the current [TUM project page](https://www.asg.ed.tum.de/gis/projekte/new-york-city-3d/):
the output is CityGML LoD2 and the buildings archive is about 2.4 GB zipped. The page says Open Data
but states **no explicit license**. TUM remains **local, user-fetched only, never shipped or included
in data releases**, until redistribution permission is confirmed. Attribute the Chair of
Geoinformatics, Technical University of Munich, and its NYC Open Data derivation. Geopipe is skipped.

```sh
capped 8G -- cargo xtask scenery fetch --region new-york --area nyc-lod2 --allow-http
capped 8G -- cargo xtask scenery build --region new-york --area nyc-lod2
```

This is explicit consent to plain HTTP for exactly
`http://www.3dcitydb.net/3dcitydb/fileadmin/public/datasets/NYC/`.
The fixed fetch is
`NYC_buildings_CityGML_LoD2/NYC_Buildings_LoD2_CityGML.zip` under that prefix. No redirect, alternative
host, encoded path escape, older `/mydata/` archive, or street-space archive is permitted. The
streamed TUM-only archive cap is 8 GB; USGS remains capped at 2 GB. HTTP cannot authenticate origin:
receipts detect subsequent local changes but do not prove transport authenticity.

ZIP/ZIP64 central-directory metadata is bounded before archive allocation. Members are not extracted;
XML is read incrementally, with a 2 MiB event limit, 16 MiB per-building fragment limit, depth limit,
and 128 GB total declared expansion cap. DTDs and unresolved/external LoD2 geometry links are
rejected. Only inline LoD2 building surfaces are converted. Roof planes, concave polygons, and holes
are retained; source materials/textures are discarded. Area batching and output use the same
3D Tiles/GLB pipeline as NYC footprints. Actual TUM archive compatibility remains to be proven.

`srsName` is required and parsed from coordinates/ancestors or the GML Envelope. Supported explicit
identifiers are EPSG:2263 and 6539 (NY Long Island US survey feet), EPSG:32118 (meters), EPSG:4326
and 4979 (official latitude/longitude order), and OGC:CRS84 (longitude/latitude). Recognized OGC URN
and definition-URL spellings are accepted; compound URNs may pair the horizontal CRS with EPSG:5703
(NAVD88 meters) or EPSG:6360 (NAVD88 US survey feet). Unknown CRS, vertical CRS, axis order, or height
units fail closed. The inverse NY Long Island Lambert projection is implemented in Rust; no
external GIS executable is needed. Bare projected CRS heights follow source CRS units unless
explicit vertical units are present; this convention is recorded as unverified, not a datum proof.
No NAD83/WGS84 realization correction or geoid separation is invented. The source CRS list is in
the report. Every TUM build has `localOnly: true`, `redistributionAllowed: false`, `previewOnly: true`,
and false acceptance flags. Publication and website packaging reject it.

### NYC visual acceptance and real-window shots

Use Medium quality, 60° vertical FOV, clear daytime conditions, and identical scene settings.
Run the offscreen GPU test below first. Flight lead must capture these real-window views:

1. Hudson corridor: 40.7100° N, 74.0350° W, **1,000 ft MSL**, true heading 000°, level view.
2. Manhattan skyline from the VFR corridor: 40.6750° N, 74.0450° W, **1,000 ft MSL**, true heading
   010°, level view. Check recognizable skyline heights and silhouettes. Check measured roof detail and distinctive silhouettes; distant rectangular proxies must
   transition cleanly. No portal-footprint or guessed-height substitutes may appear.
3. Hudson shoreline: 40.7600° N, 74.0150° W, **1,000 ft MSL**, true heading 180°, 15° down.
   Inspect NYC bank/river registration, courtyard holes, base elevations, area seams, and LOD changes.
4. **KEWR, KLGA, and KJFK** separately: 2,500 ft MSL, true north up, 60° down. Compare independently
   sourced imagery/FAA threshold positions against terrain and buildings where the dataset covers
   them; record horizontal and vertical residuals. The small shippable showcase envelope does not
   cover those airport buildings. Use the full local TUM source for its airport-specific proof,
   and do not claim absent showcase buildings establish airport alignment.
5. Browser at 1080p on WebGPU and WebGL 2: repeat corridor/skyline shots, cold and warm navigation,
   including LOD transitions and memory-pressure fallback. Record frame times, resident bytes,
   total packaged bytes, and browser network requests. No single screen capture proves 60 fps.

The renderer currently exposes geodetic heights as presentation MSL; proper ellipsoidal source
heights can disagree with terrain. Datum registration must be reviewed across the actual showcase
and airport references before setting the verified flag. Numerical fixture round trips prove math,
not real dataset accuracy. No real-window proof or real NYC image is claimed in this sandbox.

### Streaming imagery (your own key)

The native Settings → Scenery field is masked and defaults off. Enable it and choose Apply scenery;
the imagery can be used with a local city tileset. Both layers share the same GLB renderer, with
separate content-ID namespaces and bounded CPU budgets per provider. Browser HTTP transport remains
a follow-up. Browser builds work and show
no native-key control. The adapter sends the key in a sensitive Authorization header to exactly:

```
https://ibasemaps-api.arcgis.com/arcgis/rest/services/World_Imagery/MapServer
```

The adapter generates a bounded regional 3D Tiles quadtree, requests only selected content through
its decoding worker, samples read-only terrain, and encodes each 256-pixel image as an ordinary
terrain-draped GLB. Heights are bounded from the read-only source grids and runway elevations so
geometric-error selection uses terrain-sized volumes. Maximum zoom is 15, with a 40,000-node tree
limit. It fetches current service
copyright metadata before the first image and displays it with the Esri credit. Missing attribution
fails closed. Keys exist only in user preferences and transient transport state, never in URLs,
provider descriptors, records, source receipts, screenshot metadata, or error messages. Response
bodies and transport errors are not echoed; redirects are disabled. Test credentials are invented.

There is no disk cache, download command, release manifest, or offline export for this adapter.
Decoded imagery uses only bounded session memory with LRU eviction and a conservative five-minute
expiration, including GPU purging. No additional offline caching entitlement is claimed. **Current
service terms and free-account eligibility have not been verified offline**; review the linked Esri
terms in Settings, and disable pay-as-you-go in your account. The app cannot enforce account billing
settings or promise a particular free allowance. HTTP 402/429 or JSON quota errors stop requests
with an allowance message; rejected keys, redirects, and other HTTP failures stop the session too.
After fixing the account, choose Apply scenery to retry. Streaming remains opt-in, with no user
account, key, or retrieved imagery bundled in application or data releases.

Live endpoint authentication, service attribution sufficiency, current terms, free-cap behavior,
terrain drape alignment, and performance need flight-lead acceptance with their own account.
The in-process local fake HTTP server tests exercise images, copyright propagation, header-only authentication,
key redaction (including server echoes), redirects, 4xx/cap responses, and ArcGIS JSON errors without
contacting Esri. Run those with the focused checks below.

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
are 48/96/192 MiB per provider; at most two decode requests are pending, and one/two/two GPU tiles upload per frame.
Replacement parents stay visible until children are decoded **and** uploaded. The browser clamps
High to Medium and fetches the reduced, same-origin NYC showcase tileset. Quality
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
Web packaging includes accepted scenery and enforces the shared 1 GB total-site limit; Lidar-derived Fly NYC
buildings have a separate 250 MB cap. Never install local TUM content into a website data tree.

Generated schemas: `tileset.json`, `scenery-request.json`, `scenery-sources.json`, `scenery-report.json`,
`scenery-provider.json`, `scenery-footprint-{sources,review,report}.json`, and
`scenery-citygml-{sources,report}.json`, `scenery-nyc-area.json`, `scenery-nyc-lidar.json`, and
`scenery-nyc-lidar-report.json`. Tests cover hostile tilesets/GLBs, bounded image decoding, resume-range
validation, DEM registration/nodata, quadtree completeness, geometry registration, and parent coverage.
Shader tests translate both scene shaders to WebGL 2 GLSL. Direct render dependencies on `image`,
`gltf`, `serde`, and `schemars` reuse existing workspace versions; no new third-party package version is added.

## Cities lane checks and captures

Run one command at a time with the external target and four jobs:

```sh
CARGO_TARGET_DIR=$HOME/builds/cargo-target/flightcraft/fc-cities CARGO_BUILD_JOBS=4 PKG_CONFIG_PATH=$HOME/builds/pkgconfig-shim capped 8G -- cargo test -j 4 -p flightcraft-render -p xtask scenery -- --test-threads=1
CARGO_TARGET_DIR=$HOME/builds/cargo-target/flightcraft/fc-cities CARGO_BUILD_JOBS=4 PKG_CONFIG_PATH=$HOME/builds/pkgconfig-shim capped 8G -- cargo clippy -j 4 -p flightcraft-render -p flightcraft-ui -p xtask --all-targets -- -D warnings
CARGO_TARGET_DIR=$HOME/builds/cargo-target/flightcraft/fc-cities CARGO_BUILD_JOBS=4 PKG_CONFIG_PATH=$HOME/builds/pkgconfig-shim capped 8G -- cargo xtask web --check
CARGO_TARGET_DIR=$HOME/builds/cargo-target/flightcraft/fc-cities CARGO_BUILD_JOBS=4 PKG_CONFIG_PATH=$HOME/builds/pkgconfig-shim capped 8G -- cargo xtask schemas --check
CARGO_TARGET_DIR=$HOME/builds/cargo-target/flightcraft/fc-cities CARGO_BUILD_JOBS=4 PKG_CONFIG_PATH=$HOME/builds/pkgconfig-shim capped 8G -- cargo fmt --all --check
```

Once real city tiles and region data are installed, generate the three corridor images offscreen:

```sh
CARGO_TARGET_DIR=$HOME/builds/cargo-target/flightcraft/fc-cities CARGO_BUILD_JOBS=4 PKG_CONFIG_PATH=$HOME/builds/pkgconfig-shim capped 8G -- cargo test -j 4 -p flightcraft-ui render_nyc_city_reference_shots -- --ignored --nocapture --test-threads=1
```

`FLIGHTCRAFT_CITY_TILESET` and `FLIGHTCRAFT_CITY_REGION` override the default built tileset and
installed region directory. Images land in `out/scenery/nyc/`. This checks GPU validation and that
city geometry changes rendered pixels; it does not measure alignment or establish that the skyline
is correct. Airports and shoreline still require the independent checks and real-window shots above.
No real NYC images are claimed from this offline checkout without fc-scenery's reconstructed 3DEP provider.

Direct dependencies added in this lane reuse workspace packages: native render `ureq` for bounded
HTTP, wasm render `js-sys` for a browser-safe expiration clock, and xtask `flightcraft-render`, `gltf`,
and `glam` for the shared GLB validator/writer and double-precision conversion. `quick-xml` 0.41
(MIT, pure Rust, already present transitively in the lockfile) is a new direct xtask dependency for
streaming CityGML. Browser render uses existing workspace `web-sys`, `wasm-bindgen`, and
`wasm-bindgen-futures` for same-origin bounded static fetches. Native render also
uses the existing `log` package with `max_level_debug`: ureq-proto trace logging dumps raw HTTP
bytes, so trace logs are compiled out in debug and release builds to keep credentials out of logs.
Debug and higher-priority diagnostics remain available. No new third-party
package version or shader is introduced.

In this offline lane, `cargo xtask web --check` can finish strict wasm compilation but cannot
finish showcase packaging without the locked chart and terrain archives. If they are absent,
flight lead must first run `cargo xtask data fetch` online using the same target, jobs, shim,
and memory scope as above. Do not substitute fixture data or change `data.lock` to pass packaging.
