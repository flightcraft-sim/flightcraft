# Training: radio, emergencies, and instrument flying

Status: weather, IMC visuals, the hood, the [radio foundation](radio.md), a bounded subset of
[tower and approach training](atc.md), [scenarios](scenarios.md), and [emergencies](emergencies.md)
are implemented, alongside [IFR lesson files and instrument task grading](ifr-training.md) and the [exterior pre-flight walkaround](preflight.md).
Broader conversational instruction features below remain specifications.

FlightCraft is built to do what flight students actually need between lessons. In priority order:

1. **Radio work** at towered and untowered airports, so the first real call to a tower is not the
   first call ever.
2. **Emergencies:** practise the checklist, the decision, and the radio call, without the risk.
3. **Instrument flying:** view-limiting device time ("foggles") and real instrument meteorological
   conditions (IMC) with clouds, fog, ceilings, and breaking out at minimums.

The deterministic training rules and desktop recognition work offline and without an AI service
after the chosen Whisper model is installed. Browser recognition may require its provider's network
service. An AI, through the MCP server or the in-app link in `docs/crew.md`, makes it conversational and lets it invent scenarios, but the rules
decide clearances and grades.

## 1. The audio panel and the radios

- **Audio panel equipment:** each aircraft declares one, with panel buttons and commands. It selects
  the transmit radio (COM 1 or COM 2), the receive radios, NAV identification audio, the intercom,
  volume, and squelch.
- **Three channels, never mixed:**
  - **intercom:** you, the virtual pilot, the instructor, and your AI (see `docs/crew.md`);
  - **COM radios:** ATC, CTAF traffic, ATIS, and AWOS;
  - **NAV identification:** Morse identifiers from VORs, localizers, and NDBs.
- **Push-to-talk:** a key, a mouse button, or a yoke button through the hardware bindings
  (`docs/hardware-and-panels.md`). PTT keys the selected COM; a second binding keys the intercom.
- **Reception is physical:**
  - you hear a station only when a COM is tuned to its frequency, it is in range, and terrain does
    not block line of sight (on the ramp at 1N7 you do not hear New York Approach);
  - two stations on one frequency step on each other, and so do you and traffic.
- **Sound:**
  - COM audio gets a radio effect: a 300 to 3,000 Hz band, compression, light static, and a squelch
    tail. The intercom sounds like a headset.
  - Each speaker role has its own voice: tower, ground, approach, ATIS, each traffic aircraft, and
    the instructor.
- **Speech in:**
  - desktop uses local Whisper (offline and private);
  - the browser uses its speech API, with a notice that the browser may send audio to a server;
  - a constrained post-pass validates radio phraseology (callsigns, facilities, numbers, and
    requests); uncertain recognition asks the student to say again;
  - typing is always available instead of speaking.
- **Speech out:** the platform's built-in voices by default, plus an optional local neural voice on
  desktop.

## 2. Simulated ATC: rules, not AI

- **Facilities and frequencies** come from FAA data only: tower, ground, clearance delivery,
  approach and departure, CTAF, UNICOM, ATIS, and AWOS. Nothing about a real facility is invented.
- **Untowered airports** (for example 1N7, KMPO, and N05):
  - CTAF self-announce calls (taxi, departure, the pattern legs, and clear of the runway);
  - simulated traffic in the pattern that makes position calls, flies a real pattern, and reacts to
    yours;
  - the instructor or grader checks that your calls are made, timely, and correct.
- **Towered airports** (for example KMMU, KABE, KAVP, and KRDG):
  - ATIS and information letters;
  - ground: taxi instructions with a hold-short;
  - tower: takeoff and landing clearances, pattern entry instructions, "extend downwind",
    "number two following", and go-arounds;
  - approach and departure: flight following, squawk codes ("squawk ... and ident", "ident" and
    the IDENT press), traffic advisories, and handoffs.
- **Class B** (New York): "cleared into the Class Bravo" or "remain clear", with vectors and
  altitude assignments. Entering without the words is graded as a violation.
- **Readbacks:**
  - every instruction that needs a readback waits for one;
  - a wrong readback gets "negative" or "readback incorrect";
  - silence gets "say again" after a delay.
- **Compliance:** flying the instruction (heading, altitude, hold short, squawk, ident, frequency change)
  is checked against the record and graded.
- **Phraseology:** controller wording follows FAA Order JO 7110.65 and the AIM radio-technique
  chapter, so what you hear is what you will hear in the airplane.

## 3. Recorded events

Every transmission, cue, failure, and clearance is an event in the flight record, beside the
control inputs. Each event has:

- the time, the frequency or channel, and the speaker role;
- the text as spoken;
- a structured meaning, for example `{type: vector, heading: 270, altitude: 4000}`;
- its source: `rules`, `scenario`, `ai`, or `pilot`.

The debrief replays any moment ("approach said heading 270; you flew 250 for 40 seconds") and grades
against what was said, whoever said it. Audio stays outside the deterministic core; the events do
not change the golden trajectory.

## 4. Scenario scripts

A scenario is a file (`schema/scenario.json`), written by hand, by a lesson author, or by an AI
before the flight. The app runs it locally, so nothing waits on a model during the flight.

- **Triggers:**
  - position: distance to an airport or fix, on final within N miles, crossing a radial, entering
    airspace;
  - flight state: height above ground, phase (taxi, takeoff, climb, pattern leg, approach),
    airspeed, configuration;
  - the pilot: tuned a frequency, made a call, read back correctly or not, completed a checklist
    item;
  - time: since the start or since the previous step.
- **Actions:**
  - an ATC, traffic, or instructor transmission (typed meaning, rendered to phraseology by the app);
  - a failure, a weather change, or new traffic;
  - waiting for a readback or a call;
  - branching (for example, if the pilot goes around, run the go-around steps).
- **Validation before the flight:** every facility, frequency, runway, fix, and procedure must exist
  in FAA data; every trigger must be reachable on the planned route; and no instruction may put the
  aircraft below terrain or obstacle clearance or into airspace without a clearance. A script that
  fails is returned with reasons.
- **Example:** "on a 3-mile final for KABE 24, Allentown Tower clears you to land with the current
  wind; if there is no readback in 10 seconds, it says again".
- **AI playing ATC live:** an AI can also send single transmissions mid-flight
  (`comms.atc`), checked the same way. They are tagged `ai` and marked "simulated ATC" in the
  kneeboard transcript, so a training script is never mistaken for how a real facility works.
- Scenarios are shareable content, like lessons: "KMMU to KABE with a tower handoff" can be written
  once and flown by anyone.

## 5. Emergencies

- **Failures, already in the systems model:** engine (full and partial power), alternator, vacuum
  pump, pitot and static blockage, instrument and display failures, electrical buses, and fuel. Added:
  - fire (engine and electrical);
  - smoke in the cabin;
  - a door opening in flight;
  - carburettor icing and airframe icing;
  - flap asymmetry;
  - a stuck trim;
  - radio failure, with lost-communication procedures and light-gun signals at towered airports.
- **Triggers:** a failure can be set by the instructor, by a scenario, at random within a window
  ("sometime in the next 10 minutes"), or by the AI. The pilot is not told which.
- **What is graded:**
  - recognition time;
  - the memory items and checklist flow from the aircraft's own POH emergency section (aircraft
    packs carry it as data);
  - best-glide speed held within tolerance;
  - a landing site chosen within glide range (the app knows the terrain and fields);
  - the radio call: 7700, "Mayday" or "Pan-pan" on the frequency in use or 121.5;
  - the outcome.
- **The emergency debrief** shows the glide footprint at the moment of failure, the fields that were
  reachable, the one chosen, and the time spent on each step.

## 6. Instrument flying

[IFR lesson files, ACS grading, instructor controls, crew demonstrations, and debriefs](ifr-training.md).

Phase-one controls, presets, simulated practice totals, lighting fallbacks, and previews are
documented in [weather and simulated instrument practice](weather.md).

### View-limiting device ("foggles")

- A view-limiting overlay that hides the outside view above the instrument panel while keeping the
  panel visible, as a real hood does. It follows head movement in the cockpit view.
- The instructor (virtual, AI, or a person at the keyboard) removes it, typically near minimums:
  "look up". A scenario can do the same.
- Hood time is logged separately as simulated instrument time for the pilot's own records. It is not
  presented as loggable FAA time.

### Real instrument conditions

- **Weather model in the renderer:** cloud layers with bases, tops, and coverage; fog and haze with
  visibility in statute miles; precipitation that reduces visibility; and night.
- **IMC looks right:**
  - inside cloud the outside view is grey and the horizon is gone;
  - the cloud base is a real boundary, so on an approach you break out at the reported ceiling and
    see the approach lights and runway at the visibility in force;
  - runway lights and the approach light system are visible at their real intensities in low
    visibility.
- **Weather sources:** set by hand, by a scenario (for example a 600 ft overcast and 2 miles in
  mist at KABE), or from live METARs when the live weather option lands (`docs/efb-and-charts.md`).
- **Instrument training tasks:** precision and non-precision approaches from the CIFP data already in
  the GPS, holds, partial panel after a vacuum or display failure, unusual attitude recovery, and
  missed approaches. Each is graded against the instrument rating ACS tolerances.

### Ground procedures

Preflight and run-up take the longest to learn, so there are lessons for them: before start, start
and warm-up; run-up and before take-off, with seeded faults (a weak ignition lane, a failed fuel
pump, a low back-up battery, oil not yet warm) that the student must catch and stop for; and
shutdown and securing. The checklist panel ticks items from simulator state, shows the POH timers,
warns on limits and out-of-sequence actions, and the virtual pilot can show any item. The main
window switches to a layout with the checklist beside a small exterior view and the whole dash
below. See [normal procedures](normal-procedures.md).

## 7. Commands and MCP additions

- `comms.tune`, `comms.audio_panel`, `comms.transmit` (the pilot's call, as text), and
  `comms.listen` (the next transmission heard, with its meaning).
- `comms.atc`: an AI or script sends one transmission as a facility; validated before it is voiced.
- `scenario.validate`, `scenario.start`, `scenario.amend`, and `scenario.state`.
- `weather.set` and `weather.state` (layers, visibility, precipitation); `view.hood` on and off.
- Recorded events are readable through `flight.events` and appear in `lesson.debrief`.

Each command has input and output schemas from the start.

## 8. Phases

1. **Weather and IMC visuals, plus the hood (implemented; [details and limitations](weather.md)):** cloud layers, fog, visibility, breaking out at the
   ceiling, runway and approach lights, and `weather.set` and `view.hood`.
2. **Radios and untowered airports:** the audio panel, reception and line of sight, NAV Morse
   identifiers, CTAF calls with simulated pattern traffic, the recorded events, text input, and
   speech output with the radio effect.
3. **Scenarios and emergencies:** the scenario format and validator, the new failures, POH
   emergency checklists as aircraft data, and emergency grading with the glide-footprint debrief.
4. **Towered airports:** ATIS, ground, tower, approach and departure, readback checking, Class B
   clearances, and lost-communication procedures.
5. **Speech in, AI as ATC, and IFR tasks:** PTT capture, constrained normalization,
   ungraded recognition review, local Candle Whisper with pinned model delivery, and typed MCP
   controller instructions are implemented. See [speech verification and measurements](radio.md#speech-in).
   Graded instrument approaches, holds, and partial-panel tasks are covered by the
   [implemented IFR scope](ifr-training.md).

Phase 2 implementation and current coverage are described in [radios](radio.md): electrical audio
panel, typed calls and records, reception, deterministic CTAF traffic, captions, transcript, and
speech adapters. FAA COM frequencies come from the imported regional NASR data; weather observations
include current wind, visibility, cloud layers, pressure, temperature, and dew point. Browser speech
filtering and macOS distinct voices remain incomplete.

Scenario scripts, emergency effects, aircraft procedure data, and the emergency debrief are implemented;
see [scenarios](scenarios.md) and [emergencies](emergencies.md) for commands, validation, and limits.
Towered radio rules from section 2 are implemented as a bounded offline training subset; see
[tower and approach coverage, commands, previews, and limitations](atc.md). Instrument lesson coverage is documented in [IFR training](ifr-training.md).
