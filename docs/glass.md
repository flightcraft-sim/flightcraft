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
simulation steps. Paused frames and state queries consume no alignment or battery time.

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
the aircraft. Shading within each band preserves slope relief. Water retains its water colour
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
50 ft AGL red band at KMMU, the disabled pass, and sharing at 120 presentation frames per second.
It captures the equipment example with both touch displays showing synthetic vision and checks
that the installed composition uses one terrain pass.

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
preference and includes its degree sign. Rounded zero has no minus sign.

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
`track_up`), `page` (`map`, `nearest`, or `flight_plan`), `active_leg` (`index`, zero-based),
and `failure` (`target`: `ahrs`, `adc`, `pfd`, or `mfd`; `failed`: boolean). Units are SI
unless explicitly named otherwise. Baro defaults to inHg; the knob menu offers hPa.

The MFD accepts the mouse wheel for zoom. The keymap exposes unassigned bindings for map
range, page, orientation, selected altitude, and course, and reuses heading bug bindings.
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
