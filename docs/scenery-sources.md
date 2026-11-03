# Real-world scenery: sources and plan

Status: plan, with option 2 in progress. This page covers where FlightCraft gets photorealistic
scenery (ground imagery, 3D buildings, trees, and terrain), what each source costs, and what we
may do with it. The project rule is **no card and no spend**: nothing the project ships or runs may
need a payment method, and every shipped dataset must be one we are allowed to redistribute.

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
- **Water:** NHD lakes and rivers.

We build the tiles with `cargo xtask scenery fetch|build`, check that the painted runway in the
imagery lines up with FAA runway data within 2 m, and publish them as region data releases.
Golden area 1 is 1N7 ±5 km; area 2 is the Delaware Water Gap corridor; area 3 is the surrounding
40 km.

### 3. Open city models (next)

- **New York City LoD2** from the Technical University of Munich: over a million buildings with
  roof shapes, as open data including glTF
  ([project](https://www.gis.bgu.tum.de/projekte/new-york-city-3d)).
- **Geopipe New York City:** all five boroughs, under a Creative Commons Attribution licence that
  permits commercial use
  ([article](https://www.cgchannel.com/2023/02/download-the-entire-city-of-new-york-in-3d-for-free)).

**Use:** converted to 3D Tiles and published for the New York region (the Hudson corridor and the
Class B), with attribution in ATTRIBUTION.md. The licence of each file is checked before it ships.

### 4. Esri ArcGIS Location Platform free tier (next, for users)

- A monthly free tier (about 2 million basemap tiles, including World Imagery). Service stops at
  the cap unless pay-as-you-go is enabled ([FAQ](https://location.arcgis.com/faq/)).
- Not verified: whether sign-up needs a card, and whether 3D and elevation services are in the
  free tier. Terms limit offline caching.

**Use:** like option 1: a user's own key, streamed imagery, off by default, never shipped.

## Order

1. Option 2 for 1N7, then the Water Gap corridor, then the wider region.
2. Option 3 for New York City, and option 4 as a streaming provider for users with their own key.
3. Option 1 as an opt-in streaming provider, once the provider interface has proven itself on
   option 4.
4. User add-ons: a local's drone photogrammetry of their own field, contributed as a tileset.

Each step checks its licence and costs again before it ships, because provider terms change.
