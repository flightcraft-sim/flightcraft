# Roadmap

Ranked: work goes roughly top to bottom. Each item names what it depends on. The specification is
`docs/SPEC.md`.

## Now: M1 First flight

| # | Item | Depends on |
|---|---|---|
| 1 | Workspace, licences, CI, licence check (`cargo deny`), layer check (done) | nothing |
| 2 | ISA atmosphere and flat world with one runway (`world`) (done) | 1 |
| 3 | Corvid aircraft definition and coefficient tables (`aero`) (done) | 1 |
| 4 | 6-DOF rigid body, integrator, gear contact (`sim`) (done) | 2, 3 |
| 5 | Golden test: scripted takeoff, climb, and landing with a fixed trajectory hash (done) | 4 |
| 6 | Command registry with schemas and the session (`control`) (done) | 4 |
| 7 | Renderer: ground, runway, aircraft, chase camera (`render`) (done) | 4 |
| 8 | Desktop app: keyboard and gamepad, basic instruments, configurable shortcuts, theme tokens (done) | 6, 7 |
| 9 | Browser build of the same app (done) | 8 |

## Next: M2 Agent pilot

| # | Item | Depends on |
|---|---|---|
| 10 | Headless MCP server over stdio (`apps/mcp`) (done) | 6 |
| 11 | Agent test: a scripted client flies the traffic pattern and lands (done) | 5, 10 |
| 12 | MCP bridged to the running app, with screenshots (done) | 8, 10 |
| 13 | Model accuracy tests (stall, trim, phugoid, damping) (done) | 4 |
| 13a | Hold commands (altitude, heading, airspeed, climb rate) so an agent flies a pattern in tens of calls, not thousands (done: 32 calls) | 6 |
| 14 | Golden hash checked on macOS, Windows, and wasm in CI (done) | 5, 9 |

## Later

| # | Item | Depends on |
|---|---|---|
| 15 | Aircraft file format, versioned, hostile-input safe (done) | 3 |
| 16 | Constant wind, then gusts and turbulence (done) | 4 |
| 17 | Engine failure and other failures as commands (done) | 6 |
| 18 | Terrain Tiles elevation and FAA airports for the New York area and Eastern Pennsylvania and New Jersey, with WGS84 transverse Mercator, geographic commands, runway starts, and static browser loading (done) | 7, decision on data hosting |
| 19 | FAA public airports and runway markings (done); OpenStreetMap features remain planned | 18 |
| 18a | FAA controlled airspace sidecars, New York and eastern Pennsylvania shelves, volume queries, curtains, and status warnings (done) | 18 |
| 20 | A second fictional aircraft | 15 |
| 21 | Cockpit view, conventional sensed flight instruments and runtime skins, readable engine cluster, unit preferences, momentary panel controls, and fuel-driven mass/CG/inertia (done; see docs/instruments.md) | 8 |
| 21a | Corvid electronic standby pair: independent AHRS and batteries, GPS HSI and reversion, command controls, and pilot-visible MCP state (done; see docs/instruments.md) | 18, 21 |
| 21b | Corvid integrated glass PFD, MFD, shared air data, regional GPS plan, smooth terrain map, failure reversion, shared sensors, and shared heading bug (done; see docs/glass.md) | 18, 18a, 21 |
| 22 | SolidCraft export format agreed and specified | decision with SolidCraft |
| 23 | Vortex-lattice coefficients from geometry | 15 |
| 24 | An aircraft modelled in SolidCraft flies | 22, 23 |
| 25 | Touch controls in the browser | 9 |
| 26 | Real regions: New York area (1N7 to the JFK, EWR, and LGA Class B), then eastern Pennsylvania and New Jersey (KABE, KMPO, KAVP, KRDG) | 18 |
| 27 | Contributor guides and agent skills: how to contribute an aircraft, a region, or an instrument skin, written so a coding agent told "contribute to FlightCraft" knows what to do | 26, instrument modules |
| 28 | Issue claims: `/claim` and `/unclaim`, a draft PR referencing an issue claims it, the lowest-numbered PR wins, and a daily sweep frees stale claims | 27 |
| 29 | External hardware and home-built panels: HID yokes and quadrants with device profiles, a device protocol for custom boards, layouts on extra screens, and physical panels with cut-template export ([spec](docs/hardware-and-panels.md)) | 21 |
| 30 | Installed autopilots: wing leveler, two-axis legacy, and digital two-axis with flight director, envelope protection, and mode annunciation | 21 |
| 31 | The crew: a deterministic virtual pilot, watching, demonstrating, and instructing modes, intercom and kneeboard instead of a chat box, ACS-graded lessons, and an optional AI link ([spec](docs/crew.md)) | 30 |
| 32 | Map, search, and planning: navaids, fixes, airways, magnetic variation, one search box for anything, your location, addresses, pins, and a planning map with profile and conflicts ([spec](docs/map-and-planning.md)) | 18, 21 |
| 33 | Tablet apps, plates, charts, and procedures: desktop GDL 90 and simulator position broadcast to EFB apps (done; traffic encoders ready, no traffic source yet), FAA plates with georeferenced overlay, sectional and terminal charts, CIFP procedures in the GPS with ILS and VOR signals, and opt-in live weather and TFRs ([spec](docs/efb-and-charts.md)) | 30, 32 |

## Aircraft equipment

| Item | Status | Depends on |
|---|---|---|
| Panel-installed wing leveler, legacy two-axis autopilot, and digital two-axis autopilot | Behaviour modules, command controls, original skins, FD bars, shared HDG bug, servo and trim limits, failures, and New York acceptance flights implemented; [reference](docs/autopilot.md) | Systems, instrument modules, panel layers |
| Electronic-suite autopilot attachment | Digital equipment installed on Corvid glass and standby panels; shared AHRS/air-data, GPS guidance, PFD mode strip, and FD bars implemented | Standby and glass suites |
| Autopilot aural warning playback | Original two-tone synthesis, optional CPAL desktop output, Web Audio browser output, and persisted mute/volume implemented | Application audio output |
| Localizer/back-course and GP/GS receiver capture | Equipment modes and typed guidance boundary ready; operational receiver and approach data remain needed | Navaids and approaches |
