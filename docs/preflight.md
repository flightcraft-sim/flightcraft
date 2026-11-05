# Exterior pre-flight walkaround

A figure walks around the parked aircraft and does the pre-flight inspection from the aircraft's
own handbook, item by item, so the student sees what each check is, where it is, and what a bad
one looks like. Instructors and lessons seed findings, and findings the student misses have real
consequences in the flight. The engine adds no inspection steps of its own: every item, its
wording, its handbook reference, and where it happens comes from the aircraft pack's `preflight`
section ([format](aircraft-format.md#preflight-walkaround-section)). The Sling 2 pack carries
POH Rev 2.8 section 4.4 (67 items in six stations).

This is simulated practice. It does not replace the aircraft's handbook or a real walkaround.

## What the student sees

- **A pilot figure.** An original, procedurally generated low-poly mannequin (capsule limbs, sphere
  head, cap, hi-vis vest, boots) in `crates/render/src/walkaround.rs`, posed by two-bone inverse
  kinematics from the plain figure state the session supplies. No third-party or scanned assets and
  no GLB; the mesh is regenerated each frame (about 2,000 triangles), drawn through the existing
  exterior PBR pipeline, and added to the shadow pass so it casts a shadow with the aircraft.
- **Realistic walking.** The figure walks the pack's route at the pack's pace (1.3 m/s for the
  Sling), only along a closed loop of points that is validated to stay clear of the wings, tail,
  fuselage, and propeller disc. It takes the shorter way round. `preflight.speed` (0.25 to 16)
  speeds the animation up; the choreography is a pure function of time, so any speed plays
  identically.
- **Actions, so each check is visible.**

  | Action | What is drawn | Effect |
  |---|---|---|
  | Look / look and touch | Gaze at the part; hand on it for "condition" and "secure" items | Reveals a seeded finding |
  | Crouch and look | Crouched at a wheel, brake, tyre, strut, or the wing underside | Reveals a seeded finding |
  | Move a control | Hand on the trailing edge rocking an aileron or flap, or moving the elevator and its tab or the rudder | Drives the real exterior `moving_parts` nodes (`aileron_left`, `flaps_right`, `elevator_left`, `rudder`, ...) through their travel, then releases them |
  | Fuel | Opens the filler cap, leans over, looks in; a see-through tank hovers above the cap | Shows the real tank quantity (below) |
  | Oil | Reaches in and pulls the dipstick; a magnified stick beside the hand shows the level against its MIN and MAX marks | Reveals low oil |
  | Propeller | Palm on the blade, turning it one blade pitch | Only with ignition off and the engine stopped; otherwise refused |
  | Remove | Reaches, lifts, carries off a pitot cover, tie-down rope, or chock | Clears the hazard |
  | Cabin | Climbs in over the step with the canopy slid open (`aircraft.canopy`), works the control | Master switch, ignition off, flaps full down act on the real systems |

- **Fuel level.** Opening a cap shows the actual quantity of that tank from the systems model: the
  fuel fills a thin film over the tank bottom at the cover level (30 L in the Sling, per the POH
  warning), rises as the tank fills, and the 30 L line is marked. A tank holding less than that
  only wets part of the bottom. Reading comes from `systems.tanks`, so `systems.set` /
  `TankQuantity`, a different load, and a low-fuel finding all show up truthfully.
- **Oil level.** The Sling has no oil quantity model, so the dipstick level is runtime state of the
  walkaround: normal, or below MIN when `low_oil` is seeded. The method (dipstick level between
  MIN and MAX) is the Rotax procedure the POH refers to as "engine checks as per the Rotax
  manual"; the POH text itself only says oil and coolant quantity check.
- **Covers and tie-downs.** A parked-aircraft walkaround starts with the pitot cover, wheel chocks,
  and tie-down ropes fitted, drawn on the aircraft. The items that say "remove" take them off.
- **Findings.** A found problem gets text (`items[].finding`) and a visible cue on the model: a
  ring marker with a crack line or a dark patch where relevant.

## Camera

While a walkaround is active the session camera switches to `Walkaround` (the previous mode returns
when it ends), and `preflight.state.camera` carries the same framing for an inset: the target (a
point between the figure's chest and the part being checked), the station's suggested eye offset
(close and slightly elevated; z is down), the smoothed eye position, up vector `[0, 0, -1]`, and
field of view. The eye follows with a 0.55 s time constant, so station changes glide. The render
`Scene` takes the same data through `set_walkaround(session.preflight_scene())`, and
`CameraMode::Walkaround` frames it as the main outside view.

## Commands

Registered like every command (schemas by `cargo xtask schemas`, exposed over MCP):

| Command | Purpose |
|---|---|
| `preflight.begin` | Start. `lesson`, `seed`, `random_findings`, explicit `findings`, `covers_on`, `planned_fuel_l`, `speed`. Uses the active aircraft's pack data. |
| `preflight.state` | Stations (id, name, order), items (id, station, text, `poh_ref`, `done`, `finding`, `fault_seeded` with `instructor: true`), current station and item, `camera`, `figure` and `action`, hazards, fuel, grade, and the drawing `scene`. |
| `preflight.perform {item}` | Queue an item. The figure walks to its station and does the action; the item is `done` when the animation finishes. Items queue in order. |
| `preflight.fault {item, kind}` | Seed a finding. It is revealed only when that item is performed. |
| `preflight.resolve {item}` | Rectify a found finding (top up, refuel, repair), making the aircraft fit. |
| `preflight.advance {seconds}` | Run the animation clock without stepping the flight (scripts, tests). |
| `preflight.speed {speed}` | Animation speed-up. |
| `preflight.end` | Finish, return the grade; the figure leaves. Hazards stay on the aircraft. |
| `preflight.lessons` | The pre-flight lessons. |

The animation advances with `sim.step` (every fixed step feeds it), `sim.wait`, or
`preflight.advance`. In the app it also runs on wall-clock frame time while the flight is paused
(a preflight is a ground activity, and the lessons start paused), at the chosen `speed`; the flight
state does not change, only the walkaround's own effects (surfaces moved by hand, covers taken
off). While the flight runs, the fixed steps feed it instead, so it never advances twice.
Scripted runs and tests stay deterministic because only the app loop calls
`Session::advance_walkaround_paused`.

A pitot cover, chock, rope or missing hardware does not turn the Emergency menu red while parked:
they are latent hazards, and a chock has its own "will not move" message. The menu reports
injected failures, and in the air the hazards' effects too. Seeds are `DefectKind`: `pitot_cover`, `tie_down`, `chock` (props left on),
`low_oil`, `low_fuel`, `loose_cowl_fastener`, `aileron_hardware_missing`, `flap_hardware_missing`,
`damaged_tyre`, `wheel_fairing_crack`. A pack lists which kinds each item accepts; anything else is
refused.

## Consequences

Findings use the existing models; nothing here is a new failure model.

| Finding | Effect |
|---|---|
| Pitot cover left on | `failures.pitot_blocked`. The airspeed indicator reads about zero on the roll and in flight (total pressure is trapped; it only drifts with altitude). |
| Chock or tie-down rope left on | The aircraft cannot roll (horizontal velocity and yaw are held to zero while on the ground); a tie-down rope also stops it lifting off. `preflight.state.blocked` carries "Chocks are still in place: the aircraft will not move" once the throttle is up. |
| Low fuel | The seeded tank is set to 40% of the cover level via `TankQuantity`, so the gauge, the filler cap view, and the flight all agree. |
| Flap hinge hardware missing | `failures.flap_asymmetry` (the existing flap-dependent rolling moment). |
| Aileron hinge hardware missing | The existing aileron jam, at 35% deflection, once airborne. |
| Low oil, loose cowl fastener, damaged tyre, wheel fairing crack | Graded findings with a visible cue. The simulation has no oil quantity, cowl, tyre, or fairing model, so flying with them changes nothing physical. |

Hazards ride on `Sim::hazards()` (`flightcraft_sim::hazards::GroundHazards`), recomputed from the
walkaround state after every change and added to the state hash only when present, so healthy
replays keep their golden hashes. Resetting the flight ends the walkaround.

## Grading

`preflight.state.grade` (full detail with `instructor: true`):

- required items done (optional items, such as the long-term-parking propeller turn, are not
  required);
- seeded findings found (an item must be performed to reveal its finding);
- covers, ropes, and chocks still on the aircraft;
- no-go findings still open: a seeded `no_go` defect not rectified, or a cover, rope, or chock
  still fitted;
- `departed` and `flew_with_no_go`: the aircraft left the ground or moved at over 3 m/s while any
  no-go was open. Rectifying afterwards does not undo the departure;
- `passed`: everything required done, every seeded finding found, nothing left on, and not flown
  with a no-go.

Severity is part of the pack's data. In the Sling pack: low oil, low fuel, missing aileron or flap
hardware, and a damaged tyre are no-go; a loose cowl fastener and a cracked wheel fairing are
cautions (not counted as no-go).

## Lessons

Two lessons in `lessons/`, listed by `preflight.lessons` and started with
`preflight.begin {lesson}`:

- `sling-preflight` - "Sling: pre-flight walkaround". The Sling 2 parked at 1N7 runway 07 with
  covers, chocks, and ropes fitted and a 40 L planned flight. No seeded findings.
- `sling-preflight-findings` - the same, plus two findings chosen at random from the pack's seedable
  defects, on two different items. The choice is a pure function of the seed
  (`select_findings`), so a seed always seeds the same two findings. `preflight.begin` takes
  `seed` and `random_findings` to override.

Starting a lesson replaces the flight with the lesson's aircraft and airport, so the region and the
aircraft pack must be installed.

## Pace and scope

- Real pace: walking 1.3 m/s; each check takes 3 to 6 seconds; the full 67-item list at 1x takes
  about seven minutes of watching (400 s measured, nothing skipped). Use `preflight.speed` to compress it.
- Positions are taken from the exterior model (wing and tail extents, the pitot mount, the gear,
  the wing tie-down points). The filler cap positions, tail rope, parachute cover, and fire
  extinguisher locations are approximations, not measurements; they are only used for where the
  figure looks and reaches.
- Not modelled: opening the cowling (the oil check reaches the dipstick without a hatch), a fuel
  sump drain (the Sling POH 4.4 does not list one), brake fluid, tyre pressure, and any item the
  POH does not list.
