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
