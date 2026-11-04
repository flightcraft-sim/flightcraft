# Tablet apps, charts, plates, and procedures

Status: desktop tablet broadcast implemented; charts, plates, procedures, and live data remain specifications.

Pilots plan and fly with an electronic flight bag (EFB) app on a tablet. FlightCraft should feed
those apps like a real aircraft does. It should also carry the FAA's own charts and procedures,
so the same work can be done inside the simulator.

## 1. Feeding EFB apps on your tablet

The desktop app can broadcast the simulated aircraft on your local network to compatible EFB
inputs. Each tablet app controls how it uses the feed; interoperability has not been device-tested.

- **Protocols,** both public:
  - **GDL 90:** the FAA's public data interface for ADS-B receivers, on UDP port 4000. It carries
    ownship position and altitude (heartbeat, ownship report, and geometric altitude) and traffic
    reports. It also carries the published, openly documented extension messages that EFB apps
    use for device identification and AHRS attitude.
  - **The simulator position protocol** that EFB apps accept from flight simulators: plain-text
    `XGPS` position, `XATT` attitude, and `XTRAFFIC` messages on UDP port 49002.
- **Apps:** consult your tablet app's documentation for supported simulator or receiver inputs.
  No per-app compatibility has been verified.
- **Honest identity:** FlightCraft identifies itself as FlightCraft, a simulator. It never claims
  to be a particular receiver model (for example a specific commercial ADS-B receiver).
  Pretending would misrepresent the device to the app and its maker. It could also trigger
  device-specific behaviour that a simulator cannot honour (firmware updates, battery status, and
  weather uplinks). Position, track, altitude, attitude for synthetic vision, and traffic can be
  represented through these public message formats.
- **No fake weather uplink:** FlightCraft does not synthesise FIS-B weather into an EFB. Simulated
  weather must never look like real weather to an app that is also used in real aircraft.
- **Control:** off by default. "Send to tablet apps" in Preferences, or `efb.broadcast`, asks for
  the network: a broadcast address or a specific tablet's address. It shows the packet rate and
  the last error. Desktop only, since browsers cannot send UDP; a remote panel computer can act
  as the sender.
- **Plans from the tablet:** plans made in an EFB come in through the GPX and GPS flight-plan
  file import in [map and planning](map-and-planning.md).

### Fly with your tablet app

1. Put the desktop computer and tablet on the same local network. Select a geographic region,
   such as New York or eastern Pennsylvania. The flat and training worlds have no geodesy;
   FlightCraft sends nothing there and explains why in Preferences and `efb.status`.
2. Open **Preferences → Send to tablet apps**. Enter the tablet's IPv4 address, or your subnet's
   broadcast address (for example, `192.168.1.255` on the corresponding /24 subnet). Enable GDL 90,
   simulator position, or both, enable broadcasting, and choose **Apply tablet settings**.
   Settings are saved locally and restored at startup. Broadcasting defaults to off.
3. In the tablet app, look for its simulator or device listing and enable the applicable input
   if required. Consult that app's documentation for supported protocols and setup. Device discovery,
   display, and synthetic vision have not been verified against third-party apps.
4. Check the packet rate, cumulative per-protocol datagram counts, and last send error in
   Preferences or `efb.status`. Disable broadcasting and apply to stop. Successful UDP sends count
   datagrams handed to the network; they do not prove receipt by the tablet. A firewall, wireless
   client isolation, or a different subnet can prevent delivery, especially with broadcast addresses.

Only the desktop sends UDP. The browser reports **unavailable in the browser**. No weather uplinks,
real receiver identity, or real-world navigation guarantees are provided.

### Implemented wire formats

GDL 90 uses one framed message per UDP datagram, to destination port **4000**:

| Message | ID | Wall-clock rate |
|---|---|---|
| Heartbeat with UTC seconds since midnight | 0 | 1 Hz |
| Ownship report | 10 | 1 Hz |
| Ownship geometric altitude | 11 | 1 Hz |
| Device identification, version 1 | 0x65, sub-ID 0 | 1 Hz |
| AHRS attitude | 0x65, sub-ID 1 | 5 Hz |
| Traffic report, one per other aircraft | 20 (`0x14`) | 1 Hz each |

The original encoder follows FAA **GDL 90 Data Interface Specification, 560-1058-00 Rev A**:
24-bit signed semicircles, nearest-step quantization, 25 ft pressure-altitude steps with a
−1000 ft origin and `0xfff` invalid value, packed velocity, true track, light-aircraft emitter,
CCITT polynomial `0x1021` with initial zero and the GDL table recurrence, low-byte-first CRC,
`0x7e` flags, and `0x7d` XOR-`0x20` escaping. NIC and NACp are zero: simulated truth is not a
certified navigation integrity source. The address is self-assigned from the installation serial,
not an assigned ICAO address. Geometric altitude is MSL in 5 ft steps, with unavailable vertical
figure of merit. The device capability bit for WGS84 ellipsoid altitude stays clear.

The publicly documented [EFB device and AHRS extensions](https://www.foreflight.com/connect/spec/foreflight-gdl90-extension/)
use an eight-byte installation serial derived from an operating-system random value saved in local
app settings. The protocol's eight-byte short name is **FlightCr**, the longest prefix that fits;
the 16-byte long-name field contains **FlightCraft**, zero-padded. FlightCraft never impersonates a
commercial receiver. Attitude is in tenths of a degree, heading is marked true, and indicated and
true airspeed fields are unavailable (`0xffff`), rather than filled with ground speed.

Simulator messages go to destination port **49002**, one ASCII line per datagram, ending in LF:

```text
XGPSFlightCraft,<longitude_deg>,<latitude_deg>,<altitude_msl_m>,<track_true_deg>,<groundspeed_mps>
XATTFlightCraft,<true_heading_deg>,<pitch_deg>,<roll_deg>
XTRAFFICFlightCraft,<numeric_id>,<latitude_deg>,<longitude_deg>,<altitude_msl_ft>,<vertical_speed_fpm>,<airborne>,<true_heading_deg>,<groundspeed_kt>
```

XGPS runs at 1 Hz; XATT runs at 5 Hz. These cadences match the GDL position and attitude updates,
while keeping attitude smoother than position. Coordinates have seven decimal places. Position
altitude, angles, and speed have two decimal places; traffic altitude and vertical speed have one.
Longitude comes first only in XGPS. Pitch is positive nose-up, roll is positive right-wing-down,
track and heading are clockwise from true north, vertical speed is positive climbing, and airborne
is `1` in flight or `0` on the ground. This follows the public simulator-message field conventions;
FlightCraft is the source suffix. Compatibility with a particular tablet app is not claimed.

Every busy-airspace aircraft and every `traffic.set` circuit aircraft is sent as a Traffic Report
(and as an `XTRAFFIC` line when the simulator protocol is on), once a second, so a tablet shows the
same traffic as the [glass displays](traffic.md#displays). The 28-byte report follows the FAA layout
exactly:

| Bytes | Content |
|---|---|
| 0 | ID 20 |
| 1 | traffic alert status in the high nibble (1 while the glass would show a TRAFFIC alert), address type 1 (ADS-B, self-assigned) in the low nibble |
| 2-4 | address: `0xF20000` plus the fleet aircraft id, so it is stable per aircraft and run, and cannot be a real ICAO address; circuit traffic keeps its own address |
| 5-10 | latitude and longitude, 24-bit signed semicircles |
| 11-12 | pressure altitude in 25 ft steps from -1,000 ft (12 bits), then the misc nibble: airborne and true track |
| 13 | NIC and NACp, zero: simulated truth is not an integrity source |
| 14-16 | horizontal velocity in knots (12 bits) and vertical velocity in 64 fpm steps (12 bits, signed) |
| 17 | true track in 360/256 degree steps |
| 18 | emitter category from the type's maximum takeoff weight: 1 light under 15,500 lb, 2 small to 75,000 lb, 3 large, 5 heavy (every current type is 1) |
| 19-26 | callsign: the written registration, upper case, space padded |
| 27 | priority code zero |

`efb::encode::traffic_report` builds one, with exact-byte tests in `encode_tests.rs`. Traffic is
sent whenever broadcasting is on, whether or not the simulated aircraft has an ADS-B In receiver,
because the tablet is its own receiver. The aircraft's own equipment gates only the glass displays.

The app publishes a latest-value truth snapshot each frame to a dedicated sender thread.
Position comes from the active region's geodesy; pressure altitude inverts the simulator's ISA
static pressure, and geometric altitude is its MSL height. Ground velocity and attitude come
from aircraft truth, independent of instrument failures and lag. True bearings account for the
local projection's grid convergence. Sending continues while paused, showing the stationary
simulated aircraft. A snapshot older than two seconds suppresses all packets. Deadlines use wall
time, skip missed periods, and never burst to catch up. With both protocols on, the nominal total
is 15 datagrams/s: nine GDL and six simulator datagrams. The last error persists as a diagnostic;
packet counts reset when the app restarts.

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
