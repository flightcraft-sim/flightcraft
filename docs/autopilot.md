# Installed autopilots

A panel's optional `autopilot` installation selects a behaviour family, allowed modes, electrical
loads, servo authority and slew rate, bank and pitch-rate limits, load-factor limit, trim,
altitude preselect and alerting, envelope protection, and an optional yaw damper. Definitions and
skins are original, brand-neutral designs. Values are fictional engineering estimates.

`corvid-six` remains unequipped. Optional six-pack layouts `corvid-leveler`, `corvid-legacy`, and
`corvid-digital` install the three families. Select one with `aircraft.load`, supplying
`variant: "corvid-standard"`, `panel`, and the aircraft's station `load`, then reset in the air.
No aircraft or panel option changes the default golden flight.

The `corvid-glass` and `corvid-standby` aircraft install the digital family on their glass and
standby-pair panel layers. Flight-director command bars appear on the glass PFD, its standby
display, and the electronic attitude unit. The PFD mode strip places active and armed lateral
modes on the left, AP/YD coupling in the centre, and active/armed vertical modes and the vertical
target on the right. Active modes are green, armed modes white, and cautions amber.

The glass installation names `glass_ahrs` as its sensor source; the standby installation names
`electronic_attitude`. The fixed step reads the shared AHRS and air-data computer, including its
alignment, validity, and failure flags. Engagement is unavailable during the 45-second alignment.
Invalid air data or attitude disconnects the installed equipment. Other panel integrations can
supply finite, range-checked indications with `Sim::set_autopilot_sensors(Some(sensed))`; `None`
restores the systems sensors. The standalone digital example uses the six-pack's sensed attitude
and turn-coordinator electrical load.

## Controls and shared heading bug

All buttons and knobs use `avionics.set`. An autopilot action is nested inside the command's
existing tagged-setting boundary:

```json
{"setting":{"kind":"autopilot","setting":{"kind":"engage","on":true}}}
{"setting":{"kind":"autopilot","setting":{"kind":"lateral","mode":"hdg"}}}
{"setting":{"kind":"heading_bug","degrees":90.0}}
{"setting":{"kind":"heading_bug_sync"}}
```

There is exactly one heading bug, `systems.controls.heading_bug_rad`. Both `systems.set` with
`heading_bug` and `avionics.set` with `heading_bug` operate it. HDG samples it every fixed step,
turns the shortest way, and slews the bank command when it changes. Near the reciprocal, it retains
the initial turn direction through adverse yaw during roll-in. Pushing the controller's HDG sync
knob or the heading instrument's knob sets the same bug to the current displayed heading, rounded
to a whole degree. HDG has no private setpoint separate from the shared bug.

The nested autopilot setting kinds are:

| Kind | Fields | Action |
|---|---|---|
| `engage` | `on` | Couple or disconnect servos; engagement requires valid sensors, power, and pitch/roll inside ±20°/±45° |
| `disconnect` | none | Yoke disconnect, leaving physical trim in place |
| `flight_director` | `on` | Digital command bars; FD alone never drives servos |
| `lateral` | `mode` | Select or arm an installed lateral mode |
| `vertical` | `mode` | Select or arm an installed vertical mode; ALT captures present altitude |
| `level` | none | Digital AP engagement, ROL, and zero VS together; cancel armed modes |
| `turn` | `rate_rps` | Wing-leveler turn knob, ±3°/s |
| `track` | `track_rad` | Selected ground track, 0–2π |
| `selected_altitude` | `altitude_m` | Legacy altitude selector without arming; ARM then enables capture |
| `altitude` | `altitude_m` | Selected altitude, 2–10000 m; arm ALTS |
| `vertical_speed` | `vertical_speed_mps` | Selected climb or descent, ±8 m/s |
| `airspeed` | `airspeed_mps` | IAS climb or descent target, 20–90 m/s |
| `pitch` | `pitch_rad` | PIT reference, −0.25–0.35 rad |
| `yaw_damper` | `on` | Optional digital yaw servo |
| `control_wheel_steering` | `pressed` | Hold releases clutches; release recaptures vertical references |
| `failure` | `failure`, `failed` | Inject or restore a roll servo, pitch servo, trim, roll sensor, pressure sensor, or attitude sensor fault |

The wing leveler accepts only AP, TURN, disconnect, and scenario faults. Unsupported modes and
out-of-range or nonfinite settings are errors. Optional shortcuts use the persisted keymap:
AP, HDG, ALT, LVL, heading sync, yoke disconnect, and held CWS start unassigned. All panel widgets
return command inputs; neither skins nor renderer change the simulation.

`autopilot.state` returns `{ "autopilot": ... }`, null if no equipment is installed. Equipment
state includes active and armed modes, ALTS, targets, FD commands, individual servo engagement and
deflection, trim motion, mistrim, altitude alert, speed and stability protection, pilot overpower,
failure flags, disconnect reason, disconnect tone request, and flashing AP visibility. Active
modes are green, armed modes white, cautions amber, and failures red. Flash timing follows simulation
time: the AP disconnect annunciator flashes for five seconds. `disconnect_tone` requests a
one-second warning, and `disconnect_sequence` increments once per coupled disconnect so output
still receives the warning after a batched simulation step.

The original controller faces use conventional mode buttons, tactile knobs, and a display window.
The legacy unit has AP, HDG, NAV, APR, REV, ALT, UP, DN, and ARM buttons, a selected-altitude knob,
and VS indication while adjusting. Heading comes from the shared bug. The digital unit adds FD,
TRK, VS, IAS, LVL, and optional YD, an UP/DN wheel, a heading knob with push-sync, and a dual altitude
knob: inner steps 100 ft, outer steps 1,000 ft. Altitudes use whole feet with thousands separators;
selected altitude and VS steps are 100 ft and 100 fpm. IAS uses whole knots, and heading is three
digits. The selected-altitude bug is also drawn on the PFD tape.

## Audio output

The UI synthesises an original, one-second wailing two-tone warning with tapered ends. Settings
persist a warning mute and volume preference. Desktop output uses optional CPAL (Apache-2.0),
enabled by the desktop `audio` feature; `--no-default-features` builds without it. Linux output
requires the ALSA runtime library. The repository's Linux link configuration uses the stable
`libasound.so.2` ABI without requiring development headers. The browser uses Web Audio through
`web-sys` and activates its context on user input. Devices open lazily; unavailable output is a
visible settings error, and headless simulation and tests need no audio device.

## Behaviour

* `wing_leveler` senses an electric canted turn gyro, controls only roll, and limits servo torque.
* `two_axis_legacy` uses the same rate sensor for roll and a pressure-derived climb rate for pitch.
  Lateral modes are ROL, HDG, NAV, APR, and REV. Vertical modes are ALT and VS. ALT preselect and
  alerting are installation options. Electric trim unloads the pitch servo; sustained residual
  demand produces MISTRIM. Loss of the turn-coordinator breaker removes its roll mode and
  disconnects the autopilot. Static obstruction invalidates vertical control unless alternate
  static is selected. Heading gyro invalidity removes HDG.
* `digital_two_axis` computes attitude flight-director commands independently of coupling. Lateral
  modes are ROL, HDG, TRK, NAV, APR, and LOC/BC. Vertical modes are PIT, ALT, VS, IAS, GP, and GS.
  Selected-altitude capture arms ALTS and transitions to ALT during the approach to the target.
  Moving the selected altitude while holding ALT does not start a climb; `captured_altitude_m`
  remains the held reference until the pilot selects a climb/descent mode and ALTS captures.
  IAS controls pitch; throttle stays with the pilot for climb or descent. Underspeed and overspeed
  override vertical guidance while preserving the selected mode. Optional stability protection
  adds a limited corrective force beyond 45° bank or ±20° pitch with AP off. Optional YD damps
  lateral acceleration. LOC/BC, GP, and GS wait for compatible receiver guidance.

Servo torque uses normalised control force: 1 means full pilot authority. Servo commands slew at
the installation's rate and saturate at its torque limit. Pilot input beyond that torque relative
to the engagement reference releases the corresponding clutch, so the pilot can overpower it.
The pitch trim remains after disconnect. CWS releases both clutches without changing the selected
modes. Bank commands are normally limited to 25°; actual bank can overshoot during roll-in. Pitch
commands have a rate limit, and measured rate and load factor constrain pitch-servo demand.
Electrical loss, servo faults, or required sensor loss disconnect with a reason and flashing AP.

## Navigation boundary

`sim::nav_source` is independent of equipment skins and receiver implementations. `NavSource`
returns an optional `NavGuidance`: desired grid track, cross-track metres (positive right),
`ToFrom`, receiver kind (`gps`, `localizer`, or `back_course`), and optional vertical deviation
(positive above path) and path angle (negative descending). All angles are radians. Invalid or
missing guidance means loss of signal.

`GpsNavSource::from_receiver` adapts shared receiver output: supply its fix-valid flag, desired
true track in radians, signed cross-track in nautical miles, To/From, and the true bearing of grid
north at the aircraft. The adapter converts cross-track to metres and subtracts meridian
convergence from the true track. Use the shared geodesy module to obtain grid-north bearing;
receiver geodesy remains responsible for calculating great-circle desired track and deviation.
For pilot-facing glass or standby output, first convert its magnetic desired track back to true
with `Sim::heading_reference().magnetic_to_true`. Use the receiver validity and signed cross-track
values, preserving their units. Refresh the adapter from receiver output at every fixed step, and pass
an invalid fix on loss of navigation power or signal. The adapter does not recompute geodesy or
read display CDI deflection, whose sign and full-scale scaling differ between skins.

`ReceiverLeg` adapts the active leg from the merged GPS modules and evaluates it using
`sim::avionics::navigation` at every fixed step. The glass flight-plan command supplies its active
leg; the standby HSI supplies its direct-to leg. Both paths preserve shared great-circle geodesy,
convert true track to local-grid track, and propagate receiver power or failure as signal loss.
Observing avionics state does not advance either receiver or the autopilot.

`DirectTo::new(origin, destination)` supplies an original, bounded local-grid leg. Without a GPS,
`avionics.set` can select a direct-to with `{"setting":{"kind":"direct_to_local","north_m":1000,
"east_m":20000}}`. The simulation samples it every fixed step. `Sim::step_with_nav_source` samples
any receiver or test double at that same boundary; `Sim::set_nav_guidance` supplies a validated
snapshot when the host owns the receiver's fixed-step schedule. GPS adapters should not update
receiver state during observational calls. Clearing guidance disconnects a captured NAV or
approach; an armed mode waits without inventing a signal. Vertical paths capture only from a
compatible receiver with both deviation and path angle.

## Script assists and compatibility

`assist.set` and `assist.off` are the idealised holds. They are separate from installed equipment
and need no panel or electrical power. `autopilot.set` and `autopilot.off` remain deprecated aliases
for one release and log a notice on stderr, preserving MCP stdout. Existing `readout.autopilot`
continues to describe these idealised holds for wire compatibility. `autopilot.state` describes
installed equipment. Assists and coupled equipment cannot own controls simultaneously.

## Verification and previews

Acceptance flights use the Corvid in the bundled New York region, starting at 1000 m MSL near
1N7 in calm air. Tests exercise both two-axis families with successive 90° and 270° heading-bug
moves, altitude hold and capture, VS, digital IAS climb, a direct-to leg toward KMMU, LVL recovery
from 40° bank and −10° pitch within ten seconds, sensor and servo failures, trim, and overpower.
Thresholds are 3° heading, 50 ft altitude, less than 100 ft capture overshoot, 100 fpm VS, 5 kt IAS,
and 0.1 nm cross-track after interception. Unit tests cover the three behaviour families,
engagement limits, unsupported modes, FD without servos, CWS, protection, and deterministic batching.
The unequipped golden trajectory remains `0xf228fce1a4055061`.

The ignored UI test `render_autopilot_previews` rasterises the actual egui controller meshes
into `out/autopilot/{leveler,legacy,digital}.png`. The companion test
`render_autopilot_glass_preview` captures the glass PFD with HDG, ALT, the mode strip, and FD bars
into `out/autopilot/glass-pfd.png`. Additional integration flights exercise both installed
electronic suites, their shared GPS guidance, heading bug, selected altitude, and sensor failures.


## Magnetic convention

The shared heading bug, HDG, TRK, heading sync, and script-assist heading targets are magnetic.
AHRS and the compass use WMM2025 at the simulation date and aircraft position, with zero
variation in built-in worlds. The conventional heading gyro is initially set to that compass
and then drifts. Sensors and mode targets share this reference, so HDG 090 in New Jersey flies
about 077–078° true, rather than 090° true.

Navigation geometry and `NavSource` remain true/grid based. The simulation converts desired
track and measured ground track to magnetic before the installed controller compares them.
VOR radials and LOC courses retain their published station declination. Magnetic and true
angles must not be mixed in adapters or display pointer rotations. `Readout.heading_rad` is
magnetic; `heading_true_rad` and `heading_grid_rad` explicitly expose truth for geometry.
