# Radios and untowered training

The session owns communications, recorded events, and deterministic circuit traffic. The flight
integrator owns none of them: speech, captions, tuning, and transmissions do not enter the trajectory
hash. The same commands serve the desktop, browser, scripts, hardware adapters, and MCP.

## Operate the radios

Open **Radios** from the toolbar. The configurable default shortcuts are **V** for the radio panel,
**T** for selected-COM PTT and typed entry, **Y** for intercom entry, and **K** for the kneeboard.
A press opens the one-line typed entry; Enter transmits it. The microphone bindings are named
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

`comms.callsign` reads the identity or overrides either field. The radio window exposes the same
controls, and the app persists the choice. A typed `{callsign}` placeholder expands to the current
spoken form. US registrations use the aircraft type in place of November: “Sling One Two Three
Four Bravo,” then “Sling One Foxtrot Charlie” after a completed, unblocked call on that carrier.
Changing frequency starts a new contact. `abbreviate_after_contact:false` retains full callsigns.
On actual ATC frequencies, use an abbreviation only after the controller initiates it. The
[tower and approach rules](atc.md) initiate the abbreviated reply after hearing the full first call. Traffic uses fictional civil GA registrations with
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
COM PCM receives a 300–3000 Hz high/low cut, compression, quiet procedural static, and a squelch tail.
Intercom bypasses that effect. Role selection changes installed voices on Windows and Linux, and
speaking rate on macOS. macOS currently uses its default voice; distinct installed voice selection
there is follow-up work. This implementation does not add the requested third-party cross-platform
TTS crate, because it was unavailable offline.

The browser uses `speechSynthesis`, choosing installed voices by role. Its `speak(utterance)` API
routes speech to the browser/operating-system audio sink and exposes neither PCM samples, an
`AudioNode`, an `HTMLMediaElement`, nor a `MediaStream` for the utterance. WebAudio's buffer, media
element, and stream source nodes therefore cannot accept this speech. Adding a band-pass or
compressor to the separate AudioContext cannot affect it. Applying the same COM filter would
require a speech engine that returns PCM (for example a separately licensed, bundled WASM voice),
or recording system playback with browser-specific capture and user permission. No such PCM engine
or voice asset is available in this repository; platform speech remains explicitly **unfiltered**.
Morse uses WebAudio directly. Browser voice availability and audio activation depend on the platform.

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
