# Instruments and panel skins

The aircraft panel layer installs instruments by `module` and `skin` id, with independent
installation ids, positions, and sizes. Both original panels use a basic-T six-pack, a full-size
tachometer beside it, annunciator lamps, and a readable horizontal engine and systems cluster.
The Corvid installs vacuum instruments and suction indication. The Larrow has electric gyros,
so its panel has neither a suction gauge nor a LOW VAC annunciator.

Panel coordinates scale to the available rectangle at either panel height. The compact
preference uses a 260-point panel while retaining the aircraft layout. Engine strips have their
own wide, shallow canvas, so their labels and numeric values remain legible rather than shrinking
as miniature round gauges. Layout tests cover 1280 × 800 and 1000 × 640 windows at both panel
heights, US and SI units, both pressure units, gyro failure, and multiple headings and altitudes.
They measure actual egui font bounding boxes, reject text overlap within and across instruments,
and verify that labels fit their cells and do not cross circular dial edges. Paint bounds are
also checked against clipping rectangles.

## Behaviour and familiar indications

`ui::instruments::Instrument` owns deterministic presentation state and receives `InstrumentInput`.
Flight instruments read `systems.sensed`, including pressure errors, electric turn rate, gyro
drift, and the attitude-valid flag. They never receive a truth readout. Engine instruments read
operating state, and fuel instruments read the lagged tank indications grouped by wing.
`InstrumentInput::with_aircraft` supplies the resolved aircraft's fixed definition metadata,
including V-speeds and the engine RPM limit; it does not supply flight truth.

Each module returns a `DrawList` of circles, lines, convex polygons, and labels using semantic
ink roles. `DrawList::size` defines the local canvas: 200 × 180 for round instruments and
annunciators, and 320 × 22 for horizontal strips. `indicated` and `valid` remain available for
behaviour tests. `Skin::paint` receives the list without simulation access. The public `original`
skin uses only theme tokens and original procedural geometry.

| Module | Behaviour and indication | Presentation lag |
|---|---|---|
| `airspeed` | Sensed IAS; bottom zero stop; white Vso–Vfe, green Vs–Vno, yellow Vno–Vne, and a red Vne line from the selected variant | 0.25 s |
| `attitude` | Sensed roll and pitch, with the sensed gyro tumble and validity flag | 0.15 s |
| `altimeter` | 0–9 dial, each number 100 ft, 20-ft minor ticks; long hundreds hand, short thousands hand, and a ten-thousands pointer; boxed Kollsman window | 0.3 s |
| `turn_coordinator` | Electric sensed turn rate, ±6°/s; L/R at the lower standard-rate marks; mechanical slip ball | 0.5 s; ball 0.25 s |
| `heading` | Shortest-path heading card: N, 3, 6, E, 12, 15, S, 21, 24, W, 30, 33; heading bug on the card | 0.3 s |
| `vertical_speed` | Sensed VSI, ±2000 ft/min, zero at nine o'clock, with UP/DN legends | 0.7 s, after sensor lag |
| `tachometer` | Full-size RPM dial, green operating arc and the engine definition's red RPM limit | 0.3 s |
| `oil_temperature`, `oil_pressure` | Horizontal oil temperature and pressure strips | 1 s; 0.3 s |
| `egt`, `cht` | Horizontal exhaust and cylinder head temperature strips | 0.5 s |
| `fuel_left`, `fuel_right` | Tank sender quantities grouped by wing; scale ends at installed wing capacity | 1 s, after sender lag |
| `fuel_flow` | Engine fuel flow | 0.5 s |
| `ammeter` | Battery current, positive charging and negative discharging | 0.4 s |
| `volts`, `suction` | Main bus voltage and installed vacuum suction | 0.2 s; 0.5 s |
| `annunciators` | Installed low voltage, vacuum, fuel, and oil pressure lamps; warning colour when lit, dim when unlit | immediate |

Needles use exact held-input first-order responses with limits and `libm`. Simulation time
governs presentation: paused redraws consume no time, time reversal resets memory, and session
generations reset instruments even when time does not change. Changing display units leaves the
filtered indication unchanged. Altimeter hand angles are tested at 0, 450, 1250, 9990, and
12340 ft, including continuous thousands and ten-thousands gearing.

A vacuum failure spins down and topples the sensor in `sim`; the attitude indicator follows
that tumble. An invalid gyro shows its warning beneath the dial, leaving card numbers clear.
The electric turn coordinator has a `turn_coordinator` load and breaker; loss of power drives
the sensed turn rate to zero and invalidates its display. Its slip ball remains mechanical.
Sealed pitot and static failures trap pressures. With both trapped, IAS freezes; a blocked pitot
alone still responds to changing static pressure.

## Units and preferences

Preferences independently select altimeter pressure units and engine/fuel units. The defaults
are inHg and US customary units. Both choices persist, while commands and simulation state keep
their existing SI units.

* The Kollsman window defaults to inHg, with a 28.10–31.00 range and 0.01 steps. hPa mode uses
  950–1050 hPa with 1-hPa steps. Mouse wheel and keyboard adjustments use the selected units,
  quantize to their step, and send `systems.set` with `pressure_pa`.
* Temperatures default to °F, oil pressure to psi, quantities to US gallons, and fuel flow to
  gal/h. RPM, amps, and volts retain those units. Suction defaults to inHg.
* SI mode uses °C, kPa, kg, and kg/h for the corresponding gauges. Fuel volume conversion uses
  an original fixed gasoline-density estimate of 0.72 kg/L and 3.785411784 L per US gallon;
  the physical fuel system and weight-and-balance calculations still use kilograms.

Horizontal strips have green, yellow, and red operating bands. They are original fictional
estimates, not manufacturer data: oil temperature is green at 50–115 °C, oil pressure at
200–500 kPa, EGT at 400–900 °C, and CHT at 80–220 °C. Quantity becomes yellow below 25% and red
below 15%; a full tank remains green. Flow is green through the installed engine's declared
full-power flow, then yellow through 115%, and red above it. Battery current is green from
−5 to +5 A, voltage at 13–15 V, and suction at 12–20 kPa. Adjacent caution bands precede the
red regions. The tachometer's green arc spans 60–96% of the declared maximum RPM, with the
red line at that maximum.

Annunciator thresholds are main bus below 13 V, installed vacuum below 10 kPa, any tank below
15% indicated capacity, and oil pressure below 150 kPa. Electrical and fuel lamps appear only
when those systems exist. Oil temperature follows an ambient-relative power-dependent target
with a 180 s time constant. Pressure follows RPM and temperature-dependent viscosity with a
2 s time constant, falling to zero after combustion stops. These remain original engineering
estimates without aircraft-specific measurements.

## Panel controls

Switches, selectors, mixture, primer, Kollsman setting, and heading bug produce typed
`SystemSetting` values, dispatched through `systems.set`. The UI never mutates simulation state.

The magneto selector shows OFF / R / L / BOTH / START. START is momentary: holding it selects
both magnetos and engages the starter, and releasing it returns to BOTH. Its shortcut is also
held, rather than toggled. Mouse release or loss of a keyboard hold releases a UI-owned starter
command. A separately scripted starter command remains under the script's control until the UI
actively takes ownership or the pilot selects another key position.

The fuel selector shows LEFT / BOTH / RIGHT / OFF. Mixture displays a percentage of full rich
and is adjustable by slider or 5% shortcut steps. Primer adds 20% charge per stroke. Master,
avionics, alternator, fuel pump, lights, pitot heat, alternate static, and installed carb heat
use the same command path. The heading knob wheel adjusts the bug by 1°. Systems shortcuts
are bindable in the persisted keymap and start unassigned, preserving existing custom keys.

## Runtime skin extension

A pack host registers skins before loading a panel. `Registry::register_skin(id, skin)` accepts
a native `Skin` implementation. `Registry::register_pack_skin(json)` accepts bounded declarative
artwork without engine changes or executable code. `FlightCraftApp::register_instrument_skin`
provides the same registration on an app instance. Registrations survive resets and aircraft
changes. No replica artwork is distributed here.

A private pack host can load a separate skin resource and register its text before selecting
that skin id in panel installations. Skin resources are not aircraft layer documents; the host
owns discovery and registration. An example using only original geometry is:

```json
{
  "id": "night-face",
  "colors": [["needle", "amber"], ["normal", "green"]],
  "face": [
    {"kind": "circle", "center": [100, 88], "radius": 66, "ink": "face"}
  ]
}
```

Roles are `face`, `text`, `muted`, `needle`, `normal`, `warning`, `alert`, `inactive`, `sky`, and
`earth`. Theme tokens are `dial`, `text`, `muted`, `accent`, `green`, `amber`, `danger`, `sky`,
`earth`, `raised`, and `border`. Optional face artwork paints beneath dynamic items and replaces
the default face disk. Coordinates use the module's local canvas. Inputs are limited to 64 KiB,
ten colour mappings, 256 primitives, 128 polygon vertices, and 128 bytes per label, with finite
coordinates bounded by 320 × 180 and bounded sizes. Artwork must fit the selected module's canvas.
Duplicate and malformed ids are errors. Omitting `skin` selects `original`.

`Registry::validate` and `InstrumentPanel::load` reject unknown module and skin ids before
replacing the active panel. Aircraft parsing validates skin-id syntax; the UI registry validates
availability. Legacy fixed-mass files retain six ideal sensed instruments.

## Fuel, mass, and integration

After each fixed flight step, systems burn fuel and aggregate remaining tanks by station.
`Variant::loaded_mass` uses the existing weight-and-balance and parallel-axis calculations to
replace mass, CG, and inertia before the next step. Payload stays unchanged. Servicing changes
quantity immediately and mass at the next fixed-step boundary. Observations advance neither.
Placeholder systems retain fixed mass.

The golden baseline remains `0xf228fce1a4055061`: fuel burn reduces Corvid mass, while its default
fuel station at the CG leaves CG and inertia constant. The Larrow's offset fuel station changes
both. This panel revision changes presentation and controls without changing flight physics.

Rust integrations use `InstrumentInput::new(...).with_aircraft(...)`, `DisplayOptions`, and
`DrawList::size`. `InstrumentPanel::options` sets preferences; `starter_key_held` supplies the
current configured starter shortcut's hold state. The `normal` and `inactive` ink roles are new.
Current physical mass is `Sim::aircraft().mass`; active resolved metadata still describes the
reset load.

The ignored `render_panel_preview` test captures the actual egui meshes and font atlas with
supersampling. It writes `flightcraft-panels.png` and `flightcraft-panels-compact.png` to the
temporary directory, showing both aircraft at both window sizes. Run it through the build queue
with `cargo test -p flightcraft-ui render_panel_preview -- --ignored`.
