# Normal procedures: checklists, the run-up, and the ground layout

(`docs/procedures.md` is about instrument procedures from the FAA CIFP. This page is about *normal*
procedures: before start, start, warm-up, run-up, taxi, before take-off, shutdown, and securing.)

Learning to preflight and run up a real aircraft takes a long time, so the simulator teaches it:
a checklist that ticks itself from simulator state, POH timers and limits, a run-up that is
physically modelled and graded, and a ground layout with the checklist on one side, a small
exterior view on the other, and the whole dash below. For simulation only. The pack author's real
POH always governs a real aircraft.

## Checklist data in the pack

Normal procedures are optional systems-layer pack data, the sibling of the
[emergency procedures](emergencies.md#aircraft-procedures). A systems layer may carry
`procedures`, an object with its **own version** (`version: 1`; unknown versions are rejected),
the `source` document it was transcribed from, and `checklists`. Packs without it still load, and
the checklist panel says the aircraft has none. The schema is generated with the rest of the
[aircraft format](aircraft-format.md) (`schema/aircraft-systems.json`).

A checklist has an `id`, `title`, `phase` (`before_start`, `engine_start`, `warm_up`, `run_up`,
`taxi`, `before_takeoff`, `shutdown`, `securing`), a `poh_ref`, ordered `items`, and optional
`limits`. An item has:

| Field | Meaning |
|---|---|
| `id`, `text`, `response` | The challenge and response, as read from the POH ("Brakes" / "on") |
| `poh_ref` | The handbook section and item, shown in the panel and in grading reasons |
| `verify` | Optional binding to simulator state; the item ticks itself when it holds |
| `control` | Optional panel element id; the dash outlines it with a pulse while the item is current |
| `timer` | Optional POH time limit (see below) |
| `note` | A caution or detail shown with the current item |
| `unverified` | Set when a number is not from the POH ("per Rotax 912 iS operator's manual, not verified") |
| `stop_on_failure` | The item is a go/no-go gate: unmet or failed means stop, not carry on |

Items without a `verify` ("Passenger: briefed") are ticked by the pilot.

### Verify bindings

`verify` is a tagged object (`kind`):

| Kind | Holds when |
|---|---|
| `master`, `avionics` | the switch matches `on` |
| `ignition` | both magnetos, or both electronic lanes with the key on (or off) |
| `lane`, `pump` | lane `a`/`b` or fuel pump `one`/`two` matches `on` |
| `backup_battery` | the circuit (`ems`/`efis`) switch matches `on`, optionally with `min_voltage` |
| `fuel_selector` | the selector is at one of `positions` (`left`, `right`, `both`, `off`) |
| `panel` | a panel control's 0..1 value is within `min`/`max` |
| `rpm`, `oil_temp`, `oil_pressure`, `fuel_pressure`, `bus_voltage`, `throttle`, `flaps` | the reading is within the given bounds (crank rpm; bar; kPa; volts; 0..1) |
| `engine_running`, `starter` | the engine runs / the starter is engaged |
| `brake` | wheel brakes at or above half, by the hand lever or the park-brake valve |
| `park_brake` | the brake intercept valve (the park brake) is closed (`set`) or open |
| `trim_neutral` | pitch trim within `tolerance` of neutral |
| `canopy_latched` | a sliding canopy is closed and latched (always true without one) |
| `check` | a run-up check the simulator observed and found within limits (below) |
| `all` | every listed binding holds |

A **timer** is `max` (counts while `start_when` holds; going over warns: the starter's 10 s),
`within` (the item's `verify` must hold within `seconds` of `start_when`: oil pressure within
10 s of the engine running) or `hold` (the `verify` must hold continuously for `seconds`: two
minutes at 2,000 rpm). A **limit** is a condition that must not occur, with a message: "Increase
rpm only once oil pressure is steady above 3 bar". Validation bounds every list, number, and
nesting depth, requires unique ids, and rejects unknown fields.

## The Sling 2 lists, and where the numbers come from

The Sling 2 pack carries eight lists transcribed (in original wording) from the **Sling 2 POH
DC-POH-002-X-B-2.8, Revision 2.8**: before starting engine (4.5.1), engine start (4.5.2 with the
Rotax 912 iS supplement 9.4.12), warm-up and engine check (4.5.3), taxi (4.6), before take-off
(4.7.1, all 18 items), engine shutdown (4.15), and parking and tie-down (4.16).

The 912 iS specifics are **in the Sling POH itself**, supplement 04/2012 section 9.4: the start
sequence with the EMS back-up battery and the lane warning lights (9.4.12), oil pressure steady
above 3 bar before raising rpm, and the ignition check at 4,000 rpm (9.4.13): no rpm drop with
lane B off, less than 180 rpm drop with lane A off, one lane at a time.

The Rotax 912 iS operator's manual (BRP-Rotax, the document the POH points to) could not be
fetched: only forum excerpts were reachable (rotax-owner.com threads quote 250 rpm at 4,000 rpm and
about three seconds per lane step, and a 2023 edition for the 912/914 that quotes 500 rpm for a
single circuit), none citing a document number, revision, or section. They are **not** used. These
items are marked `unverified` and show it in the panel:

* the fuel pump check (each pump alone keeps pressure) and its 280 kPa limit, which is the pack's
  own FUEL PRESSURE annunciation threshold;
* the 25 rpm measurement allowance on the "no drop" lane B check;
* the back-up battery voltage limit (11.5 V; the POH says only "verify the voltage");
* the static maximum rpm (4,600) and 8 s acceleration limit of the full-power check, which are
  estimates for this simulator's fixed-pitch propeller (the POH's 5,500 to 5,800 applies to the
  take-off roll).

Where the POH and its supplement differ, the warm-up list uses the longer figure: 2,000 rpm for
two minutes (4.5.3), though supplement 9.4.12 item 14 allows one; then, as the supplement says,
raise rpm until the main bus shows 13.8 to 14.4 V (alternator B online; the model needs about
3,000 rpm for that), then 2,500 rpm until the oil is at 50 C, then the EMS back-up off.

The Sling pack places every control a list operates: the master switch and flap selector on the
dash, and the throttle, hand brake lever, and park-brake valve on the drawn centre console, with the
pilot's stick (trim buttons) on the floor beside it and the passenger's opposite, as POH 7.2, 7.4, 7.9,
7.10, 7.11 and 7.15 describe (see [equipment](equipment.md) for the element
kinds). The items that use them name them in `control`, so the pulse leads the student to each one,
and the cold-and-dark lesson can be worked from the panel alone, without Show me
(`crates/ui/src/cockpit_tests.rs` does exactly that). Controls below the dash outline are reached
with the dash's Console button, which pulses while such an item is current. The Corvid has a short
original fictional set (before start, start, run-up, shutdown).

## The checklist runtime

`checklist.list`, `checklist.state`, `checklist.select {id}`, `checklist.complete {item, force}`,
`checklist.abort {reason}`, `checklist.show_me {item}`. The runtime evaluates at 10 Hz of
simulated time, inside the fixed step, so it is deterministic and identical on desktop and web.

* Items tick in order. An item whose binding holds when it becomes the current one ticks at once.
* An action performed ahead of the current item (a switch, a check) is flagged "Out of sequence".
  Readings that pass through a range (rpm on the way to 3,000) never are.
* Limit warnings show while their condition holds and are recorded once per occurrence.
* A **gate** item that is unmet or failed, followed by later items being done, a later action, a
  forced tick ("Continue anyway"), or lifting off, sets `continued_past_gate`.
* `checklist.abort` records the stop. It is the right answer to a failed gate.
* **Show me**: the virtual pilot performs one item the way a pilot would (including holding
  4,000 rpm, switching a lane off for three seconds, and the pump and control sweeps) and says what
  and why on the intercom. The run-up observer records it exactly as it records a student's.

## The run-up is modelled, not scripted

* **Lanes.** `engine.json` may give `lane_a_only_power_fraction` and `lane_b_only_power_fraction`
  (default: `single_lane_power_fraction`). The Sling has 1.0 (lane B off: no drop) and 0.93
  (lane A off: about 100 rpm drop at 4,000). A seeded weak lane multiplies its lane-alone fraction
  by 0.82, giving a drop of several hundred rpm.
* **Fuel pumps.** Pressure holds on either pump; a failed pump alone gives none, and the engine
  quits. The observer records pressure after a pump has run alone for two seconds.
* **Back-up battery.** Terminal voltage is nominal until half the charge is gone, then sags to
  80% (`BackupCharge` setup sets the charge). A quarter-charged EFIS battery reads about 10.8 V.
* **Oil.** The existing thermal model gives the warm-up gate; `OilTemperature` sets a cold start.
* **Controls.** Elevator, aileron and rudder must each reach 0.8 both ways; trim is checked for
  neutral. **Full power.** From a low throttle to full, peak rpm and the time to reach the minimum
  are measured after two seconds at full.

The observer (`checklist/runup.rs`) only watches. Measurements are in `checklist.state` under
`runup`. Seeded faults are applied by the lessons through `systems.set` settings that also work
from any script: `equipment` failure components `lane_a_weak`, `lane_b_weak`, `pump1`, `pump2`;
`backup_charge {circuit, remaining_ah}`; and `oil_temperature {value_c}`. They default off and
leave the golden trajectory hash unchanged.

## Procedure lessons and grading

`procedure_lesson.list`, `.start {id}`, `.debrief`, `.finish`. A lesson names the aircraft pack, a
starting state (`cold_and_dark`, `engine_idling`, `engine_running_after_flight`), at most one
seeded fault, and one task per checklist. A `complete` task passes only if the list is worked
through with no out-of-sequence action, no limit warning, no expired timer, and no needless stop.
A `stop_on_fault` task passes only if the student stops before carrying on past the defect.
Every reason in the debrief carries its POH reference.

`setup.place` (`region`, `airport`, `runway_end`, `spot`) says where the lesson happens. Every
Sling lesson uses eastern-pa, 1N7, runway 07: `ramp` (before start, shutdown, and the pre-flight
lessons, which may also set `spot`) or `run_up_area` (the run-up lessons); `hold_short` is also
available. The region has runways, not aprons or taxiways, so a spot is placed from the real
runway threshold and heading, not surveyed. When the region is not installed (for example the
web build) the lesson starts on the default training field and the debrief's `notice` says so;
`preflight.begin` with a lesson refuses with "lesson region is not installed".

| Lesson | Tasks |
|---|---|
| Sling: before start, start, and warm-up | 4.5.1, 4.5.2 with 9.4.12, 4.5.3 |
| Sling: run-up and before take-off | run-up (4.5.3, 9.4.13), 4.7.1 |
| Sling: run-up with a weak ignition lane | stop on the lane drop |
| Sling: run-up with a failed fuel pump | stop at the pump check |
| Sling: run-up with a low backup battery | stop at the back-up voltage |
| Sling: run-up before the oil is warm | stop; do not run up cold |
| Sling: shutdown and securing | 4.15, 4.16 |

These are a separate document type (`lessons/sling-*.json`) from the instrument lessons of
[training](training.md#6-instrument-flying), which are built around an approach. Both are
bundled the same way and start atomically.

## Ground layout

`view.layout {mode}`: `auto` (default), `ground`, or `normal`; also in **Flight, Procedures and
view**. In the ground layout the first outside view's band is split: the checklist panel on the
left (62%), a smaller exterior view on the right, and the dash below, untouched and at the same size
as in the normal layout. A window narrower than 700 px stacks them, exterior underneath.
`auto` shows it while a checklist is open and the aircraft is on the ground. Above 12 m/s ground
speed, or airborne, the window returns to the normal layout whatever was chosen. The stored
screen layout is never modified. The current item's control gets a pulsing outline.
The **Preflight** tab appears only when the walkaround provides the `preflight.state` command
(stations and items are read from `preflight.state`, and `preflight.perform {item}` is called by name).

## Exterior inset

`view.exterior_inset {mode, hold_s, placement}`: **off**, **on the ground** (default), or
**always**. It is a small box showing the aircraft from outside over the cockpit view, on the
page machinery's own scene (`exterior-inset`, drawn by the same `Outside` page as any screen).
You can drag it and resize it by the corner; it is clamped inside the outside view and never covers
the dash. The setting persists with the other view choices (`flightcraft.view.v1` in the
preferences store).

The default view is a three-quarter rear chase sized from the aircraft. When the pilot moves a
control the inset eases to frame that part for `hold_s` (default 4 s), then eases back:

| Pilot action | Framing |
|---|---|
| flaps | trailing edge of the flap nodes, from behind and slightly above |
| ailerons | the outboard aileron (the deflecting wing tip) |
| elevator, trim, rudder (in the air or stopped) | the tail |
| canopy | the cockpit |
| brakes, or rudder while taxiing (nosewheel steering) | the wheels |

Positions come from the node transforms of the pack's own exterior model, found through the
`moving_parts` entries of `exterior.json` (`flightcraft_aero::exterior_framing`), so any aircraft
with an exterior works. While `preflight.state` reports `active: true` with a `camera` (body-frame
`target_m`, `eye_m`, `fov_deg`), the inset uses that walkaround framing instead.

## Tests

`crates/control/tests/checklists.rs` (data validation, every verify kind through the lessons, lane
drops, weak lane continue/abort, pump, backup, cold oil, timers, determinism),
`exterior_inset.rs`, `crates/aero/tests/exterior_framing.rs`, and in `crates/ui` the ground layout
tests (`ground_layout_tests.rs`: the dash fully visible at 1920x1080 down to a 390x844 phone), the
inset framing test, and the preferences round trip. The control tests run on a copy of the Sling
pack in `crates/control/tests/fixtures/sling-pack` (without its exterior), refreshed from the pack
repository.
