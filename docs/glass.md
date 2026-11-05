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
takes 56.3 percent of the width (its first 12 percent is the engine column) and the MFD pane the rest,
measured from the split-screen figures (pilot's guide 1.5). The MFD has a page bar along its bottom
(above the status bar): the current page's name on the left and abbreviations on the right (Map, Wpt,
FPL, Ter, Trf, Info, Eng); touching one selects it, and touching the name opens the **Select Page** menu
(pilot's guide 1.6): the main pages, or the nearest pages while the nearest group is showing. The nearest
pages are reached with NEAREST and are not in the main knob cycle.

| Mode | `layout` | Notes |
|---|---|---|
| Full PFD | `integrated_pfd`, `full_pfd`, `svt` | Attitude, tapes, HSI, data bar, status bar, and the pack's insets (no engine column) |
| Split | `split` (or the older `pfd_map`) | PFD left, pane right showing the display's `mfd_page` |
| Full MFD | `mfd` (or the older `engine`, `flight_plan`, `nearest`, `traffic`) | `mfd_page` fills the screen; the map page keeps its data column |

The two-screen defaults map straight onto this: the left display opens as a full PFD, the right as a full
MFD on the map page. A split or a full MFD page chosen on either display is saved for the aircraft variant
beside the older `display_page` and `display_data` settings, which still work and name their own page.

MFD pages: map, waypoint, active flight plan, terrain, traffic, info, engine, and nearest. Weather,
charts, checklists, and video are not modelled. Waypoint and info are simple status pages, and terrain
is the map on the terrain base.

### Primary flight display

The PFD's placement is the model's, not the pack's or the skin's:
`DisplayModel::pfd_geometry(PfdMode::Full | Split)` returns the rectangle of every element in the model's
native 1280 x 768 pixels, and the drawing (`ui::glass::touch_pfd`) and the touch boxes both read it. The
numbers are measured from the pilot's guide's figures (full-screen PFD, section 2.1; split mode, section
1.5), which give arrangement but no dimensions, so each is good to a few percent. Geometry tests check the
arrangement, not the pixels: tapes either side of a centred attitude and compass, the true-airspeed box
above and the ground-speed box below the airspeed tape, the selected-altitude box above and the baro box
below the altitude tape, the vertical-speed scale against the altitude tape's right edge, the heading,
selected-heading and course boxes in one row over the compass, the wind box left of the compass, and the
AFCS box top-centre under the data bar.

| Element (full PFD, px of 1280 x 768) | Rectangle (x, y, w, h) |
|---|---|
| Data bar / radio box / split button | 0,0,1280,60 / 0,0,238,60 / 1212,8,56,43 |
| AFCS status box | 448,65,368,33 |
| Airspeed tape, TAS box, GS box | 345,127,86,269 / 345,98,86,29 / 345,400,86,27 |
| Altitude tape, selected altitude, baro | 824,135,111,253 / 840,98,95,29 / 840,399,95,32 |
| Vertical speed scale | 935,135,39,253 |
| Slip box, turn-rate scale | 578,361,143,28 / 545,407,190,24 |
| Heading, selected heading, course boxes | 608,423,64,34 / 483,435,84,27 / 708,435,92,27 |
| Wind box | 393,469,90,47 |
| Aircraft symbol, roll arc | centre 640,261, radius 135 |
| Compass rose | centre 637,603, radius 131 |
| CAS window | 436,102,108,54 |
| Flap position indicator, elevator trim indicator | 440,282,26,104 / 472,282,26,104 |

In split mode the tapes keep their pixel size and are pinned to the pane's edges, and the compass is
centred in the pane; the data bar loses its middle fields and gains the transponder box.

What is drawn, from the guide: airspeed tape from 30 kt with ticks every 5 and numbers every 10, the speed
range strip (red low-speed, white flap range, green normal, yellow caution, red above never-exceed), the
magenta trend vector (6 second prediction), speed references (R, X, Y), and a pointer that turns red above
never-exceed; the altitude tape with 100 ft numbers and 20 ft ticks, a cyan selected-altitude bug pinned to
the tape edge when off the scale, the magenta trend vector, and a rolling-number pointer; the vertical
speed scale (compressed above 1,000 fpm) with its pointer and digital readout; the attitude indicator with
pitch marks every 2.5 degrees, the roll scale (ticks at 10, 20, 30, 45, and 60 degrees, an inverted triangle
at zero), the slip ball, and the standard-rate turn scale; the heading-up compass with ticks every 10 and 5
degrees, the selected-heading bug, the track triangle, the course pointer and deviation bar (magenta for
GPS, green for a radio), and the source and CDI scale annunciations; the wind box with an arrow and speed
over direction; and the top data bar (COM1 active and standby, then waypoint, bearing, distance, time en
route, ground speed, track, minimum safe altitude, and vertical speed required) and the bottom status bar
(the function of each knob's two rings, outside air temperature, Messages). The minimum safe altitude is
not modelled (its slot reads dashes, or shows the clock or flight timer, below), and vertical speed required
reads dashes until a vertical path exists.

Colours are by meaning, as in the guide: cyan for selected values, bugs, baro, and the standby frequency;
magenta for the flight director, trend vectors, GPS guidance, and data bar values; green for active modes,
the active frequency, and a valid VOR or localizer; white for scales and armed modes; yellow for the
aircraft symbol and cautions; red for warnings and failure flags.

**AFCS status box.** `flightcraft_control::touch_pfd::afcs_box` turns the autopilot's state into the box:
autopilot and yaw damper at the far left, lateral modes in the centre, vertical modes on the right; active
modes green, armed modes white, a disconnect flashing the autopilot annunciation yellow; ROL, HDG, TRK, the
navigation source name (GPS, VOR, LOC) or BC; PIT, ALT, VS, IAS, GP, GS, and ALTS armed; the reference of
altitude hold, vertical speed, and indicated airspeed beside the vertical mode. The box is the existing
mode machine's output; nothing is stored.

**Flight director.** Single-cue magenta bars that move with the pitch command and bank with the roll
command: hollow while the pilot hand-flies with the director alone, solid with the autopilot engaged, and
absent in reversion.

**CAS window.** `flightcraft_control::touch_cas::window` turns the aircraft's active annunciations (the
sources of the master caution and warning lamps) and the autopilot's failures into the crew alerting
window of the pilot's guide (3.7, printed pages 106 and 107; appendix 10.5, pages 376 and 377): warnings in
red first, then cautions in yellow, then advisories in white, up to three lines, and a down-pointing
triangle when more are active. Nothing is drawn while no message is active. A warning or caution
that has not been acknowledged flashes (twice a second); touching the window, or pressing BACK with no
menu open, acknowledges every active one (`equipment.set` display action `acknowledge_cas`, which clears the
same master latches as the panel's CAUTION and WARNING lamps). The window is the existing annunciators'
second face, not a second set of alerts: nothing is stored, and a message ends when its cause does. The
pack's own annunciation labels are what the window shows (LOW VOLTAGE, FUEL PRESSURE, COOLANT, CHUTE ARMED
and so on); an autopilot failure reads ROLL SERVO, PITCH SERVO, PITCH TRIM, or AP SENSOR. The guide hides the
window in split-screen on the GDU 450 and 470 only; this model is the 46X, so it shows in every PFD arrangement.

**Trim and flap position.** Two vertical indicators left of the aircraft symbol, from the guide's "Trim and
Flap Position Indicators" figure (page 81, an optional indication that an installer enables): the flap scale
runs from 0 at the top to the last detent at the bottom, with the deflection in degrees when the variant
declares `deflection_deg` on its `flap_stages` (the Sling 2 declares 0, 10, 20, and 32 from POH 7.2), else
F; the elevator trim scale runs DN at the top to UP at the bottom with a green neutral mark. White pointers
show the selector position (`controls.flaps`) and the trim control (`controls.trim`, plus or minus 0.3).
The engine's flaps have no travel time, so the pointer is the commanded position. A host that supplies no
pilot controls draws neither indicator. Aileron and rudder trim are not modelled.

### Touch entry

Touching a PFD box opens a keypad over the display, as the guide describes (section 1.6 data entry; 2.1
for each box). `flightcraft_control::touch_entry` is the pure part: which characters each target accepts,
how typing edits the field, how an entry parses and range-checks, and the shortcut buttons.

| Box touched | Keypad | Accepts | Shortcuts |
|---|---|---|---|
| Selected altitude | altitude | 100 to 30,000 ft | Set To Current (nearest 100 ft) |
| Baro | baro | 27.50 to 31.50 inHg (2992 reads as 29.92) or 900 to 1100 hPa | Set To Standard, Set For Field |
| Minimums (under the baro box) | minimums | 0 to 15,000 ft | Minimums Off |
| Selected heading | heading | 0 to 360 (360 is north) | Set To Current |
| Course | course | 0 to 360 | Set To Current (desired track, else heading) |
| DIRECT key | waypoint | 1 to 6 letters or digits | |
| COM1 standby, NAV1 standby | frequency | 118.000 to 136.975 on 25 kHz; 108.00 to 117.95 on 50 kHz | |
| Transponder | code | four digits, 0 to 7 | |
| FPL page, Insert Before / Insert After | waypoint | 1 to 6 letters or digits | |
| FPL page, Alt At / Alt At/Above / Alt At/Below | waypoint altitude | 0 to 45,000 ft | |

A hardware keyboard types into the pad as well. ENTER is enabled only for an entry that parses, the pad
shows why it does not, and the engine rejects a bad entry with the keypad left open (`equipment.set`
display actions `open_overlay`, `enter`, `shortcut`).

### Baro minimums

The pilot sets a barometric minimums altitude (decision altitude or minimum descent altitude, feet MSL) in the
minimums box under the baro box, or with `avionics.set` glass setting `{"kind":"minimums","altitude_m":76.2}`
(`-500` to `15000` m, finite; `{"kind":"minimums_off"}` removes it; the keypad accepts 0 to 15,000 ft and has a
Minimums Off button). While set, the altitude tape carries an amber minimums bug on its left edge, pinned to
the tape's edge when off the scale, and the box shows `MIN 1,000`; unset it shows `MIN ----`.
`flightcraft_control::minimums` is the pure part, comparing the indicated (baro) altitude with the minimums:
more than 100 ft above, nothing but the bug; within 100 ft above, an amber outlined **MINIMUMS** annunciation
at the top of the altitude tape; at or below the minimums, the same annunciation solid amber with dark text, so
the two states differ by more than the word. The guide's own colours for this annunciation were not available
to the model; the choice here keeps amber for a caution and reserves red for warnings, as the rest of the
display does.

The audio alert is a spoken **"Minimums."** callout on the crew voice path (the one that says "Five hundred"
and "One hundred"), sounded once on the step that first finds the aircraft at or below the minimums after a
step that found it above. It sounds again only after a climb back above, never at the moment the minimums are
set below the aircraft, and never while the altitude data is invalid. The warning tone (`audio::Audio`) is
tied to the autopilot disconnect and has no named callouts, so the voice path is the hook the engine has.

### Vspeed bugs

The airspeed tape draws a letter right of the tape at each enabled Vspeed: **R** (Vr), **X** (Vx), **Y** (Vy),
**G** (Vg, best glide), and **A** (Vapp, approach). MENU on a full-screen PFD, then **Vspeed bugs**, opens the
configuration page: one row per bug, touching the row turns it ON or OFF, and **Set** opens a speed keypad
(20 to 400 kt; the keypads for Vr, Vx, and Vy also offer **Pack Default**). Vr, Vx, and Vy default to the
aircraft pack's `v_speeds` (`vr_mps`, `vx_mps`, `vy_mps`) and are on, as the tape always showed them; the pack
format has no best-glide or approach speed, so Vg and Vapp start off with no speed and show only once the pilot
sets one. A bug off the scale is not drawn. The glass settings are `vspeed` (`bug`: `r`, `x`, `y`, `g`, `app`;
`enabled`) and `vspeed_value` (`bug`, `speed_kt`, 20 to 400 or null for the pack's); both are in
`glass_suite.settings.vspeeds`.

### Clock and flight timer

The data bar's minimum safe altitude slot (the engine models no minimum safe altitude, so it read dashes) is
also the **user field**: touching its left half cycles dashes, the clock, and the flight timer; with the timer
shown, touching its right half starts and stops it. The same choices are glass settings, so they work from
`avionics.set` and replay: `data_bar_field` (`field`: `safe_altitude`, `clock`, `timer`), `clock_format`
(`format`: `utc` or `local`), `clock_set` (`utc_seconds`, 0 to 86,399), and `timer` with an `operation` of
`{"kind":"start"}`, `stop`, `toggle`, `reset`, `{"kind":"direction","direction":"up"|"down"}`, or
`{"kind":"preset","seconds":n}` (1 to 359,999 s, finite; a bad preset is refused and changes nothing).

Both run on **simulation time** (`flightcraft_sim::avionics::glass_aids`, pure and unit tested), so they stop
when the simulation is paused and give the same readings on replay. The sim has no calendar clock: the flight
starts at 17:00 UTC (the renderer's default illumination hour) unless `clock_set` says otherwise, which moves
the clock's zero point. Local time is UTC plus the aircraft's nautical time zone, one hour per 15 degrees of
longitude rounded; there is no time-zone database, daylight time, or date. The timer counts up from zero or down
from its preset and holds while stopped; a count-down stops at zero, cannot be started again until reset, and
turns amber on the data bar. The clock reads `HH:MM:SS` labelled UTC or LCL; the timer reads `MM:SS`, or
`H:MM:SS` from an hour. `avionics.state` carries the readings in `glass_suite.clock`.

### Vertical navigation (VNAV)

A flight-plan waypoint can carry an **altitude constraint**: `at`, `at_or_above`, or `at_or_below` an altitude
(`GpsWaypoint.altitude_constraint`, optional, 0 to 15,000 m MSL, validated wherever a plan is installed and
carried through every FPL edit, so inserting or removing other waypoints leaves it on its waypoint). On the FPL
page, select a waypoint and touch **Alt At**, **Alt At/Above**, or **Alt At/Below** for a keypad (0 to 45,000 ft),
or **Alt Clear**; the constraint shows on the row (`3000FT`, `2500FT A`, `4000FT B`). The same edit is the
`equipment.set` display action `flight_plan` with `{"kind":"altitude","constraint":{...}|null}` and the glass
setting `waypoint_altitude`. Approach legs are not edited (an approach has its own published path).

`flightcraft_sim::avionics::vnav` computes the path, in pure geometry on route distance and altitude:

- **Backwards from the constraints.** Only constrained waypoints are vertices. From the last constraint, each
  earlier one gets the altitude a descent at the **descent angle** (default 3.0 degrees; setting `vnav_angle`,
  1 to 6) would need there to reach the next vertex, held to its own constraint: *at* pins it, *at or above*
  raises it to at least the constraint, *at or below* lowers it to at most the constraint. Between vertices the
  path is a straight line; ahead of the first it is the descent-angle line through that vertex.
  Past the last constraint the path is level at its altitude.
- **The target** is the first vertex the aircraft still has to meet; an *at or below* it already satisfies is
  skipped.
- **Deviation** is the aircraft's indicated altitude minus the path altitude at its position: positive above.
- **Vertical speed required** is the vertical speed that reaches the target altitude at the target waypoint at the
  current ground speed (negative descends); with no ground speed there is no value, never a guess.
- **Top of descent** is the route distance at which the descent-angle path meets the current altitude.

`avionics.state` carries the result in `glass_suite.vnav` (target identifier, altitude and distance, path altitude,
deviation, `dots`, `required_vs_fpm`, `top_of_descent_nm`, the angle, and whether the autopilot is tracking). It is
absent with no constraint ahead and while an approach is loaded (the approach's own path rules then).

On the PFD (guide items 43 and 44, page 22) the **VNAV indicator** is a column of dots (100 ft each, full scale
2.5) just left of the altitude tape with a magenta diamond and the VNAV label above it: the diamond sits below the
middle when the aircraft is above the path and above the middle when below, so the pilot flies toward it. It
turns green while the autopilot is flying the path. The data bar's **VSR** field reads the vertical speed
required in feet per minute (`-600FPM`, rounded to 10) and dashes otherwise.

**Autopilot.** The VNAV key (and `{"kind":"autopilot","setting":{"kind":"vertical","mode":"gp"}}`) arms the existing
GPS glidepath mode, whose tracking law is reused unchanged: the active leg's receiver guidance now carries a
vertical deviation and path angle from the VNAV path, so GP captures within 30 m of the path, then holds it
(feed-forward from groundspeed and the path angle, plus a deviation term) down to the constraint. After the
constrained waypoint is sequenced the path stays level at its altitude while GP is flying, so the autopilot levels
off instead of losing the path and disconnecting; any other vertical mode, or a new plan, ends that hold. With no
constraint the leg carries no vertical guidance and GP stays armed, as before. The guidance rides on the receiver
leg and is left out of the serialised leg when absent, so flights without constraints hash exactly as before. The
selected altitude still arrests a descent as it does in any vertical mode (set it to the constraint to stop at it).

### Radio and transponder windows

The data bar's radio box has the guide's two touch targets (section 4.1 and 4.3): the active frequency
transfers active and standby, and the standby frequency opens the COM1 window, which shows both
frequencies, the keypad, and XFER. NAV1 has the same window from the main menu, as does the transponder
(code keypad and STBY, ALT, ON, and IDENT keys; section 4 transponder code and mode selection), which in split mode
also has its box in the data bar. They are wired to the existing comms tuning (`comms.tune`), the NAV radio,
and the glass transponder setting; no new state exists. Not modelled: COM monitor, volume and squelch, the audio
panel page. **COM2 and NAV2 are deliberately absent** (they are not a gap in the model of the Sling
but in the Sling): the Sling 2 POH lists one GTR 200 COM radio (6.1 installed equipment, 7.10 item 9) and no
second radio or navigation receiver, so the touch displays' radio box and windows address radio 1 only. The
engine's second logical receiver exists for aircraft that fit one, and the Sling pack's audio-panel load
lists it only as a lumped assumption (pack `panel-review.md`); a pack with a second radio would need the
windows to take a radio index first.

### Flight-plan editing

The FPL page lists the active flight plan (identifier, bearing, altitude constraint, distance) as touch rows with eight buttons
beneath, from the guide's editing procedures (6.5 and 6.6, printed pages 191 to 193): touch a waypoint to
select it; **Activate Leg** makes it the waypoint flown to; **Insert Before** and **Insert After** open the
waypoint keypad and put the entered identifier beside the selected waypoint; **Remove** deletes it. On an
empty plan only Insert After is offered, and the entry becomes the plan. The page edits the glass's own flight
plan, the one the data bar, map, direct-to, and `avionics.set` use (the navigator's plan follows), so every
display shows the result at once. `flightcraft_control::touch_fpl` holds the list arithmetic: the active
waypoint stays the same waypoint when one is inserted or removed before it, a removed active waypoint hands
over to the next, plans are limited to 64 waypoints, and the selection moves to what was inserted. The
`equipment.set` display actions are `flight_plan` with `edit` `{"kind":"select","index":n}`,
`{"kind":"insert","after":bool,"ident":"KMMU"}`, `{"kind":"remove"}`, and `{"kind":"activate"}`, and the keypad
targets `fpl_before` and `fpl_after`. An unknown or ambiguous identifier is refused and the plan is left
alone; a loaded approach cannot be edited until it is cleared. Not modelled: Find tabs (recent, nearest,
user, facility name, city), Edit on Map and drag and drop, saved flight plans, import and export, invert
and the other list-page menu items, and approach and departure editing.

### Nearest pages

NEAREST opens the nearest group, whose tab strip chooses a page, or the large MFD knob steps through them
(guide 1.6 and 5): airports (existing), **VORs, NDBs, intersections, and airspace**. Lists hold the 15
nearest facilities within 200 NM, nearest first, with identifier, bearing, distance and frequency, and
read "None Within 200 NM" when empty. Airspace lists what the aircraft is inside and what it would enter
within 10 minutes at its present speed, track, and altitude, with "Inside" or the time to entry, and reads
"No Immediate Airspace" otherwise. The lists are built from the loaded navigation data only while a display
shows that page. Weather, user waypoints, cities, centres, flight service stations, and visual reporting
points are not modelled.

### Knobs

Each knob is two rings; turning a ring sends `DisplayAction::Knob`, pushing the inner ring sends `KnobPush`.
Roles follow the pilot's guide (1.5, "knob operation"):

| Mode | Left knob inner / outer | Right knob inner / outer |
|---|---|---|
| Full PFD | Heading bug / altitude bug | Course / baro (default "right side knob action") |
| Split, MFD pane on the right (default) | Heading bug / altitude bug | Map range / MFD page |
| Full MFD | Map range / MFD page | Map range / MFD page |

The "right side knob action" setting selects the alternate pair: course/baro, flight-director bug/baro, or
heading/altitude. "Press to toggle knob action", when on, makes an inner push swap main and alternate
functions for 10 seconds on a PFD knob. With it off, pushing the heading knob syncs the bug to the current
heading (rounded to a degree), and pushing the course knob syncs the course to the active leg's desired
track (else the heading), rounded to a degree. The flight director bug knob steps the reference of the
active vertical mode by the guide's table (vertical modes, section 9.3): 0.5 degree in pitch hold, 100 fpm
in vertical speed, 1 kt in indicated airspeed; pushing it sets the pitch reference to the present pitch.
Altitude hold's 10 ft step needs a hold-altitude setting the engine's autopilot does not have, so that mode
does not step. Detents: 1 degree for heading and course, 100 ft for the altitude bug, 0.01 inHg (or 1 hPa)
for baro. The **range ladder has the guide's 23 steps**: 500, 800, 1,200, 2,000, 3,000, 5,000 ft, then 1.2,
2, 3, 5, 8, 12, 20, 30, 50, 80, 120, 200, 300, 500, 800, 1,000, and 1,200 NM; the guide states the count
(section 5, map range) and prints 2 to 120 NM in its map setup lists, and the other steps extend the same
progression. The range knob and the map's zoom buttons share it, and the range shows at the map's lower
right. Clockwise zooms in. A host supplies `time_s` with each knob event, so the 10-second revert needs no
clock in the engine and replays exactly.

### Keys and menus

| Key | Press | Hold |
|---|---|---|
| NEAREST | Open the nearest page (split, if on a full PFD) | |
| DIRECT | Open direct-to entry; ENTER activates a direct-to to that identifier | |
| MENU | Page menu (PFD options on a full PFD), then the main menu, then clear | |
| BACK | Close the open menu or keypad, else return to the previous page | Toggle full and split |

The page menu lists traffic and track-up on map pages and reads "No Options" elsewhere; the main menu lists
the radio and transponder windows, the pages, and display backup. The state machine is
`flightcraft_control::touch_efis` (pure functions, no session access). The `equipment.set` display actions
are `key`, `knob`, `knob_push`, `mfd_page`, `toggle_split`, `close_overlay`, `direct_to`, `knobs` (alternate
pair, push-to-toggle, pane side), `skin`, `open_overlay`, `enter`, `shortcut`, `nearest_page`, `transfer`,
`transponder_mode`, and `map_zoom`.

### Reversion

As on the real unit (pilot's guide, appendix F), a failed or unpowered display makes the surviving one
present the PFD and the engine page **in split-screen**, and it returns to its chosen page when the other
display returns. The backup setting forces the same on the screen that is not showing a PFD. The
autopilot follows the guide's rule: if the primary flight display is lost the autopilot disconnects, once
(it can be engaged again, with basic modes available, but the flight director is gone, so the command bars
are not drawn); if the multi-function display is lost the autopilot carries on. The primary display is the
one whose default page is a PFD for the pilot's seat, and the rule runs every simulation step in
`Session::apply_display_loss_rules`. The guide's REV and VFR annunciations for a lost external GPS
navigator do not apply: the engine has only the internal navigator.

### Skin packs

A skin pack is artwork for a display model, shipped apart from the engine and from aircraft packs. It is a
directory with a `skin.json` manifest (`kind: skin_pack`, `format_version`, `id`, `version`,
`display_name`, SPDX `licence`, `display_model`), an optional palette (bezel, trim, key face and text, knob,
knob mark, screen surround; key text needs 4.5:1 contrast on the key face), an optional `instruments`
section (below), up to eight short key labels, and up to 16 declared font assets with their own licences.
It carries no behaviour and no geometry.

The `instruments` section styles the PFD's faces. `palette` may set `sky`, `ground`, `tape_fill`,
`tape_text`, `box_fill`, `box_border`, `box_text`, and the six meaning colours `white`, `cyan`, `magenta`,
`green`, `yellow`, `red`. The meaning colours must stay in their hue family (cyan 170 to 205 degrees,
magenta 280 to 335, green 90 to 160, yellow 40 to 66, red 345 to 14, white near white) and keep 3:1 against
the box fill; tape and box text need 4.5:1 against their fill. `typography` sets multipliers (0.7 to 1.4)
for the tape numbers, box text, data bar text, and mode annunciations. Tape fills draw translucent over the
attitude display. Any field left out keeps the built-in value, which already follows the guide's colour
meanings; a skin that breaks a rule is reported and skipped whole, and the display shows the built-in skin.

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

The public skin is in the shared skins repository: bezel and key colours, labels, an open-licensed font, and
an instrument palette and typography that make the colour meanings explicit. The map and the placement of
everything are the engine's for every skin.

### Matched, and not yet matched

The model is not a screen-for-screen copy. **Matched** (from the guide's figures and text): the full-screen
and split-screen PFD arrangement and colour meanings, the AFCS status box and flight director bars, the data
bar and status bar, the CAS window, the trim and flap position indicators, touch flight-plan editing
(select, insert before and after, remove, activate leg), the CAS message list page (opened from the
window's triangle or Select Page; one message acknowledged at a time), transponder IDENT, the touch entry
keypads (baro, heading, altitude, course, minimums, direct-to) with their shortcuts, baro minimums (bug,
annunciation, and callout), the data bar's clock and flight timer, the Vspeed bugs and their configuration page, vertical navigation (altitude
constraints on flight-plan waypoints, the descent path computed back from them, the deviation indicator,
vertical speed required, and path tracking by the autopilot), the COM1, NAV1, and transponder windows, the
23-step range ladder with zoom buttons, the
flight director bug knob and course-sync push, the Select Page menu, the VOR, NDB, intersection, and
airspace nearest pages, and reversion with the autopilot rule.

**Remaining gaps**, honestly:

- The placement numbers are measured from figures to a few percent; outer bezel dimensions and control
  positions are estimates; pixel type sizes, line weights, and the pitch scale are the engine's.
- The map range ladder's steps other than 2 to 120 NM are extrapolated, not printed in the guide.
- The minimum safe altitude (data bar), the second bearing pointer, and the G meter are not modelled.
- **VNAV** is a descent-path model (below): no climb constraints or step-climb paths, no
  speed constraints, no wind-corrected top of descent, no VNAV direct-to, and no RNAV approach vertical
  guidance beyond the loaded approach's own glidepath. The autopilot's vertical-path mode is the GPS
  glidepath mode, so the AFCS box reads GP, not the unit's VPTH, when it flies the path.
- The CAS window has no external lamps, and shows only the aircraft's own annunciations and the
  autopilot's failures (autopilot failures are listed but have no acknowledgement of their own). The trim
  and flap indicators have no PFD-options toggle (always shown when the
  host supplies the pilot's controls) and no aileron or rudder trim.
- Touching the AFCS box (autopilot controls on the display), the selected-altitude and selected-heading
  bugs' "Set to Current" on the tapes themselves, PFD options beyond synthetic vision, and the pitch offset
  are not drawn.
- Flight director altitude-hold reference stepping (needs a hold-altitude autopilot setting).
- COM monitor, volume, squelch, 8.33 kHz entry, the audio panel and intercom windows.
  COM2 and NAV2 are not offered because the Sling's installation has one COM radio and no navigation
  receiver (see the radio windows above).
- Nearest pages for weather, user waypoints, cities, centres, flight service stations, and visual
  reporting points; the FPL page's Find tabs, map editing, saved plans, and import; weather, charts,
  checklists, and video pages.
- Synthetic vision shows the existing engine view; the guide's pathway boxes are not drawn.
- The round-gauge PFD presentation, and the guide's inset window set beyond what a pack lists.

### CAS message list and IDENT

**CAS message list.** The CAS window's down-pointing triangle (shown when more messages are active than the
window holds) opens the CAS page on the MFD pane; Select Page offers it too, as `CAS messages` after the main
pages, and `equipment.set` display action `mfd_page` with `page: "cas"` selects it. The page lists every active
message: warnings (red), then cautions (yellow), then advisories (white), newest first within each. An
unacknowledged message is boxed and flashes, with "touch to acknowledge"; touching its row sends the glass
setting `cas_acknowledge` with `index`, the row's position in that order. An acknowledged message stays listed
until its condition clears, and flashes again if the condition returns. The master caution or warning lamp goes
out when the last message of its severity is acknowledged. Touching the CAS window on the PFD, or BACK,
still acknowledges everything at once. The state is per-annunciation (`acknowledged` and `sequence` in
`systems.modern.annunciations`), and `glass_state().cas` carries the same list for any front end. An index past
the end of the list is an error; acknowledging an autopilot failure message, or one already acknowledged, does
nothing. The legacy application MFD has the same list as `page: "cas"` in the `page` setting (and a CAS button).

**Transponder IDENT.** The transponder window has an IDENT key beside STBY, ALT, and ON. It sends the glass
setting `ident` (no fields; also the unassigned keyboard action "Transponder IDENT"). The ident flag stays up
for 18 seconds of simulated time (`settings.ident_remaining_s` counts down), the status bar shows the code
followed by IDENT, and the PFD's transponder box reads IDENT in place of the mode. Like a real transponder it
works only in ON or ALT: in OFF or STBY the setting is rejected, and switching to STBY or OFF cancels a running
ident. Pressing it again restarts the 18 seconds. Approach and departure controllers ask for it with
"squawk ... and ident" and a bare "Ident."; the press is graded as the `ident` compliance item and answered with
radar contact (see [ATC](atc.md)). The press time is recorded when `ident` runs, so a press made through
`avionics.set` counts the same as a touch.

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

`ident` (no fields) presses the transponder IDENT button and `cas_acknowledge` (`index`) acknowledges one
CAS message; both are described under "CAS message list and IDENT" below. Other settings are `heading_bug` (`heading_rad`), `course` (`course_rad`), `selected_altitude`
(`altitude_m`), `baro` (`pressure_pa`), `pressure_units` (`hpa`), `bearing_pointer`
(`source`: `off` or `gps`), `map_range` (`range_nm`: 1–200), `orientation` (`north_up` or
`track_up`), `traffic` (`shown`), `traffic_filter` (`filter`), `page` (`map`, `nearest`, `flight_plan`, or `cas`; the application MFD), `display_page` (`display`, `page`), `display_data` (`display`, `area`), `pilot_seat` (`seat`), `display_reversion` (`enabled`), `active_leg` (`index`, zero-based),
`minimums` (`altitude_m`) and `minimums_off`, `clock_format`, `clock_set`, `timer`, `data_bar_field`,
`vnav_angle` (`degrees`: 1 to 6), `waypoint_altitude` (`index`, `constraint`: `{"kind":"at"|"at_or_above"|
"at_or_below","altitude_m":n}` or `null` to clear),
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
