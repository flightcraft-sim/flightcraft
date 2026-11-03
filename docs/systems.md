# Aircraft systems

Operational systems are declared in the version 2 systems layer's optional `definition`.
Omitting it retains the legacy placeholder installation. The Corvid declares a carburetted
engine and vacuum attitude/heading gyros. The Larrow declares injection and an essential-bus
electric gyro load with a 1 Ah backup battery. Both have a 24 Ah battery, an RPM-dependent
alternator, main and essential buses, and two wing tanks.

`sim.state`, `sim.reset`, and `sim.step` return `systems` beside `state` and `readout`.
`systems.sensed` contains indicated airspeed, altitude, vertical speed, attitude, heading,
electric turn rate, lateral specific force, and an attitude-valid flag. `readout` remains
truth-based for scripts and diagnostics; the panel uses sensed values.
Switch positions, failures, breaker states, bus voltages, battery charge, individual equipment
power and backup charge, tank quantities and indications, suction, gyro spin and errors,
trapped pressures, fuel flow, engine status, primer charge, carb ice, EGT, CHT, oil temperature,
and oil pressure are reported.
Units are SI, except RPM, amperes, ampere-hours, volts, and temperatures in degrees Celsius.

Every action uses `systems.set` with a tagged `setting`:

```json
{"setting":{"kind":"switch","switch":"master","on":true}}
```

| Setting kind | Fields and values |
|---|---|
| `switch` | `switch`: `master`, `avionics`, `alternator`, `lights`, `fuel_pump`, `pitot_heat`, `alternate_static`, `carb_heat`, or `starter`; `on`: boolean |
| `magnetos` | `position`: `off`, `left`, `right`, or `both` |
| `fuel_selector` | `position`: `off`, `left`, `right`, or `both` |
| `mixture` | `value`: 0–1; zero is idle cutoff |
| `altimeter` | `pressure_pa`: 80000–110000; Kollsman setting |
| `heading_bug` | `heading_rad`: 0–2π; displayed heading reference |
| `primer` | `charge`: 0–1; cranking consumes primer charge |
| `breaker` | `load`: installed load id; `closed`: boolean; closing resets the trip memory |
| `load_multiplier` | `load`: installed load id; `value`: 0–10; scenario fault, 1 restores rated demand |
| `tank_quantity` | `tank`: installed tank id; `mass_kg`: 0 through tank capacity; scenario servicing |
| `environment` | `temperature_c`: sea-level temperature, -60–60; `relative_humidity`: 0–1; local temperature follows the ISA altitude difference |

`failure.set` adds `vacuum_pump`, `alternator`, `pitot_blocked`, and `static_blocked` kinds to the
existing engine and surface failures. `failure.clear` accepts the same target names, or `all`.
Port blockages are sealed physical obstructions: total pressure is trapped in a blocked pitot,
and static pressure is trapped in a blocked static line. Alternate static bypasses the external
static obstruction with the declared pressure error. Pitot heat is a switched electrical load;
it does not remove a forced physical obstruction. Static position error is interpolated against
flap position. The altimeter inverts ISA pressure with its selected setting; the VSI differentiates
pressure altitude before applying its declared lag, so changing the setting does not drive the VSI.

Reset initializes a running engine, full-rich mixture, both magnetos, both tanks selected,
master and avionics on, a charged battery, spun-up gyros, and other switches off. Fuel comes
from the selected station load; tanks sharing a station divide it by capacity. Selecting an empty
tank or off, cutting mixture, or turning magnetos off stops combustion. Restoring fuel, mixture,
and magnetos requires the starter and sufficient electrical power; a cold carburetted engine
also needs primer charge. Single-mag operation reduces shaft power; leaning produces an EGT
peak, followed by lean power loss, and first-order EGT/CHT changes. Humid, moderate-temperature
air forms carb ice according to the two declared tables; carb heat clears it with a power penalty.
Injected engines neither form carb ice nor accept carb heat.

The alternator supplies declared demand and up to 5 A of battery charging. The battery supplies
the deficit, including the starter, with charge-dependent voltage and current-dependent sag.
Without usable battery capacity, insufficient alternator output sheds the main bus first.
Breakers integrate squared overcurrent and trip persistently; resetting an unresolved fault
trips them again. Equipment with a backup battery continues until its internal charge runs out.
Vacuum suction follows engine RPM, including windmilling. Loss of suction or electrical gyro
power spins the sensor down; tracking weakens, drift accumulates, and its sensed attitude topples.
Restored drive spins it up and erects it gradually.

Systems advance once per fixed flight step, outside RK4 substages. Reading state consumes no
time or sensor updates. The flight model receives an additional shaft-power fraction only when
operation reduces available power. Full-rich, both-mag, healthy operation retains the original
shaft-power arithmetic. The Corvid golden trajectory with fuel-driven mass is
`0xf228fce1a4055061`. Fuel burn and servicing update rigid-body mass, CG, and inertia once per
fixed step through the existing station-load weight-and-balance calculation. See [instruments](instruments.md) for gauges and the oil model.
All tables and parameters are original fictional engineering estimates.

Rust callers constructing a flight from a `ResolvedAircraft` call `Sim::configure_systems` on
the new simulation; the session does this on every aircraft application and reset. Standalone
Corvid and Larrow constructors install their built-in defaults. `Sim::systems` exposes sensor
and operating state, and `Sim::set_systems` accepts the same typed panel actions as commands.

The [audio panel](radio.md) reads the declared panel and COM electrical loads, including breaker,
avionics-switch, and bus power. Existing aircraft share the existing `avionics` load by default;
the equipment demo declares independent loads. Communications stay outside systems state and its
hash, so default aircraft demand and the golden trajectory are unchanged.

Additional training faults, cockpit smoke, and their bounded physical effects are documented in
[emergencies](emergencies.md). [Scenario scripts](scenarios.md) use the same failure commands.
