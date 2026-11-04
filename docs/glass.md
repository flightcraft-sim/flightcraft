# Integrated glass cockpit

Select **Corvid glass** in the aircraft picker (`corvid-glass`), or select its
`corvid-glass-variant` and `corvid-glass-panel` layers. The panel has a primary flight display,
a multi-function display, an independent electronic standby attitude instrument, and round
pressure airspeed and altitude backups. The public artwork is original procedural geometry,
using the general colour, tape, failure, and reversion principles in FAA AC 25-11B and
AC 23.1311-1C. It is a fictional training installation.

The displays implement `Instrument` and return `DrawList` geometry. The `original` skin paints
that geometry without simulation access. Both displays consume the same `GlassState` snapshot
returned by `avionics.state`; skins do not calculate navigation or update sensors. Invalid
indications carry validity flags, and the pilot sees a red X. No display changes flight dynamics.

## Sensors and power

`sim::avionics::sensor` implements the AHRS and ADC for both the standby pair and the glass
suite, with independent instances for independently powered units. The AHRS integrates true
body rates in a quaternion with deterministic seeded sensor noise, independent of vacuum gyros,
with a 45-second alignment after power-up or an AHRS failure. Alignment advances only on fixed
simulation steps. Paused frames and state queries consume no alignment or battery time. A runway
`sim.reset` (including a named airport runway) starts ready for departure, with the engine running,
avionics powered, and primary and standby attitude sources already aligned to the starting attitude.
An airborne `sim.reset` (`air`, `airport_air`, `geographic_air`) is an aircraft already flying and starts
the same way: engine running, avionics powered, AHRS aligned and valid, so no red X or "ALIGNING"
appears at t=0. A `parked` start is a cold aircraft and keeps the 45-second alignment after power-up.

The ADC consumes `systems.sensed` pressure airspeed, altitude, and vertical speed. Sealed
pitot and static lines therefore give physically consistent wrong indications. A blocked port
does not raise an ADC failure flag. Baro changes the sensed pressure altitude, and the existing
VSI remains independent of baro adjustments. True airspeed uses indicated airspeed, sensed
static pressure, and temperature. Trend vectors show a six-second prediction; IAS acceleration
has a two-second filter. Temperatures and wind are displayed in degrees Celsius and knots.

The matching systems layer has no vacuum equipment. PFD and MFD loads use the switched
avionics supply; their common computer uses the essential bus and the avionics switch.
The independent essential-bus standby instrument has a 0.5 Ah battery at a 0.5 A rated load
(one hour nominal endurance), using the existing electrical model's discharge and recharge.
It reports battery time when running on backup. Losing the MFD transfers a compact engine
strip to the PFD. An AHRS fault crosses out attitude and HSI heading; an ADC fault crosses
out both tapes and inhibits terrain shading. The standby uses its independent computer.

## Flight and map displays

The PFD includes banked attitude, pitch ladder, roll marks, slip indication, airspeed limit
bands and rotation/climb bugs from variant V-speeds, airspeed and altitude trends, selected
altitude, baro, VSI, heading bug, course and GPS CDI, optional GPS bearing pointer, wind,
OAT, TAS, ground speed, ground track, and transponder code and mode. GPS normally drives the
course pointer; moving the course knob selects a manual course until a new plan is loaded.

The application MFD map and its pop-out use the same rectangular map widget as the full-window
planning map. Aero, terrain, sectional, and TAC bases, orientation, range, and overlay opacity
share `map.set` state; legacy glass range and orientation controls update it too. The vector map
uses a 48-interval terrain mesh and the region's original runway, airspace, and navigation data.
Web Mercator chart textures are projected into that same local metre frame. See
[map and planning](map-and-planning.md#shared-planning-and-mfd-map) for gestures, palettes,
chart editions, coverage, and cache limits.

Terrain warnings are an optional overlay. On the MFD they use indicated aircraft altitude:
yellow at or above 1,000 ft below the aircraft, and red at or above 100 ft below it. Invalid air
data inhibits those warnings; simulation truth never replaces the failed indication. Uncovered
terrain remains explicitly unknown. These bands are situational shading, not a certified terrain
warning system. Range rings and the scale bar preserve their shape on rectangular hosts.
Standalone `GlassState` instrument snapshots retain their 64–256-interval elevation raster and
legacy procedural map artwork for consumers that do not host the interactive widget.

### Traffic

Aircraft with a traffic receiver show other aircraft on the MFD map and annunciate alerts on the
PFD. The receiver is the aircraft's ADS-B In (or TIS-B) installation: the pack's
`systems.definition.adsb_in` boolean. Omission means installed whenever the aircraft has a glass
display (a `glass_pfd` or `glass_mfd` load, or an installed touch display), so the Sling's glass
installation has it with no pack change; `false` declares an aircraft with displays but no
receiver, and then no glass page shows traffic, the TFC keys are absent, and `traffic_installed`
in the glass state is false. The conventional analog panels have none.

Symbols follow the common glass-cockpit ADS-B convention, from the simulator's busy-airspace
traffic and circuit traffic, in one merged list:

| Class | Rule | Symbol |
|---|---|---|
| Other | everything else | hollow arrow along its track |
| Proximate | within 6 NM and 1,200 ft | filled white arrow |
| Alert | within 2 NM and 500 ft, or on course to pass inside 1 NM and 500 ft within 45 s (straight-line closest point of approach, inside 6 NM) | amber filled circle with the arrow, and TRAFFIC on the PFD |

Aircraft on the ground never alert. Each symbol has a relative-altitude tag in hundreds of feet
with a sign (`+05`, `-12`), and an arrow beside it when the vertical rate exceeds 500 fpm.
Alert and controller-named aircraft also show their registration. The map footer shows the range
ring and `TFC <filter>`. The filter is the altitude window, as on glass units: **normal**
(2,700 ft above to 2,700 ft below), **above** (9,900 / 2,700), **below** (2,700 / 9,900), and
**unrestricted** (9,900 / 9,900). An alert is always drawn, whatever the filter, and the TRAFFIC
annunciation ignores the map toggle. A failed PFD removes the annunciation; a failed MFD removes
the map traffic.

Controls: the TFC key shows or hides traffic, and FLT cycles the filter. Both send
`avionics.set` glass settings `traffic` (`shown`) and `traffic_filter` (`filter`: `normal`,
`above`, `below`, `unrestricted`). The PFD map inset and the pop-out MFD use the shared moving
map, which draws the same arrows. When a controller names an aircraft (a sequencing call such as
`Number two, follow the Cessna on left base`, or a traffic advisory), that aircraft gets a magenta
ring on the MFD for about 20 s. See [busy-airspace traffic](traffic.md#displays).

### Synthetic vision

Synthetic vision ignores [visual weather](weather.md), cloud, visibility, and night. The cockpit
hood masks the outside view while leaving this instrument image visible.

The **SVT** control enables pilot-eye terrain behind the PFD's pitch ladder, tapes, and flight
director. It is off by default. Every touch display has its own SVT soft key; changing one display
leaves the other displays' selections alone. AHRS or ADC invalidity inhibits synthetic vision,
restoring the conventional background and failure flags. Failed or unpowered displays show their
existing failure indications. The independent standby keeps its conventional background.

The existing wgpu terrain renderer supplies an offscreen sky gradient, a level horizon,
elevation colours, water surfaces where supplied by the region, and runway outlines. Airport and
runway threshold labels appear within 10 nm, with screen-space label spacing to preserve the
attitude symbology. An open green flight path marker projects ground velocity, including crosswind
drift and climb or descent; it disappears below 5 m/s or outside the attitude viewport.

The camera and pitch ladder share a perspective projection with 2.5 local pixels per degree at
the optical centre. Position and ground velocity come from the immutable flight snapshot, roll
and pitch from the shared AHRS, and terrain awareness from indicated altitude. Terrain within
1,000 ft below the aircraft is yellow, and terrain within 100 ft is red, including terrain above
the aircraft. Shading within each band preserves slope relief. Awareness bands are inhibited
on the ground and below 500 ft true AGL everywhere. From 500 to 1,000 ft AGL they remain inhibited
within 1 NM horizontally of any loaded runway segment (including its thresholds), covering
initial departure and final approach. Above 1,000 ft AGL the bands are enabled even near a runway.
These are FlightCraft advisory-display design thresholds, not certified TAWS alert envelopes.
Background for airport nuisance-alert inhibition is
[FAA AC 23-18, Installation of Terrain Awareness and Warning System](https://www.faa.gov/documentLibrary/media/Advisory_Circular/AC_23-18.pdf).
The 500/1,000 ft and 1 NM values are our presentation choices, not thresholds attributed to that AC. Inhibition uses true
position and ground height, so a barometric setting error cannot paint the departure pavement red.
Only the awareness colours are inhibited; terrain relief, runway outlines, and SVT remain visible.
Water retains its water colour
outside the awareness bands. Coverage depends on the loaded region; absent tiles are not invented.

All enabled displays share one 750 × 285 terrain texture and one depth target, updated at most
30 times per second. Both the main scene and the PFD reuse the same cached terrain buffers;
the PFD selects fine detail near the aircraft and coarse detail farther away. No enabled, visible,
valid PFD means no offscreen pass. A single prepare callback updates the shared image, which egui
composites beneath each display without a separate scene pipeline switch for every display.
Symbology still draws each presentation frame. WebGPU and WebGL 2 use the same WGSL; derivatives
remain outside material-dependent branches.

Use `avionics.set` with `{"setting":{"kind":"glass","setting":{"kind":"synthetic_vision",
"enabled":true}}}` for the glass PFD. Touch displays use `equipment.set` with
`{"setting":{"kind":"display","id":"left_display","action":{"kind":"synthetic_vision",
"enabled":true}}}`. State exposes `glass_suite.settings.synthetic_vision` and each touch display's
`synthetic_vision`; flight reset clears these selections. These are advisory terrain pictures,
not a certified terrain warning system.

The ignored `render_synthetic_vision_previews` UI test captures actual GPU PFD images over the
Kittatinny ridge near 1N7 and on final to KMMU 23 in `out/synthetic-vision`. It also checks the
50 ft AGL awareness inhibition at KMMU, the disabled pass, and sharing at 120 presentation frames per second.
It captures the equipment example with both touch displays showing synthetic vision and checks
that the installed composition uses one terrain pass.
The focused `render_1n7_runway_without_terrain_awareness_bands` GPU test compares runway-start
terrain pixels at the actual indicated altitude and an altitude above all awareness bands. With
inhibition, both pictures must match exactly; it writes `out/polish/svt-1n7-runway-gpu.png`.

Rust panel hosts supply `InstrumentInput::with_vision(world, state)`; after installing the scene
renderer, `render::initialize_synthetic_vision(render_state, context)` registers its shared image.
Custom skins must preserve the position and order of the runtime-only `DrawItem::SyntheticVision`
underlay. Renderer palettes add `terrain_caution` and `terrain_warning`, using the same theme
tokens as the MFD.

The GPU test uses downlevel WebGL 2 limits; set `FLIGHTCRAFT_TEST_BACKEND=gl` to exercise GLES.

The engine strip shows RPM, fuel flow, oil pressure and temperature, EGT, CHT, volts, amps,
and tank sender quantities. RPM, maximum flow, and tank capacities come from the resolved
engine and systems. Temperature, oil pressure, current, and voltage bands use the same
original engineering estimates documented for the conventional instruments; they are not
measured aircraft limits. The strip follows the same engine and fuel unit preference as the round instruments: US
units by default (gal/h, psi, °F, and US gal), with SI available. OAT follows that temperature
preference and includes its degree sign. Manifold pressure uses inHg in US mode (one decimal),
while oil and fuel pressure retain psi; SI mode retains kPa. Rounded zero has no minus sign.

The Sling pack's idle manifold pressure is an estimate (0.30 of ambient, about 9 inHg near sea
level); its POH gives the 1,400 rpm minimum idle but no idle MAP.

The nearest page lists eight airports with distance, bearing, and ETE. The flight-plan page
shows the first eight airport legs with distance, initial magnetic bearing, and ETE; plans accept
up to 64 airports. The active leg is highlighted. Activate a later leg through the editor.
Automatic sequencing occurs within 0.3 nm of a waypoint. Direct-to captures the present
position as the leg origin. All pilot headings, courses, desired tracks, bearings, ground tracks, and wind directions are magnetic.

`sim::avionics::navigation` provides the shared WGS84 inverse distance and bearing,
great-circle cross-track and along-track distance, and local desired-track tangent using
`libm`. Its bounded WGS84 iteration has a spherical fallback; great-circle tracking uses
a mean Earth radius of 6,371,008.8 m. Turn anticipation is not modelled. Regional procedures, NAV receivers, and approach coupling are
described in [instrument procedures](procedures.md). Ordinary GPS CDI full scale is 2 nm en route
and 1 nm within 30 nm of the destination.
Right-of-track displacement produces a left needle correction. A world without a geographic
region has no GPS leg. ETE is unavailable below 1 m/s ground speed.

## Two-screen installations

An installation with two touch displays (the Sling 2) is set up for a pilot in the left seat. The
left display opens on the PFD (attitude, tapes, HSI, engine strip) and the right on the MFD: a
large moving map with traffic, airspace, the flight plan, and the active leg in magenta with its
DTK, XTK, and distance, beside a data column that shows the engine page, flight plan, nearest, or
traffic. Each display can show the PFD, SVT, PFD with map, the MFD map, the engine page, flight
plan, nearest, or traffic, from its soft keys, the Settings window's Cockpit displays section,
or `avionics.set`; the choices are saved per aircraft variant. A pilot-seat setting puts the
PFD on the right. Losing either display, or pressing display backup, reverts the other to the PFD
with an engine strip. See [equipment](equipment.md#pages-roles-and-the-pilots-seat) for the page
list, settings, and reversion rules. The Corvid glass variant has this arrangement by design:
its PFD is the left display and its MFD, which opens on the map, the right; its MFD failure moves
the engine strip to the PFD. The Corvid displays are fixed instruments, so they have no seat swap.

## Landscape touch EFIS model

A touch display can declare `"appearance": {"bezel": {"model": "landscape_touch_efis"}}`. The model is the
engine's description of a **10.6-inch landscape touch EFIS**: the screen shape, the row of physical controls
beneath it, the split-screen arrangement, the page set, and what each knob and key does. A pack names the
real unit it copies in its own readme; the engine does not. Without a `model` a display keeps the generic
bezel (soft keys and corner knobs) unchanged.

An installation with a `model` must have the model's bezel aspect (272 : 195, within 3 percent); the display
is drawn at the largest bezel that fits its rectangle, so it scales uniformly on a desktop or a phone.

### Geometry

| Item | Value | Source |
|---|---|---|
| Screen | 1280 x 768 px, aspect 5:3 | Manufacturer's published specification, as quoted in a public forum post; not read from the installation manual's text |
| Diagonal | 10.6 in (269.2 mm), so the visible screen is 230.9 x 138.5 mm | Pilot's guide 1.1 (display unit, 10.6 inch, landscape); the size follows from diagonal and aspect |
| Panel cutout | 262.00 x 185.10 mm | Installation manual, outline drawing (section 8), read from secondary text |
| Outer bezel | 272 x 195 mm | **Estimate**: the drawing's dimensions could not be read |
| Controls | Left knob, NEAREST, DIRECT, data-card slot, MENU, BACK, right knob, left to right in one row **below** the screen | Pilot's guide 1.2, "Bezel Overview" figure |
| Control positions | Fractions of the bezel width at 7.5, 20.5, 30, 50, 70, 79.5, 92.5 percent | **Estimate** (the figure gives order, not dimensions) |

The pilot's guide's bezel has no soft keys: pages and options are reached by touch, and there are two
dual-concentric knobs (small inner ring, large outer ring, each with a push on the inner ring). The model
exposes the geometry as `flightcraft_aero::display_model::DisplayModel::geometry()`; tests check the aspect,
that the controls fit the bezel without overlapping, and their order.

### Screen modes and pages

Three modes, from the pilot's guide 1.5: **full PFD**, **full MFD**, and **split** (PFD and one MFD pane).
The on-screen SPLIT/FULL button or a BACK press-and-hold toggles between full and split. In split the PFD
takes 60 percent of the width (estimate) and the MFD pane the rest. The MFD has a page bar along its
bottom: the current page's name on the left and abbreviations on the right (Map, Wpt, FPL, Ter, Trf, Info,
Eng); touching one selects it. The nearest pages are reached with NEAREST and are not in the knob cycle.

| Mode | `layout` | Notes |
|---|---|---|
| Full PFD | `integrated_pfd`, `full_pfd`, `svt` | Attitude, tapes, HSI, data bar, engine strip, insets |
| Split | `split` (or the older `pfd_map`) | PFD left, pane right showing the display's `mfd_page` |
| Full MFD | `mfd` (or the older `engine`, `flight_plan`, `nearest`, `traffic`) | `mfd_page` fills the screen; the map page keeps its data column |

The two-screen defaults map straight onto this: the left display opens as a full PFD, the right as a full
MFD on the map page. A split or a full MFD page chosen on either display is saved for the aircraft variant
beside the older `display_page` and `display_data` settings, which still work and name their own page.

MFD pages: map, waypoint, active flight plan, terrain, traffic, info, engine, and nearest. Weather,
charts, checklists, and video are not modelled. Waypoint and info are simple status pages, and terrain
is the map on the terrain base.

### Knobs

Each knob is two rings; turning a ring sends `DisplayAction::Knob`, pushing the inner ring sends `KnobPush`.
Roles follow the pilot's guide (1.5, "knob operation"):

| Mode | Left knob inner / outer | Right knob inner / outer |
|---|---|---|
| Full PFD | Heading bug / altitude bug | Course / baro (default "right side knob action") |
| Split, MFD pane on the right (default) | Heading bug / altitude bug | Map range / MFD page |
| Full MFD | Map range / MFD page | Map range / MFD page |

The "right side knob action" setting selects the alternate pair: course/baro, flight-director bug/baro
(the engine has no flight-director pitch bug, so that inner ring does nothing), or heading/altitude.
"Press to toggle knob action", when on, makes an inner push swap main and alternate functions for 10 seconds
on a PFD knob. With it off, pushing the heading knob syncs the bug to the current heading (rounded to a
degree). Detents: 1 degree for heading and course, 100 ft for the altitude bug, 0.01 inHg (or 1 hPa) for
baro, and the range ladder 1, 2, 3, 5, 7, 10, 15, 20, 30, 50, 70, 100, 150, 200 NM (the real unit has 23
ranges; the engine's range is limited to 1 to 200 NM). Clockwise zooms in. A host supplies `time_s` with each
knob event, so the 10-second revert needs no clock in the engine and replays exactly.

### Keys and menus

| Key | Press | Hold |
|---|---|---|
| NEAREST | Open the nearest page (split, if on a full PFD) | |
| DIRECT | Open direct-to entry; ENTER activates a direct-to to that identifier | |
| MENU | Page menu (PFD options on a full PFD), then the main menu, then clear | |
| BACK | Close the open menu or keypad, else return to the previous page | Toggle full and split |

The page menu lists traffic and track-up on map pages and reads "No Options" elsewhere; the main menu lists
the pages and display backup. The state machine is `flightcraft_control::touch_efis` (pure functions, no
session access). The `equipment.set` display actions are `key`, `knob`, `knob_push`, `mfd_page`,
`toggle_split`, `close_overlay`, `direct_to`, `knobs` (alternate pair, push-to-toggle, pane side), and
`skin`.

### Reversion

As on the real unit (pilot's guide, appendix F), a failed or unpowered display makes the surviving one show
the PFD with its engine strip, and it returns to its chosen page when the other display returns. The
backup setting forces the same on the screen that is not showing a PFD. The autopilot follows the
primary-display rule in the pilot's guide for a PFD failure; the engine's autopilot does not model that
disconnect.

### Skin packs

A skin pack is artwork for a display model, shipped apart from the engine and from aircraft packs. It is a
directory with a `skin.json` manifest (`kind: skin_pack`, `format_version`, `id`, `version`,
`display_name`, SPDX `licence`, `display_model`), an optional palette (bezel, trim, key face and text, knob,
knob mark, screen surround; key text needs 4.5:1 contrast on the key face), up to eight short key labels,
and up to 16 declared font assets with their own licences. It carries no behaviour.

- **Limits.** Manifest 64 KiB; one font 4 MiB; a pack 8 MiB; at most 32 packs; portable relative paths only
  (no `..`, hidden names, or symlinks). A bad pack is reported and skipped, never fatal. Fonts are parsed
  before use; one that does not parse is ignored and the built-in family is used.
- **Loading.** Native builds read the `skins` folder beside `packs`, then the folder named by the
  `FLIGHTCRAFT_SKINS_DIR` environment variable (later ids replace earlier ones). The browser build reads
  `?skin=URL` parameters. Release builds bundle the skins listed in `bundled-skins.json` (a repository and
  commit pinned once, plus the subdirectories to bundle) with `scripts/bundle-skins.py`, which copies only
  `skin.json`, the declared assets, and readme, attribution, and licence files, and fails on a path escape.
- **Choosing.** A display shows the skin the pilot chose in Settings (Cockpit displays), else the skin its
  aircraft pack names in the appearance field `recommended_skin`, else the engine's built-in public skin.
  The recommendation is a soft reference: if that skin is not installed, or is for another model, the
  built-in skin is shown with no error. "Built-in skin" in Settings overrides a recommendation.
- **Private skins.** A skin that reproduces a manufacturer's screens exactly stays private and is never
  published. To use one, put its directory (same manifest format, `display_model: landscape_touch_efis`)
  in a folder outside every repository, and start the app with `FLIGHTCRAFT_SKINS_DIR` set to that folder.
  It appears in the display's skin list beside the public ones, and a pack can recommend it by id. The
  older per-installation `EquipmentSkin` painter hook (`Registry::register_equipment_skin`) remains for
  painters that replace a whole installation's artwork.

The public skin is in the shared skins repository: bezel and key colours, labels, and an open-licensed
font. Instrument faces (tapes, attitude, HSI, map) are drawn by the engine from FAA display conventions
for every skin.

### Not yet matched

The model is not a screen-for-screen copy. Known differences: the PFD's exact element placement, scales and
colour fills follow FAA conventions rather than the guide's figures; the data bar fields are the engine's;
the baro, heading, altitude, and course touch-entry keypads, COM/NAV/transponder tuning windows on the
display, the Select Page menu, nearest sub-pages (VOR, NDB, intersection, airspace), flight-plan editing
by touch, and the AFCS status box on the PFD are not drawn; the inset set and the 23-step range ladder are
reduced; outer bezel dimensions and control positions are estimates.

## Commands and controls

All soft keys, knobs, squawk edits, map range and orientation, direct-to, and plan editing
send `avionics.set`. Its tagged `setting` has a `glass` family containing a tagged equipment
setting. Inputs are validated before replacement; invalid airports, non-finite numbers,
oversized plans, and non-octal squawks return errors.

```json
{"setting":{"kind":"glass","setting":{"kind":"direct_to","airport":"KMMU"}}}
```

```json
{"setting":{"kind":"glass","setting":{"kind":"flight_plan","airports":["1N7","KMMU"]}}}
```

```json
{"setting":{"kind":"glass","setting":{"kind":"transponder","code":"1200","mode":"alt"}}}
```

Other settings are `heading_bug` (`heading_rad`), `course` (`course_rad`), `selected_altitude`
(`altitude_m`), `baro` (`pressure_pa`), `pressure_units` (`hpa`), `bearing_pointer`
(`source`: `off` or `gps`), `map_range` (`range_nm`: 1–200), `orientation` (`north_up` or
`track_up`), `traffic` (`shown`), `traffic_filter` (`filter`), `page` (`map`, `nearest`, or `flight_plan`; the application MFD), `display_page` (`display`, `page`), `display_data` (`display`, `area`), `pilot_seat` (`seat`), `display_reversion` (`enabled`), `active_leg` (`index`, zero-based),
and `failure` (`target`: `ahrs`, `adc`, `pfd`, or `mfd`; `failed`: boolean). Units are SI
unless explicitly named otherwise. Baro defaults to inHg; the knob menu offers hPa.

The MFD accepts the mouse wheel for zoom. The keymap exposes unassigned bindings for map
range, page, orientation, selected altitude, course, traffic show/hide (TFC), and the traffic
altitude filter (FLT), and reuses heading bug bindings.
The PFD and MFD pop-out buttons open resizable displays for either window size. Pop-outs
paint the same snapshot and remain available while the flight is paused.

The single `avionics.state` response retains the standby fields and adds `glass_suite`, including settings, both sensor computers and flags,
GPS guidance, engine strip values and ranges, a compact elevation raster, airport runway geometry, and
projected airspace rings. Map coordinates are normalized to a square spanning twice the
selected range, centred on the aircraft. No truth substitution occurs for failed pressure
instruments. Snapshot reads do not advance the simulation.

## Verification

Tests cover alignment and failure independence, sealed-port indications, standby battery
switchover, tape coordinates, colour boundaries, New York Class B geometry, regional
navigation against an independent central-angle calculation, schemas, and actual egui
font bounds at both window sizes. The ignored `render_glass_preview` test rasterizes the
real egui meshes and font atlas over the Hudson near 3,000 ft, writing
`flightcraft-glass_pfd.png` and `flightcraft-glass_mfd.png` in the temporary directory.

Rust panel hosts pass `Session::glass_state()` through `InstrumentInput::with_glass` before
updating the `glass_pfd`, `glass_mfd`, or `glass_standby` module. `Session::set_glass` is the
validated equipment hook used by `SetAvionics`; applications should use the command path.
`InstrumentPanel::take_glass_commands` drains wheel actions for command dispatch. The
`avionics.set` family tag is additive, so a host can expose several equipment families under
one command registry entry and one state response.

## Shared heading selection

The aircraft has one selected heading in `systems.controls.heading_bug_rad`. The conventional
heading indicator, standby HSI, glass HSI, and an engaged autopilot heading hold all use it.
Selecting a heading hold initializes that same bug. Changing the bug updates an engaged heading
hold immediately. The selected heading is displayed as `HDG 057`, independently of actual heading.
Every rose uses the same notched marker geometry. Partial-arc skins pin an out-of-view selection
to the nearest edge and point toward the shorter turn.

Each heading unit draws an HDG knob. Wheel or horizontal/vertical drag turns it; a centre push
syncs the bug to the current displayed heading, rounded to the nearest degree. New keymaps bind
comma and period to decrement/increment and H to sync; existing custom keymaps keep their keys.
The controls send the same commands available to hardware encoders:

```json
{"setting":{"kind":"heading_bug","degrees":57}}
{"setting":{"kind":"heading_bug_step","detents":2,"interval_ms":120}}
{"setting":{"kind":"heading_bug_sync"}}
```

A detent is 1°. Supplying an inter-event interval below 80 ms accelerates it to 10°; omitting
the interval retains 1°. Counts are bounded to ±360 per command, and selection wraps through
north. Timing is supplied by the controller, so replay does not read a clock. Tests exercise
359 + 2 = 001, reverse wrap, sync, acceleration, all display readouts, and heading-hold coupling.

## Resolved navigation waypoints

`avionics.set` additionally accepts glass settings `flight_plan_waypoints` and
`direct_to_waypoint`. Each waypoint carries `ident`, `kind` (`airport`, `navaid`, `fix`, `pin`,
or `coordinate`), and `position: {latitude_deg, longitude_deg}`. The original airport-only
settings remain available. `plan.activate` sends the editable plan through this same resolved
GPS hook. `GlassSettings.waypoints` contains positions in plan order; the existing `flight_plan`
ident list remains available. Repeated idents can occupy different positions, because guidance
resolves by leg index. Automatic sequencing, desired track, CDI, and the MFD flight-plan list
use these resolved positions. The legacy `NavigationLeg.airport` field is retained for compatibility
and now carries the waypoint ident for every kind.

The GPS and planning leg labels report magnetic courses using WMM2025 and the deterministic
simulation date; planning summaries also expose explicitly named true courses. See
[map and planning](map-and-planning.md).


## Magnetic reference

AHRS headings and the standby and glass HSI roses use WMM2025 declination at the aircraft's
position, altitude, and simulation date. Built-in worlds use zero variation. The independent
`avionics.state.magnetic_compass` is an ideal passive whiskey compass, without turning or
acceleration errors. The conventional free heading gyro is set to the compass at reset and
then retains its existing drift; it is not slaved when the date or position changes.

Heading bugs, HDG and TRK targets, OBS/CRS, GPS bearings, desired tracks, nearest-airport bearings,
and indicated wind directions are magnetic. LOC and VOR course selections retain published
station references. Procedure leg labels retain published CIFP magnetic courses and variation;
live geometric GPS tracks use current WMM, so older published variation can differ slightly.

Track-up rotates map geometry by the grid ground track; heading-up converts the magnetic AHRS
heading back into the grid before rotating. Numeric heading/track readouts stay magnetic.
North-up follows true geographic north. This avoids rotating the map twice by the declination.

## Display layout

The attitude viewport covers the entire PFD, including the space beneath the tapes, HSI,
corner data boxes, and insets. The HSI has a translucent backing confined to its rose.
Standby attitude uses the full face, with compact readout boxes along the lower edge.
Touch PFD and integrated PFD pages share this composition; their insets overlay the bottom
corners. Tape readouts use larger type than secondary data. Map pages fill their content
rectangle beside a narrow engine column, with map information overlaid at the bottom.

The layout regression tests measure uncovered background across Corvid, standby, and touch
PFD content and verify panel width at desktop and phone viewport sizes. The ignored
`render_layout_previews` UI test writes 1920×1080 captures under `out/layout/after/`;
set `FLIGHTCRAFT_LAYOUT_STAGE=before` to capture the baseline before changing layouts.
An optional `FLIGHTCRAFT_LAYOUT_PACK` directory includes a local external pack in the capture.
CPU captures show the analytic attitude background; synthetic terrain requires the GPU preview.

The HSI symbol cluster scales uniformly within wide touch and pop-out displays, so the rose,
CDI, heading bug, and translucent disk remain circular while the attitude background fills
the rectangle. The extended synthetic-vision viewport preserves the optical centre and
focal scale used by the pitch ladder and flight-path marker.
