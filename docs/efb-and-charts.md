# Tablet apps, charts, plates, and procedures

Status: specification, not yet implemented.

Pilots plan and fly with an electronic flight bag (EFB) app on a tablet. FlightCraft should feed
those apps like a real aircraft does. It should also carry the FAA's own charts and procedures,
so the same work can be done inside the simulator.

## 1. Feeding EFB apps on your tablet

The desktop app can broadcast the simulated aircraft on your local network. EFB apps then show it
as ownship on their maps, plates, and synthetic vision.

- **Protocols,** both public:
  - **GDL 90:** the FAA's public data interface for ADS-B receivers, on UDP port 4000. It carries
    ownship position and altitude (heartbeat, ownship report, and geometric altitude) and traffic
    reports. It also carries the published, openly documented extension messages that EFB apps
    use for device identification and AHRS attitude.
  - **The simulator position protocol** that EFB apps accept from flight simulators: plain-text
    `XGPS` position, `XATT` attitude, and `XTRAFFIC` messages on UDP port 49002.
- **Apps:** ForeFlight, Garmin Pilot, FltPlan Go, SkyDemon, Avare, and others accept one or both
  protocols.
- **Honest identity:** FlightCraft identifies itself as FlightCraft, a simulator. It never claims
  to be a particular receiver model (for example a specific commercial ADS-B receiver).
  Pretending would misrepresent the device to the app and its maker. It could also trigger
  device-specific behaviour that a simulator cannot honour (firmware updates, battery status, and
  weather uplinks). Everything a pilot needs (position, track, altitude, attitude for synthetic
  vision, and traffic) works through the honest path.
- **No fake weather uplink:** FlightCraft does not synthesise FIS-B weather into an EFB. Simulated
  weather must never look like real weather to an app that is also used in real aircraft.
- **Control:** off by default. "Send to tablet apps" in Preferences, or `efb.broadcast`, asks for
  the network: a broadcast address or a specific tablet's address. It shows the packet rate and
  the last error. Desktop only, since browsers cannot send UDP; a remote panel computer can act
  as the sender.
- **Plans from the tablet:** plans made in an EFB come in through the GPX and GPS flight-plan
  file import in [map and planning](map-and-planning.md).

## 2. Approach plates and airport diagrams

- **Source:** the FAA's digital Terminal Procedures Publication (d-TPP), which is public domain
  and published every 28 days. It covers:
  - instrument approach procedures;
  - airport diagrams;
  - departure procedures and arrivals;
  - takeoff and alternate minimums;
  - hot spots.
- **Import:** `cargo xtask charts-faa` reads the cycle's metadata file and its PDFs for a region's
  airports, and renders each page to images at two resolutions with a pure-Rust PDF renderer. It
  writes a `plates/` folder with an index (airport, chart type, name, cycle, and the image files)
  beside the region.
- **Viewer:** a plates viewer on the kneeboard, as an MFD page, and as a layout element. It has
  search by airport and procedure, pan, zoom, a night mode (inverted, dimmed), and the cycle
  dates.
- **Ownship on plates (georeferencing):** a plate is a picture, so it has to be matched to the
  map before the aircraft can be drawn on it.
  - **Airport diagrams** are drawn to scale with latitude and longitude ticks. The importer finds
    those ticks and the runway ends, and fits the transform from known runway coordinates.
  - **Approach plan views** are fitted from the printed fix and navaid labels matched against
    the navigation data, ignoring insets marked "not to scale".
  - **Quality gate:** every fit stores its control points and residual error. Ownship is drawn
    only when the error is below a stated limit; otherwise the plate is shown without ownship.
  - **Manual fallback:** a control-point file per plate, editable by hand and contributable in
    pull requests, covers plates the automatic fit gets wrong.
- **Overlay on the map:** a georeferenced plan view can be laid over the planning map and the MFD
  map, with adjustable opacity. Airport diagrams can appear automatically at close zoom.

## 3. Charts

- **VFR sectional and terminal area charts,** then IFR low-altitude enroute charts. They are FAA
  georeferenced GeoTIFFs, public domain, and published every 56 days.
- **Import:** `cargo xtask charts-faa` decodes them in pure Rust, removes the collar (margins and
  legends) using the chart boundary, reprojects to the map's projection, and writes map tiles for
  the region at several zoom levels.
- **Use:** the chart is a base layer under everything else on the planning map and the MFD, so
  you can fly the Hudson corridor on the New York terminal area chart.
- **Size:** chart tiles are the largest data in a region. They are optional per region and loaded
  on demand in the browser.

## 4. Procedures in the GPS

- **Source:** the FAA's Coded Instrument Flight Procedures (CIFP), the ARINC 424 database behind
  the published procedures. It is public domain and published every 28 days.
- **Import:** a new importer reads the procedures for a region's airports: approaches (RNAV with
  LNAV, LNAV/VNAV, and LPV minima; ILS; LOC; and VOR), departures, arrivals, holds, and the
  terminal fixes they use.
- **GPS:** the glass and standby GPS load procedures into the flight plan. That means legs and
  path terminators, missed approaches, and activating vectors-to-final. It also covers sequencing
  through a hold and approach mode scaling (CDI scaling from 1 nm down to 0.3 nm, or angular for
  LPV).
- **Simulated ILS and VOR signals:** these come from the navaid and ILS data: localiser and
  glideslope deviation with real course widths, and VOR radials with cone-of-confusion behaviour.
- **Autopilot:** APR couples to LPV glidepaths and ILS glideslopes, as in
  [the autopilot](autopilot.md).
- **Plates and procedures stay linked:** loading an approach opens its plate.

## 5. Live data (opt-in, network)

- **Weather:** METARs, TAFs, PIREPs, and winds aloft from the public aviationweather.gov data API.
  - Shown on the planning map.
  - A "real weather" option drives the simulation's wind, visibility, clouds, temperature, and
    pressure from the nearest reports.
- **TFRs:** temporary flight restrictions from the FAA's public TFR feed, as map polygons with
  their times. They are checked against a flight plan.
- **NOTAMs:** need the user's own FAA API key, entered and stored locally, never shared. They are
  shown per airport.
- **Real traffic:** later, and only from feeds whose terms allow it, using the user's own access.
- **Cycle dates and the source** appear with every item. Live data never runs unless the user has
  turned it on.

## 6. Everything else an EFB does, inside FlightCraft

- **Nav log:** printable, from the active plan.
- **Weight and balance:** already built.
- **Checklists:** per aircraft, from the aircraft pack, checked off by the pilot or the crew.
- **Synthetic vision on the glass PFD:** terrain from the region, with obstacles from the FAA
  Digital Obstacle File (public domain).
- **Logbook:** every flight logs times, airports, landings, and approaches automatically.
- **Track logs:** replay on the map, or export as KML and GPX.

## 7. Data cycles

FAA data changes every 28 or 56 days. Each region records the cycle of each dataset, the app shows
it, and `cargo xtask` can rebuild a region for a new cycle in one command. Every chart, plate, and
procedure screen carries "for simulation only, not for real-world navigation".

## 8. Phases

1. **EFB broadcast:** GDL 90 and the simulator position protocol, with tests against the public
   message formats (CRC, byte stuffing, and field encodings).
2. **Procedures:**
   - the CIFP importer;
   - the GPS flight plan with procedures, and approach mode;
   - simulated ILS and VOR signals;
   - autopilot coupling.
3. **Plates:** the d-TPP importer and renderer, the viewer, georeferencing with the quality gate,
   and map overlay.
4. **Charts:** sectional and terminal area chart tiles as a base layer.
5. **Live data:** weather, TFRs, NOTAMs with the user's key, and real-weather mode.
6. **EFB extras:** the nav log, checklists, synthetic vision with obstacles, the logbook, and
   track logs.
