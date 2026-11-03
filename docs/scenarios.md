# Scenario scripts

Scenarios are version 1 JSON files described by [the schema](../schema/scenario.json). The
[offline library](../scenarios/) contains seven original training exercises. Open **Scenarios**
in the toolbar, select the Eastern Pennsylvania region, position the aircraft for the exercise,
and choose **Validate** and **Start on current flight**. Starting a script preserves the aircraft,
position, weather, and failures. Resetting a flight clears scenario progress and its debrief.

The library includes engine failure after takeoff on 1N7 runway 07, engine failure between 1N7
and KMPO, electrical fire near KABE, vacuum failure on the KABE ILS 06 with the IFR weather preset,
a door opening after takeoff at KMMU, a three-mile-final landing clearance for KABE runway 24,
and carburettor icing while descending into KAVP. These are exercises, not operating instructions.
The approach exercise declares the published `KABE:I06` procedure; it does not activate GPS guidance.

Each document supplies an id, title, authorship flag, deterministic seed, planned route, referenced
procedures, and ordered steps. Each route point names an imported airport, fix, or navaid, gives
an MSL altitude in metres, and lists any airspace clearance assumptions by imported volume id.
The library's 2,200 m route altitudes describe transit; manually set up the final approach or
takeoff before starting its exercise. Scripts do not fly the route or grant clearances implicitly.

```json
{
  "version": 1,
  "id": "power-loss-practice",
  "title": "Seeded engine failure on the 1N7 to KMPO route",
  "ai_written": false,
  "seed": 3407,
  "route": [
    {"reference": "1N7", "altitude_msl_m": 2200, "clearances": []},
    {"reference": "KMPO", "altitude_msl_m": 2200, "clearances": []}
  ],
  "procedures": [],
  "steps": [{
    "id": "power-loss",
    "when": [{"kind": "random_window", "earliest_s": 30, "latest_s": 90}],
    "actions": [{"kind": "failure_set", "failure": {"kind": "engine"}}]
  }]
}
```

All conditions in a step must hold. Conditions include airport/fix distance in NM, runway final
or departure alignment, radial crossings in true degrees, entering an imported airspace volume,
AGL height, indicated airspeed, flight phase, flap configuration, tuned channel, completed pilot
call, readback correctness or absence, aircraft checklist completion, elapsed time, seeded random
time, and named flight events. Final requires a heading within 20 degrees of the runway, a
quarter-mile lateral corridor, an airborne aircraft, and positive distance from the threshold.
Flight phase uses measured ground state, height, speed, and vertical speed; it is a coarse training
classification. Radial crossings compare successive fixed-step positions, ignoring the reciprocal
bearing discontinuity.

Actions send typed facility transmissions, set or clear failures, replace validated weather,
start the existing untowered circuit traffic, deliver instructor intercom cues, wait for a
condition, branch forward to a named step, or emit a named flight event. Traffic spawning uses
the existing zero-to-three-aircraft CTAF model; towered traffic remains outside this module.
Named events (`scenario.signal`) are an integration hook for future controller events. The engine
contains no tower, approach, sequencing, or clearance policy.

Time uses the 120 Hz simulation clock. A random-window trigger samples its due time from the
scenario seed and step index with fixed integer mixing. Reads do not advance the engine. Steps
execute in file order, at most one per evaluation; fixed ticks and explicit event/checklist submissions
evaluate the engine, and branches can only jump forward. Waiting blocks
the next step. The call/readback window begins when the preceding step executed, so an old pilot
call cannot satisfy a new clearance. Matching normalizes case and punctuation and uses a declared whole-word
phrase; it does not parse full FAA phraseology. A missing readback requires no completed pilot
call in the window after the timeout. Use a branch for incorrect or missing responses.

| Command | Input / result |
|---|---|
| `scenario.validate` | `{scenario}` → `{valid, errors}`; collects and deduplicates errors |
| `scenario.start` | `{scenario}` → progress and debrief; validates before installation |
| `scenario.amend` | `{scenario}` → progress; atomically replaces only unexecuted content |
| `scenario.state` | `{}` → current step, waiting/completion, timestamped actions, and emergency data |
| `scenario.signal` | `{name}` → progress; supplies a named flight event |
| `scenario.instructor_failure` | `{failure, within_seconds, seed}` → progress; zero triggers immediately |
| `comms.atc` | `{facility, channel_khz, role, instruction}` → typed transmission with id and reception; one validated live transmission |

These commands are registered with input and output schemas and automatically exposed through
[MCP](mcp.md). No command opens a dialog. An amendment cannot change the id, seed, authorship, or
executed prefix. It retains the current wait, timestamps, events, and emergency debrief. A failed
validation changes nothing. Runtime action errors are retained on the corresponding scenario event.

Facility transmissions reference the exact FAA station id, role, and channel from `navdata.comms`.
Runtime-injected radio facilities cannot satisfy this check. Typed meanings include landing,
go-around, altitude at a named point, airspace clearance, readback request, and advisory text.
Landing and go-around require the matching airport's imported tower and runway. Altitude
instructions require finite altitude, at least 1,000 ft terrain clearance at the named point, and
a recorded clearance for B/C/D containment (or a declared route clearance during script validation).
The kneeboard and human transcript tag facility calls **Simulated ATC**; that tag is never spoken.
Facility speech addresses the pilot first, then uses the NASR radio call name and role, with spoken
runway and wind digits and “wind calm” at zero wind. Live calls carry source `ai`;
every action and transmission from an `ai_written` document carries `scenario (ai-written)`.
Ordinary scripts carry `scenario`. Nonspoken actions are recorded as state events in `flight.events`,
without alerting the pilot through speech. The kneeboard shows provenance beside each spoken event and hides internal state changes.
Scenario wording is isolated behind `scenario::radio::scenario_radio_text` for integration with
the shared ATC renderer; the scenario engine contains no wording templates.

Validation checks document limits, unique step ids, forward branch destinations, referenced
procedures, airports, runways, fixes, airspace, frequencies, weather, traffic installation, and
failure parameters. It samples route legs at up to 250 m intervals for terrain coverage,
1,000 ft clearance, and B/C/D clearance assumptions. Position triggers must intersect the planned
route corridor; radial crossings and airspace entry also receive geometric reachability checks.
The document is limited to 512 steps, 32 conditions and actions per step, 256 route points, and
64 procedures. Instructor scheduling holds at most 64 pending failures.

The route check is advisory and sampled: it cannot prove performance reachability for every
combination of phase, configuration, height, and pilot behavior. Terrain between samples and
obstacles absent from the region data are not covered. Clearance assumptions are author-supplied;
a separate controller rules engine must verify actual clearances and compliance. Advisory text rejects common control-instruction verbs; it and readback text are not a semantic
substitute for typed altitude or landing instructions. There is
no arbitrary command action or code execution. These exercises are not approved flight training.

See [emergency effects and grading](emergencies.md), [radio events](radio.md),
[training scope](training.md), and [aircraft pack data](aircraft-format.md).
