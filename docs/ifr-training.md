# Instrument training

IFR lessons are original JSON documents in [lessons](../lessons/), described by
[schema/lesson.json](../schema/lesson.json). They embed the existing scenario format for timed
cues, weather, failures, and radio events. The setup selects an installed region and aircraft,
a published procedure, an inbound position, weather, the hood, instrument failures, and an
explicit training minimum. Starting validates a replacement flight before installing it; invalid
content leaves the current flight intact. A lesson starts paused. Airborne lessons continue a filed IFR clearance with assigned
transponder code 4201, ready for a handoff to the imported approach facility. KABE ILS 06
starts southwest of the airport at 4,300 feet MSL, offset from final for a real vector intercept.
KMMU starts seven miles out at its published 2,000-foot intermediate altitude, below the
glidepath, leaving time for ATIS and approach contact before the FAF.

Open **Instructor station**, choose an instrument lesson, and select **Start IFR lesson**.
Review the briefing, then Resume. The library includes KABE ILS 06, KABE RNAV 06 LPV and LNAV,
KMMU RNAV 05 LPV and LNAV, KABE LOC 06, missed approaches at DA and MDA, JISTO holding,
Corvid vacuum/AHRS/display failures, unusual attitude recovery, and a KABE busy-frequency radio
discipline lesson. Both regional catalogues
are required to start the entire library. Regional terrain must be installed through the normal
data installation workflow.

## Standards and minima

Task references are to [FAA-S-ACS-8, Instrument Rating – Airplane ACS](https://www.faa.gov/training_testing/testing/acs).
The following tolerance excerpts are the only ACS quotations used in these lessons:
**±100 ft**, **±10°**, **±10 kt**, and **¾-scale CDI**. They apply in their relevant task segments;
the altitude band is not a requirement to remain at one altitude while descending a glidepath.
MDA is a floor, not a symmetric tolerance. The rules grade measured flight data, independently
of who flies or speaks.

| Task | ACS reference | Assessment |
|---|---|---|
| Nonprecision final, including LNAV and LOC | IR.VI.A | Lateral indication, airspeed, MDA floor, level altitude |
| ILS and LPV final | IR.VI.B | Lateral and vertical indications, airspeed, decision at minimums |
| Published missed approach | IR.VI.C | Initiation, published legs, navigation deviation, altitude, arrival at missed hold |
| Holding | IR.III.B | Actual arrival/entry geometry, inbound timing, leg length, altitude, wind correction |
| Approach with failed primary instruments | IR.VII.D | Final approach evidence while the failure is present |
| Unusual attitude recovery | IR.IV.B | Controlled return to stable flight after instructor handback |
| Look up and visual transition | IR.VI.E | Hood removal and measured cloud boundary |
| Radio discipline on a busy frequency (`busy-frequency`) | IR.III.A | Two traffic advisories answered; no answers to other aircraft's calls, no unanswered calls, no transmitting over traffic ([details](atc.md#traffic-advisories-and-radio-discipline)) |

The ACS does not supply the lesson's hold timing/leg-length acceptance margins or numerical
unusual-attitude completion thresholds. Those are explicitly instructor criteria: 10 seconds,
0.5 NM, and a stable recovery indication. The debrief gives the reason for every failure.
Pending tasks do not pass simply because no error was recorded; finishing without observing
the required outcome fails the task as incomplete.

CIFP supplies route and guidance geometry, but not chart decision minima. Lesson authors choose
and review the minimum explicitly. Bundled values were checked against the cached FAA d-TPP
plates: KABE ILS/LPV 594 feet MSL, KABE LNAV 840 feet, KABE LOC 920 feet, KMMU LPV 879 feet,
and KMMU LNAV 880 feet. The KABE ILS minimum has chart equipment conditions; the exercise is a
training configuration, not an operational authorization. No plate image is bundled or drawn.

## Instructor actions

The instrument section provides hood on/look up, exact or seeded failure scheduling, and
unusual attitude setup. Equal earliest/latest times schedule an exact simulated moment; a
window uses deterministic integer mixing of the seed. Zero triggers immediately. Instrument
failure choices are the vacuum pump, glass AHRS, and primary flight display. Normal scenario
failure controls remain available below these controls.

Unusual attitude setup requires the human pilot to own controls and at least 2,000 feet of
terrain clearance. It preserves position, velocity, weather, systems, and flight time, sets the
hood, disconnects assistance, and freezes the flight for handback. Resume begins the recovery.
No FAA logbook credit is claimed for hood or IMC totals.

At minimums the instructor says “Minimums. Look up.” and removes the hood in visual-transition
lessons. The weather model still controls visibility: removing the hood inside cloud cannot
make a runway visible. The published-missed lesson keeps the weather below minimums. Nonprecision visual-transition
lessons place the cloud base 50 feet above MDA and level at MDA until visual reference is revealed.

## Crew demonstrations

`crew.intent` accepts `approach` and `hold_at_fix`. Use **Fly me…** to brief the intent, then
complete the existing two-party exchange of controls. An approach requires an airborne inbound
setup; starting its lesson provides one. Coupled flight requires an installed autopilot; hand
flight uses ordinary control commands. The crew uses the shared procedure receiver and NAV
radio, with intercom calls for localizer/course acquisition, glidepath capture, and minimums.
It lands only with visual reference and the accepted landing clearance for the assigned runway.

```json
{"intent":{"kind":"approach","airport":"KABE","procedure":"I06","mode":"ils","minimum_ft":594,"coupled":false}}
```

```json
{"intent":{"kind":"hold_at_fix","airport":"KABE","procedure":"I06","fix":"JISTO"}}
```

The instrument demonstration's crosswind criterion is 15 knots, an engineering training
criterion; it is not represented as a certified aircraft limit.

## ATC

CRAFT, departure, and the readback queue use the existing ATC engine. The airborne IFR check-in
uses aircraft identity, present altitude, and ATIS information (see
[AIM 5-4-3](https://www.faa.gov/air_traffic/publications/atpubs/aim_html/chap5_section_4.html)).
Approach responds with radar contact and measured position, followed by a final vector and
clearance; the pilot must read back the heading, the altitude restriction **until established**,
and the correct approach/runway. IFR lessons do not request VFR flight following. Intercom
calls follow the heard clearance and actual navigation capture. Tower handoff is triggered just
outside the published FAF (0.25 NM), rather than at a fixed elapsed time or distance from the airport. Instrument
approach clearances use the shared phraseology renderer, including “Cleared ILS runway zero six
approach.” `atc.ifr` adds validated final vectors, published missed instructions, and published
hold clearances with a UTC HHMM EFC. Final vectors apply [JO 7110.65 5-9-1/5-9-2](https://www.faa.gov/air_traffic/publications/atpubs/atc_html/chap5_section_9.html)
intercept-angle limits
(30 degrees, reduced to 20 near the final approach fix), an intercept at least two miles outside the approach gate for an angled vector, an
altitude at or below the glidepath, and local terrain clearance. The combined heading,
altitude-until-established restriction, and approach clearance follow
[JO 7110.65 4-8-1](https://www.faa.gov/air_traffic/publications/atpubs/atc_html/chap4_section_8.html). KABE vectors maintain the published 3,000-foot intermediate altitude until established so that
glideslope capture precedes the 2,400-foot SHAGY FAF; a 2,300-foot vector would capture inside
that fix in the regional procedure data. These bounded offline rules
are not a general radar controller or an obstacle-clearance service.

```json
{"airport":"KABE","instruction":{"kind":"hold","procedure":"I06","fix":"JISTO","efc_utc":1530}}
```

A clearance is sent over the normal imported facility frequency, waits for a heard transmission
and a readback, and records the readback grade. Issuing a command does not silently accept a
clearance or change the aircraft's controls.

## Live grades and debrief

**Debrief** overlays the measured ground track on the regional procedure's fix/leg geometry,
with missed legs in amber. Deviation plots show ACS bands for CDI, glidepath, airspeed, and
altitude. Measurements are sampled on the fixed simulation timeline at 2 Hz, bounded to 14,400
samples; reaching that limit fails the unfinished task and freezes the debrief. Instrument final
grading begins at the FAF. Setup samples remain visible for context. Reads do not advance time.
Live debrief state clears on a new lesson/reset. Flight saves and exports retain the measured
lesson debrief with the flight record, and the Debrief page also displays a loaded saved lesson.

| Command | Input / output |
|---|---|
| `lesson.list` | `{}` → bundled lesson documents |
| `lesson.start` | `{lesson}` → live debrief |
| `lesson.debrief` | `{}` → measured samples, geometry, and task results |
| `lesson.finish` | `{}` → frozen final grades |
| `lesson.instructor_failure` | `{failure, earliest_s, latest_s, seed}` → debrief |
| `lesson.instructor_attitude` | `{pitch_deg, bank_deg}` → debrief |
| `atc.ifr` | `{airport, instruction}` → ATC state |

All commands have input and output schemas and are exposed by the normal MCP registry.
Instructor/debrief state also travels with display snapshots.

## Verification and captures

The control integration tests are in `crates/control/tests/ifr.rs`. They fly the six approach
configurations in calm air and a 15-knot crosswind, exercise hand and coupled ILS flight,
check the published missed and hold, sweep hold-entry headings, and retain failure reasons
through synthetic deviations and recoveries. Optional region tests follow the existing local
skip/CI-required data convention. Offscreen GPU previews use
the actual egui instructor/debrief widgets and the renderer/PFD. Generated artifacts belong in
`out/ifr/`; no real-window capture is attempted in the sandbox. Flight lead should capture these
real-window views: KABE ILS 06 PFD in IMC with LOC/GS indications; the cockpit breaking out at
594 feet MSL; the final debrief map with task grades and deviation bands; and the instructor
station with lesson selection, failure scheduling, and unusual attitude controls.

Run the captures with the same target directory and UI shim used by the other preview tests:

```sh
CARGO_TARGET_DIR=$HOME/.cache/buildq/target/flightcraft/fc-ifr \
PKG_CONFIG_PATH=$HOME/builds/pkgconfig-shim \
cargo test -j 4 -p flightcraft-ui --lib render_ifr_previews -- --ignored --nocapture
```

The test writes `pfd-ils-imc.png`, `minimums-breakout.png`, `debrief.png`, `instructor.png`,
`kabe-ils-06-transcript.txt`, and the measured JSON debrief in `out/ifr/`.
