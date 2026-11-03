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
| 21c | Shared pilot-eye synthetic vision for glass and touch PFDs, perspective ladder, terrain bands, runway labels, and ground-relative flight path marker (implemented; see docs/glass.md) | 18, 21b |
| 22 | SolidCraft export format agreed and specified | decision with SolidCraft |
| 23 | Vortex-lattice coefficients from geometry | 15 |
| 24 | An aircraft modelled in SolidCraft flies | 22, 23 |
| 25 | Touch controls in the browser | 9 |
| 26 | Real regions: New York area (1N7 to the JFK, EWR, and LGA Class B), then eastern Pennsylvania and New Jersey (KABE, KMPO, KAVP, KRDG) | 18 |
| 27 | Contributor program: golden samples (Sling 2 as the aircraft reference, Corvid, Larrow, library scenarios, 1N7 and KABE detail), one agent skill per contribution type, tests as the acceptance gate; regions stay script-built ([plan](docs/contributor-program.md)) | 34, 35 |
| 28 | Issue claims: `/claim` and `/unclaim`, a draft PR referencing an issue claims it, the lowest-numbered PR wins, a daily sweep frees stale claims, and labelled wanted-aircraft and scenario issues ([plan](docs/contributor-program.md#4-claims)) | 27 |
| 29 | External hardware and home-built panels: HID yokes and quadrants with device profiles, a device protocol for custom boards, layouts on extra screens, and physical panels with cut-template export ([spec](docs/hardware-and-panels.md)) | 21 |
| 30 | Installed autopilots: wing leveler, two-axis legacy, and digital two-axis with flight director, envelope protection, and mode annunciation | 21 |
| 31 | The crew: a deterministic virtual pilot, watching, demonstrating, and instructing modes, intercom and kneeboard instead of a chat box, ACS-graded lessons, and an optional AI link ([spec](docs/crew.md)) | 30 |
| 32 | Navigation data, WMM2025, search, pins, opt-in location and geocoding, flight-plan commands, and glass GPS activation (done); shared planning/MFD map, route gestures, profiles, Aero and terrain bases, and FAA sectional/TAC tiles are implemented; satellite, street, and live overlays remain planned ([spec](docs/map-and-planning.md)) | 18, 21 |
| 33 | Tablet apps, plates, charts, and procedures: desktop GDL 90 and simulator position broadcast to EFB apps (done; traffic encoders ready, no traffic source yet), sectional and terminal chart tiles (done), FAA plates with georeferenced overlay, CIFP procedures in the GPS with ILS and VOR signals, and opt-in live weather and TFRs ([spec](docs/efb-and-charts.md)) | 30, 32 |
| 34 | Training for students: weather and IMC visuals with a view-limiting hood, radios with CTAF traffic and towered ATC, readback checks, scenario scripts, emergencies graded against the POH, and instrument tasks ([spec](docs/training.md)) | 21, 31, 33 |
| 35 | Freeze v1 formats (aircraft pack, panel, scenario, lesson, skin, hardware profile) with `format_version`, readers for every released version, `cargo xtask migrate`, and CI testing golden samples against `main` ([plan](docs/contributor-program.md#5-versioned-formats)) | 34 core phases |
| 36 | Real-world scenery as OGC 3D Tiles: NAIP imagery on 3DEP lidar terrain with real trees, buildings, and water for 1N7 and the Delaware Water Gap (in progress); then open city models for New York and user-key streaming providers (Esri, then Google through Cesium ion), all with no card and no spend ([plan](docs/scenery-sources.md)) | 18 |
| 37 | Photorealistic aircraft exteriors: Blender-scripted Sling 2 with PBR paint, glass, interior, and baked detail; full PBR exterior shading in the engine (in progress; [format](docs/exterior-models.md)) | 15 |

## Aircraft equipment

| Item | Status | Depends on |
|---|---|---|
| Panel-installed wing leveler, legacy two-axis autopilot, and digital two-axis autopilot | Behaviour modules, command controls, original skins, FD bars, shared HDG bug, servo and trim limits, failures, and New York acceptance flights implemented; [reference](docs/autopilot.md) | Systems, instrument modules, panel layers |
| Electronic-suite autopilot attachment | Digital equipment installed on Corvid glass and standby panels; shared AHRS/air-data, GPS guidance, PFD mode strip, and FD bars implemented | Standby and glass suites |
| Autopilot aural warning playback | Original two-tone synthesis, optional CPAL desktop output, Web Audio browser output, and persisted mute/volume implemented | Application audio output |
| Localizer/back-course and GP/GS receiver capture | Geometric NAV receivers, procedure GP, and APR coupling implemented | Navaids and approaches |

Instrument procedures: regional FAA CIFP import, fixed-step procedure GPS, geometric NAV radios,
APR coupling, plate metadata links, WMM magnetic pilot references, and the PFD procedure preview are implemented. See
[procedure coverage and limitations](docs/procedures.md). Automatic plate decision minima and
turn anticipation remain open.


## Optional electronic equipment

- [x] Declarative dual-lane injected controls, two pumps, backup lane power, three-position key,
  gearing, coolant, and manifold indications; existing engines retained.
- [x] Placed breakers and rockers, latched master lights, floor fuel selector, and a deliberately
  operated recovery parachute with fixed-step deployment, inflation, descent, and impact.
- [x] Original touch navigator with shared GPS and COM/NAV tuning, independently arranged touch
  displays, failure reversion, bounded panel backgrounds, and a fictional example pack.
- [x] Shared NAV1 tuning and CDI selection, CIFP procedure controls, magnetic indications, and
  planning-map hosts on the navigator and touch displays. COM audio uses the session radio service; see [radio coverage](docs/radio.md).
- [x] Millimetre shell artwork, placards, physical controls, guarded selectors, full controller
  faceplates, and configurable display compositions, with previews at both sizes.

See [equipment definitions, commands, and previews](docs/equipment.md).

## Geographic data pipeline

One-command region builds, bounded fixture-tested HTTP transport, offline source imports, FAA
edition refresh PRs, compact terrain v2, lossless WebP charts, verified GitHub release archives,
and a showcase Pages budget are implemented. National navigation builds produce geographic
source and normalized type/state shards with an index; fixed 2° terrain cells carry bounds,
origins, checksums, and validation reports. See [the build guide](docs/terrain.md#build-a-region).
The first data release still requires maintainer publication before merging. Nationwide runtime
catalogue discovery and streaming between cells remain follow-up engine/application integration.

Radio foundations and untowered training are implemented with explicit data, weather, audio, and preview limitations; see [radios](docs/radio.md).

Scenario scripts and phase-three emergency training are implemented with bounded validation, aircraft
procedures, deterministic failures, and saved glide debriefs; see [scenarios](docs/scenarios.md) and
[emergencies](docs/emergencies.md). General lesson grading remains separate.
Tower and approach training now adds deterministic ATIS, CRAFT, ground/tower/radar instructions,
readbacks, compliance events, and lost-communications light guns; see [ATC coverage and limits](docs/atc.md).
