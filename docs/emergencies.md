# Emergencies and debriefs

The scenario engine and instructor controls share `failure.set` and `failure.clear` with scripts
and MCP. Open **Scenarios** for the instructor panel. Choose a fault and zero minutes to trigger
it immediately, or a window of up to 60 minutes and a seed for a reproducible surprise. The
simulation clock controls delivery, including pause and explicit stepping. Clearing a fault
retains the emergency debrief; resetting the flight clears it.

| Failure kind | System effect and indication |
|---|---|
| `engine_fire` | Available shaft power falls to 15%; CHT and oil heat rise; smoke accumulates |
| `electrical_fire` | Main and essential buses lose supply; smoke forms while master is on |
| `cabin_smoke` | Cockpit smoke accumulates over 15 s and obscures the cockpit view |
| `door_open` | Additional drag coefficient 0.025; door annunciation and airflow noise in the app |
| `carb_icing` | Accelerated carb ice only with carburettor induction, humidity above 50%, and local temperature −15 to 30 C; heat clears it |
| `airframe_icing` | In visible freezing moisture, lift falls 30%, drag increases 0.065, and downward load increases by 3% of aircraft weight |
| `flap_asymmetry` | Flap-dependent rolling moment; panel warning |
| `trim_runaway` | Elevator demand has a positive 0.65 hardover bias, clamped to travel |
| `trim_stuck` | Elevator demand has a fixed positive 0.20 trim bias; pilot retains elevator authority |
| `radio_failure` | Both COMs lose transmit and receive capability; intercom remains available |

The existing engine, partial-power, surface-jam, vacuum, alternator, and pitot/static failures
remain available. New faults default off and add hash words only when present, preserving the
healthy golden trajectory. Airframe ice requires precipitation or a cloud layer at the aircraft
altitude and a lapse-adjusted temperature below freezing. This is a bounded fault model, with
immediate aerodynamic effects, not an accretion, fire propagation, structural damage, or trim-motor
simulation. Cabin smoke clears over 30 s after its source is removed. Fire remains latched until
cleared by the instructor; operating isolation controls reduces associated smoke production.
The cockpit obscuration is a visual overlay, independent of the outside atmospheric weather.
Warnings appear in the toolbar's **Emergency** menu and instructor panel.

## Aircraft procedures

The optional `emergencies` array in an aircraft's systems layer carries its procedures. Each has
an id, title, ordered items with an id, instruction, and `memory` flag, and optionally a best-glide
airspeed in m/s, glide ratio, speed tolerance, and a `note` (POH warnings and notes for the whole
procedure). This permits separate memory items and subsequent checklist steps, with
aircraft-specific wording and references to the installed systems. Limits and unique item/procedure
ids are validated on pack loading, and the glide ranges (speed 10 to 150 m/s, ratio 2 to 40,
tolerance 0.1 to 20 m/s) apply whenever a value is given. Old packs can omit the array, and old
procedures that give all three glide numbers load unchanged.

The three glide numbers are optional because gliding does not apply to every emergency (a fire on
the ground, an engine failure in the take-off run). A procedure with no glide numbers starts a
debrief with no glide footprint and no landing-area choice, and best-glide speed is not graded.
The Vg bug default is the best-glide speed of the `engine_failure` procedure, else of the first
procedure that gives one.

The failure scenarios start the procedure whose id matches the failure: `engine_failure`,
`engine_fire`, `electrical_fire`, `cabin_smoke`, `door_open`, `carb_icing`, `airframe_icing`,
`flap_asymmetry`, `trim_runaway`, `trim_stuck`, `radio_failure`, `vacuum_pump`. An engine fire on
the ground uses `engine_fire_ground` when the pack has it. A pack may carry further procedures (an
engine fire in the take-off run, a forced landing, a spin recovery); they are not started by a
failure but are listed in the checklist panel.

### The Emergency tab

The checklist panel has an **Emergency** tab whenever the aircraft has `emergencies`. It reads the
pack's procedures directly (they are not copied into the normal checklists). Memory items are marked
`MEMORY` and shown in red, the glide line appears when the procedure has glide numbers, and the
`note` is shown under the items. When a failure raises an emergency the panel jumps to that
procedure once; its items can then be completed there with the same timing and ordering as the
scenario window's **Complete** buttons.

In flight (and on the ground without a checklist open) a running emergency also puts the checklist
panel beside the outside view in the main window, on the Emergency tab at its procedure, so the
student sees the memory items without opening anything. The top-bar **Emergency** menu lists the
active faults and has **Hide emergency procedure** / **Show emergency procedure** to give the
outside view the whole band back; the panel opens again by itself for the next emergency.

Corvid procedures are original fictional guidance for carburettor, magneto, and vacuum systems.
Larrow procedures are original fictional guidance for its injected engine and electrical gyros;
it does not offer carb-ice or vacuum-pump procedures. No external aircraft pack is modified. The
format can hold procedures transcribed by a pack author from a properly licensed real POH, but
these fictional procedures are not a real POH and must never guide operation of a real aircraft.
The schema is generated with the rest of the [aircraft format](aircraft-format.md).

## Grading and footprint

The first active failure with a procedure in the current aircraft starts a debrief. It captures
failure time, provenance, the aircraft procedure, glide geometry, wind, and reachable candidate
fields. The **Emergency recognized** button records recognition delay. Each **Complete** button
records the current simulation time, checks the next unfinished item, and flags an out-of-order
completion. Duplicate and unknown item submissions are rejected. These are pilot acknowledgments;
there is no automatic inference that the pilot physically completed a switch or memory action.

`emergency.record` has an `observation` tagged as `recognized`, `checklist` with an `item`, or
`choose_field` with an `id`. It returns the same state as `scenario.state`. A checklist acknowledgment
also satisfies a scenario checklist trigger. All speed samples while airborne count toward the
best-glide tolerance total. The debrief retains counts, rather than only a rounded percentage.
It detects completed pilot COM calls containing Mayday, Pan-pan, or Pan pan on the channel in use
at failure or 121.5, and observes the existing avionics transponder's actual 7700 selection.
Outcome records landing or crash, with a separate indication of proximity to the chosen field.
No overall ACS score or certification claim is produced.

The advisory footprint samples 72 ground tracks in five-degree increments. On each, wind crab and
along-track wind determine ground speed, and airspeed/glide ratio plus vertical wind determine
sink. Terrain is checked every 100 m out to 30 km, retaining 15 m clearance. Missing region coverage
stops the ray conservatively. Strong crosswind or headwind can make a track unreachable. The
footprint is frozen at failure time; flying later does not move the debrief boundary.

Candidates include imported airport runway thresholds inside the footprint and at most six
ranked terrain rectangles. A bounded grid search samples 600–1,200 m long, 60 m wide strips
along the wind (four possible orientations in calm conditions), with 25 longitudinal stations
and three cross-strip samples. Every sample must lie inside the reach polygon, have grass
surface, local slope at most 3%, and total relief at most 2% of strip length. Candidates rank
by length, slope, and distance, with at least 900 m between selected centres. Labels give
bearing, distance, sampled length, and maximum sampled slope. These are **unverified surfaces**:
the terrain surface category does not establish the absence of forest, towns, crops, fences,
or power lines. Water and pavement samples are excluded. No field boundary or obstacle
clearance is established, and coarse elevation grids can miss significant local hazards.

The model assumes an immediate established glide, constant wind, fixed best glide, and no
turn penalty, maneuvering loss, or emergency-specific reduction in glide ratio. Reaching an
airport threshold does not prove sufficient alignment, landing distance, or obstacle clearance.

The emergency view uses the shared planning map's terrain base, projection, and FAA airport/runway
renderer, centred on the failure. The frozen reach boundary, selected field, flown track, and
touchdown or impact are overlaid in that frame. Track samples are recorded approximately every
25 m, bounded to 4,096 points. The human debrief describes failure height and location, recognition,
airspeed tolerance, radio response, and outcome relative to the chosen runway. The preview uses
the released New York terrain and an actual fixed-step engine-out flight at 1N7.

Current state includes one emergency debrief per flight. Concurrent later faults still affect the
simulation, but do not replace that first debrief. Scenario action history, emergency geometry, and
acknowledgments persist in the flight record and saved-flight archive. The saved-flight radio view shows the frozen footprint and timings;
terrain relief is drawn only when the active region matches the saved footprint frame. `scenario.state`
also exposes the live debrief. Integration with the future `lesson.debrief` remains separate.
See [scenarios](scenarios.md), [systems](systems.md), [planning terrain](map-and-planning.md),
[crew lessons](crew.md), and [training](training.md).

A sliding-canopy aircraft derives `door_open` while airborne and unlatched. The canopy remains a
separate physical state; clearing injected failures cannot latch it. The shared procedure retains
id `door_open`, with the title “Canopy unlatched”. Opening is refused in flight; close and then
latch through `aircraft.canopy`. The state drives the same exterior and cockpit presentation.
