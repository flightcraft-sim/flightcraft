# Radios and untowered training

The session owns communications, recorded events, and deterministic circuit traffic. The flight
integrator owns none of them: speech, captions, tuning, and transmissions do not enter the trajectory
hash. The same commands serve the desktop, browser, scripts, hardware adapters, and MCP.

## Operate the radios

Open **Radios** from the toolbar. The configurable default shortcuts are **V** for the radio panel,
**T** for selected-COM PTT and typed entry, **Y** for intercom entry, and **K** for the kneeboard.
When speech input is disabled, a press opens the one-line typed entry; Enter transmits it.
With speech input enabled in Preferences, hold the action to record and release to recognize;
review the caption before transmitting. **Type a call** remains available in the radio window. The microphone bindings are named
`ComPtt` and `IntercomPtt` in keyboard and gamepad maps. `comms.audio_panel` also accepts `ptt` and
`intercom_ptt` booleans for external hardware. The typed transmission is the duration-bearing
carrier; holding an empty PTT entry does not transmit speech.

Select COM1 or COM2 MIC, independently select their receivers, enable NAV identification or intercom,
and adjust volume and squelch. Speech and captions have independent switches. The transcript always
records transmissions, including missed and blocked calls. Global audio mute suppresses speech
without suppressing captions, event recording, or grading. The radio panel shows active and standby
channels, a swap button, and 25/8.33 kHz spacing. The touch navigator operates the same COM1 tuning.

Channel designators are integer kHz. For 8.33 kHz spacing, labels ending .005, .010, and .015 select
three carriers in each 25 kHz block. .020, .045, .070, and .095 are invalid. .000 and .005 select the
same carrier, so they interfere with each other. Radio reception requires tuning, selected receive,
electrical power, positive volume, range, and line of sight for the whole transmission. Two audible
transmitters on the same carrier block both calls. Different carriers remain separate, even when
both receivers are selected. Squelch shortens the usable service radius by up to 25%.

Propagation samples terrain at approximately 100 m and includes a 4/3-Earth curvature bulge. Missing
terrain coverage fails reception. Antenna heights and service radii are engineering estimates, not
FAA antenna measurements. This is a geometric training model, without diffraction or RF link budgets.

## Aircraft installations

Panel files accept an `audio_panel` object:

```json
{"audio_panel":{"power_load":"audio_panel","com1_load":"com1","com2_load":"com2"}}
```

Each id must exist in the resolved aircraft's electrical loads. The equipment demo declares separate
1 A audio and 2 A COM loads, and places an `audio_panel` equipment face with MIC, RX, NAV ID, intercom,
volume, squelch, speech, and caption controls. All controls issue commands. Existing panel files
omit the object and share their existing `avionics` load; their electrical demand is unchanged.
Opening a declared breaker or losing the avionics bus silences that equipment. A paused observation
consumes no charge or radio time.

A navigator with `"audio_panel": true` on its equipment element hosts the panel's shared audio
installation. Its Home **Audio** tile provides COM1/COM2 MIC and receive selection, NAV ident,
intercom, volume, squelch, speech, and captions through the same `CommsAudioPanel` commands and
`CommsSnapshot` as the Radios window. Volume and squelch percentages and selections reflect
changes made by any host. The navigator uses its existing rectangle; separate AudioPanel faces
are suppressed. This opt-in defaults off. Audio and receiver power still follow
`panel.audio_panel`'s load references. See [equipment](equipment.md) for pack fields and validation.


## FAA station data

Communications facilities are `ComStation` records in the optional `Navdata.comms` array. Each keeps
its airport, facility kind, channel, geographic position, elevation, engineering antenna/range
limits, and source record. Supported kinds are CTAF, UNICOM, AWOS, ASOS, ATIS, tower, ground, approach,
departure, and clearance. The [tower and approach rules](atc.md) use this facility vocabulary for controlled-airport training.

The New York and eastern Pennsylvania regions bundle official NASR communications from the
2026-10-29 28-day CSV subscription, discovered through the FAA APRA `nasr/chart` endpoint.
`data.lock` pins the immutable archive URL, size, and SHA-256. `xtask comms-nasr` verifies that pin
before reading any table. It emits 226 New York and 224 eastern Pennsylvania station records.

```text
cargo xtask comms-nasr --archive <official-CSV.zip> --regions assets/terrain
```

Omit `--archive` to fetch the pinned HTTPS URL through the normal bounded transport and source
cache. Region builds also run the importer; offline region source directories supply `nasr.zip`.
Legacy offline fixtures without that file contain no COM catalogue. The older `comms-faa` command
still accepts normalized exports, but production ingestion uses the official layouts directly.

FRQ.csv is the channel authority. It can publish multiple services on a row, so `APCH/P DEP/P`
produces both roles. Class B/C, TRSA, VFR advisory, and STAR channels map to approach;
DP channels map to departure, and GCO channels map to clearance. Their exact published use
is retained, and GCO dialing is not simulated. AWOS.csv supplies surveyed weather-station coordinates and type metadata;
this cycle has **no frequency column in AWOS.csv**, so its channels are joined from FRQ rather than
invented. APT_BASE joins FAA/ICAO identifiers and reference elevations. ATC_BASE, ATC_ATIS, and
ATC_SVC retain calls, tower/ATIS/control hours, and services. Phones are never emitted. UHF, NAV,
and unsupported service labels are excluded from the COM model. Sectorization is preserved.

A station's `airport` is the airport served, while `metadata.provider` identifies the provider.
For example, 1N7 CTAF and UNICOM are 123.000; Newark approach/departure is 124.450 west and
123.775 east. Remote antenna coordinates are not published in these tables: the provider's airport
reference point is an explicitly marked estimate. Stored hours are descriptive; the simulator does
not yet evaluate local schedules or NOTAMs. Catalogues are cycle snapshots, never navigation advice.

Each region has `comms-manifest.json`, a source receipt, cycle, counts, and normalized navdata hash;
its `sources.json` retains the upstream archive receipt. `cargo xtask data publish` emits navigation
archives alongside terrain and charts, writes release manifests and `data.lock`, and seeds the
offline cache. It only prints upload commands. Review and upload the generated assets separately.

## Radio identity

Aircraft identity objects accept optional `registration` and `spoken_type` strings. Old packs still
load; their compatibility default is the first display-name word and training registration N371FC.
Declare both in new aircraft packs, for example:

```json
{"registration":"N371FC","spoken_type":"Sling"}
```

In **Radios**, enter a registration and spoken type, and choose **Set callsign**. Lowercase input
becomes uppercase. The choice is saved per aircraft variant in the existing desktop preferences or
browser storage, restored on launch and aircraft load, and painted at the pack's registration node
on both sides of the Corvid, Larrow, and installed Sling. Original procedural stroke-and-arc glyphs
preserve the pack's color, placement, and letter height at both detail levels; no font is required.
**Use pack** clears that variant's saved choice and restores its baked lettering and radio identity.

US civil N-numbers follow the FAA rules associated with 14 CFR 45.21–45.29: N plus one to five
characters, starting with 1–9, then digits and at most two trailing letters. I and O are excluded,
and letters cannot precede digits. N371FC and N12345 are valid. N0FC explains that a number can't
start with zero; N12IO explains that I and O aren't used; NFC1 explains that letters belong at the end.
Select **Not a US registration** for a prefix-dash registration such as C-GABC or G-ABCD, up to
12 characters using A–Z, 0–9, and one separating hyphen. Foreign registrations retain their nationality
prefix in speech, use the ICAO alphabet (including India and Oscar), and omit the hyphen in speech.
This validates the number's format; it does not check availability or establish real registration.

`comms.callsign` reads the identity or overrides either field. It accepts `not_us`, `persist`
(default `true`), `reset_to_pack`, and `abbreviate_after_contact`. For example:

```json
{"registration":"n371fc","spoken_type":"Sling","persist":true}
```

`persist:false` changes the current session without replacing the saved choice. Validation failures
return typed `CallsignError` reasons through `CommandError::Callsign` and leave the current identity
and preferences intact. MCP tool errors expose `error.kind: "callsign_validation"` and a typed
`error.reason` in `structuredContent`, including when bridged to the desktop. Headless clients can serialize `Session::callsign_preferences()` and restore
it with `restore_callsign_preferences`; the app stores it automatically on desktop and web.

A typed `{callsign}` placeholder expands to the current spoken form. Following AIM 4-2-4, the
aircraft type replaces November in a US call: “Sling Three Seven One Foxtrot Charlie.” The pilot
uses “Sling One Foxtrot Charlie” only after receiving a controller's abbreviated call on that carrier.
A pending, blocked, or unheard reply, a pilot's first transmission, traffic, or automated weather
does not enable abbreviation. Changing frequency starts a new contact.
`abbreviate_after_contact:false` retains full callsigns. The [tower and approach rules](atc.md)
initiate the abbreviated reply after hearing the first call. Traffic uses fictional civil GA registrations with
Cherokee, Cessna, and Skyhawk type names, phonetic letters, and no airline identifiers. GDL 90 sends
the registration in its eight-character field, while captions and speech use the spoken form.

## Pattern traffic and call checks

`traffic.set` starts zero to three circuit aircraft on the selected real runway geometry. It
requires an imported CTAF or tower. Controlled circuits address tower and use the ATC rules;
untowered circuits retain the CTAF call checks. The circuit uses conventional
left traffic unless `right_pattern` is explicitly selected, a nominal 1,000 ft AGL circuit altitude,
1 NM lateral spacing, and 1.5 NM final/upwind extensions. These dimensions and handedness are
training defaults; the current airport catalogue does not publish local traffic-pattern directions
or altitudes. Confirm local information before selecting a right circuit.

Traffic follows deterministic kinematic circuit segments, climbs on departure, descends on final,
and queues leg-change calls on the selected CTAF or tower channel, listening for a clear channel before speaking. Aircraft are distributed around the circuit and slow behind
traffic or an ownship ahead on the same leg. Every update uses the same preceding traffic snapshot.
Traffic is a training target, with sharp segment transitions and continuous circuits, rather than a
second six-degree-of-freedom aircraft or a collision-avoidance controller. Traffic does not modify
ownship forces. The shared glass/touch map draws targets with relative altitude; desktop GDL 90
emits each target's stable synthetic address and callsign through the existing sender.

Pilot typed calls are checked against the airport name or identifier, the runway (digits or spoken
digits), and the aircraft's measured circuit phase. Taxi, departing, crosswind, downwind, base, final,
and clear calls are recognized. Correct calls also require the CTAF carrier and an unblocked complete
transmission. A queued grade is provisional until its transmission ends. A missing call generates
an instructor/intercom cue after 20 seconds in a phase, or on leaving a phase occupied for more than
eight seconds. The kneeboard shows the final grade. Geography, heading, ground state, and speed drive
phase classification; ambiguous positions outside the circuit have no expected phase. Grading is
active while a traffic training circuit is configured.

AWOS/ASOS/ATIS use their imported channel and the active simulation weather. Reports speak magnetic
wind, visibility in statute miles, obscuration/precipitation, every sky layer, the lowest broken or
overcast ceiling, temperature, dewpoint, and a four-digit altimeter setting. Weather layers are MSL;
the report subtracts the station elevation to announce AGL hundreds of feet. Clear sky has no ceiling.
There is no fabricated real-world observation time: these are simulation observations.

`weather.set` also accepts `temperature_c`, `dewpoint_c`, and `sea_level_pressure_pa`. Temperature and
dewpoint are sea-level references; station reports use 6.5 and 2 C/km lapse approximations, clipping
dewpoint to temperature. Defaults are 15 C, 5 C, and 101325 Pa. Changes to QNH affect the pressure
ports, so setting the reported pressure restores the indicated altitude. Airframe aerodynamic
density still uses ISA; this is not a complete nonstandard-atmosphere or moisture model.

## Recorded events and audio

Every transmission has an id, simulated time, duration, channel, frequency, speaker role, text,
structured meaning, source (`rules`, `scenario`, `ai`, or `pilot`), reception, reason, and optional
call grade. Meanings include position, weather, NAV ident, instructor cue, and text. `flight.events`
pages these beside timestamped changes to control inputs. `FlightRecord::from_json` provides a
bounded version-1 debrief import (128 MiB maximum), with the format in `schema/flight-record.json`.
Loading a debrief does not execute events or replace live flight dynamics. Live records are capped
at 20,000 events and 100,000 control changes, with visible overflow counters.

The application automatically checkpoints each flight every five wall-clock seconds and before a
reset or aircraft change; normal application save/close also checkpoints. Desktop records live in
the platform app-data `flightcraft/flights` directory, one JSON file per flight, with synced writes,
a complete prior-file backup during replacement, and crash recovery from that backup. Browser
records use the `flightcraft-flights` IndexedDB database, separate record and summary stores in one
transaction. Transaction completion determines success; quota/permission errors remain visible.
Abrupt browser termination can lose work since the last completed five-second checkpoint. Browser
records belong to the site origin and are subject to browser data clearing and storage eviction;
JSON export provides a separate copy.

Open **Saved flights / debrief** in Radios to list records, load their transcript, and export JSON.
Headless clients use `flight.save`, `flight.records`, `flight.load`, and `flight.export`, without
modal dialogs. `flight.records` with `refresh:true` starts listing; `{}` polls status. Browser
save/list/load return asynchronous status (`busy`, `pending_operations`, and `error`); poll until
complete. `flight.export` returns current JSON, or a loaded record selected by `id`. The app exports
a new desktop file or requests a browser download. Saving and exporting never change the physics hash.

`comms.listen` consumes completed heard transmissions without advancing time. An empty result means
step the paused session or wait for running flight, then poll again. The heard queue is bounded to
512 entries; the full event record is the authoritative transcript. Captions last seven simulated
seconds after completion, so pausing also pauses the caption. Missed or stepped-on words remain in
the debrief record, without being voiced as successfully heard audio.

Desktop speech uses an original Rust adapter to local operating-system tools: `say` on macOS,
System.Speech through PowerShell on Windows, and espeak-ng/espeak on Linux. Linux needs one of those
programs installed separately. No speech engine code or voice asset is bundled or linked. Desktop
synthesis runs on a bounded worker, with a timeout and bounded PCM import, and plays through CPAL.
Intercom bypasses the radio effect. This implementation does not add a third-party cross-platform
TTS crate, because none was available offline under a permissive licence.

### Voices

Every spoken event gets a `VoiceProfile` from `flightcraft_control::speaker_voice` (gender, timbre
variant, pitch, words per minute, and a radio character). It is a pure function of the speaker, so it
never changes during a flight, and the user's own transmissions are never re-voiced.

- **Simulated traffic** ([busy-airspace traffic](traffic.md)): a pilot's profile is one of eight pilot
  palette entries (`call.voice`) shifted by a per-aircraft pitch and rate step (`call.aircraft`), so
  two aircraft on the same base voice still differ; a controller's is one of eight controller
  entries chosen by facility (`call.voice`): Tower, Ground, Departure, Approach (high and low sectors
  differ), Clearance, ATIS, AWOS, CTAF never share a voice at one airport. Controllers speak faster
  and in a different palette than pilots.
- **Other speakers**: scenario ATC uses the voice of its facility kind; circuit traffic hashes its
  callsign; the instructor and crew have fixed voices.
- **Linux** passes the profile to espeak as `-v en-us+m1..m7/f1..f5`, `-p` pitch, and `-s` rate.
  **macOS** picks an installed stock voice of the right gender (from `say -v ?`) and sets `-r`.
  **Windows** picks an installed voice of the right gender by timbre number and sets `Rate`. Pitch is
  not adjustable through `say` or System.Speech, so on those two platforms voices differ by voice,
  gender and rate.
- **Browser**: `speechSynthesis` voices are not labelled by gender, so the profile is spread over the
  installed English voices and expressed through `pitch` and `rate`.

### Radio character

Every COM transmission (not intercom) passes through `radio_speech::dsp`, original DSP shared by
desktop and browser:

- a **squelch break** (45 ms carrier click and noise burst) before the voice and a 120 ms **squelch
  tail** with a closing click after it;
- a 300 Hz high cut and two low-pass sections near 3 kHz (each radio's edges, drive and hiss differ by
  voice) with soft compression, so every voice sounds like a different radio;
- a noise floor that rises with the square of the transmitter's distance as a fraction of its service
  range (`Session::transmission_distance_fraction`: the station's range for a controller, the 40 NM pilot
  range for traffic), crackling in bursts past half range, while the voice weakens up to 45%;
- a **heterodyne squeal** (two tones a few tens of hertz apart over garbled static, capped at four
  seconds) for a call that was **blocked** by overlapping carriers. When you key the mic over a
  traffic or ATC call, you hear the squeal instead of either voice; your own call is not voiced.
  The traffic engine sees the block and repeats the call after "Blocked, I say again" (controller) or
  "Blocked, say again" (pilot); see [traffic](traffic.md).

Volume follows the COM volume on the audio panel and the global gain; speech is silent when the audio
panel's speech switch is off, the radios are unpowered, or the receiver is not selected. Squelch
shortens service range as before.

### Speech duration and pacing

Radio speech is heard **as it is spoken**: a received call's audio (squelch break, then the voice)
starts when the transmission starts, while its event is still `Pending` and your receiver can hear it,
and ends with the carrier. It does not wait for `Heard`. The UI asks `Session::on_air(id)` each frame:
`Audible` starts the call, `Blocked` plays the squeal, `Unheard` (wrong frequency, out of range,
receiver off or retuned, speech switch off) never plays. Calls you cannot hear are not queued and never
voiced late. The user's own transmissions are not voiced.

`speech_duration_s(text, wpm)` is the deterministic estimate (0.2 s, plus words at the voice's rate,
plus 0.08 s per comma and 0.2 s per sentence end; 1 to 45 s). Simulated-traffic transmissions use it
with their voice's rate for both the recorded `duration_s` and the scheduler. Audio and carrier
coincide, so the fleet's normal 1.5 to 3.5 s gap between calls on a frequency is all the pacing
needed. The desktop renderer synthesizes at the same rate and, if the engine runs over the call's slot,
renders again faster (up to 1.8x) and, as a last resort, fades out at the slot's end, so audio never
runs past the carrier. Two receivers on different frequencies overlap in time and are mixed. Calls from
the older CTAF circuit traffic and ATC rules keep their previous 2.7 words-a-second estimate.

If you key the mic over a call that is being voiced, the call becomes `Blocked`: the voice is cut at
once and the heterodyne squeal plays for the rest of that call's carrier time.

Only that call is cut. The planner emits `Stop(Some(event id))` for the blocked call and leaves the
others alone, so a call on the other receiver keeps playing. (`Stop(None)` silences everything and is
used when the flight changes, speech goes off, or a critical crew call interrupts.) On desktop each
call has its own track in the mixer, so the cut zeroes only that call's remaining samples and drops
any render of it still in flight. In the browser, `speechSynthesis` cannot cancel one utterance, so
the cut cancels the speech queue and re-issues every other live call with the words it has left,
estimated from elapsed time at the utterance's rate; that restart can repeat or skip a word, and a
call still waiting in the browser's queue is estimated as if already speaking. The WebAudio radio bed
of the blocked call is stopped exactly, per event.

The browser's `speak(utterance)` API routes speech to the browser/operating-system audio sink and
exposes neither PCM samples, an `AudioNode`, an `HTMLMediaElement`, nor a `MediaStream`. The browser
voice is therefore unfiltered and cannot be time-fitted: it starts, with the WebAudio radio bed (squelch
break, noise rising with distance, tail, sized to the call's duration), when the event starts, is
cancelled when the carrier's time is up or the call is blocked, and a blocked call plays the same
squeal. Applying the band-pass to the voice would require a speech engine that returns PCM (for example a
separately licensed, bundled WASM voice). None is available in this repository. Browser voice
availability and audio activation depend on the platform.

Offline checks: `cargo test -p flightcraft-ui --features desktop-audio --lib radio_speech` runs the DSP
tests (band limit, squelch lead and tail, noise growing with distance, squeal determinism), the
planner tests (audio starts with the event, fits the carrier, is cut by a block, never plays when
unheard) and, where espeak is installed, renders two different voices through the engine and the radio
and checks the audio fits the call's slot.

NAV Morse is original 1020 Hz tone synthesis on both
platforms. Tuned usable VOR/localizer receivers supply identifiers; optional `adf_khz` in `comms.tune`
selects an NDB from FAA navaid data for identification. NDB reception currently uses the same
geometric propagation approximation, without groundwave/night effects. Captions and transcripts work
with all sound output disabled or unavailable.

## Verification and previews

Radio tests cover channel labels, tuning, range, power, terrain and curvature blocking, overlapping
transmissions, CTAF call checks, weather, traffic separation and batching, record round trips,
command schemas, invalid input, and unchanged state hashes. Existing golden trajectory tests remain
unchanged. DSP tests check finite samples, frequency attenuation, and Morse timing. Platform audio
adapters require device/platform smoke tests beyond the automated suite.

Generate actual egui previews with:

```text
cargo test -p flightcraft-ui render_radio_previews -- --ignored
```

Use the build queue. Outputs are `out/radio/equipment-audio-panel.png`,
`out/radio/radio-controls.png`, `out/radio/pattern-captions-kneeboard.png`, `out/radio/full-pattern.txt`, and
`out/radio/flight-record.json`. The pattern preview uses real FAA 1N7 runway geometry, NASR 2026-10-29
COM frequencies and original terrain, staged ownship replay samples, and live deterministic
traffic. It is not a claim of a flown full-pattern acceptance test or a verification of present-day operational information.

See [training](training.md), [crew and captions](crew.md), [equipment](equipment.md),
[systems power](systems.md), [hardware bindings](hardware-and-panels.md), and [MCP](mcp.md).

The optional `render_radio_audio_preview` test (with the `desktop-audio` feature) writes
`out/radio/traffic-radio.wav` and `out/radio/nav-ident.wav` using the installed speech tool and
original DSP/tone synthesis.

The IFR observation preview is `out/radio/awos-ifr.png` with its text and optional WAV. It uses
Sky Manor's real 121.725 AWOS channel, because the source catalogue has no AWOS transmitter at 1N7.

Set `FLIGHTCRAFT_SLING_PACK` to a readable Sling pack directory and run the ignored UI test
`render_sling_navigator_audio_preview` through the build queue to write
`out/navaudio/sling-audio-panel.png`, `sling-audio-navigator.png`, matching Home views, and
`preview.txt`. The test opts into hosting in memory and verifies that the source pack files
remain unchanged. It adds no separate audio face. These CPU captures omit GPU synthetic vision;
station reception and platform speech still require separate verification.

A browser archive smoke harness is `flightcraft-control` example `flight_archive_browser_smoke`
(cdylib for wasm). Its exports exercise committed save, listing and loading from a fresh session,
JSON export, and a previously saved record after restarting the browser. Browser execution requires
a host that permits Chromium IPC; compilation alone does not verify IndexedDB runtime behavior.

[Scenario scripts](scenarios.md) now send typed facility transmissions and record nonspoken actions.
`comms.atc` validates one live facility transmission and marks it as simulated ATC; it supplies no
controller rules. AI-authored script events use source `scenario (ai-written)`.

Scenario ATC speech addresses the pilot by spoken callsign, followed by the facility's NASR radio
call name and role. Runway digits and wind use the radio number helpers; zero wind is “wind calm.”
**Simulated ATC** is a kneeboard/caption/transcript tag, never part of synthesized speech. Internal
scenario state changes remain in the structured flight record and do not appear as instructor speech.
Scenario transmissions and `comms.atc` use the shared [phraseology renderer](phraseology.md).
[Tower and approach training](atc.md) now builds on this radio service: ATIS information letters,
clearance delivery, ground/tower/radar instructions, readbacks, compliance grades, controlled
circuit traffic, and lost-communications light guns share the same reception checks and flight record.

## Speech in

The first keyboard/yoke PTT gesture offers speech setup; the radio window also has **Set up speech
input**. This offer does not enable capture or start a download. Speech input is opt-in under
**Preferences → Microphone**. Select the native input device,
choose tiny.en (default) or base.en, and download its weights explicitly. The microphone level
meter runs during PTT; enabling intercom VOX explicitly keeps it open for intercom commands.
Keyboard and yoke bindings use the existing `ComPtt` and `IntercomPtt` actions. The radio window
also has hold-to-talk COM and intercom buttons. COM1/COM2 MIC selection determines transmit
routing. Release PTT to stop capture and recognize; losing window focus discards captured audio.
Calls are limited to 30 seconds. Selecting a microphone does not itself open it.

The caption shows **You said** before anything is graded, and an ungraded recognition event is
already in the transcript. Choose **Transmit recognized call** after checking it, or retry by
holding PTT. The post-pass uses the shared bounded grammar, pack callsign/full abbreviation,
region NASR facility names, and the current ATC readback vocabulary. It accepts exact digit
representations (`371`, `3, 7, 1`, or spoken words) and exact name/phrase spacing variants. Fuzzy matching only repairs
unique proper-name tokens, words from a heard pending readback, or position-report words after an
accepted closed-traffic clearance one edit away. Accepted closed traffic is included in `atc.state`
and screen snapshots, so browser copies use the same expected call set as the owner. Missing words are
never inserted, and numeric words are excluded from fuzzy matching. It never replaces a number,
runway, squawk, negation, or instruction with an expected answer. Valid but incorrect readbacks remain incorrect. Unknown
phrasing or confidence below 0.65 asks the student to **Say again**, with no grading. This is a
recognizer score threshold, not a calibrated probability of a correct call.

The command path is `comms.recognize` (printable text, confidence, intercom selection, and an optional
client correlation ID for browser transports; each UI window uses a distinct seed within JavaScript’s
exact integer range), then
`comms.confirm_speech` with its draft ID (`comms.discard_speech` discards a retry), then the existing typed `comms.transmit`. Programmatic
calls never ask for microphone permission or open dialogs. A changed COM selection/frequency
invalidates confirmation. Recognition previews do not trigger scenarios or enter the heard-radio
queue. Raw audio is held in memory and is not saved to the flight record or uploaded on desktop.

Intercom recognizes **your controls**, **my controls**, **I have the controls**, and **go around**.
The existing positive handover still requires the other participant's acknowledgment; one voice
utterance cannot impersonate both participants. Go-around uses the existing validated crew intent.
VOX pauses capture while a draft awaits review; confirm it or choose Retry to rearm.
These crew actions never come from COM. An MCP instructor uses `crew.say` for an AI-tagged,
spoken intercom cue; it does not change ownership or supply a clearance. Typing remains available with **Type a call**.

### Model delivery and verification

The native adapter is CPU-only Candle Whisper (`candle-core`, `candle-nn`, and
`candle-transformers`, Apache-2.0/MIT) behind the `whisper` feature, with a Rust tokenizer. Weights
are separate OpenAI Whisper MIT assets. `assets/speech/models.json` is the only download authority;
`tools/pin-speech-models.py` resolves immutable Hugging Face revisions and hashes artifacts for
maintainer review. The app stores verified artifacts under the user data directory's `speech/`
folder. Downloads run on a bounded worker, resume `.partial` files with HTTP Range, check the
response's range and total size, and verify SHA-256 before an atomic rename. Offline/interrupted
requests retain progress and report a retryable error. No model is fetched in ordinary tests.

The checked-in manifest pins OpenAI's tiny.en revision
`87c7102498dcde7456f24cfd30239ca606ed9063` and base.en revision
`911407f4214e0e1d82085af863093ec0b66f9cd6`. The complete downloads are **153.5 MB** and
**292.8 MB**, respectively (float32 weights, tokenizer, and configuration). These are separate
from the desktop executable. `FLIGHTCRAFT_WHISPER_DIR=/path/to/whisper` can point to a read-only
cache containing `tiny.en/` and `base.en/`; all pins are still verified before loading. Ordinary
recognition stays offline. The downloader itself requires a network connection.

Candle 0.11.1 and tokenizers (0.21 in the adapter, 0.23 through Candle) use CPU Rust kernels and
`fancy-regex`, with no CUDA, Metal, MKL, or C++ tokenizer feature. `tools/check-speech-licenses.py`
audits registry license fields from Cargo metadata against `deny.toml`; CI retains the complete
cargo-deny license-text check.
The original Rust frontend uses a mixed-radix FFT with centered periodic-Hann windows,
single-sided spectral power, and area-normalized Slaney mel bands. Inference uses a dedicated
Rayon pool capped at the available logical CPUs minus two (at least one worker); the reservation
leaves capacity for the simulation and UI. `RAYON_NUM_THREADS` can request a lower cap. The model
starts loading on PTT press, while the student is speaking, and stays cached for later calls.
Release builds compile Candle, GEMM, and tokenizers at opt-level 3. The development dependency
override is opt-level 2 and does not apply to the reported release measurements.

Greedy decoding uses self-attention KV caches, cached cross-attention projections, no temperature
fallback, and a 64-token limit with end-of-text stopping. Unterminated calls ask the student to
say again. The encoder processes a shortened audio context and slices its sinusoidal positional
embedding to match. The desktop diagnostic can compare actual duration (rounded up to a whole
second), a 10-second minimum, and the original 30-second padding.

Previous-context prompt tokens come from live state: the callsign and its spoken form, tuned
NASR facility names, ATIS letter, runway, and instructions actually heard and awaiting readback.
This is decoder bias; reference WAV text never enters recognition. The constrained post-pass
continues to preserve numeric mistakes and never inserts expected clearance values. Results
below 0.65 confidence or outside the grammar ask the student to say again before any grading.

For the browser, this implementation chooses **browser speech recognition**, avoiding a model
payload and main-thread CPU inference. Preferences explicitly warns that the browser provider may
receive audio and require a network connection. It requests `getUserMedia` only after a user
gesture and uses the browser's selected default microphone; device selection is in browser site
settings. PTT release stops recognition and microphone tracks. Unsupported browsers and denied
permissions report errors and retain typed input. VOX is intercom only. Browser latency is shown
for each final recognition result; it has **not been measured in this sandbox**, which has no
interactive browser microphone. No claim is made that browser recognition works offline or that
Candle wasm fits a measured size/time budget.

### Speech tests, measurements, and previews

There are 84 normalizer table cases, including malformed slots and unrelated speech, plus stateful
checks for spelling across pattern-word boundaries. A repair requires an expected leg and preserves
the spoken direction, numeric slots, and negation. Sixteen original WAVs in `tests/fixtures/speech/`
use installed Linux platform TTS: ten eSpeak NG radio calls, three Flite RMS retries, a separate
4.827-second eSpeak latency call, and two negative clips. The calls now use **N371FC**, “Sling Three
Seven One Foxtrot Charlie”; the abbreviation is “Sling One Foxtrot Charlie.” All clips were regenerated
for this registration, with SHA-256 receipts. No third-party recordings or speech-engine assets are bundled.

The default deterministic test uses a labeled fixture recognizer over decoded PCM, then the actual
grammar, review commands, MCP boundary, FAA facility validation, readback monitor, and transcript.
It does not measure Whisper accuracy and never downloads a model. With `whisper` enabled,
`real_whisper_wav_pipeline_and_latency` runs real tiny.en only when `FLIGHTCRAFT_WHISPER_DIR` is set.
It feeds every pilot attempt through recognition, review, and MCP `tools/call`, and checks that a
wrong runway survives the prompt and a wrong callsign asks the student to retry. CI can cache tiny.en
and run this single gated pipeline. Both-model diagnostics remain explicitly ignored/manual.

On an AMD Ryzen 7 3700X (eight physical cores, 16 logical CPUs), the **actual desktop release
executable**, float32 weights, greedy cached decoder, 14 inference workers, and a 10-second minimum
context produced these release-to-typed-call measurements on the **4.827-second N371FC recording**:

| Model | First load | Release to typed call, three runs |
|---|---:|---|
| tiny.en | 818 ms | 774 ms, 759 ms, 1,082 ms |
| base.en | 1,444 ms | 1,720 ms, 1,409 ms, 1,511 ms |

Mean stage times over those three runs:

| Model | Resample | Mel FFT | Encoder | Decoder | Normalize/draft | Total |
|---|---:|---:|---:|---:|---:|---:|
| tiny.en | 0.8 ms | 30.8 ms | 340.5 ms | 487.9 ms | 7.9 ms | 871.8 ms |
| base.en | 0.9 ms | 28.3 ms | 634.2 ms | 876.6 ms | 4.5 ms | 1,546.7 ms |

Total also includes prompt construction/tokenization, pool dispatch, and small bookkeeping costs.
First load includes pin/hash verification, configuration, tokenizer, weights, decoder, frontend,
and worker-pool initialization, with a warm OS file cache. The app prepares the model on PTT press,
while the student speaks. Per-call timing excludes model load, WAV disk I/O, UI frame scheduling,
and user review. No quantized model is needed to meet the measured 1.5-second/3-second targets.
These results describe this machine and recording, not a latency guarantee on other hardware.

The Rust 1.98.0 / Candle 0.11.1 upgrade was rechecked on the same CPU in release mode with
the cached greedy decoder, 14 workers, a 10-second minimum context, and the heard landing
clearance in the live-state prompt. The 4.827-second latency clip took **743, 755, and 760 ms**
with tiny.en (419 ms first load), and **1,268, 1,301, and 1,153 ms** with base.en (677 ms first
load). These diagnostic times cover recognition; the desktop release-to-typed measurements
above also include prompt construction and normalization. The gated real tiny.en pipeline
again accepted all nine first attempts, used zero retries, and passed three readbacks, with
one raw word error in 122 words. Wrong-runway and wrong-callsign audio checks passed too.

Profiling the original desktop path with four threads on the former 4.930-second N1234B recording
showed tiny.en spending **0.9 / 75.6 / 1,785 / 8,811 ms** in resampling, mel, encoder, and decoder,
respectively (**10,675 ms total**); base.en spent **0.7 / 66.8 / 3,234 / 16,977 ms** (**20,282 ms total**).
Using 14 threads without algorithm changes still took **10,471 / 19,035 ms**. Release compilation was
already opt-level 3. The major gains came from incremental greedy decoding and reduced encoder work.

Prompt comparison on the **same nine new N371FC WAVs**, with the same optimized decoder and
10-second minimum context; only the prompt changes:

| Model | Old names-only hint: correct typed calls / raw WER | Live-state prompt: correct typed calls / raw WER |
|---|---|---|
| tiny.en | 0/9 · 43.44% | **9/9 · 0.82%** |
| base.en | 1/9 · 46.72% | **9/9 · 2.46%** |

Digit-canonical WER for the names-only hint was 18.85% / 15.57%; for the live-state prompt it was
0.82% / 2.46%. This additional measure expands digit spellings without repairing any wrong value,
facility, or phrase. Raw WER is measured before the constrained post-pass. The previous report's
6/9 result used N1234B recordings and is historical, not the matched baseline for this new set.
The real tiny.en MCP rehearsal also completed **nine first attempts, zero retries, and three readbacks**.

Context comparison with the live-state prompt: actual duration rounded up yielded **8/9 tiny.en**
and **9/9 base.en** typed calls; the 10-second minimum and full 30-second context both yielded
**9/9 for each model**. Full-context raw WER was 1.64% / 0.82%, compared with 0.82% / 2.46% at
10 seconds. The 10-second minimum retains every typed call while meeting the latency targets;
full context is slower. This small synthetic sample does not establish student-microphone accuracy.

On the same current-main Linux x86_64 source, the audio-only release executable is **51.08 MB**;
with Whisper and its headless profiler it is **62.30 MB (+11.22 MB)**. Stripped sizes are
**40.56 MB and 49.42 MB (+8.85 MB)**. These measurements isolate the `whisper` feature against
`--no-default-features --features audio`; model weights are external and excluded. Exact byte
counts are in `out/speech/binary-size.json`.

Receipts and real-model previews are in `out/speech/`: `latency.json`, `real-pipeline.json`,
`dependency-licenses.json`, `preferences-microphone.png`, `ptt-recognized.png`,
`kabe-transcript.png`, `kabe-transcript.md`, and `kabe-record.json`. The KABE transcript includes
raw ungraded recognition, rejected attempts, reviewed transmissions, and heard Simulated ATC.
This is a complete radio pattern rehearsal on original flat test terrain, not a physically flown
circuit. The normal CI stub remains separate from the gated real-recognition test.

`render_speech_previews` writes the microphone page (explicitly a sample meter) and a **stub**
caption/transcript for CI. Run it before the real test. Set `FLIGHTCRAFT_SPEECH_PREVIEW=1` on the
real test to replace the caption with a real recognized call, then run the ignored
`render_real_speech_transcript` test to render the saved real flight record. GPU capture is optional
for the CPU recognition test; normal CI requires neither a microphone nor model weights.

Flight lead should capture these real-window shots on kwin_wayland:

1. First PTT speech-setup offer, selected tiny.en, and the opt-in download progress bar.
2. Preferences → Microphone, selected headset, installed tiny.en, and a live PTT level meter.
3. Cockpit caption immediately after releasing PTT on “Allentown Tower, Sling Three Seven One Foxtrot
   Charlie, left downwind runway zero six,” before transmitting the recognized call.
4. KABE kneeboard with real-Whisper WAV pilot lines, heard Simulated ATC responses, and readback grades.
5. Browser microphone permission/provider notice and a recognized caption, with its observed latency.

See the [AI ATC skill](../skills/ai-atc/SKILL.md) for live MCP controller operation.

To reproduce the desktop profiling measurements without a window or microphone:

```sh
CARGO_TARGET_DIR=$HOME/builds/cargo-target/flightcraft/fc-speech CARGO_BUILD_JOBS=4 \
PKG_CONFIG_PATH=$HOME/builds/pkgconfig-shim FLIGHTCRAFT_SPEECH_EXPORT=/tmp/fc-speech-cases \
cargo test --offline --release -j 4 -p flightcraft-ui --features whisper --lib \
recorded_wavs_through_recognizer_normalizer_review_and_mcp_kabe_pattern

CARGO_TARGET_DIR=$HOME/builds/cargo-target/flightcraft/fc-speech CARGO_BUILD_JOBS=4 \
PKG_CONFIG_PATH=$HOME/builds/pkgconfig-shim cargo build --offline --release -j 4 -p flightcraft-desktop

FLIGHTCRAFT_WHISPER_DIR=$HOME/builds/whisper python3 tools/profile-speech.py \
--executable $HOME/builds/cargo-target/flightcraft/fc-speech/release/flightcraft \
--cases /tmp/fc-speech-cases --context-seconds 10 --prompt-mode state \
--output out/speech/profile-state-10.json
```

The exporter runs the deterministic MCP rehearsal and captures the session immediately before
each call. The profiler passes WAV PCM and those live snapshots to the actual release executable.
Expected text and typed answers stay in the Python scoring process, outside recognition.
`--context-seconds 0` uses the rounded actual duration; `30` restores full padding.
`--prompt-mode legacy` reproduces the old names-only hint; `none` removes the prompt.
`--threads` overrides the inference budget independently of Cargo's four build jobs.
The executable's `--speech-benchmark <request.json>` entry point writes per-stage JSON and never
opens a window, accesses the microphone, or downloads a model. Its release-to-typed timer includes
prompt construction/tokenization, resampling, inference, normalization, and draft creation;
WAV disk I/O, model load (reported separately), user review, and UI frame scheduling are excluded.
