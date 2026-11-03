# Controlled airspace

A terrain region may include an `airspace.json` beside `region.json`. Native discovery and browser
region URLs load this optional sidecar before installing the world. A missing file is allowed; an
invalid, oversized, or unreadable file rejects that region. Flat world and the training valley retain
their existing appearance and simulation. Airspace contributes to the region's content identity.

The New York asset includes FAA Class B shelves, Class C and D areas, surface Class E areas, and
Mode C veils. Its origin is 40.97113° N, 74.99750° W. It was generated against a temporary grid
covering 40.25–41.30° N and 75.20–73.25° W because the terrain manifest was not yet present. The grid
runs from north −82,000 m and east −18,000 m, with eight northward and eleven eastward 16 km tiles.
The sidecar needs the New York terrain manifest and tiles installed alongside it to become selectable.

The eastern Pennsylvania asset uses the same 1N7 origin and covers 40.20–41.50° N and
76.20–74.30° W. Its manifest was also absent at generation time. The temporary grid starts at
north −88,000 m and east −104,000 m, with ten northward and eleven eastward 16 km tiles. Install
the matching terrain manifest and tiles alongside `assets/terrain/eastern-pa/airspace.json`.

## Offline FAA import

Download the FAA ADDS public-domain Class_Airspace layer outside the repository. This is the exact
ArcGIS REST query for the requested geographic envelope, in longitude/latitude order:

```text
https://services6.arcgis.com/ssFJjBXIUyZDrSYZ/arcgis/rest/services/Class_Airspace/FeatureServer/0/query?where=1%3D1&geometry=-75.20%2C40.25%2C-73.25%2C41.30&geometryType=esriGeometryEnvelope&inSR=4326&outSR=4326&spatialRel=esriSpatialRelIntersects&outFields=*&returnGeometry=true&f=geojson
```

For eastern Pennsylvania, use this exact query:

```text
https://services6.arcgis.com/ssFJjBXIUyZDrSYZ/arcgis/rest/services/Class_Airspace/FeatureServer/0/query?where=1%3D1&geometry=-76.20%2C40.20%2C-74.30%2C41.50&geometryType=esriGeometryEnvelope&inSR=4326&outSR=4326&spatialRel=esriSpatialRelIntersects&outFields=*&returnGeometry=true&f=geojson
```

Keep all properties and full polygon coordinates. Check the service's transfer-limit indication;
if exceeded, retrieve every page with `resultOffset` and `resultRecordCount` and combine their
features into one FeatureCollection. Never use an incomplete page as the source. The importer reads
local files only and makes no network requests:

```sh
cargo xtask airspace-faa --region region.json --airspace downloads/Class_Airspace.geojson --out assets/terrain/new-york/airspace.json
```

The output parent directory must exist, and the output file must be new. Conversion and validation
finish before creating the output. The importer reports source and retained output vertex counts.
The New York conversion processed 70,798 vertices and retained 3,806 vertices across 34 volumes.
The eastern Pennsylvania conversion processed 98,586 vertices and retained 3,116 vertices across
29 volumes.

B, C, D, E2, E3, E4, and Mode C volumes intersecting the region grid are retained. `--keep-a` also
keeps Class A, and `--keep-e5` keeps E5 transition areas. Those flags are off by default. The grid
intersection test includes crossings, containing polygons, holes, and MultiPolygons. Whole volumes
are retained without clipping so queries across the grid boundary remain meaningful. A closed-ring,
iterative Douglas–Peucker implementation simplifies in projected metres with a 15 m tolerance;
polygons smaller than the tolerance remain intact. Coordinates are converted from GeoJSON
longitude/latitude to the document's latitude/longitude order. FAA feet remain feet, flight levels
become feet with reference `STD`, and upper sentinel values such as −9998 become 18,000 ft `STD`.

## Document and query semantics

The generated schema is [schema/airspace.json](../schema/airspace.json), checked with
`cargo xtask schemas --check`. Each volume has a unique `id`, a `name`, a `class`, a `kind` (`class`
or `mode_c`), optional `sector`, `floor`, `ceiling`, and `polygons`. `class` is null for Mode C veils;
A is supported for the optional importer flag, and B, C, D, and E are the default classes. Each
altitude is `{ "value_ft": 3000, "reference": "MSL" }`, with `MSL`, `SFC`, or `STD` references.
Each polygon is an array of rings of `[latitude_deg, longitude_deg]`: exterior first, holes after.
The document also requires a nonempty `attribution` string.

Files are limited to 8 MiB, 4,096 volumes, and 50,000 total vertices, including closure vertices.
Unknown fields, duplicate IDs, non-finite values, invalid geographic coordinates, empty or degenerate
rings, invalid kinds, and floors at or above ceilings are errors. Three distinct vertices can be
closed by repeating the first vertex; already closed rings are accepted. Runtime projection is the
same deterministic local tangent plane used by terrain, with `libm` for transcendental functions.

The world query uses an even-odd ring test, subtracts holes, and unions polygon components. Exterior
edges count as inside, and hole edges count as outside. Vertical intervals include the floor and
exclude the ceiling. SFC heights add local terrain height, so a zero SFC floor contains a taxiing
aircraft. STD uses nominal MSL height; pressure-dependent flight-level correction is not modelled.
Class G in the status chip means none of the loaded class volumes contains the aircraft; Mode C
membership does not change the class label. Omitted E5 and Class A are outside the default dataset.

`World::with_airspace` validates and projects a document once. `World::airspace` exposes immutable
projected rings for presentation, and `World::airspace_at` returns containing volumes and the nearest
B/C shelf above or alongside the point. Distance is to the lateral footprint, zero underneath it;
`floor_above_m` is the nonnegative height to its floor. The nearest shelf minimises combined lateral
and vertical distance, breaking ties by the lower floor. A shelf already containing the point or
entirely below it is excluded from the entry warning. The warning activates within 1 nm laterally
at shelf altitude, or within 300 ft below its floor when underneath or within 1 nm of its footprint.

## Display and commands

Airspace is shown by default. Preferences offers **Show airspace curtains and outlines**, and the
default configurable shortcut is **U**. The command is `view.set_airspace` with `visible: true` or
`false`. Visibility affects presentation only. Blue Class B and magenta Class C curtains have solid
floor and ceiling outlines. Class D uses dashed blue outlines, and E uses dashed magenta outlines.
Mode C veils use faint grey rings. All colours come from theme tokens. The cached meshes draw after
opaque terrain and aircraft, with depth testing, alpha blending, and depth writes disabled.

The status chip reports the current class and vertical limits and turns amber near a B/C entry.
Its hover text carries the airspace attribution. `airspace.list`, `airspace.at`, and `sim.state`
provide the same region data to scripts and MCP clients; see [the command reference](mcp.md).

[Tower and approach training](atc.md) grades airborne B/C/D entries against explicit Bravo
clearance and controlling-provider two-way communications, using this same immutable query.
