# The crew: the app flies, your AI flies, or someone teaches you to fly

Status: specification, not yet implemented.

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
virtual-pilot conversation, handover, speech recognition, and instructor lesson rules above remain
separate work.
