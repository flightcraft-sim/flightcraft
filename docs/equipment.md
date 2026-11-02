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
an outer MHz and inner channel knob, and a touch home screen. COM uses 25 kHz channels from
118.000 through 136.975 MHz; NAV uses 50 kHz channels from 108.000 through 117.950 MHz.
Channel-grid violations and out-of-range frequencies are errors. The outer knob steps 1 MHz;
the inner knob steps a channel. Both wrap within their band.

Home tiles open Flight plan, Direct-to, Procedures, Nearest, Map, Waypoint info, Frequencies,
and optional Transponder. The navigator operates the session's existing glass settings and
GPS receiver: plan edits, direct-to, CDI, automatic sequencing, and autopilot GPS guidance share
one source. Its power load can keep that GPS source available independently of a display.
NAV active and standby frequencies read and operate the shared NAV1 radio; tuning through
`avionics.set` is immediately reflected on the navigator. The CDI source selector chooses GPS
or NAV1 for both the displays and the installed autopilot. The Procedures tile embeds the
existing procedure/NAV controls: load CIFP procedures and transitions, activate an approach,
select vectors, command a missed approach, sequence legs, and exit holds. Receiver power follows
the navigator breaker when it is installed. COM audio is not modelled.

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
(`map`, `engine`, or `navigation`). The top bar reports shared COM, baro, transponder, waypoint,
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
