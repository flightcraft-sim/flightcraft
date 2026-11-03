# Map, search, and flight planning

Status: navigation data, magnetic variation, search, pins, explicit location/address actions,
the command-driven flight-plan model, the shared planning/MFD map, and regional FAA chart tiles are implemented.

A planning map you can open at any time, a single search box that finds anything, and flight plans
that the GPS, the autopilot, and the virtual pilot all fly.

## 1. One search box for everything

Type anything; results come back ranked, with their kind and distance from the aircraft or the map
centre.

| You type | You get |
|---|---|
| `1N7`, `KMMU`, `MMU`, `Morristown` | airports, by FAA ident, ICAO ident, or name |
| `SBJ`, `Solberg` | navaids (VOR, VOR/DME, VORTAC, NDB, DME), with frequency and type |
| `BOPPS`, `25007` | fixes and waypoints |
| `SBJ270015` or `SBJ/270/15` | a point on a radial and distance from a navaid (270° magnetic, 15 nm) |
| `40.9711 -74.9975`, `40 58.27N 074 59.85W`, `N40°58'16" W74°59'51"` | a coordinate in any common form |
| `my location` | where your computer is (section 3), after you allow it |
| `350 Fifth Ave, New York` | a street address, through a geocoder (section 4), after you confirm the lookup |
| `V3`, `V36` | an airway, highlighted on the map, with its fixes |

- **Local data first:** everything except addresses and "my location" is searched offline, in the
  region's data.
- **Network only when needed:** an address goes to a geocoder only when no local result matches,
  and only after you confirm. The app says which service it will ask.

## 2. Navigation data

- **Imported per region,** like airports and airspace: `cargo xtask navdata-faa` grows to read
  these FAA ADDS open-data layers (US government work, public domain):
  - `NAVAIDSystem` and `NavaidComponent`: navaids with type, frequency, channel, and magnetic
    variation;
  - `DesignatedPoint`: fixes and waypoints;
  - `ILS_Component`: localisers, glideslopes, and markers;
  - `ATS_Route`: airways and their segments;
  - `HoldingPattern`: published holds.
- **Output:** a `navdata.json` sidecar beside `airports.json`, with a published schema and strict
  validation.
- **Magnetic variation** comes from the NOAA World Magnetic Model (public domain): its coefficient
  file is bundled and evaluated in Rust. The model's published test values are checked in CI.
  Courses and headings are shown magnetic by default, as pilots use them, with true available.
- **Freshness:** the importer records the data's effective date. The app shows it and labels it
  "not for real-world navigation".

## 3. "Where am I?": your computer's location

- **Opt in:** the app asks only when you choose "my location" or "start near me". It explains that
  the position stays on your device and is not saved unless you save it as a pin.
- **Browser:** the Geolocation API, which brings up the browser's own permission prompt.
- **Desktop:** the operating system's location service where available (GeoClue on Linux, the
  system location service on macOS and Windows). Otherwise, enter it by hand or by address.
- **What you can do with it:**
  - see yourself on the map;
  - list the nearest airports;
  - start a flight at the nearest airport;
  - plan "fly over my house", which fits the watching mode in [the crew spec](crew.md).
- **Outside a region:** if your location is outside every installed region, the map still shows it
  on the world outline and says which region would be needed. A later feature can build that
  region from the same open data.

## 4. Addresses (geocoding)

- **Pluggable providers** behind one small interface:
  - **US Census Geocoder:** US addresses, public domain, no key.
  - **Nominatim (OpenStreetMap):** worldwide. It follows the usage policy: at most one request a
    second, an identifying user agent, no bulk lookups, and local caching. It shows "© OpenStreetMap
    contributors" with every result.
- **Privacy:** each lookup is opt-in. The app shows the query it will send, sends nothing else,
  and caches results locally so repeat lookups stay offline.
- **Results** become pins (section 5) that can be used as waypoints.

## 5. Pins

- **User pins:** you can drop a pin anywhere on the map, from search results, or from the
  aircraft's current position. A pin has a name, a note, and an optional altitude.
- **Storage:** pins are saved locally to a pins file with a published schema, and can be exported
  and imported. They can be shared in packs ("scenic spots of the Delaware Water Gap").
- **Use:** a pin works as a waypoint, a start position, and a target for the crew ("fly me
  there").

## 6. The planning map

- **Opening it:** a full-window map view, toggled with a key or button. It also appears as the
  MFD's map page and as an element in [layouts](hardware-and-panels.md) (for example on a second
  monitor or a tablet).
- **Layers,** each switchable:
  - terrain shading;
  - airports, with runway diagrams at close range;
  - navaids with compass roses;
  - fixes and airways;
  - controlled airspace with floors and ceilings;
  - user pins;
  - your location;
  - the aircraft, with its track history.
  - FAA VFR sectional and terminal area charts as raster bases, imported per region from public-domain GeoTIFFs;
  - Later: OpenStreetMap features with attribution.
- **Planning:**
  - Click to add waypoints, or add from search results.
  - Drag a leg to insert a waypoint, and drag a waypoint to move it.
  - Each leg shows magnetic course, distance, estimated time, and fuel, using the aircraft's
    cruise performance and the current winds.
  - Totals show time, fuel, and the reserve.
  - A planned altitude per leg, with a vertical profile below the map: terrain along the route,
    airspace floors and ceilings, and the planned altitude. Conflicts are marked, for example "leg
    2 enters the New York Class B at 3,000 ft" or "less than 1,000 ft above terrain".
  - **Actions:**
    - send the plan to the aircraft's GPS (standby or glass), where the autopilot can fly it;
    - give it to the virtual pilot ("fly this");
    - start the flight at the first waypoint.
- **Files:** plans are saved as a FlightCraft plan file with a published schema. They also import
  and export GPX and the common XML flight-plan format used by GPS units and planning apps, so
  plans move between FlightCraft and the tools pilots already use.

## 6a. Base maps and overlays

The planning map and the MFD map share one map engine with switchable base maps and stackable
overlays. The picker sits on the map itself, so switching from "Aero" to "Sectional" is one tap.

**Base maps** (one at a time):

| Base map | What it is | Source |
|---|---|---|
| **Aero** | our own clean vector aeronautical map in our own style: terrain tint, water, airports with runway shapes, airspace with floors and ceilings, navaids with compass roses, fixes, and airways, all crisp at any zoom and in day and night colours | the region's own data |
| **VFR sectional** | the FAA sectional chart | FAA GeoTIFF, public domain |
| **Terminal area** | the FAA terminal area chart where one exists (New York, Philadelphia) | FAA GeoTIFF, public domain |
| **IFR low** and **IFR high** | the FAA enroute charts | FAA GeoTIFF, public domain |
| **Terrain** | shaded relief with elevation colours | the region's elevation data |
| **Satellite** | aerial imagery | USGS National Map imagery (public domain), on demand, cached |
| **Street** | roads, towns, and place names | rendered from OpenStreetMap data (© OpenStreetMap contributors, ODbL), not from the public OSM tile servers |

**Overlays** (any combination, each with opacity):
- the route and its vertical profile;
- airspace;
- TFRs;
- obstacles from the FAA Digital Obstacle File;
- plates and airport diagrams, georeferenced;
- terrain warnings relative to the aircraft's altitude;
- flight-category dots from METARs;
- winds aloft;
- weather radar (NOAA MRMS or NEXRAD mosaic, public);
- satellite clouds (NOAA GOES, public);
- user pins;
- your location;
- the aircraft and its track;
- simulated traffic.

**Map behaviour:**
- **Orientation:** north-up, track-up, or heading-up.
- **Range:** rings and a scale bar.
- **Tap anything for details:** a card for an airport, navaid, fix, airspace, TFR, or obstacle, with
  actions (direct to, add to plan, start here, open plates).
- **Rubber-band routes:** drag a leg to insert a waypoint.
- **Measure:** a distance and bearing tool between any two points.
- **Night colours:** a night palette for every base map; raster charts are dimmed.

Every live-data overlay is opt-in and shows its time and source (see
[tablet apps, charts, plates, and procedures](efb-and-charts.md) section 5). The Aero map's look is
FlightCraft's own design, not a copy of any app's styling.

## 7. Commands and MCP additions

- **`search.query`:** text in; ranked results with kind, ident, name, coordinates, and details
  out. Network-backed kinds (address, my location) are reported as available but not run.
- **`geocode.lookup`:** an address and a provider; it runs only when the host allows network
  lookups, and returns pins.
- **`location.current`:** the device location, if the user has allowed it.
- **`pins.*`:** list, add, update, remove, import, and export.
- **`plan.*`:**
  - `new`, `add`, `insert`, `move`, `remove`, and `set_altitude`;
  - `summary`, which returns legs, totals, and conflicts;
  - `profile`, which returns the vertical profile samples;
  - `save`, `load`, `import`, `export`, and `activate`, which hands the plan to the GPS.
- **`map.set`:** base map, overlays and their opacity, orientation, range, and centre; `map.state`
  reports them.
- **`navdata.nearest`:** the nearest navaids, fixes, and airports to a point.

## 8. Phases

1. **Navigation data and search:**
   - the importer for navaids, fixes, ILS components, airways, and holds;
   - the World Magnetic Model;
   - the search parser and index;
   - `search.query` and `navdata.nearest`;
   - both bundled regions regenerated with `navdata.json`.
2. **Pins, location, and addresses:** pins, browser geolocation and desktop location services,
   and the geocoder providers.
3. **The planning map:** layers, plan editing, leg math, the vertical profile, conflicts, plan
   files, and activation into the GPS.
4. **Base maps:** Aero (vector), sectional, terminal area, IFR low and high, terrain, satellite, and
   street, plus the overlays above as their data arrives.

## Implementation reference

Phases 1–3, Aero and terrain bases, and regional sectional and terminal chart bases are
implemented, including a top-bar search panel and graphical route editing. Use the
configurable **Focus search** action (default `/`) for the search field. Results show their kind,
distance, details, source dates, and the navigation disclaimer. **Start here** makes a paused air
start 600 m above local terrain, **Direct to** selects a resolved glass GPS waypoint, **Add to plan**
uses 1,000 m MSL initially, and **Pin** saves the result locally.

### Navigation sidecars and magnetism

The sibling offline importer leaves the existing airport/runway importer unchanged:

```sh
cargo xtask navigation-faa --region assets/terrain/example-region/region.json \
  --layers downloads/faa --snapshot-date 2026-11-01 \
  --out assets/terrain/example-region/navdata.json
```

The layer directory contains `NAVAIDSystem.geojson`, `NavaidComponent.geojson`,
`DesignatedPoint.geojson`, `ILS_Component.geojson`, `ATS_Route.geojson`, and
`HoldingPattern.geojson`. Source paths are never embedded in the output. The output must be new.
An optional `--effective-date YYYY-MM-DD` records a verified dataset cycle date. Without it,
`effective_date` is null: component and fix record dates are historical edits, not evidence of a
current FAA cycle. `snapshot_date` is a separate required date. The supplied snapshot has no
verified cycle date; the app displays **cycle unknown**. These data are not for real-world navigation.

Systems join components by ident, equipment family in the FAA client key, and proximity, avoiding
VOTs or localisers sharing an ident. VOR/DME and VORTAC component families are compatible for
joining historical records; each component retains its source client key and subtype so snapshot
disagreements remain visible. System type codes 3, 5, 6, and 8 in the bundled source mean
NDB/locator, DME, VOR/DME, and VORTAC. Component subtype codes, frequencies converted to Hz,
channels, source magnetic variation, and original effective timestamps are retained. Missing
frequency units use kHz for NDB components and MHz for VHF components, as in the FAA source.
Fix DMS positions are checked against point geometry. ILS system/type codes and magnetic bearing
are retained. Airways preserve whole intersecting segments, endpoint UUIDs, resolved endpoint idents
when available, and geometry. Holds preserve reference UUID, inbound course, and turn direction.
Navaids within 40 nm of the grid are retained for radial searches; other point features must be
inside the grid. Segment crossings are retained even when both endpoints are outside.

`schema/navdata.json` publishes the version 1 document. Runtime files are bounded to 8 MiB,
4,096 navaids, 20,000 fixes, 4,096 ILS components, 20,000 route segments, 100,000 route vertices,
and 4,096 holds. IDs are unique, coordinates and optional frequencies are checked, and malformed
or transfer-limited GeoJSON is rejected. Native and browser region loaders validate the optional
sidecar before installation. Browser static packaging includes it automatically.

`world::geomag::MagneticModel` evaluates the degree-12 WMM2025 spherical harmonic expansion in
Rust with `libm`. Coordinates use WGS84 and ellipsoidal altitude; declination is east positive.
Dates must be within 2025–2029. `sim.set_date` sets a validated deterministic calendar date,
defaulting to 2026-01-01 and preserved by resets. `sim.state.simulation_date` reports it.
All 100 NOAA test vectors check the field and annual change. Search radial destinations use
magnetic radial plus WMM declination. GPS guidance retains its documented true-bearing convention;
plan summaries provide both true and magnetic courses.

### Local storage and explicit external actions

`schema/pins.json` defines up to 1,000 pins with unique IDs, names, notes, positions, and optional
MSL altitudes, bounded to 1 MiB. Desktop hosts configure `Session::load_navigation_storage`; the
apps use the operating system's local data directory. Browser pins use local storage, catching
denied access and quota failures through the browser bindings. A failed write leaves session
pins unchanged. Import merges by default and rejects duplicate IDs; `replace: true` replaces them.
Export returns the complete document without opening a file dialog.

**My location** requests permission only after its button or result action is selected. The browser
uses Geolocation with a ten-second timeout. Linux uses GeoClue2 over pure-Rust D-Bus (`zbus`),
requests city-level accuracy, and stops/deletes its client after the lookup. The application identity
is `flightcraft`; the desktop service's policy must allow it. macOS and Windows currently return
“unavailable on this platform, enter a location”. No returned device position is written to disk
unless the user explicitly saves a pin.

An unmatched address offers provider buttons and shows the exact query before sending it.
`geocode.lookup` requires `allow_network: true`, even for cached results. Census requests use the
public current address benchmark. Nominatim uses a globally serialized one-request-per-second gate,
an identifying desktop User-Agent, no bulk API, a bounded local cache, and **© OpenStreetMap
contributors** on every result. The browser supplies its normal Referer. Both providers accept one
address per call and at most 20 results; responses and the 128-entry cache are bounded to 1 MiB.
Desktop HTTP uses `ureq` and Rustls; its `ring` cryptography backend includes C and assembly, so
this optional network transport is an exception to the pure-Rust dependency goal. Provider fixtures
are original hand-written examples of their documented JSON formats. Live network and permission
services require platform testing; no external lookup runs during tests.

### Plans and interchange

`schema/plan.json` defines version 1 plans of at most 64 resolved waypoints, with an MSL altitude
per waypoint and a reserve in seconds (45 minutes by default). Waypoints distinguish airports,
navaids, fixes, pins, and coordinates. Indices are zero based. Invalid edits leave the plan intact.
`plan.move` uses the final index after reordering. Altitudes interpolate between adjacent waypoints.

Summaries use WGS84 inverse geodesics, WMM courses at leg origins, a wind triangle from the current
mean wind at each leg's mean altitude, and cruise fuel flow. Gusts are excluded from estimates.
An impossible wind triangle yields null ETE and fuel, with a conflict. Totals include reserve fuel.
A plan may supply explicit `cruise: {airspeed_mps, fuel_flow_kg_s}`; otherwise the current aircraft's
Vno × 0.8 and engine flow table at 65% power are used as an explicitly labelled planning estimate,
because aircraft definitions do not yet publish measured cruise tables.

Profiles sample great-circle positions at a requested 100–10,000 m spacing, default 1,000 m, with
at most 20,000 samples. Each sample includes terrain, planned altitude, and the floors and ceilings
of all lateral airspace footprints. SFC limits follow terrain. Conflicts identify B/C/D containment
between floor and ceiling, clearance below 1,000 ft, and missing region coverage. Narrow terrain
or airspace features between samples can be missed; these profiles are advisory, not a clearance
or obstacle-avoidance system.

`plan.save/load` use a simple local name (letters, digits, hyphens, and underscores), never an
arbitrary path. `plan.import/export` exchange documents directly:

- **JSON:** the complete FlightCraft plan schema, preserving kinds, cruise, and reserve.
- **GPX 1.1:** exactly one route (`rte/rtept`) or track (`trk/trkseg/trkpt`) on import; export writes
  a route. Latitude/longitude attributes, `ele` in metres MSL, and waypoint `name` are supported.
  Missing elevations become 1,000 m. GPX waypoints import as coordinates.
- **Garmin FlightPlan v1 XML:** namespace `http://www8.garmin.com/xmlschemas/FlightPlan/v1`,
  a `waypoint-table`, and one `route` with `route-point` references. Identifier, type, and country
  code jointly resolve references. Export uses unique `W000` identifiers and `USER WAYPOINT`
  records, with lat/lon and display idents in `comment`. FPL does not preserve planned altitudes;
  imported points default to 1,000 m. The importer also accepts an optional `elevation` extension
  in metres. This is the documented FlightCraft subset, not an implementation of every device
  extension.

XML is limited to 1 MiB and 10,000 nodes, with DTD/entity expansion disabled. GPX and FPL do not
preserve the FlightCraft reserve or cruise override; JSON does. `plan.activate` copies all resolved
waypoints into the merged glass GPS plan through `Session::set_gps_waypoints`, resetting active
leg and course selection. GPS sequencing, CDI, ETE, and flight-plan display use those positions.
The original airport-only avionics commands remain supported. Activation also retains the active
plan in the session; the glass display must be installed to see its guidance.

Bundled snapshot counts:

| Region | Navaids | Fixes | ILS components | Route segments | Holds |
|---|---:|---:|---:|---:|---:|
| New York area | 21 | 887 | 7 | 316 | 15 |
| Eastern Pennsylvania and New Jersey | 20 | 760 | 3 | 378 | 15 |

Unidentified ILS markers retain a null ident rather than acquiring a fabricated navigation ident.

The source snapshot provides no DME component for the EWR system; its component array remains
empty. Some airway endpoint UUIDs have no record in the supplied point layers and retain null
endpoint idents, while their UUIDs and segment geometry remain available.

### Shared planning and MFD map

Open **Map** in the top bar, or use the configurable **Show / hide planning map** action (default
M). **Map zoom in/out** default to +/−. Pan by dragging; wheel and pinch zoom both work. The
planning widget takes a rectangle, clips its drawing and controls to it, and can be hosted by
future layouts. The application MFD map and its pop-out use the same widget renderer, projection,
chart cache, and presentation state. Existing standalone glass instrument snapshots remain usable.
Legacy glass range and orientation controls also update the shared map state. Planning hides
the instrument and autopilot strips to use the full content area, including at a 1000 × 640 window.

The vector Aero base draws a terrain mesh, sea-level/water tint, airport symbols and runway
widths at closer ranges, airspace rings and floor/ceiling labels, navaid roses, fixes, and airways.
Water tint currently uses the region's water surface flags and nonpositive elevation, rather than
a separate shoreline catalogue. It is an approximate coastline. Terrain adds relief shading from
the DEM surface normals. All vector drawing uses theme tokens, with separate day and night
palettes; night dims chart textures. Unknown terrain remains visibly distinct. North-up, track-up,
and heading-up use one local metre frame, preserving bearings and range on rectangular displays.
North-up references true north at the map centre, including grid convergence. Heading-up and
track-up expose WMM magnetic bearings at the aircraft and simulation date; rotation converts
those bearings back to the region frame so the aircraft heading or track remains at the top.
Physical attitude and velocity supply that conversion independently of pilot-facing readouts.
Web Mercator chart tiles are subdivided and projected into that same region frame. Terrain tint
ends exactly at the grid boundary. Available navigation and airspace still draw beyond it, on a
neutral Aero background. Aero also remains visible outside a raster chart's coverage.

Fixes appear below a default 15 NM horizontal map span; fixes in the editable or active GPS route
remain visible at any span. Airways use thin, faint lines, with labels below 20 NM across.
`map.set.declutter` exposes both thresholds, and the map picker provides matching controls.

Each available overlay has an enabled flag and 0–1 opacity: route, airspace, terrain warnings,
airports, navaids, fixes, airways, pins, aircraft, track, and range rings. Aircraft history retains
at most 2,048 positions, spaced by at least 50 m, and clears on reset. Satellite and Street bases,
and live-weather, TFR, and plate overlays, are visibly disabled as coming soon. No network service
is contacted for those choices.

Click an airport, navaid, fix, airspace boundary/interior, or pin for a details card. Runway
sizes and surfaces, navaid frequencies, and airspace vertical references come from the region.
The airport catalogue currently has no communications frequencies, and the card says so. Cards
send existing direct-to, plan, reset, and pin commands; **Open plates** stays disabled.

Enable **Edit route** to add coordinate waypoints by clicking empty map space. Feature cards
add resolved waypoints. Drag a waypoint to relocate it, drag a leg to insert a point, and
right-click or long-press a waypoint to remove it. Waypoints exposes planned MSL altitude controls.
`plan.move` accepts an optional `position` alongside its existing `from` and `to` indices, allowing
relocation and reorder to validate atomically. Dragging preserves the waypoint's ident and kind.
Leg labels show magnetic course, nautical miles, and ETE; the totals strip includes time, fuel,
reserve, and conflict count. Fuel follows the engine/fuel preference (US gallons by default,
using the instruments' 0.72 kg/L conversion; kg in SI). Altitude controls use feet, and planning
airspeed uses knots. The profile uses `plan.profile` samples and `plan.summary` conflicts,
showing filled terrain, B/C/D shelf outlines with labelled floors and ceilings, and planned altitude.
Mode C veils are omitted from the profile. Its axes show feet MSL and nautical miles. Red route segments
identify legs with reported conflicts. Existing sampling and cruise-estimate limitations still apply.

**Activate GPS** sends `plan.activate`. **Import / export** supports GPX, FPL, and JSON through
the existing commands. Desktop exports create a new numbered file under the local data directory's
`exports/` folder without replacing an existing file; browser exports download a file. Import
accepts dropped files or pasted text and validates through `plan.import` before replacement.
Copy text is also available. Programmatic exports still return text and never open a dialog. The **Measure** tool takes two clicks and
shows WGS84 distance and WMM magnetic bearing at the first point, using the simulation date.

### Offline FAA chart tiles

The local-only, pure-Rust importer reads a palette8 or RGB8 GeoTIFF, metric Lambert conformal
conic 2SP GeoKeys, ellipsoid parameters, and a six-coefficient world file. It supports NAD83 and
WGS84 chart coordinates; datum differences smaller than the chart pixel resolution are not
modelled. A sibling FAA `.htm` supplies the verified beginning and ending dates. Source paths are
never written into the output. Inputs are read-only. For example:

```sh
cargo xtask charts-faa --region assets/terrain/example-region/region.json \
  --geotiff downloads/chart.tif --tfw downloads/chart.tfw \
  --kind sectional --out assets/terrain/example-region/charts/sectional
```

The output must be a new directory. Default zooms are z8–z11 for sectional and z9–z11 for TAC;
`--min-zoom` and `--max-zoom` can select a different range, up to z13. Each output is limited to
65 MiB. The chart footprint is the region grid plus a 40 NM margin on every side, independently
of terrain coverage. Pixels outside that expanded footprint become transparent. Collar detection selects the
largest coloured map-face component at a coarse image scale, fills enclosed label holes, and
masks the disconnected legend. For charts with ambiguous colour boundaries, supply a verified
geographic map-face extent with `--neatline west,south,east,north`. Inspect a generated map before
redistributing a new chart. The importer also writes a temporary source overview for inspection.
Synthetic tests exercise collar isolation, GeoTIFF generation and decoding, georeferencing, and
hostile input.

`schema/charts.json` describes `charts/<kind>/index.json`, recording the edition, expiry,
public-domain attribution, zoom bounds, tile coordinates, `coverage_margin_nm`, and encoded byte count. Version 1 PNG tiles use
`<z>/<x>/<y>.png`; version 2 uses lossless WebP at `<z>/<x>/<y>.webp`. The viewer reads both versions. See [data sizes](data-size.md) for current encoded sizes. The checked-in editions are effective **2026-10-29 through 2026-12-23**:

| Region | Base | Zooms | Tiles | PNG bytes |
|---|---|---|---:|---:|
| New York | VFR sectional | 8–11 | 485 | 52,609,137 |
| New York | Terminal area | 9–11 | 178 | 13,533,163 |
| Eastern Pennsylvania and New Jersey | VFR sectional | 8–11 | 543 | 63,124,909 |

The 1,206 tiles total 129,267,209 bytes, excluding the small indexes. TAC coverage follows the chart
face, so western parts of the terrain region remain uncovered. The map displays the edition and
expiry alongside **not for navigation**, or an explicit unavailable/loading message. No base
silently substitutes another chart. Aero is explicitly shown outside chart coverage, and the
map labels that underlay. The TAC top zoom is reduced from z12 to z11 to keep both regions
within the 150 MB chart budget (129,339,184 bytes including indexes).

Desktop reads encoded tiles from the selected terrain directory. Browser maps fetch them from
the static region location, with bounded streaming reads and at most eight requests in flight.
`cargo xtask web` copies chart subdirectories recursively, and Trunk's terrain directory copy
includes them as well. The cache holds at most 96 decoded 256-pixel textures per widget, four
indexes, and a bounded failed-tile set. Loading is limited to four new visible tiles per frame;
a whole chart is never decoded by the runtime.

The ignored `render_planning_map_previews` test writes the actual Aero, sectional, and TAC widget
meshes and font atlas to `out/map/` at full, compact (1000 × 640), and compact content-area
(1000 × 546) sizes, with the 1N7 → SBJ → KMMU plan and vertical profile. Regular
tests cover region-frame and Mercator round trips, tile edges, card hits, simulated drag commands,
and clipped rectangular layouts.

The [emergency debrief](emergencies.md#grading-and-footprint) draws a terrain- and wind-aware glide
footprint at failure time with reachable runway thresholds and unverified terrain candidates.
