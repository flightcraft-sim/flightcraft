# Simulation API

`Sim::new(&Start)` starts the original Corvid. `Start::Runway` puts its CG above
runway 36's threshold, pointing north, on three loaded springs, with an idling
engine. `Start::Air` starts at the requested height, speed, and heading, with a
level-flight force/moment trim and matching initial controls. Outside the
achievable trim envelope, the closest bounded trim is used. Valid air starts
have finite height 2–20,000 m, speed 1–150 m/s, and finite heading.

`set_controls(Controls)` holds normalised controls until replaced. Elevator,
aileron, and rudder range from −1 to +1; positive values mean nose up, right bank,
and right yaw. Throttle, flaps, and brake range from zero to one. Non-finite
control inputs become zero, and finite values are clamped.

`step(n)` accepts any `u32` and integrates at `DT_S = 1/120 s` using RK4, including
contact forces in all four evaluations. Attitude is a body-to-NED unit quaternion
`[w, x, y, z]`, normalised after every step. A hard touchdown (wheel sink rate
above 4.5 m/s), exhausted 0.30 m suspension travel, or a non-gear ground strike
sets `Readout::crashed` and freezes subsequent steps. If integration exits the
finite numerical envelope, it retains the last finite state and sets the same
flag. Reset with `new` to fly again.

`state()` returns the rigid-body state in SI units. `readout()` fills every
instrument field; IAS uses the square root of the density ratio, altitude is CG
height, AGL is the lowest wheel's height (zero in contact), and load factor is
upward aerodynamic/propulsive body-z force divided by weight. Gear loads are
excluded from that aerodynamic load factor. Airflow, IAS, TAS, angle of attack,
and sideslip use velocity relative to the moving air mass. Ground speed and wheel
contact continue to use ground velocity. `controls()` and
`steps()` expose the held controls and completed fixed steps.

`from_state(State, Controls)` adds a validated entry point for flight-dynamics
experiments and replay. It normalises a nonzero quaternion and rounds time to the
nearest fixed step. Positions must be within 100,000 km of the origin, individual
velocity components within 500 m/s, body rates within 50 rad/s, quaternion
components within ±2, and time within 0–10¹² s. Ground strikes in restored states
are immediately marked crashed. It returns `SimError` for invalid input.

`state_hash()` is FNV-1a over little-endian bits of the step count, crash flag,
time, position, velocity, quaternion, and body rates. Nondefault scenarios also include weather, the PRNG and filter
memory, and failures. The default calm, healthy scenario retains the original hash
stream. The golden test folds this
hash at 0.1 s intervals through a complete takeoff, rectangular pattern, landing,
and stop. The stored constant changes only with reviewed model/pilot changes.

The model uses ISA geopotential height up to 20 km, terrain contact, full-tensor inertia,
a normally aspirated piston engine, a fixed-pitch propeller, and instantaneous
control deflection, with optional jams. Fuel burn updates station mass, CG, and inertia
once per fixed step. Torque and propeller slipstream are future extensions. The original aerodynamic estimates
support plausibility checks, not aircraft certification or training.


`Sim::new_with_weather(&Start, Weather)` includes the weather seed in the start.
Air starts add initial mean wind to the requested air velocity; runway starts stay
fixed to the ground. `set_weather` replaces weather and restarts the gust filters
without changing ground velocity. Wind direction is degrees FROM; speed is m/s at
10 m AGL. Optional shear is a 1/7 power law capped at 1,000 m AGL. Linear Dryden
filters use a fixed 533.4 m scale length, with longitudinal/lateral axes aligned
with the horizontal air-relative flight path and a vertical axis; light, moderate,
and severe mean per-axis RMS of 1, 2, and 4 m/s. They start at zero, so allow a
warm-up transient when measuring statistics. Angular gusts are omitted. Filter
memory advances once per 120 Hz step, and readouts never consume noise.

`set_failures(Failures)` validates and atomically replaces engine power availability
and jams of elevator, aileron, rudder, and flaps. Zero power removes idle power and
adds a windmilling propeller estimate (equivalent drag area 0.10 m², advance ratio
1.5). A partial fraction scales shaft power, including idle. Jams override pilot
and hold demands until cleared. Readout includes weather, total wind and gust
velocities in NED, failures, and actual `effective_controls`; `controls` remains
the pilot or hold demand. Cloning `Sim` preserves all process memory for exact
continuation. `from_state` restores only rigid-body state, with calm weather and
healthy systems; replay weather changes from a seeded start to reproduce gusts.

The command interface accepts, for example:

```json
{"command":"weather.set","input":{"wind_from_deg":270,"wind_speed_mps":3,"shear":false,"turbulence":"light","seed":42}}
{"command":"failure.set","input":{"failure":{"kind":"engine"}}}
{"command":"failure.set","input":{"failure":{"kind":"partial_power","fraction":0.5}}}
{"command":"failure.set","input":{"failure":{"kind":"jam","surface":"elevator","position":0.1}}}
{"command":"failure.clear","input":{"target":"elevator"}}
{"command":"failure.clear","input":{}}
```

These three commands return both current weather and failures. `sim.reset` accepts
an optional `weather` object with the same fields; omission resets to calm air.
Reset clears failures and holds. To fly a power-off glide with speed control, use
`autopilot.set` with `airspeed_mode: "pitch"` and the desired `airspeed_mps`.
