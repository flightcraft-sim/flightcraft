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

`sim.reset` accepts `"world": "flat"` (the default), `"world": "training-valley"`, or an installed
region ID (see below).
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

## Importing real SRTM elevation offline

The elevation importer accepts documented public-domain HGT elevation. It needs no network connection,
account credentials, or external conversion tools. The New York area region is bundled under `assets/terrain/new-york/`. The simulator
continues to start in the flat world.

1. Choose an airfield and record the latitude and longitude of its runway threshold in decimal
   degrees, field elevation in metres MSL, true runway heading in degrees clockwise from north,
   runway length, and runway width in metres. Use independently sourced public-domain information.
2. In [USGS EarthExplorer](https://earthexplorer.usgs.gov/), make a free account, select the region on the map, choose the SRTM
   elevation collection, and download the intersecting HGT tiles. Alternatively, sign in with a
   free [NASA Earthdata](https://search.earthdata.nasa.gov/) account, find the SRTM global elevation collection, select the region, and
   download its HGT granules. Public 3-arc-second SRTM mirrors also offer degree tiles without an
   account. Confirm the mirror distributes unmodified NASA/USGS SRTM and preserves the public-domain
   provenance; do not substitute another DEM under this importer’s attribution.
3. Fetch every one-degree tile intersecting the local grid, plus a small margin for interpolation
   and void neighbours. SRTM typically covers 60°N to 56°S. A filename such as `N37W123.hgt` means
   the square from 37°N to 38°N and 123°W to 122°W. Its samples include both edges. Supported
   resolutions are 1 arc-second (3601² samples) and 3 arc-seconds (1201²). Do not rescale or swap
   the big-endian signed 16-bit samples. HGT rows run north to south, and columns west to east.
4. Place raw `.hgt` files or `.hgt.zip` archives in one local directory. Keep only one source per
   degree square. Use canonical names such as `N37W123.hgt` and `N37W123.hgt.zip`; an archive must
   contain exactly one regular file with the matching HGT basename, using stored or DEFLATE
   compression. If a provider adds a product suffix, rename the HGT and its containing archive to
   these canonical names. Archives are read directly and never extracted to filesystem paths.
5. Write a small region definition, for example the following **illustrative coordinates and
   elevation**. Replace them with the chosen airfield’s values; this example does not identify an
   operational runway.

```json
{
  "id": "example-region",
  "name": "Example region",
  "origin": {
    "latitude_deg": 37.5,
    "longitude_deg": -122.5,
    "field_elevation_m": 100.0,
    "runway_heading_deg": 90.0,
    "runway_length_m": 900.0,
    "runway_width_m": 23.0
  },
  "southwest_m": [-6000.0, -6000.0],
  "tiles_north": 6,
  "tiles_east": 6,
  "spacing_m": 31.25,
  "tile_size": 65,
  "runway_blend_m": 200.0
}
```

6. Run the importer with a new output directory:

```sh
cargo xtask terrain-srtm --region example-region.json --hgt downloads/srtm --out assets/terrain/example-region/
```

The output contains numbered `.fcht` tiles and a `region.json` manifest. The input and output share
one schema, published as `schema/region.json`; generated fields `source`, `licence`, `attribution`,
and `tiles` may be omitted from input. When omitted, the importer defaults these to `SRTM`, `public domain`,
`NASA Shuttle Radar Topography Mission, public domain; distributed by USGS`, and the ordered tile
filenames. Explicit source, licence, and attribution strings are preserved. It refuses to overwrite an existing directory. Missing coverage, malformed files, and
unfillable voids produce errors before output creation. A failed disk write removes the new output;
metadata is written last, so discovery never offers a partially written world.

The grid’s southwest corner is `[north, east]` in local metres; each tile spans
`(tile_size - 1) * spacing_m`. At most 256 tiles, 257 samples per side, 64 HGT files, and 256 MiB of
raw HGT data are accepted. Installed regions share a 128 MiB tile budget. Coordinates stay within ±250 km on each horizontal axis of the origin,
which must be below 85° absolute latitude. The grid must enclose the runway and its blend margin.
IDs contain lowercase ASCII letters, digits, and hyphens, up to 64 characters, and cannot replace
`flat` or `training-valley`. Source directories are read in sorted order; duplicates are errors.

### Projection, voids, and runway preparation

The horizontal projection is WGS84 transverse Mercator, with central meridian equal to the
origin longitude, central scale `k0 = 1`, zero false easting, and false northing chosen to put the
origin at `[0, 0]`. `world::geo::LocalProjection` implements forward and inverse Krüger series
through the fourth power of the third flattening, with `libm` throughout. The compatibility
re-export `world::srtm::LocalProjection` remains available. Round trips are tested below 1 mm over
the full ±250 km square, including high latitudes and the antimeridian. Independently evaluated
closed-form transverse Mercator equations from USGS Professional Paper 1395 check forward values.
This is a map projection, so grid bearings and distances differ slightly from true bearings and
ellipsoidal distances away from the central meridian.

SRTM elevations are orthometric MSL heights (normally EGM96), retained directly. The simulation’s
vertical coordinate is negative MSL elevation; field elevation does not get subtracted from samples.

Resampling is bilinear in source latitude and longitude. A `-32768` void corner first uses a valid
sample from an adjoining HGT at the same location, then searches symmetric square rings in its
source grid, averaging the first ring containing valid neighbours. Rings cross source tile edges;
search stops after 32 source samples and fails if none are valid. Large water or missing-data gaps
may therefore require a better SRTM product. Missing source coverage is always an error. Source
heights outside the tile format’s ±20 km envelope are rejected during tile construction.

The runway rectangle starts at the origin and extends along the given true heading. Its height
is the supplied field elevation. A two-output-sample apron is also flattened, ensuring bilinear
cells intersecting the runway remain flat. Beyond the apron, a cubic smoothstep blends to the
original terrain over `runway_blend_m`, with zero derivative at both ends. Samples shared by tiles
are computed from the same global grid indices. Runway collision and rendered markings use the
region’s width, length, heading, and elevation. A runway start parks at field elevation, aligned
with its heading. `ResetStart::Air.height_m` remains an absolute MSL altitude. Beyond the installed grid,
region worlds fall back to grass at field elevation.

### Selecting a region

Native desktop builds discover child directories containing `region.json` under `assets/terrain`
at startup. Override this root with `--regions-dir <directory>`. The headless MCP app also scans
`assets/terrain`. Rust hosts may use `Session::load_regions_directory`, `read_region`, or
`Session::install_region`. Invalid regions are reported and skipped independently; every declared
tile must exist, decode, and match the manifest grid before a region becomes selectable.

In the browser, put a region directory beside the static site and add
`?region=terrain/example-region/region.json` to the page URL. Repeat `region` for multiple manifests.
Absolute HTTP(S) URLs also work when the static host allows CORS. Manifests and tiles are fetched
with bounded streaming reads before the app starts, like aircraft packs; there is no tile service.

The app’s World menu and `world.list` show built-ins plus successfully installed regions. Select
one by ID through the same command interface:

```json
{"world":"example-region","start":{"kind":"runway"},"paused":true}
```

Pass that input to `sim.reset`. `sim.state.world` reports the selected ID. Unknown or incomplete
regions fail without changing the active flight. The status bar displays the **active world’s**
attribution and changes when the world changes. The SRTM attribution row template is also in
`ATTRIBUTION.md`; future redistributed regions should keep their provenance there and in metadata.
`WorldChoice` now includes `Region(String)` and is `Clone` rather than `Copy`; existing built-in Rust
variants and JSON names remain supported. The original flat-world golden trajectory is preserved.

### Airspace sidecars

Regions can also carry an optional `airspace.json`, loaded by native and browser transports.
See [controlled airspace](airspace.md) for the FAA offline importer, schema, containment semantics,
commands, and display settings. The airspace file is validated before the region becomes selectable.
## Public airports and runway starts

A region may name `"airports": "airports.json"`. That bounded, adjacent catalogue follows
[`schema/airports.json`](../schema/airports.json): an attribution string and an `airports` array.
Each airport records its FAA ident, optional ICAO ident, name, MSL elevation, latitude, longitude,
and runways. Each runway records its designator, dimensions in metres, surface, and two ends
with ident, WGS84 latitude/longitude, MSL elevation, and true heading in degrees. The loader checks
finite values, unique idents and designators, grid containment, lengths, grades, and a maximum of
2,000 runways. Catalogues are limited to 4 MiB. Missing or invalid catalogues prevent installation.
Rust `Region` literals now supply `airports: None` when no catalogue is declared. Located host
starts use `Start::RunwayAt` and `Start::AirAt`; typed control starts use
`ResetStart::AirportRunway` and `ResetStart::AirportAir`. Both `Readout` and `StateOutput` gain
optional latitude and longitude fields. Native and browser transports finish airport validation
with `region_world_with_airports(region, tiles, airports_bytes)`, then attach any adjacent airspace
data through the airspace loader. Static packaging copies adjacent files alongside both regions.

Runway collision surfaces are straight sloped planes through their endpoint elevations. Terrain
preparation flattens a two-sample apron and applies the same smooth blend margin as the origin
runway. Exact runway surfaces take precedence over neighbouring blend margins; pavement wins at
crossings with other surfaces. When equally ranked source planes disagree at a crossing, the
upper plane owns the overlap, matching the visible surface; endpoint heights remain the original
terrain samples. Such source disagreements are not survey-grade runway profiles. The renderer draws each runway at its precise width, with threshold
bars, centreline dashes, and an original stroke alphabet for both end idents. Turf and gravel use
ground surfaces, asphalt and concrete use pavement, and water uses water. Regional rendering uses 120 m near meshes and 480 m far meshes, refining
cells near runway aprons to the original 60 m samples in both levels. Distance to the tile
footprint selects near detail even at the edges of a large tile. Built-in worlds retain
their original runway artwork.

The **Start** menu searches the active region by ident, ICAO ident, or name, lists its runway ends,
and shows the airport catalogue attribution. These selections use `sim.reset`:

```json
{"world":"new-york","start":{"runway":{"airport":"1N7","end":"07"}},"paused":true}
```

An airport-relative air start uses true bearing clockwise from north, horizontal distance in
metres, and height above airport elevation. The heading is a local grid heading in radians:

```json
{"world":"new-york","start":{"air":{"airport":"JFK","distance_m":5000,"bearing_deg":220,"height_m":600,"airspeed_mps":45,"heading_rad":0.7}}}
```

The existing `{ "kind": "air", "height_m": 600, "airspeed_mps": 45, "heading_rad": 0 }` retains
its absolute MSL altitude and origin position. Omitting a selected runway keeps the origin runway.
`sim.state` and its readout report optional `latitude_deg` and `longitude_deg`; they are null in
built-in worlds or beyond projection bounds. The state output also supplies optional
`altitude_msl_m` for a geographic position. The readout's existing MSL altitude remains available
in every world. `world.geodetic` converts either `{north_m, east_m}` or
`{latitude_deg, longitude_deg}` and returns both coordinate pairs. `world.airports` lists the active
catalogue, with optional `nearest_to_m: [north, east]` to sort by airport distance.

## Importing FAA ADDS data offline

Fetch the two FAA Aeronautical Data Delivery Service layers outside the repository. The FAA data
is US government work, public domain. For each of `US_Airport` and `Runways`, make this ArcGIS REST
query, substituting the layer name and the envelope for the region. This example covers the New
York area; the output must have no transfer-limit truncation. If a service reports
`exceededTransferLimit`, paginate with `resultOffset` and combine the feature arrays before import.

```sh
curl --get 'https://services6.arcgis.com/ssFJjBXIUyZDrSYZ/arcgis/rest/services/US_Airport/FeatureServer/0/query' \
  --data-urlencode 'where=1=1' \
  --data-urlencode 'geometry={"xmin":-75.20,"ymin":40.25,"xmax":-73.25,"ymax":41.30,"spatialReference":{"wkid":4326}}' \
  --data-urlencode 'geometryType=esriGeometryEnvelope' \
  --data-urlencode 'inSR=4326' --data-urlencode 'outSR=4326' \
  --data-urlencode 'spatialRel=esriSpatialRelIntersects' \
  --data-urlencode 'outFields=*' --data-urlencode 'f=geojson' \
  --output downloads/faa/US_Airport.geojson
```

Repeat with `/Runways/FeatureServer/0/query` and `downloads/faa/Runways.geojson`. Then run:

```sh
cargo xtask navdata-faa --region assets/terrain/example-region/region.json \
  --airports downloads/faa/US_Airport.geojson --runways downloads/faa/Runways.geojson \
  --out assets/terrain/example-region/airports.json
```

Import only after `terrain-srtm`. The importer keeps public-use land airports (`TYPE_CODE=AD`,
`PRIVATEUSE=0`) inside the region grid, joins `AIRPORT_ID` to `GLOBAL_ID`, skips helipads and water
lanes, and converts FAA feet to metres. Closed quadrilateral runway polygons supply centreline
endpoints at the midpoints of their short edges. A discrepancy above 5% against `LENGTH` is an
error identifying the airport and runway. Runway designators orient the centreline; true headings
are measured against the local geodetic meridian. Each end samples terrain before airport
flattening, falling back to airport elevation when terrain has no coverage. The importer prepares
the tile apron, writes `airports.json`, and publishes its reference in `region.json` last. It refuses
to replace an existing catalogue. Elevation provenance remains the region's own metadata.

## Bundled New York area

`new-york` uses Blairstown (1N7) runway 07's imported threshold as its origin. Its public-domain
Terrain Tiles elevation sources are USGS 3DEP and NASA SRTM. Six one-degree, 1-arc-second HGT tiles
cover the input. The grid uses 60 m spacing and 257-sample tiles, enclosing latitude 40.25–41.30
and longitude −75.20–−73.25. The bundled manifest carries the exact grid and attribution. FAA airport
and runway attribution is separate and appears in the picker. Source HGT and GeoJSON files remain
outside the repository. To regenerate, use the manifest's origin and grid as the `terrain-srtm`
input, omitting generated `tiles` and retaining `airports: "airports.json"`, then run `navdata-faa` as above.
Declaring an airport catalogue in the elevation input preserves raw terrain for endpoint sampling;
the FAA importer prepares all runway aprons together. The intermediate region is not selectable
until its catalogue exists.

Desktop discovery includes this directory by default. The static browser build copies it to
`terrain/new-york/`; open
[New York area](https://flightcraft-sim.github.io/flightcraft/?region=terrain/new-york/region.json).
The region URL selects its origin start. With no region URL, the browser retains the flat default.

`eastern-pa`, **Eastern Pennsylvania and New Jersey**, shares the same Blairstown runway 07
origin and attribution. Its 110 tiles enclose latitude 40.20–41.50 and longitude −76.20–−74.30,
using the same 60 m spacing and 257-sample tiles. This adds Lehigh Valley (KABE), Pocono Mountains
(KMPO), Wilkes-Barre/Scranton (KAVP), Reading (KRDG), and nearby New Jersey fields. Regeneration
uses the additional HGT degree squares west to 77°W and a separate FAA envelope query with those
bounds. Both installed regions remain within the shared 128 MiB terrain budget.
Open [Eastern Pennsylvania and New Jersey](https://flightcraft-sim.github.io/flightcraft/?region=terrain/eastern-pa/region.json).

Current bundled sizes (tiles, manifests, and catalogues) are approximately 29.1 MB for `new-york`
and 36.3 MB for `eastern-pa`. Together their elevation tiles use 65,293,272 bytes. New York has
26 public airports and 46 runways; Eastern Pennsylvania and New Jersey has 43 public airports
and 61 runways.
