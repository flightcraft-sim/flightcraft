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
excluded from that aerodynamic load factor. There is no wind. `controls()` and
`steps()` expose the held controls and completed fixed steps.

`from_state(State, Controls)` adds a validated entry point for flight-dynamics
experiments and replay. It normalises a nonzero quaternion and rounds time to the
nearest fixed step. Positions must be within 100,000 km of the origin, individual
velocity components within 500 m/s, body rates within 50 rad/s, quaternion
components within ±2, and time within 0–10¹² s. Ground strikes in restored states
are immediately marked crashed. It returns `SimError` for invalid input.

`state_hash()` is FNV-1a over little-endian bits of the step count, crash flag,
time, position, velocity, quaternion, and body rates. The golden test folds this
hash at 0.1 s intervals through a complete takeoff, rectangular pattern, landing,
and stop. The stored constant changes only with reviewed model/pilot changes.

The model uses ISA geopotential height up to 20 km, flat ground, diagonal inertia,
a normally aspirated piston engine, a fixed-pitch propeller, and instantaneous
control deflection. Torque, propeller slipstream, fuel consumption, wind, and
terrain variation are future extensions. The original aerodynamic estimates
support plausibility checks, not aircraft certification or training.
