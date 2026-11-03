# Electronic engine and touch-panel equipment

The optional equipment definitions describe original, fictional installations. Real aircraft,
engine, and avionics definitions belong in separate packs. The runnable example is
[`examples/packs/equipment-demo`](../examples/packs/equipment-demo/pack.json). Load that pack,
then select `equipment-demonstrator`. Existing aircraft retain their engines and panels.

## Engine and fuel

The engine layer's `control.kind` defaults to `magnetos`. `electronic_dual_lane` requires
injection and four independent electrical loads: `lane_a_load`, `lane_b_load`, `pump_1_load`,
and `pump_2_load`. Their electrical switch is `always`; equipment switches independently control
their demand. Each lane requires a declared backup battery. Pumps can also have backup capacity.
The existing electrical model supplies their bus power, battery depletion, and breaker trips.

Both healthy lanes give full power. Either lane alone gives `single_lane_power_fraction`;
losing both, selecting key OFF, losing selected fuel, or losing both pumps stops combustion.
A stopped engine requires key START and sufficient starter voltage for the declared start time.
ON releases the starter. Electronic controls reject magneto, mixture, primer, carb-heat, and
legacy starter/pump commands. Injection produces no carb ice. Lane lights and the engine-control
warning indicate a lane that cannot run, including a deliberately switched-off lane.

`gearbox_ratio` is engine revolutions per propeller revolution, defaulting to one. Propulsion's
RPM limits and power balance describe the propeller. `readout.engine_rpm` and the tachometer
show engine speed. `systems.modern.propeller_rpm` reports propeller speed separately.
The fuel-pressure parameter supplies pressure while at least one powered, healthy pump runs
and selected usable fuel exists. Selecting OFF removes pressure and starves combustion.
`selector_both` declares whether BOTH is available; otherwise reset selects LEFT.

Coolant temperature is a table against shaft-power demand, with `coolant_lag_s`. Manifold
absolute pressure is ambient pressure times the declared throttle-dependent fraction. CHT,
oil, EGT, and fuel-flow models remain available. All supplied values are fictional estimates,
and no table is measured engine data.

## Annunciations and switches

The systems layer's optional `modern` object declares up to 32 annunciations. Each has a
`source`, `severity` (`caution` or `warning`), printable `label`, and `threshold`. Sources are
`low_voltage`, `low_fuel_pressure`, `lane_fault`, `high_coolant_temperature`, `door_open`,
`parachute_armed`, and `engine_shutdown`. Threshold units are volts, pascals, and degrees Celsius;
boolean sources use zero. A newly active source latches its master light. Pushing the light
acknowledges its severity. A continuing source stays in the active list without relatching;
a clearing source must activate again to relatch. Other newly active sources relatch immediately.

Breakers use the existing squared-overcurrent trip model. The panel shows a pulled button
when open and a warning ring after a trip. Clicking pulls or pushes the breaker. Rockers have
load labels, a backlit ON marker, and lane fault lights. Every interaction emits a command.

## Recovery parachute

The optional `parachute` defines `hold_s`, `deployment_delay_s`, `inflation_s`,
`descent_rate_mps`, and negative `hanging_pitch_rad`. Removing the pin arms the handle.
The panel requires a continuous pointer hold, displaying progress; release cancels progress.
`parachute_deploy` is the explicit scripted pilot action and bypasses only the hold.
The safety pin blocks both paths. No failure, attitude, warning, or ground proximity deploys it.

Deployment progresses from `stowed` through `deploying`, `inflating`, and `descending`.
The fixed step blends velocity and attitude during inflation. Full inflation constrains motion
to vertical descent at the declared rate, with zero horizontal speed and the declared nose-down
attitude. This is a deliberately simplified recovery model without canopy aerodynamics or drift.
Ground contact freezes the flight as an impact, recording the vertical impact speed in
`impact_vertical_speed_mps`; it does not declare a safe landing. Deployment raises the configured
engine-shutdown advice while combustion continues. The pilot retains engine control.

Hold, delay, inflation, latching, and thermal state advance with simulation steps. A paused
panel and repeated observations consume no simulation time. Continue the simulation to advance
a held handle. The UI releases only starter and handle commands it owns.

## Navigator and displays

The panel-mount navigator has COM and NAV active/standby channels, independent flip-flops,
an outer MHz and inner channel knob, and a touch home screen. COM defaults to 25 kHz channels from
118.000 through 136.975 MHz; the radio panel can select 8.33 kHz channel labels. NAV uses 50 kHz
channels from 108.000 through 117.950 MHz.
Channel-grid violations and out-of-range frequencies are errors. The outer knob steps 1 MHz;
the inner knob steps a channel. Both wrap within their band.

Home tiles open Flight plan, Direct-to, Procedures, Nearest, Map, Waypoint info, Frequencies,
and optional Transponder and Audio. The navigator operates the session's existing glass settings and
GPS receiver: plan edits, direct-to, CDI, automatic sequencing, and autopilot GPS guidance share
one source. Its power load can keep that GPS source available independently of a display.
NAV active and standby frequencies read and operate the shared NAV1 radio; tuning through
`avionics.set` is immediately reflected on the navigator. The CDI source selector chooses GPS
or NAV1 for both the displays and the installed autopilot. The Procedures tile embeds the
existing procedure/NAV controls: load CIFP procedures and transitions, activate an approach,
select vectors, command a missed approach, sequence legs, and exit holds. Receiver power follows
the navigator breaker when it is installed. COM audio and reception use the [radio service](radio.md).

A navigator element can set `"audio_panel": true` to host the aircraft panel's single
`panel.audio_panel` installation. The association belongs to that navigator installation id;
its default is `false`, so existing packs keep their pages and separate audio faces. The
installation's `power_load`, `com1_load`, and `com2_load` must all reference existing electrical
loads. They may share the navigator load, as a lumped installation, or retain independent loads.
For example, the navigator element can be:

```json
{"kind":"navigator","load":"navigator","transponder":false,"audio_panel":true}
```

The **Audio** home tile opens `NavigatorPage::Audio` (`"audio"` in commands). Calling
`equipment.set` with `{"setting":{"kind":"navigator","action":{"kind":"page","page":"audio"}}}`
requires an opted-in, powered navigator. The page reads the shared `CommsSnapshot`; MIC1/MIC2,
RX1/RX2, NAV ID, ICS, volume, squelch, VOICE, and CAP issue `CommsAudioPanel` through
`EquipmentSetting::AudioPanel`. Volume and squelch show their current percentages. Audio power
remains controlled by the audio installation, independently of the screen's power. All page
captions share one fitted font size, including in the narrow 235 × 98 navigator. Hosting reuses
the navigator rectangle and suppresses every separate `PanelElement::AudioPanel` face, including
private artwork on those faces. Home retains the other enabled pages and fits an extra tile row
when both Audio and Transponder are installed.

Direct-to and flight-plan query actions resolve airports, navaids, fixes, pins, and coordinates
through the shared offline search. Plans update both the editable planning route and the shared
GPS receiver. Resolved waypoint actions remain available. Every displayed heading, course,
waypoint bearing, and wind direction uses the magnetic convention in [MCP](mcp.md).

The navigator map and every display map or inset use the same planning map widget, projection,
map state, overlays, range, and gestures. They send `map.set`, and use the shared `map.state`.

Any number of touch displays, within panel limits, share the suite's sensor and GPS snapshot.
Each installation independently chooses `full_pfd`, `pfd_map`, or `map_engine`, plus an engine
strip. Bottom soft keys rearrange the screen; corner knobs adjust heading and map range.
Mouse clicks and touch taps use egui's same pointer path. Display failure and electrical loss
trigger full-PFD reversion, with engine indications, on surviving displays. Restoring the
failed display restores each survivor's selected layout. The independent standby retains its
own computer and backup power.

## Declared panel and command boundary

All `panel.equipment.size`, element `position` and `size`, shell points, cubic control points,
screw positions, and placard font sizes are millimetres. The host scales the panel uniformly,
preserving its aspect ratio. Equipment rectangles and artwork control points must fit the declared
panel size. Inputs are bounded, and text must be printable. Background colours use theme tokens.

An optional `shell` declares `outline` (`polygon` with `points`, or `cubic` with closed, joined
`curves`, each four control points), `fill`, optional `glare_shield`, and up to 128 `screws`.
Concave outlines are triangulated for drawing. Existing rectangular backgrounds remain supported.

In addition to installed avionics, breakers, rockers, master lamps, the key, recovery handle,
and guarded rotary fuel selector, generic element kinds are `placard`, `toggle`, `dimmer`,
`push_pull`, `vent`, `elt`, `pitch_wheel`, `lamp`, `bracket`, and `guard_bar`. Placards declare
`text`, `font_size_mm`, and theme `color`. Toggles declare `label`, `guarded`, and `lamp`.
Rockers support `backlit` labels and an optional `sublabel`; bracket elements label groups.
Individual beacon, navigation, strobe, taxi, and landing switches have independent electrical
loads. Breakers read their amp ratings from the electrical layer and show a white collar and
warning ring when popped. Master lamps use amber and red lenses, with a bright latched state.

Generic physical controls use `panel_control` (`id`, finite `value` from zero to one), and guards
use `panel_guard` (`id`, `open`), nested systems equipment actions. Only declared installation ids
are accepted. A closed toggle guard blocks its control. These bounded values are exposed in
`systems.modern.panel_controls` and `panel_guards` for hosts to connect additional subsystems;
vents, placards, brackets, and guard bars are decorative. The ELT is a simulated remote control,
with no external transmission. Existing lane, pump, electrical, fuel, and engine controls retain
their typed commands.

A touch display's optional `composition` declares `top_data_bar`, `engine_strip_fraction`
(0.1–0.3), `engine_style` (`bars` or `dials_and_bars`), `hsi`, and up to four distinct `insets`.
Each inset has `corner` (`top_left`, `top_right`, `bottom_left`, or `bottom_right`) and `content`
(`map`, `terrain`, `synthetic_vision`, `engine`, or `navigation`). The top bar reports shared COM, baro, transponder, waypoint,
and distance data. Round RPM/manifold dials and bar gauges share the engine model. Flight content
fills its complete viewport; the soft-key row and corner knobs remain outside it. The installed
autopilot uses the complete existing controller faceplate and encoder behaviour, scaled together
with its pointer hit regions, with a seven-point minimum label size. Soft keys fit between the
corner knobs. Below a 140-point short side, map insets omit airspace outlines,
compass roses, and off-route fixes, space their small airport symbols, and retain the active route
without changing the shared map settings. Recovery pin and pull labels fit their separate button rectangles.

`equipment.set` has a tagged `setting`:

```json
{"setting":{"kind":"systems","setting":{"kind":"lane","lane":"a","on":false}}}
{"setting":{"kind":"systems","setting":{"kind":"key","position":"start"}}}
{"setting":{"kind":"navigator","action":{"kind":"flip","band":"com"}}}
{"setting":{"kind":"display","id":"left_display","action":{"kind":"layout","layout":"pfd_map"}}}
```

Systems actions additionally include `pump` (`one` or `two`, `on`), `failure` (`component`:
`lane_a`, `lane_b`, `pump1`, or `pump2`, plus `failed`), `acknowledge` (`severity`), `door`
(`open`), `parachute_pin` (`inserted`), `parachute_handle` (`held`), and `parachute_deploy`.
The same actions are accepted by `systems.set` under `kind: equipment`. Breaker and fuel-selector
commands retain their existing names. Navigator actions include `page`, `tune`, `flip`,
`knob_band`, `knob`, `direct_to` (`query`), `flight_plan` (`queries`), `avionics` (an existing
`avionics.set` setting, including procedures and CDI source), and `shared` (a glass setting). Display actions include `layout`,
`engine_strip`, `synthetic_vision` (`enabled`), and scenario `failure`. Each display has its own
SVT selection; see [synthetic vision](glass.md#synthetic-vision).

`avionics.state.glass_suite.touch` reports navigator and per-installation display state;
`sim.state.systems.modern` reports engine controls, active annunciations, master latches,
and recovery state. `equipment.set` returns touch state. All actions have generated input and
output schemas. Rust hosts use `Session::set_equipment`, `Session::touch_state`,
`ModernSetting`, and `InstrumentPanel::take_equipment_commands`; application mutations use
registered commands. Panel painting and skins never step equipment or change physics.

## Verification and previews

Tests exercise individual and dual lane loss, pump failures, backup power, key start, gearing,
breaker pull and trip, selector starvation, master-latch acknowledgement, hold cancellation,
deployment delay and inflation, terminal descent and impact, batching, radio flip-flops and
channel wrapping, shared plans, independent display layouts, reversion, and panel bounds.
The default Corvid golden trajectory remains `0xf228fce1a4055061`.

The ignored UI test `render_equipment_previews` rasterizes actual egui meshes and its font atlas.
It writes `out/equipment/panel.png`, `navigator.png`, and `display.png`, plus `-compact`
variants. Setting `FLIGHTCRAFT_EXTENDED_PANEL_PREVIEWS` additionally captures the procedure
and map pages at both sizes. Run it through the build
queue with `cargo test -p flightcraft-ui render_equipment_previews -- --ignored`.

## Optional fidelity artwork

[`panel-fidelity-demo`](../examples/packs/panel-fidelity-demo/pack.json) is a second,
original equipment pack illustrating these options. The original `equipment-demo` JSON is
unchanged. Omitted options retain the original theme, faceplates, layouts, and controls.

Shell outlines, cubic curves, glare-shield outlines, and screws retain their existing mm
coordinate format. `shell.centre_console` adds another outline in the same panel coordinate
system. `shell.palette` accepts optional `face`, `glare_shield`, `bezel`, and `placard_text`
colours as opaque sRGB `#RRGGBB` strings. The face palette also enables the shell silhouette
against the surrounding background. Omitted colours use theme tokens. Label contrast must be
at least 4.5:1 against the panel and bezel; malformed colours and unreadable combinations are
load errors. The console shares the face colour.

Every installation can declare an optional `appearance` object:

| Field | Meaning and default |
|---|---|
| `style` | Optional original physical artwork; omission retains existing artwork |
| `binding` | `lane_a` or `lane_b`, on a toggle only; omission retains a generic panel value |
| `bezel` | Optional touch-display hardware configuration, described below |
| `controller` | Optional autopilot sections and controls, described below |
| `navigator_layout` | `classic` (default) or `home_columns` |
| `skin` | Optional host equipment-artwork registry id |
| `synthetic_vision` | Initial SVT selection, default false; `integrated_pfd` initially enables it |

Styles are `black_rocker`, `push_pull_breaker`, `rounded_pull_handle`, `silver_key`,
`louvred_eyeball`, `elt_plate`, `red_t_handle`, `arc_dimmer`, and `cabin_heat`. Styles are
validated against their element kind. Black rockers put the label above the face and retain
the optional sublabel below it. Breakers show the electrical definition's amp number. Key,
fuel-selector, and dimmer styles use the existing silver bezel, guarded red T-handle, and
arc-scale drawings. The rounded recovery handle retains separate pin and continuous-hold
regions. Vents, ELT plates, and cabin heat use original generic artwork. Captions are measured
with the installed font and shrink into their own reserved rectangles.

A toggle's typed lane binding reads switch position and the lane's actual fault lamp from
the equipment snapshot. Its `panel_control` action operates that engine lane. The existing
`panel_guard` command must open a guarded toggle before `panel_control` can change it.
Direct typed lane commands remain available to scripts and other cockpit controls.

A touch-display `appearance.bezel` has `color` and `edge_trim` (optional sRGB colours),
`margins` (`[left, top, right, bottom]` in installation-local mm), `softkeys` (boolean),
`softkey_count` (1–12), `corner_knobs` (zero to two installation-local mm centres), and
`card_slot` (boolean). The resulting screen must have positive width and height. Soft keys
operate PFD, map, engine strip, and SVT selections; knobs retain heading sync and map range.
Omitting the entire bezel retains the original hardware and hit regions.

`touch_display.layout` additionally accepts `integrated_pfd`: an 8% top data bar, a 20%
engine strip with manifold/RPM dials and bars, the remaining PFD and HSI, a bottom-left map,
and a pack-selectable bottom-right terrain or synthetic-vision inset. The bottom-right `composition.insets` entry selects `terrain`, `synthetic_vision`, or
`map`; omission defaults to terrain. Terrain uses the MAP page renderer with a local base
override, without changing the shared map settings. This page selects its other composition defaults; other
pages retain the existing optional `composition`. ETE and ETA use simulated flight time; the unmodelled second COM is shown as unavailable.
The page uses the existing synthetic
vision renderer and sensed flight data. SVT is still independently selectable per display.

`appearance.controller` contains up to eight `sections` (`label`, `rect`) and 1–32
`controls` (`action`, `rect`, optional sRGB `lit_color`). Each rectangle is `[x, y, width,
height]` in installation-local mm, and must fit the installation. Control hit rectangles must not overlap, and actions must be unique:
`ap`, `fd`, `yd`, `lvl`, `hdg`, `nav`, `apr`, `vs`, `alt`, `ias`, `vnav`, `heading_knob`,
`altitude_knob`, and `pitch_wheel`. The face fills the complete installation rectangle.
Knob pushes sync heading or selected altitude to sensed values; scrolling and dragging adjust
heading, selected altitude (100/1,000 feet), or the current pitch/VS/IAS target. Buttons issue
existing autopilot commands. `vnav` arms the existing GPS glidepath (`gp`) vertical mode;
it does not introduce an en-route vertical flight-plan model. Installed autopilot capability
validation still applies.

`home_columns` retains the navigator's shared pages and commands, with HOME and CDI source
controls on the left, a 4 × 2 home tile grid in the centre, and COM active/standby, swap,
transponder status, band selection, and concentric encoders on the right. Font sizes follow
the available height and measured caption bounds, including a 235 × 98 installation ratio.

## Switched backup buses and alternating lights

The systems definition's `modern.backup_buses` defaults to an empty list. Up to two entries
can declare `circuit` (`ems` or `efis`), `capacity_ah` (0.01–100), `voltage` (9–32), and
`loads` (1–64 existing electrical load ids). Circuits and load membership must be unique.
The panel rocker switches `ems_backup` and `efis_backup`, or the equipment action
`{"kind":"backup_power","circuit":"efis","on":true}`, connect the corresponding bus.
Switches start OFF. A connected bus powers its named, switched-on loads when their normal
bus is unavailable, respects their breakers, and consumes its own charge at their actual
load current. An existing internal load battery remains a separate source. Bus switch,
remaining charge, and voltage appear in `systems.modern.backup_buses`. Charging these separate
backup batteries is not modelled; reset restores declared capacity.

Optional `modern.wigwag` declares `left_load`, `right_load` (distinct existing landing-light
loads), and `half_period_s` (0.1–5). The `landing_wigwag` rocker cycles OFF, steady, alternating,
and OFF. `{"kind":"wigwag","on":true}` enables alternating operation and the landing
switch; false restores steady operation if the landing switch is still on. The phase uses
fixed simulation steps and stops advancing when paused. The individual light switches and
breakers continue to apply. No wigwag definition means the action is rejected.

## Equipment artwork registry hook

Hosts can implement `instruments::EquipmentSkin` and register it through
`Registry::register_equipment_skin(id, painter)`. Its `paint` method receives an egui painter,
the exact installation rectangle, and an `EquipmentSnapshot` containing the installation and
read-only `InstrumentInput` (sensed systems, navigation, displays, and installed autopilot).
It cannot send commands, mutate the session, or advance time. The host continues to own all
hit regions, guarded actions, encoder behaviour, and starter/recovery hold releases.

Resolve a pack's `appearance.skin` through that registry, or call
`Registry::set_equipment_skin(installation_id, skin_id)` to bind artwork to a particular
installation id at host startup. A host override takes precedence over the JSON selection.
Omission uses original artwork; an explicitly selected unknown id is a validation error.
Register skins before selecting a panel. Conventional `register_skin` and
`register_pack_skin` remain the conventional-instrument API.

A private derivative can preserve the public pack's installation ids and rectangles, set
its own registered artwork ids, and implement its artwork in a private host extension.
The callback replaces visual content, including display/map artwork, while the original
command regions remain active. Private painters must present the shared snapshot's state
and current page; they must not implement receivers, guidance, flight dynamics, or power
behaviour. Artwork remains private, and no reference-image pixels belong in a public pack.

## Local panel comparison

```
cargo xtask panel-compare --pack examples/packs/panel-fidelity-demo \
  --reference /path/to/local-reference.jpg --out out/comparison
```

The tool selects the pack's first aircraft definition, renders actual egui meshes and the
font atlas at the reference image's dimensions, and writes `render.png`, `side-by-side.png`,
`overlay.png` (50%), `edge-difference.png`, and `alignment.json`. It limits image dimensions
and decoded allocation. Repository output is restricted to the git-ignored `out/` tree;
other repository paths, including paths through symlinks, are refused before writing. Output file symlinks and overwriting the reference are also refused.
Reference-derived comparison images remain local under ignored `out/` or an external directory.

Scores compare Sobel edges symmetrically within four pixels in each declared installation
rectangle. A score of one means all edges have nearby counterparts; zero includes empty
edge sets. Counts and pixel rectangles accompany each score. The edge image uses red for reference
edges, green for rendered edges, and blue for disagreement. This is an appearance-sensitive
alignment aid, not a certification of measured placement or a manufacturer-artwork match.
The CPU capture renders egui mesh content and shared vector maps; GPU synthetic-vision
callbacks and online map tiles require an application capture to compare those layers.

The public `panel_preview::render_equipment_panel` helper captures a session without stepping
it. The ignored UI test `render_panel_fidelity_previews` writes an original example panel and
navigator, controller, and display crops under `out/equipment-fidelity/`.

Navigator home tiles use one measured font size for the complete grid, with at most two lines
per label. The frequency column independently shares one font size across its captions and
values. Bound lane toggles reserve separate lamp, caption, and switch/guard areas, in that order.
When no geographic region is installed, the same MAP renderer shows a local-coordinate moving
map with its grid, range rings, runway, aircraft, and track; it does not invent GPS coordinates.
The fidelity preview uses original analytic terrain, independent of downloaded data.

## Window fit and essential instruments

Panel views fit the main dash with a uniform scale, using the full available width or height.
For shells with a separately authored centre console, the fitted bounds cover the dash outline
and glare shield. Controls below that dash open through **Console → Lower console**. Pack
previews fit the complete authored panel. Both fits keep their visible controls inside the host.
Dedicated hardware and multiple-screen layouts retain their independently specified physical fit.

Equipment panels may set `essential: [left, top, width, height]` in panel millimetres. The
rectangle must be finite, positive, and wholly inside `size`. It remains accepted as panel
metadata; visible geometry now determines the fit. Artwork, controls, maps, and preview crops
share the same transform, so hit targets follow the visible equipment.

The ignored `render_layout_previews` UI test captures the equipment and panel-fidelity demos,
Corvid PFD and MFD, and an optional external pack at 1920×1080 under `out/layout/after/`.

`panel_preview::equipment_transform` returns the pixel-space left, top, and uniform scale
used by complete equipment captures, so comparison tools can locate crops after fitting.
The ignored `render_layout_cockpit_previews` test adds GPU captures of the outside scene
behind the demo panels and optional external pack under `out/layout/after/`.
## Audio panel and COM reception

The equipment demo now declares separate audio-panel, COM1, and COM2 electrical loads and an
`audio_panel` equipment face. MIC, receive, NAV ident, intercom, volume, and squelch controls use
`comms.audio_panel` through the same command registry. The toolbar radio window adds typed PTT,
tuning, spacing, captions, and the kneeboard. Navigator COM1 active/standby tuning is shared with
`comms.tune`. See [radios](radio.md) for FAA frequency import, reception, traffic, audio, and limits.

Aircraft identity objects may declare `registration` and `spoken_type` for radio identification.
The shared radio service expands them into spoken GA callsigns; pilots can override them with
`comms.callsign`. See [radio identity and flight history](radio.md#radio-identity).

The ignored `render_sling_navigator_audio_preview` UI test loads the pack selected by
`FLIGHTCRAFT_SLING_PACK` read-only and enables the navigator's `audio_panel` field in memory.
It writes `out/navaudio/sling-audio-panel.png`, `sling-audio-navigator.png`, corresponding
`-home-` views, and `preview.txt`. These CPU egui captures omit GPU synthetic vision and do not
verify speech devices or station reception. The navigator audio tests click every control,
check the shared comms state, and check caption bounds, overlap, and uniform grid fonts at both
235 × 98 and compact scale.

For an external pack preview, pass its location through the build queue's Cargo configuration:

```text
buildq run flightcraft <worktree> -- cargo test -p flightcraft-ui render_sling_navigator_audio_preview --config 'env.FLIGHTCRAFT_SLING_PACK="<pack-root>"' -- --ignored
```
