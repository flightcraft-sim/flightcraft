# Tower and approach training

The control layer adds deterministic tower and radar rules to the [radio service](radio.md).
Completed pilot transmissions drive the rules; clearances, readbacks, grades, and light-gun
indications are typed [flight events](training.md#3-recorded-events) with source `rules`.
Spoken values and facility calls use the [shared phraseology renderer](phraseology.md).
No language model decides a clearance. ATC state and flight events stay outside the physics trajectory hash.
Radio failure shares the simulator failure state used by scenarios and emergency grading. Resetting a flight clears ATC state.

Use **Radios → Tower and approach training** to select a towered airport. The same panel shows
current facility, information letter, pending readback, latest grade, and lost-communications
indication. Use the existing configurable COM PTT action to enter a typed call. Every facility
and channel comes from the active region's NASR records. There are no built-in replacement
frequencies. Missing data produces an explicit refusal or no reachable facility.

## A radio rehearsal

Select a region, choose a runway start, and configure the training airport. For example:

```json
{"command":"atc.configure","input":{"airport":"KABE","destination":"KMMU"}}
{"command":"comms.tune","input":{"radio":1,"channel_khz":126975}}
{"command":"sim.step","input":{"steps":120}}
{"command":"atc.state","input":{}}
```

ATIS uses the imported ATIS frequency, the current weather report, the runway end most nearly
into wind, and hold-short remarks. Calm wind selects the lowest runway identifier. Information
starts at Alpha, advances on weather/runway changes and every 30 simulated minutes, and wraps
after Zulu. Reading state does not advance the information letter or flight time. Report
`with information alpha` (or `with information A`) on initial contact. Missing or stale
information gets a challenge before taxi, departure, inbound, or approach instructions.

Tune the imported clearance, ground, tower, and departure channels in turn. Example pilot calls
use the callsign placeholder, which expands to the current full or abbreviated aircraft identity:

```text
Morristown Clearance Delivery, {callsign}, with information alpha, request IFR clearance to Allentown
Cleared to Allentown as filed, climb and maintain two thousand two hundred,
    expect five thousand in ten, departure one one niner point two,
    squawk four zero zero three, {callsign}
Morristown Ground, {callsign}, with information alpha, request taxi
Runway zero five, hold short of runway zero five, {callsign}
Morristown Tower one one eight point one, {callsign}
Morristown Tower, {callsign}, ready for departure runway zero five
Runway zero five, cleared for takeoff, {callsign}
```

The clearance's assigned altitude, code, and departure frequency come from the actual response;
read those values back. These example values are illustrative. IFR clearances use CRAFT:
clearance limit, `as filed` route, initial altitude, expected altitude/time after departure,
departure frequency, and an octal squawk. Configure
a destination first. VFR departure requests to clearance delivery receive a squawk assignment.
Taxi instructions have an explicit runway and hold-short requirement. The current region model
has no taxiway graph, so taxi routing uses the runway/hold-short fallback and never invents named
taxiways. Clearance delivery initiates contact with ground after the readback; ground initiates the tower
transfer after the taxi readback. Tower assigns departure after takeoff above 60 m AGL, unless
closed traffic was requested. Radar hands the flight to the destination approach within 30 NM;
approach initiates the tower transfer for an inbound arrival within 10 NM. Published sector
changes can also trigger a transfer. Pilots read back and tune the assigned channel.

For arrivals, select the arrival airport with `atc.configure`, get ATIS, then call approach
`inbound for landing`. Selecting a different airport clears the previous facility and reported
information, while retaining outstanding readbacks. Read back its tower handoff, tune tower, and request `inbound straight in`,
`inbound left downwind`, `inbound right downwind`, `inbound left base`, or `inbound right base`.
Tower sequences the pilot behind a moving circuit aircraft when one exists. Reports of downwind,
base, or final lead to extend-downwind, landing, or go-around instructions according to the traffic
snapshot. An extension is released with a base turn when runway traffic clears. Requests for
closed traffic are supported. After landing, report `clear of the runway`, read back the ground
frequency, tune it, and report clear to ground for parking instructions. Ground issues a typed `parking` instruction, which requires a `taxi to parking` readback. The watching-mode pilot then makes a terrain-checked walking-pace parking move; the instruction does not imply surveyed taxiway or stand geometry. Automatic handoffs wait for the radio channel to become idle. Accepted runway assignments persist through position reports unless a new instruction changes them.

Enable up to three circuit aircraft with `traffic.set`. Controlled circuits reuse the existing
fixed-time kinematics and pilot/traffic spacing reactions. Traffic position reports address tower;
untowered CTAF call grading continues at untowered airports. Wake cautions accompany sequencing
behind traffic identified as Heavy; the standard three circuit aircraft are light types.

## Radar services and controlled airspace

Approach and departure accept `request flight following`, position reports such as
`departure, leaving 400, heading 060`, and `request bravo transition`.
Flight following assigns a deterministic four-digit octal code. Select it with the shared
transponder control (`avionics.set`, glass transponder setting). Radar position reports produce
radar contact, clock-position/range/altitude traffic advisories from the simulated traffic,
and heading/altitude assignments or controller-initiated sector handoffs. Once radar contact has
been heard, a frequency change between sectors of the same NASR radar facility continues that
identification without repeating “radar contact.” A new facility identifies the aircraft again.

NASR sector selection uses published bearing ranges, `FROM WEST`, and `3000 FT & BLW` / `ABV 3000 FT`
labels. A change to a different matching sector can trigger a frequency handoff. Destination
handoffs use `atc.configure.destination`, so a departure rehearsal can hand off from Allentown
to Newark or the reverse. Frequencies and provider identities come from the selected station. `two_way_providers` retains raw
NASR provider IDs (for example, `ABE`, `N90`, or `PHL`), which are distinct from airport ICAO IDs.

A Newark-provided Bravo transition remains outside until an acknowledged squawk assignment is
actually selected and two-way contact exists. The next request can issue the explicit words
`cleared into the Class Bravo`, together with heading and altitude assignments. Permission is
recorded only when that controller transmission is heard in full. A pilot saying those words
cannot create a clearance. Class C/D contact is established by a completed, heard, addressed
controller response from the controlling provider, including callsign-specific `say again`.
Unanswered, blocked, and untuned calls do not establish contact.

A heard IFR clearance carries implicit Class B authority along the cleared flight. It does not
require the VFR Bravo-transition words. VFR flights still require the explicit clearance.
The live [airspace query](airspace.md) grades airborne VFR entry into B without explicit clearance,
C without controlling two-way communications, and D without controlling contact. Grades are
recorded on changes of compliance. Newark Bravo permission is scoped to the New York Class B;
provider/contact matching for C/D uses imported facility calls and airspace names.

Load a CIFP approach using the existing [GPS procedure command](procedures.md), then request it:

```json
{"command":"avionics.set","input":{"setting":{"kind":"load_procedure","airport":"KABE","procedure":"I06"}}}
{"command":"comms.transmit","input":{"text":"{callsign}, with information alpha, request ILS runway zero six approach"}}
```

The approach must match the loaded airport, CIFP identity, approach family, and runway. Supported
families are ILS, LOC, VOR, NDB, and RNAV; other identifiers receive an explicit refusal. ATC does
not activate or fly the GPS procedure. Read back the approach clearance, then use the existing
procedure activation and autopilot controls as appropriate.

In clear conditions, a pilot can instead report `airport in sight, request visual approach runway
zero six`. The rules require an airborne aircraft, that report, a range within the weather's
visibility, and no cloud at the aircraft. The visual clearance is typed separately from a loaded
CIFP procedure, uses “cleared visual approach runway zero six,” and leads to the tower handoff
within 10 NM. The flown IFR preview uses a descent
assigned by approach, then a visual-approach clearance, tower contact, and a landing clearance.
The descent target is a training altitude above airport elevation; no minimum-vectoring-altitude
or obstacle-clearance database is implied.

## Readbacks, grammar, and compliance

Every typed `AtcInstruction` waits for a callsign and its required values. A correct readback
releases it; an incorrect or incomplete readback produces `Negative` and a correction. Silence
produces `Say again readback` after ten simulated seconds from the end of the heard instruction,
then at thirty-second intervals. Reminders wait while another COM call is in progress. Retuning,
radio power loss, range/terrain loss, or overlap during a call prevents acceptance. Outstanding
instructions cannot be bypassed by calling another facility.

The grammar accepts aircraft type plus full or abbreviated spoken registration, compact
registration, and November plus spoken registration. Numeric slots accept digits, spoken digits
(including `niner`, `tree`, and `fife`), hundreds, and thousands. Frequencies accept `123.45`,
`123450`, or `one two three point four five`; runways accept `06`, `zero six`, and `04L` / `zero four left`.
Named facility addresses are checked against the imported facility calls. Unknown requests receive
`say again request`; an absent identity receives a callsign challenge.
This is a bounded phrase grammar, not general natural-language understanding or speech recognition.

The monitor samples the same flight state recorded by the session. Heading tolerance is ±10°;
altitude tolerance is ±100 ft. Vector checks begin after 30 seconds; squawk and frequency checks
begin after ten seconds. Hold-short compliance uses an estimated protection rectangle 30 m outside
the runway pavement, since surveyed hold-short lines are absent. Takeoff clearance releases that
hold-short instruction only after a correct readback. Altitude checks allow a normal climb/descent toward a newly assigned altitude; a newer assignment
supersedes the older target. Frequency assignments retire after the assigned channel is selected.
Compliance delays start after the readback, and IFR altitude checks arm after actual departure.
Typed grades remain in the event stream; spoken instructor cues are plain English, queued at least
eight seconds apart, and prioritize runway/airspace safety over altitude, heading, and radio setup.
Grades are available in `atc.state.compliance`
and as `Meaning::Compliance` events beside the transmissions.

## Lost communications

`failure.set` accepts `{"failure":{"kind":"radio_failure"}}`. Both COMs stop transmitting and receiving;
intercom, the transcript, and the failure event remain available. `failure.clear` accepts
`{"target":"radio_failure"}`; clearing all failures also restores COM. The simulator failure state
is shared by ATC lost-comms rules and scenario emergency grading.
Radio loss does not apply aerodynamic forces; it is included in deterministic failure state and replay.
`weather.state.radio_failed` reports it alongside the existing systems information. The former
`radio` command spelling remains an alias for this single `radio_failure` kind.

At a configured towered field, automatic rules provide flashing green for return/taxi, steady red
for conflicting traffic, steady green on a clear low arrival, and flashing red to clear the runway
after touchdown. All six signals can be selected explicitly for training:

```json
{"command":"atc.light_gun","input":{"airport":"KABE","signal":"steady_green"}}
```

An explicit selection remains until changed or the failure is cleared. The state includes the
signal, ground/air meaning, NED position, estimate flag, and start time. Renderer hosts pass the
read-only state to `Scene::set_light_gun`. A world-projected lamp uses theme red/green/white,
fixed-flight-time flashes, terrain occlusion, visibility range, and hood/cloud suppression.
NASR supplies a facility/airport reference, so this is explicitly an estimated lamp location.

| Signal | In flight | On ground |
|---|---|---|
| Steady green | Cleared to land | Cleared for takeoff |
| Flashing green | Return for landing; await steady green | Cleared to taxi |
| Steady red | Give way and continue circling | Stop |
| Flashing red | Airport unsafe; do not land | Taxi clear of runway in use |
| Flashing white | No airborne meaning | Return to starting point |
| Alternating red/green | Exercise extreme caution | Exercise extreme caution |

## Commands, tests, and previews

`atc.state`, `atc.configure`, and `atc.light_gun` have generated input/output schemas and are
automatically exposed by the shared [MCP registry](mcp.md). Use `comms.transmit`, `comms.listen`,
`comms.tune`, `comms.facilities`, and `flight.events` for the radio and event stream. Calls never
open dialogs. Pending instructions, replies, compliance history, and event buffers are bounded.

The control unit tests include the complete KABE departure and arrival radio sequences, missing
and stale ATIS, wrong/missing/blocked readbacks, actual Class B/C/D position queries, explicit
Bravo permission, compliance checks, CIFP identity matching, controlled traffic, all six signals,
schemas, and hostile input. The unchanged simulation golden test remains authoritative for physics.

Generate original artifacts with the ignored `write_atc_previews` control test and
`render_atc_previews` UI test. The ignored `validate_generated_atc_previews` test audits saved tracks,
pavement stops, completed transmissions, wording, and grades.
The output directory is `out/atc/`:

- `index.html`: preview entry point, with filterable transcript pages;
- `kmmu-kabe.{html,md,json}`: KMMU departure to KABE arrival;
- `kabe-pattern.{html,md,json}`: KABE circuit sequencing with traffic;
- `hudson-bravo.{html,md,json}`: Hudson-adjacent Bravo request and clearance;
- `lost-comms.{html,md,json}`: lost communications and the light-gun catalogue;
- corresponding `*-state.json` files: typed final training state;
- corresponding `*-track.json` files: sampled continuous flight trajectories;
- `atc-panel.png`, `light-gun-world.png`, and `render-evidence.txt`: actual egui/wgpu captures.

All four transcripts now advance a single aircraft continuously with hold commands on elevations
resampled from the fetched New York and eastern Pennsylvania tiles. The KMMU–KABE generator starts
on the runway extension before the estimated protection boundary, takes off, climbs, flies the
route, descends, lands, and stops on runway 06. No position is replaced after the initial start.
There is no taxiway graph: the preview makes no claim to a named taxi route or runway exit.
The pattern ends with a practice go-around; the Hudson exercise climbs into Bravo only after its
clearance; the lost-comms exercise lands under steady green and demonstrates ground signals while
stopped. Each `*-track.json` records sampled flight time, position, velocity, altitude, and heading.
The regular unit tests keep small fixtures; the flown preview generator is deliberately ignored
in ordinary test runs. Actual egui/wgpu screenshots separately validate the projected lamp pixels.

NASR provider coordinates can locate an off-region provider airport rather than a remote antenna.
For approach/departure records explicitly marked estimated, a provider position more than 10 NM
from its served airport uses that served airport as an estimated coverage proxy. The source record,
radio call, and published channel are retained. This is not a surveyed transmitter location.

## Coverage limits

This is an offline training subset of FAA [JO 7110.65](https://www.faa.gov/air_traffic/publications/atpubs/atc_html/)
and the [AIM radio chapter](https://www.faa.gov/air_traffic/publications/atpubs/aim_html/chap4_section_2.html),
with light-gun meanings from AIM airport operations. It is not a certified training device or a
source of operational clearances, frequencies, routing, or obstacle clearance.

There is no taxiway graph, surveyed hold-short or tower geometry, controller staffing/hours model,
full airport surface simulation, certified separation/wake model, radar sensor/identification model,
minimum-vectoring-altitude database, IFR route validation, clearance amendment workflow, AVEF/MEA
lost-communications routing, or speech input. Pattern and runway occupancy use the existing light
circuit traffic phases. Radio failure retains previously received clearance/contact evidence;
it does not itself revoke a clearance. Squawk codes are deterministic local training assignments, not coordinated
with a national code allocation system. CRAFT `as filed` and vector altitude assignments are training
instructions and do not validate obstacles or IFR routing. Sector labels are descriptive NASR data,
not complete radar-sector polygons; only the documented label forms are interpreted. C/D facility
ownership uses name matching and is not a complete airspace-to-facility authority database. Bravo
training is currently scoped to Newark and the New York Class B. Explicit light-gun selections do
not enforce a pilot's compliance, and radio failure does not implement all regulatory lost-comms
procedures. Saved flight records retain the typed evidence; resuming ATC's runtime from an exported
record is not implemented.

See [training](training.md), [weather](weather.md), [procedures](procedures.md), and [crew](crew.md).

[IFR training](ifr-training.md) adds validated final intercepts, published missed instructions,
hold clearances with EFC, and instrument approach tower handoffs through the same readback queue.
