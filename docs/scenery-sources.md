# Real-world scenery: sources and plan

Status: **Fly NYC ships lidar-derived buildings only**, from public-domain USGS 3DEP. The
40.66–40.84° N, 74.04–73.90° W envelope and 250 MB building allocation sit inside the website's
shared 1 GB limit. fc-scenery owns EPT acquisition, LAZ decoding, clustering, and roof reconstruction;
this lane owns the area/LOD/budget consumer and the native/browser provider. Reconstruction output
and real-data visual/datum acceptance are still pending. **NYC Open Data portal footprints and TUM
are local, user-fetched only, never shipped or included in data releases.** The earlier NYC reuse
claim was incorrect. Option 4 remains an opt-in native user-key adapter awaiting live acceptance.

## One renderer path: OGC 3D Tiles

All scenery uses one format, [OGC 3D Tiles 1.1](https://www.ogc.org/standard/3dtiles/): glTF
content in a quadtree, with level of detail chosen by geometric error. Google and Cesium serve this
format, and so do most city-model and photogrammetry tools. With one format, every source below
goes through the same streaming and rendering code:

| Source | Provider | Shipped with FlightCraft? |
|---|---|---|
| Our generated scenery (option 2) | local tileset | yes, as region data releases |
| City models (option 3) | local tileset | yes, where the licence allows |
| A local's drone capture of their field | local tileset | a user add-on |
| Streaming photoreal (options 1 and 4) | URL plus the user's own token | no; opt-in, streamed only |

A provider is a local tileset path or a URL, plus an optional per-user token. Tokens live in the
user's config, never in the repository.

## The options

### 1. Google Photorealistic 3D Tiles through a free Cesium ion account

- Cesium ion's free Community plan includes Google Photorealistic 3D Tiles without a Google Cloud
  account. The quota is 1,000 root tiles a month, roughly 1,000 sessions
  ([pricing](https://cesium.com/platform/cesium-ion/pricing/),
  [announcement](https://community.cesium.com/t/photorealistic-3d-tiles-from-google-maps-platform-are-now-included-in-your-cesium-ion-account/27630)).
- The Community plan is for personal, non-commercial use. Tiles are streamed only: no caching
  beyond the provider's terms and no redistribution. Google attribution must be shown.
- Not verified: whether sign-up asks for a card. Google-side rate limits (HTTP 429) have been
  reported.
- Google's own Map Tiles API needs a billing account, and its 3D tiles cost $6 per 1,000 requests
  after 1,000 free a month
  ([Google pricing](https://developers.google.com/maps/billing-and-pricing/pricing-categories)).
  That fails the rule, so we only consider the Cesium route.

**Use:** a later opt-in "Photoreal streaming" setting where each user pastes their own free token
and gets their own quota. It's off by default and never part of a shipped build.

### 2. Generated from US public data (the default; in progress)

Everything here is US public domain, free to download, and free to redistribute:

- **Imagery:** USDA NAIP orthoimagery at 0.3 to 0.6 m, from the USGS NAIP ImageServer
  (`imagery.nationalmap.gov`).
- **Terrain:** USGS 3DEP 1 m DEMs near airports and along scenic routes, with the existing Terrain
  Tiles data elsewhere.
- **Trees and buildings:** USGS 3DEP lidar point clouds (also mirrored as AWS open data in
  Entwine/EPT form, free to read). Ground, vegetation, and building classes give every tree its
  measured height and every building its real roof shape. Footprint datasets are only a fallback;
  Microsoft and OpenStreetMap footprints are ODbL and would be kept in their own file with that
  notice.
- **Water:** NHD lakes, ponds, reservoirs and river areas (public domain), fetched from the TNM Access API as HU-8 shapefiles and built into the tiles with a reflective shader; implemented for the 1N7 test area (docs/scenery.md, "NHD water").

We build the tiles with `cargo xtask scenery fetch|build`, check that the painted runway in the
imagery lines up with FAA runway data within 2 m, and publish them as region data releases.
Golden area 1 is 1N7 ±5 km; area 2 is the Delaware Water Gap corridor; area 3 is the surrounding
40 km.

### 3. Fly NYC: public-domain 3DEP buildings; portal/TUM local-only

**Shipped source:** USGS 3DEP [NY_NewYorkCity EPT](https://s3-us-west-2.amazonaws.com/usgs-lidar-public/NY_NewYorkCity/ept.json),
public domain (US government data), free to read. Flight lead supplied: 4,755,025,996 points,
EPSG:3857, a Classification dimension, and cube bounds
`[-8267099, 4937860, -31153, -8204119, 5000840, 31827]`. Cube padding is not ground elevation or
vertical-datum evidence. Attribution: USGS 3DEP, NY_NewYorkCity classified lidar; buildings
reconstructed by FlightCraft, with original procedural materials.

fc-scenery reconstructs buildings directly from classified points: LAZ decode, clustering-derived
footprints, and roof-plane fitting. This lane consumes the reconstructed 3D Tiles/GLBs through the
existing [provider contract](../schema/scenery-provider.json) and
[lidar provenance schema](../schema/scenery-nyc-lidar.json), without duplicating reconstruction.
The [machine-readable Fly NYC area](../assets/scenery/fly-nyc.json) fixes the geographic envelope,
classification 6, source metadata, 250 MB building budget, and shared 1 GB web budget. The consumer
selects whole measured buildings, emits area tiles and multiple LODs, retains fine roof geometry,
and reports complete package bytes. Same-origin browser loading uses the same renderer and budgets.
Both the consumer and static-provider/release gates reject portal footprints, TUM, external
footprint dependencies, and provenance outside this exact USGS EPT project. No guessed-height or
portal-extruded fallback can ship. Real source reconstruction, vertical/FAA acceptance, actual byte
size, and corridor/skyline images remain to be checked by flight lead. See
[the handoff and commands](scenery.md#fly-nyc-lidar-only-shipped-building-provider).

**NYC portal footprints local only:** the [portal terms](https://www.nyc.gov/html/data/terms.html)
bind users to NYC.gov's terms plus supplying-agency terms. Flight lead corrected the earlier reuse
claim: these are not a clean open license. Historically, the City reserved a right to stop "any and
all display, distribution or other use" for any reason; OpenStreetMap rejected NYC data on those
grounds in November 2011. We have no approved redistribution license for dataset `nqwf-w8eh`.
The Socrata fetch and height-extrusion converter remain optional local/user-fetched tools, with
receipts and height review, but all output is permanently local-only and preview-only. Geometry
acceptance cannot authorize publication. Static browser loading, data releases, and website
packaging reject even stale portal builds carrying the earlier accepted flags. **Do not fetch
Socrata for the shipped Fly NYC workflow.** No portal footprints or heights feed its lidar meshes.

**TUM local only:** flight lead verified the [TUM project page](https://www.asg.ed.tum.de/gis/projekte/new-york-city-3d/)
and [workflow repository](https://github.com/tum-gis/3d-model-new-york-city). It describes Open Data,
derived from NYC Open Data, but states no explicit license. TUM remains a **local, user-fetched
provider only**, never website data or a data release. NYC's terms do not license the TUM derivative.
[ATTRIBUTION.md](../ATTRIBUTION.md) records these three distinct source policies.

The documented TUM buildings archive is about 2.4 GB zipped:
`http://www.3dcitydb.net/3dcitydb/fileadmin/public/datasets/NYC/NYC_buildings_CityGML_LoD2/NYC_Buildings_LoD2_CityGML.zip`.
Its optional fetch requires `--allow-http`, the exact documented HTTP prefix, no redirects/path
escapes, and an 8 GB streamed cap; USGS limits remain unchanged. Build streams bounded CityGML
fragments, validates `srsName`, preserves inline LoD2 roofs, and emits local-only 3D Tiles/GLBs.
No source compatibility or real-world alignment proof is fabricated. Separate street-space and
older `/mydata/` archives are not fetched. Geopipe is skipped because it needs an account and credits.

### 4. Esri ArcGIS Location Platform (native adapter; live acceptance pending)

Settings → Scenery → **Streaming imagery (your own key)** is off by default. The user supplies an
ArcGIS key in a masked field. Only the user's eframe preferences persist the key; provider descriptors
contain a config-key name. Keys are absent from request URLs, receipts, recordings, and screenshot
metadata. Authorization headers are sensitive, redirects are refused, and error bodies are never shown.

The native adapter requests World Imagery only for selected regional tiles, drapes it over read-only
terrain samples, and returns ordinary embedded-image GLBs through `TileProvider`. It has no bulk
fetch, disk cache, offline export, or release path. CPU residency is a bounded memory LRU; GPU
residency follows valid CPU content, with a conservative five-minute maximum lifetime. Current
service copyright text accompanies the on-screen Esri credit. HTTP 402/429 and ArcGIS JSON quota
errors stop further requests with a clear allowance message; rejected keys and other service errors
also stop the session. Apply scenery again to retry after correcting account settings.

This implements a conservative session cache, **not a claim that current Esri terms have been
verified offline**. Review [the current terms](https://www.esri.com/en-us/legal/terms/full-master-agreement)
and [account FAQ](https://location.arcgis.com/faq/) before using the service. Free allowances,
sign-up requirements, and enforcement can change; no two-million-tile or zero-spend guarantee is
encoded. Disable pay-as-you-go in the user's account. No account, key, imagery, or billable service
is bundled with FlightCraft. Browser transport remains pending; all application crates still build
for wasm. Tests use an in-process local fake HTTP server without sockets or DNS, never Esri.

## Web showcase: "Fly NYC"

The website is one polished showcase, not the whole world: the Hudson River corridor and the
Manhattan skyline. It uses public-data scenery (NAIP imagery at 0.6 m near the river and coarser
outward, the USGS 1 m DEM, NHD water, and USGS 3DEP lidar buildings) sized to fit GitHub Pages'
roughly 1 GB site limit with no new account. The desktop app carries everything else (1N7 in full
detail, the Water Gap, eastern Pennsylvania, and later the whole US) through GitHub Releases.

**Built and shipped as `new-york/hudson-corridor`** (how it is fetched, built, gated and
published: [scenery.md](scenery.md#hudson-corridor-fly-nyc)). The website opens it with **no
query**: southbound at about 1,000 ft over the Hudson at the George Washington Bridge, skyline
ahead. Why that and not a Teterboro (KTEB) departure: KTEB is about 9 km west of the river and
outside the fly-nyc.json lidar envelope, so a departure spends its first minutes over New Jersey
with flat NAIP and no buildings, while the bridge start puts the measured skyline in the first
frame. `?start=kteb` still departs Teterboro runway 06 (imagery and terrain cover the field and
the departure path; there are no hangars or terminal there), `?start=runway` keeps the Blairstown
runway start, and `?region=none` keeps the original flat world.

**Two facts about the source that the area definition did not anticipate**, both measured on the
real data:

* NY_NewYorkCity's building returns are **unclassified**. The 10.3 million Midtown points
  (181 EPT nodes) carry only classes 1, 2, 7, 9, 10, 17 and 18 and no class 6 at all: the Empire
  State Building's 8,652 returns within 40 m are all class 1, up to 455 m. Buildings are therefore
  inferred from planar, elevated unclassified returns exactly as for the NJ survey (every building
  record says `inferred_classification: true`), still from the lidar alone. The `classification: 6`
  in `assets/scenery/fly-nyc.json` is kept as the fixed area record but is not a property of the data.
* The envelope's north edge (40.84 N) stops 1.4 km short of the bridge (about 40.850 N), so the
  first two kilometres of a bridge start have no buildings (imagery, terrain and water cover it).
  Widening the envelope to 40.87 N is a one-line change to `fly-nyc.json` and `NYC_BOUNDS`.

If the showcase outgrows 1 GB, only the scenery tiles move to a free host without that cap
(Cloudflare Pages: up to 20,000 files of 25 MB each with no bandwidth cap; or a Hugging Face
dataset repository). Both need a free account, which is the owner's decision.

## Order

1. **Website critical path: 3DEP lidar-derived buildings for Fly NYC**, within the shared 1 GB budget, alongside fc-scenery imagery and terrain.
2. Option 2 for 1N7, then the Water Gap corridor and wider region. TUM is secondary and local-only; option 4 remains user-key opt-in.
3. Option 1 as an opt-in streaming provider, once the provider interface has proven itself on
   option 4.
4. User add-ons: a local's drone photogrammetry of their own field, contributed as a tileset.

Each step checks its licence and costs again before it ships, because provider terms change.
