# The crew: the app flies, your AI flies, or someone teaches you to fly

Status: watching-mode implementation is available; the implemented scope and verification are recorded below. The remaining sections describe the wider crew design.

FlightCraft has two kinds of non-human pilot and three ways to share the cockpit with them. None of
them uses a chat box beside the window. Conversation happens the way it does in a real cockpit:
over the intercom, on the kneeboard, and by pointing at things.

## 1. Two pilots that are not you

1. **The virtual pilot (built in, deterministic, no AI service needed).**
   - It is a Rust module that flies the aircraft through the same commands a person uses: the
     controls, the switches, and the aircraft's own autopilot when it has one.
   - It flies whole procedures: before-takeoff checks, takeoff, climb, cruise to an airport, pattern
     entry, the pattern, approach, landing, and taxi off the runway. It also flies manoeuvres:
     steep turns, slow flight, stalls with recovery, ground-reference manoeuvres, and go-arounds.
   - It always works, offline and in the browser, with no account. "The app flies the plane for
     you" means this pilot.
2. **Your AI (a language model connected through the MCP server, or through the in-app link
   below).**
   - A model is too slow and too costly to hold a stick 120 times a second, so it never does. It
     decides what to do and says so; the virtual pilot does the flying. The model gives intents
     such as "fly to KABE, join a left downwind for runway 24, and land", and it talks to you.
   - It reads everything the MCP server exposes: state, systems, airspace, airports, avionics
     displays, and the flight record.
   - "Your AI flies the plane" means a model giving intents to the virtual pilot and talking to
     you, while you watch or fly.

Every decision the virtual pilot makes is visible: its current intent, its next step, and why
(for example "descending: 2 nm from the 45° entry, pattern altitude 1,400 ft"). The same text feeds
the intercom when someone is talking.

## 2. Three ways to share the cockpit

| Mode | Who flies | Who talks | What it feels like |
|---|---|---|---|
| **Watching** | the virtual pilot, directed by you or your AI | the pilot in command, to you, as to a passenger | a scenic flight with a friendly pilot; you ask for things |
| **Demonstrating** | the virtual pilot | the instructor, explaining each step as it flies | "watch me do it first" |
| **Instructing** | you | the instructor, watching you and coaching | a lesson with an instructor in the right seat |

You can move between them at any moment with a positive exchange of controls, as in a real
cockpit:

- "Your controls." "My controls." "Your controls."
- The handover is a command (`crew.handover`), a button, a shortcut, a yoke button, or a spoken
  phrase. Control changes only when both sides have confirmed.
- In instructing mode the instructor may take the controls on its own for safety (below 200 ft
  above ground with a sink rate it judges unsafe, a stall below 1,000 ft, or entering controlled
  airspace without a clearance). It says "My controls" first and tells you why afterwards.

## 3. Where conversation lives (not a side chat box)

1. **The intercom.** The main channel.
   - Push-to-talk (a key, a mouse button, or a yoke button through the hardware bindings) to speak.
     Speech recognition comes from the platform: the browser's speech API, or the operating
     system's. Typing works too, in a one-line field that appears only while push-to-talk is held
     or after pressing the talk key.
   - The crew answers in speech (platform text-to-speech, with voice choice) and in captions: a
     thin strip across the top of the view, like radio subtitles. Captions fade after a few
     seconds. Speech and captions can each be turned off.
   - The intercom is not ATC. A future radio channel with simulated ATC stays separate, so "what
     the instructor says" and "what the controller says" never mix.
2. **The kneeboard.** The memory of the conversation.
   - A clipboard on your knee in the cockpit view, or a slide-out panel in other views. It holds
     the briefing, the plan, a running transcript of the intercom, the instructor's notes, and the
     checklist in use.
   - You can scroll back through it, but it is not where you talk.
3. **Pointing.** Talk about things by showing them.
   - The crew can highlight an instrument (a soft ring around the airspeed indicator: "watch your
     airspeed"), a switch ("carb heat on"), a place in the world (a marker on the threshold: "aim
     here"), or a path (a ghost track of the downwind they want you to fly).
   - You can point too: hold the talk key and click an instrument or a point in the world, then
     ask "what's that?" or "can we go there?". The crew receives what you pointed at.
4. **Briefings and debriefs.**
   - Before a lesson or a scenic flight, a short briefing card: route, weather, objectives, and
     what the instructor will look for.
   - After landing, a debrief: a timeline of the flight with the moments that mattered (a
     ballooned flare, airspeed 8 kt fast on final, a good crosswind correction), each one playable
     from the recording, with the instructor's comment and the standard it was judged against.

All of these work with the deterministic virtual pilot alone. Without an AI service, the virtual
pilot speaks in fixed, well-written phrases, and the instructor still coaches and debriefs from
the rules in section 4. A language model makes the crew conversational: it can answer questions,
take free-form requests, and explain in context.

## 4. Instruction is graded against published standards

- **Standards:** lessons are built from the public FAA Airman Certification Standards and the
  Airplane Flying Handbook. Each task has the published tolerances, for example ±100 ft and
  ±10 kt in steep turns, and rollout within ±10° of the entry heading.
- **Rules, not a model, decide the grade.** Deterministic code checks the recorded flight against
  each tolerance, so grading is repeatable, testable, and the same with or without an AI service.
  A model may phrase the feedback, but it cannot change the grade.
- **Coaching cues** follow the same rules: early, short, and specific ("bank's going past 50",
  "a bit more right rudder"), at most one cue every few seconds, and none during critical moments
  unless they concern safety.
- **Lessons** are files (`schema/lesson.json`): objectives, the setup (region, airport, aircraft,
  weather, start), tasks, tolerances, cues, and a briefing. Contributors can write new lessons
  without code.
- **Progress:** each lesson keeps a local record of attempts and grades. It stays on the device
  unless the user exports it.

## 5. Watching mode should be fun

- **Ask for things:** "take me over the Delaware Water Gap", "let's see the Manhattan skyline at
  2,500 ft", "fly me to Pocono and land". The pilot plans the flight and checks airspace (it stays
  clear of the New York Class B unless the route is legal without a clearance). It says what it
  will do, then does it.
- **Cameras:** a director mode picks shots (cockpit, wing, chase, tower, and flyby). A model can
  choose them too ("show me the landing from the tower").
- **Narration:** what you are flying over, from the region's airports, airspace, and terrain, and
  later from map features. The narration is clearly marked as coming from data, and a model is
  told not to invent facts about real places.
- **Time:** sim rate up for long cruise legs, back to normal for anything interesting.

## 6. The AI link

- **External agents:** any MCP client (Claude Code, Claude Desktop, or others) can already
  connect, headless or to the running app. The crew commands in section 7 make it a full crew
  member.
- **In-app link:** the app can call a model API directly, using a key the user enters. The key is
  stored only on that device (the operating system's credential store on desktop, browser storage
  on web, with a warning) and is never logged or sent anywhere else. The default provider is
  Anthropic's Claude; the link has a small provider interface so others can be added.
- **No key, no problem:** everything in sections 2 to 5 works with the virtual pilot's fixed
  phrases. The model adds conversation.
- **Cost and latency control:**
  - The in-app link sends compact state summaries, not raw telemetry, and only on events (a phase
    change, a cue, or a question).
  - It caps calls per minute and shows what each session has used.
  - Responses that need speed (safety takeovers and cues) never wait for the model; the rules
    handle them.

## 7. Commands and MCP additions

- **`crew.mode`:** set watching, demonstrating, instructing, or solo (no crew).
- **`crew.handover`:** propose or confirm an exchange of controls (`to`: `pilot` or `crew`).
- **`crew.intent`:** give the virtual pilot an intent. Examples are fly to an airport or a
  coordinate, join the pattern, land on a runway, fly a manoeuvre, hold an altitude and heading,
  take off, or go around. It returns the plan, its legs, and any refusal with the reason (for
  example "route enters Class B").
- **`crew.say`:** speak to the pilot.
  - Fields: text, priority, an optional highlight target (an instrument id, a switch id, a world
    position, or a path), and an optional ghost path.
  - The app speaks it, captions it, adds it to the kneeboard, and draws the highlight.
- **`crew.listen`:** wait for the next thing the pilot says or points at, with a timeout. It
  returns the text and any pointed target.
- **`crew.state`:** the virtual pilot's intent, plan, next step, and reason; the current mode; who
  has the controls; and the active lesson task with its live tolerance status.
- **`lesson.list`, `lesson.start`, `lesson.debrief`:** lessons and their graded debriefs.

Each command has input and output schemas from the start, like every other command.

## 8. Phases

1. **The real autopilots** (a separate lane; see `docs/autopilot.md` when it lands).
2. **The virtual pilot:**
   - takeoff, climb, cruise to an airport, pattern, and landing in the Corvid and the Larrow;
   - `crew.intent` and `crew.state`;
   - a "fly me to …" watching mode with fixed phrases;
   - captions and the kneeboard.
3. **Intercom:** push-to-talk, platform speech in and out, `crew.say` with highlights and ghost
   paths, `crew.listen`, and pointing.
4. **Instruction:** lessons as files, rules-based grading against the ACS, coaching cues,
   debriefs on the recording, and first lessons (straight and level, climbs and descents, turns,
   steep turns, slow flight, power-off stalls, the traffic pattern, and normal landings).
5. **In-app AI link:** the user's own key, the provider interface, budgets, and conversational
   watching and instructing.

Each phase ships with docs, tests (the virtual pilot is tested by flying: it must land on the
runway within stated touchdown limits in calm and in gusty crosswind, from every runway in the
bundled regions), and an agent skill so an MCP client knows how to be a good crew member.

The [radio implementation](radio.md) supplies a shared caption strip, an intercom channel, typed
PTT entry, and a kneeboard transcript. Its role and event models are ready for crew messages;
virtual-pilot watching mode and handover now use those channels. Speech recognition and instructor lesson rules remain separate work.

[Emergency debriefs](emergencies.md) provide aircraft checklist timings, recognition delay, speed
tolerance totals, radio checks, outcomes, and frozen glide footprints through `scenario.state`.
These are inputs for future lesson grading; there is no general lesson engine yet.


## Implemented watching mode

The deterministic pilot lives in `control::crew`, runs at 10 Hz on the fixed simulation timeline,
and issues the same `SetControls` commands as a person. It does not reposition the aircraft or
use idealised script holds. The simulation and default golden trajectory remain independent of
crew presentation. Solo is the default, and resetting or replacing the aircraft clears the crew.

The command surface is `crew.mode`, `crew.handover`, `crew.intent`, and `crew.state`; see the
[wire examples and schemas](mcp.md#virtual-pilot-and-watching-mode). Positive handover requires a
proposal and a matching acknowledgement from the other participant. A single button press or
command never transfers control. Installed autopilots disconnect at a confirmed crew takeover;
this implementation flies by hand, including aircraft equipped with an autopilot.

**Fly me…** in the toolbar opens the passenger panel. Select an airport/runway, a scenic return
flight, or the current flight plan, review the briefing and route, then select **Your controls**
and **Confirm exchange**, then **Resume** if paused. The configurable `CrewHandover` keymap action (default F9) uses the same two steps. The
panel exposes pause/resume, a go-around request, and cruise-only sim rate. Phase transitions
restore normal rate. The director uses cockpit, wing, chase, tower, and flyby views. Tower frames the assigned runway and aircraft together on short final. Flyby plants an observer ahead along track for a close pass. The desktop selects the bundled Sling 2 IFR variant when installed. Student speeds and altitudes use knots and feet; engineering checks and leg samples are in the collapsed **Details** section.

Taxi exits are sampled at 0.5 m spacing for excessive grades, surface steps, and water. Without
a safe path, the pilot stops on the runway; it never drives blindly off an unverified edge.

The flight state machine includes before-takeoff checks, acceleration and rotation, best-rate
climb, navigation legs, descent, a 45-degree entry to left downwind, base, final, flare, rollout,
and a terrain-checked taxi-to-clear manoeuvre. An already aligned aircraft can join straight-in.
Takeoff-only ends in an altitude/heading hold; coordinate and flight-plan intents hold at their
end; orbit follows a checked circle; go-around rejoins the circuit. Guidance uses the aircraft's
rotation speed and Vy, derives approach speed as 1.3 Vso with landing flaps, and uses wind-corrected tracks. V-speeds are indicated; guidance converts them to true speed at the current density. Tailwheel
geometry selects gentler rollout braking and a nose-up ground attitude. Named runway
starts place all landing gear inside the pavement; default starts and airborne state are unchanged.
Control changes are recorded at guidance frequency, even inside a batched `sim.step`. The flare
accounts for measured vertical wind and local surface grade, and starts near the threshold.

The plan includes regional `plan.profile` samples at 100 m spacing. Missing coverage, insufficient
terrain clearance, and Class B penetration cause a refusal without replacing the active plan.
Cruise requires 1,000 ft of terrain clearance; within 6,500 m of the departure or arrival runway,
the profile checks terrain intersection instead. These corridors are not obstacle surveys or
published departure/arrival procedures. The current aircraft format lacks a demonstrated
crosswind field; the pilot uses an explicitly reported 5.2 m/s engineering crosswind envelope.
It does not claim this is an aircraft's demonstrated limit.

Intercom narration is a separate `crew` speaker with rules provenance, platform speech, captions,
and a kneeboard briefing/transcript. It is available while paused. The crew voice uses a separate
voice role and warm intercom rate/pitch; intercom bypasses native radio DSP. Voice quality and the
number of distinct voices depend on the platform's installed speech engine. Native speech needs
the desktop audio feature and a local speech engine; the browser needs speech synthesis support
and user activation. Speech failures remain visible, and captions continue to work.

Critical phase calls and descending 500/100-foot AGL crossings interrupt narration. There is no
invented minimums call: a general VFR flight has no configured instrument minimum. Ordinary phase
narration is limited to once per twelve simulated seconds, and GNIS landmark narration to at most once per two minutes. Only features explicitly ranked as notable in the regional `landmarks.json` are narrated: gaps, named mountains and ridges, major rivers, selected large lakes and reservoirs, and towns. Minor ponds remain reference data. Names, classes, states, positions, and short original geographic summaries come from that sidecar; relative clock/wing directions are computed from the current heading. The Water Gap route uses GNIS positions, including Mount Tammany, Delaware Water Gap, and Tannersville.

Fly me (`fly_me`) and the director camera (`director`) are independent screen pages and layout elements. The layout editor can place either in any window; `examples/layouts/watching.layout.json` combines them. Each director element owns its camera resources. Crew commands run on the session owner, with crew state included in browser/remote display snapshots.

The virtual pilot listens to ATIS on COM2, reports its information letter, and calls the imported NASR approach facility before arrival. It reads back every heard typed instruction through the shared renderer and waits for the ATC grader to accept it. Squawk changes affect the installed transponder; heading, altitude, runway, and pattern assignments change the real guidance. Runway changes rebuild the arrival geometry, including right patterns and base or straight-in entries. The pilot requests and reads back takeoff and landing clearances at towered fields, stops unsolicited base/final reports once cleared, reads back Tower's exit/ground instruction during rollout, and contacts Ground for taxi to parking after clearing. No command writes the ATC clearance flags.

A prospective airspace check holds outside Class B without applicable clearance and Class C/D without provider contact. A final without an accepted clearance for the actual runway causes a go-around. Departures remain braked until the matching clearance is accepted. Untowered inbound calls use actual distance/direction at about ten miles, full callsigns, and AIM-style CTAF bookends. A short local circuit calls from its actual distance rather than inventing a ten-mile position.

Cruising levels above 3,000 ft AGL follow 14 CFR 91.159 using magnetic course: odd thousands plus 500 eastbound, even plus 500 westbound. Lower cruise levels are rounded to hundreds of feet. Pattern altitude uses published MSL TPA when supplied, otherwise field elevation plus 1,000 ft rounded to the nearest 100. Spoken altitudes use the shared AIM number renderer.

### Verification and preview recipes

`crates/control/tests/crew.rs` checks two-party handover, rejected plans preserving state,
schemas, observational reads, exclusive controls, reset behaviour, fixed-step batching, default
physics equivalence, and a complete Corvid/Larrow pattern. The extended regional matrix and
preview flights require the released region files (`cargo xtask data fetch`). Run the named
ignored tests explicitly through the build queue:

```text
cargo test -p flightcraft-control --test crew regional_runway_acceptance_matrix -- --ignored --nocapture
cargo test -p flightcraft-control --test crew write_crew_flight_previews -- --ignored --nocapture
cargo test -p flightcraft-ui render_crew_previews -- --ignored --nocapture
```

The regional matrix attempts both ends of every runway at 1N7, KMPO, KABE, KAVP, and KMMU in the
Corvid, Larrow, and bundled Sling 2, with calm air and a deterministic light-turbulence crosswind. Touchdown limits
are within 600 m of the threshold (or the runway's length if shorter), within the smaller of
half the runway width and 10 m of centreline, and at most 2 m/s pre-contact sink. The tests also
require rollout and a stop without a crash, and towered flights must also park with no violations, unanswered instructions, repeat requests, or uncleared landing. Taxi clearance is recorded separately; when no
terrain-safe exit can be found, the pilot holds the brakes on the runway and reports why. Each result is measured from simulated
position and pre-contact velocity, with no resets during a flight.

Preview outputs under `out/crew/` are `water-gap` and `1n7-kabe` transcripts, WebVTT captions,
phase snapshots, and touchdown reports. GPU screenshots consume those recorded snapshots and
render the actual scene and passenger panel. See `out/crew/validation.md` for the checks and
remaining acceptance failures from this worktree.

### Remaining scope

The Hudson corridor preset explicitly refuses flight until a verified Class B exclusion route
and its special flight rules are available. The other presets are authored route points and
still pass the shared terrain/airspace checks. Taxi exits and short parking moves are terrain-checked estimates; there is no published taxiway graph or stand assignment. The Sling exterior is an original simplified silhouette. Traffic-pattern direction metadata, installed-autopilot coupling, validation of arbitrary aircraft beyond the three tested packs, instructional lessons, speech recognition,
pointing, `crew.say`/`crew.listen`, and model integration are not implemented here. Demonstrating
and instructing select presentation modes; they do not supply a lesson engine or safety takeover.

[Tower and approach training](atc.md) supplies deterministic ATC instructions, required readbacks,
and compliance events for later instructor/debrief integration. These rules do not implement the
virtual pilot, control handover, or ACS lessons specified here.
