# Map, search, and flight planning

Status: specification, not yet implemented.

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
  - Later:
    - FAA VFR sectional charts as a raster base layer (public domain, imported per region from the
      FAA's georeferenced chart files);
    - OpenStreetMap features with attribution.
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
4. **Charts:** sectional chart base layers per region.
