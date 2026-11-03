# Instrument procedures and NAV receivers

Instrument procedures come from the FAA CIFP, with plate names and PDF basenames linked from
FAA d-TPP metadata. Both bundled geographic regions carry cycle **2611**, effective **2026-10-29**.
For simulation only, not for real-world navigation.

Rebuild a region offline with the original fixed-column importer:

```text
cargo xtask procedures-faa --region assets/terrain/new-york/region.json --cifp <FAACIFP18> --dtpp <d-tpp_Metafile.xml> --out assets/terrain/new-york/procedures.json
```

Repeat with `eastern-pa`. `--dtpp` is optional; without it, plate links remain null. The metadata
cycle must agree with CIFP. The importer prints departure, arrival, and approach counts by airport.
Only selected airports' terminal records and referenced enroute points survive extraction.
No network request or simulator-derived data is involved.

## File and column contract

`schema/procedures.json` is generated and checked with the command schemas. Version 1 stores
qualified fix references, procedures grouped by airport and ident, route types and transitions,
ordered legs, localizer/glideslope sites, and SBAS path points. Terminal point IDs include airport,
ICAO region, section, and ident; enroute IDs omit the airport. References must resolve. Unknown
JSON fields, nonfinite values, duplicate sequences, broken references, and excessive counts fail
validation before installation. Files are capped at 16 MiB, with 30,000 points, 10,000 procedures,
100,000 total legs, 128 routes per procedure, and 256 legs per route.

The importer accepts 132-column ASCII standard records and a 128 MiB source maximum. Header
records identify the supported cycle. Coordinates use hemisphere, degrees, minutes, and seconds
with hundredths, or ten-thousandths for PP coordinates. Numeric fields are parsed strictly, with
blank optional fields retained as null. The following columns are one-based and inclusive:

| Record | Fields |
|---|---|
| All terminal records | Section 5 (`P`), airport 7–10, ICAO region 11–12, subsection 13 |
| PA | Position 33–41/42–51, variation 52–56, elevation 57–61 |
| PC and EA | Ident 14–18, point ICAO region 20–21, continuation 22, position 33–41/42–51, variation 75–79 |
| D | Ident 14–18, ICAO region 20–21, continuation 22, position 33–41/42–51, variation 75–79 |
| PG | Runway ident 14–18, position 33–41/42–51, threshold elevation 67–71 |
| PI | Ident 14–17, frequency 23–27 (10 kHz), runway 28–32, LOC position 33–41/42–51, course 52–55 (tenths), GS position 56–64/65–74, full course width 84–87 (hundredths), GS angle 88–90 (hundredths), variation 91–95, threshold crossing height 96–97, GS elevation 98–102 |
| PP primary | Approach 14–19, runway 20–24, LTP position 38–48/49–60, angle 67–70 (hundredths), FPAP position 71–81/82–93; original payload retained |
| PD, PE, and PF primary | Procedure 14–19, route type 20, transition 21–25, sequence 27–29, fix/ICAO/section 30–34/35–36/37–38, continuation 39, waypoint description 40–43, turn 44, path terminator 48–49 |
| Procedure guidance fields | Recommended navaid 51–54/55–56/79–80, RF radius 57–62 (thousandths NM), theta 63–66 (tenths), rho 67–70 (tenths NM), course 71–74 (tenths), distance 75–78 (tenths NM), or `T` plus tenths of a minute |
| Procedure constraints | Altitude descriptor 83, altitude 1/2 at 85–89/90–94 (feet, or `FL` hundreds of feet), speed 100–102 (knots), vertical angle 103–106 (hundredths), arc centre 107–111/113–114/115–116 |
| PF continuation W | Continuation 39, application 40; availability at 41, 52, and 63 supplies LPV, LNAV/VNAV, and LNAV capability respectively |

The fourth waypoint-description character `M` identifies the MAP. Subsequent legs carry
`missed: true`; the MAP itself remains on the final segment. The third character `M` identifies
a missed leg. Per-record revision cycles are historical revision identifiers; the catalogue cycle
comes from the file header. Zero-angle PP records are retained as lateral-only records.
Fixtures are small, hand-written records built by their documented columns.

## GPS commands and sequencing

The app’s **Procedures / NAV** menu selects an airport, procedure, transition, or vectors;
loads and activates approaches; commands a missed approach or hold exit; and tunes NAV radios.
Every action sends a command. Use `avionics.set` with a top-level tagged setting. Examples:

```json
{"setting":{"kind":"load_procedure","airport":"1N7","procedure":"R07","transition":"KIMKE"}}
{"setting":{"kind":"activate_approach"}}
{"setting":{"kind":"missed_approach"}}
{"setting":{"kind":"hold_options","suspend":false,"exit":true}}
```

`load_procedure` supports departures, arrivals, and approaches. `transition` is optional, and
`vectors_to_final` defaults to false. Loading retains one procedure of each kind in departure,
arrival, and approach order. Approach activation is explicit. `vectors_to_final` selects the
published final segment and activates the approach. `clear_procedure` restores the ordinary GPS
plan. Loading an ordinary plan or selecting a direct-to also clears procedure guidance.

The receiver executes at 120 Hz inside `Sim`, including when a caller batches many steps.
`avionics.state.procedures` reports loaded identities and plate links, resolved legs, the active
leg, validity, signed deviations, vertical path, CDI scale, waypoint alert, suspension, and mode.
State queries do not consume receiver time or sequence legs. Glass and standby installations use
the same receiver. Loss of receiver power removes guidance rather than selecting another source.

IF, TF, CF, and DF use shared geodesic tracking. CF uses the published inbound course. RF and AF
use centre/radius geometry and the published turn direction. Altitude, distance, radial, and
intercept terminators wait for their specified condition. FM and VM wait for the explicit `sequence_leg` action.
Holds report direct, parallel, or teardrop entry, use standard-rate turns and timed or distance
outbound legs, and retain HM suspension until explicitly released. HF exits after a circuit;
HA waits for the altitude condition. Procedure turns use a bounded holding-style reversal.
Heading legs account for the simulated wind when converting heading to desired ground track.
No turn anticipation, terrain clearance, or obstacle checking is claimed by this receiver.

At the MAP, automatic sequencing suspends. `missed_approach` explicitly selects the first missed
leg and removes approach vertical guidance. Enroute full scale is 2 NM, terminal is 1 NM, and an
activated approach uses 0.3 NM. LPV uses angular scaling bounded between 0.035 and 0.3 NM. A
published final angle anchors a vertical path to its published endpoint altitude. LNAV/VNAV and
LPV guidance require the corresponding published capability; LPV additionally requires an SBAS
path point with a nonzero angle. A published descent angle alone does not confer LPV capability.
The 1N7 R07 data supplies LNAV only.

## Radio reception and autopilot coupling

`nav_radio` contains a tagged radio setting: `standby` (`radio`, `frequency_hz`), `swap` (`radio`),
`course` (`radio`, `degrees`), `back_course` (`radio`, `on`), `ident` (`radio`, `on`), or
`service_volume` (`radio`, `class`: `terminal`, `low`, or `high`). Radio numbers are 1 and 2.
Frequencies are 50 kHz channels from 108 through 117.95 MHz. `cdi_source` selects `gps` or `nav1`.
Active/standby settings and both evaluated signals appear as `nav_radios` and `radio_signals`.

VOR reception uses the station's magnetic variation, an OBS-selected radial, TO/FROM, a cone
of confusion within 40 degrees of the vertical, and class-dependent altitude/range limits.
DME is slant range and remains independent of the VOR cone. ADDS sidecars currently omit station
elevation and service class: the receiver uses terrain elevation and defaults to Low service;
an explicit class setting overrides that default. Ident is a signal-valid tone request stub,
with no audio synthesis. These are deterministic geometric signals, without multipath or noise.

Localizers prefer the richer CIFP PI record; a NAV-frequency localizer from `navdata.json` can
also supply lateral guidance. Course widths use the published data, or a width subtending 700 ft
at the threshold. LOC reception extends 18 NM within 10 degrees and 10 NM within 35 degrees.
Back course reverses sensing and inhibits GS. GS uses its published site, elevation, and angle
within 10 NM, eight degrees laterally, and 0.45–1.75 times the glide angle vertically. The model
produces no false lobes. A sidecar without a published GS angle never invents one.

All receivers supply the existing `NavGuidance` boundary: grid track, signed cross-track metres,
TO/FROM, kind, and optional vertical deviation positive above path. APR arms compatible GS or GP
automatically, captures lateral guidance near centreline, and uses the published descent angle
for feed-forward vertical speed. The legacy unit couples GS; the digital unit also couples GP.
The PFD shows source, approach annunciation, lateral CDI, vertical diamond, and active/armed modes.

The KMMU flight test starts on vectors to ILS Y runway 23, checks APR/GS capture and a sustained
half-dot descent to the published unstarred 436 ft MSL DA from the cycle 2611 FAA plate. CIFP
leg constraints do not encode category decision minima; that acceptance endpoint is a cited test
constant rather than an automatically imported minimum.
The ignored UI test `render_procedure_ils_preview` writes the actual egui PFD to
`out/procedures/kmmu-ils23-pfd.png` after capture.

## Bundled airport counts

Counts are procedure identities, with transitions retained inside each procedure. Cells show
departures / arrivals / approaches; a dash means the airport has no procedures in that region.

| Airport | New York | Eastern Pennsylvania and New Jersey |
|---|---:|---:|
| 06N | — | 0 / 0 / 2 |
| 12N | 0 / 0 / 1 | 0 / 0 / 1 |
| 1N7 | 0 / 0 / 2 | 0 / 0 / 2 |
| 22N | — | 0 / 0 / 2 |
| 39N | 1 / 1 / 2 | 1 / 1 / 2 |
| 3N6 | 0 / 0 / 2 | 0 / 0 / 2 |
| 47N | 1 / 2 / 2 | 1 / 2 / 2 |
| 4N1 | 0 / 0 / 2 | 0 / 0 / 2 |
| KABE | — | 0 / 0 / 10 |
| KAVP | — | 1 / 0 / 6 |
| KCDW | 0 / 3 / 4 | 0 / 3 / 4 |
| KCKZ | — | 0 / 0 / 2 |
| KDYL | 0 / 0 / 2 | 0 / 0 / 2 |
| KEWR | 1 / 6 / 22 | — |
| KFRG | 2 / 4 / 5 | — |
| KFWN | 0 / 0 / 1 | 0 / 0 / 1 |
| KHPN | 2 / 4 / 8 | — |
| KHZL | — | 0 / 0 / 3 |
| KJFK | 2 / 4 / 24 | — |
| KLDJ | 0 / 3 / 0 | 0 / 3 / 0 |
| KLGA | 5 / 5 / 15 | — |
| KMGJ | — | 0 / 0 / 6 |
| KMMU | 0 / 3 / 8 | 0 / 3 / 8 |
| KMPO | — | 0 / 0 / 3 |
| KPTW | — | 0 / 0 / 3 |
| KRDG | — | 0 / 0 / 8 |
| KSMQ | 0 / 3 / 2 | 0 / 3 / 2 |
| KTEB | 1 / 3 / 13 | — |
| KTTN | 0 / 3 / 8 | 0 / 3 / 8 |
| KUKT | — | 0 / 0 / 2 |
| KWBW | — | 0 / 0 / 2 |
| KXLL | — | 0 / 0 / 1 |
| N07 | 0 / 1 / 2 | 0 / 1 / 2 |
| N30 | — | 0 / 0 / 2 |
| N40 | 0 / 0 / 3 | 0 / 0 / 3 |
| N43 | — | 0 / 0 / 1 |
| N47 | — | 0 / 0 / 1 |
| N51 | 0 / 1 / 2 | 0 / 1 / 2 |
| N85 | 0 / 0 / 2 | 0 / 0 / 2 |
| N87 | — | 0 / 1 / 2 |


## Magnetic headings and courses

Pilot headings, selected HDG/TRK, OBS/CRS, GPS desired tracks, waypoint bearings, and displayed
wind directions are magnetic. AHRS and the passive compass use WMM2025 at the simulation date
and aircraft position; built-in worlds use zero variation. The free heading gyro starts on the
compass and subsequently drifts. `avionics.state.magnetic_compass` is independent of AHRS power.

CIFP and localizer courses retain their published magnetic reference. IMMU's published localizer
course is 229.1° magnetic, with a 216.1° true beam at its published 13° west variation. Procedure
leg `course_magnetic_deg` labels use the published course, or the leg geometry with the published
CIFP variation. Live GPS desired tracks convert true geodesic tangents using current WMM; they
can differ slightly from an older published magnetic reference. VOR radials and radial search
positions use the station's own published declination, with WMM only as a missing-data fallback.

`NavGuidance` remains a grid-frame integration interface. Before equipment control, its desired
track and the measured ground track are converted to the same magnetic reference as HDG/TRK.
`Readout.heading_rad` is magnetic; `heading_grid_rad`, `heading_true_rad`, and the explicitly
named true procedure track remain available for geometry and truth transports. No renderer or
geographic coordinate is rotated into the magnetic frame.

[Approach control](atc.md) can issue a typed clearance for the matching approach already loaded
in this GPS. It validates the airport, procedure identity, approach family, and runway; activation
and flight guidance continue through the existing avionics commands.

[IFR lessons](ifr-training.md) select the approach service explicitly. `avionics.set` with
`{"setting":{"kind":"approach_service","lnav_only":true}}` retains LNAV lateral guidance
and suppresses vertical guidance and LPV upgrading for that loaded approach. The receiver anchors
published glidepaths to the final runway endpoint, including approaches with an intermediate
vertical-angle fix.
