#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use super::*;
use flightcraft_control::{EventSource, Meaning, Reception, Speaker, mcp};
use pipeline::{AudioClip, Recognized, Recognizer};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/atc_fixture.rs"));
fn fixture_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/speech")
}
/// Explicit test double: identifies the decoded PCM fingerprint of our original TTS fixtures.
/// It tests audio ingestion, routing, normalization, review, MCP, and ATC, not Whisper accuracy.
struct FixtureRecognizer {
    fixtures: Vec<(Vec<f32>, String)>,
}
impl FixtureRecognizer {
    fn new() -> Self {
        let rows: Vec<Value> =
            serde_json::from_str(&std::fs::read_to_string(fixture_root().join("manifest.json")).unwrap())
                .unwrap();
        let fixtures = rows
            .into_iter()
            .map(|row| {
                let bytes = std::fs::read(fixture_root().join(row["file"].as_str().unwrap())).unwrap();
                assert_eq!(
                    format!("{:x}", Sha256::digest(&bytes)),
                    row["sha256"].as_str().unwrap()
                );
                (
                    AudioClip::from_wave(&bytes).unwrap().mono_16k().unwrap(),
                    row["text"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        Self { fixtures }
    }
}
impl Recognizer for FixtureRecognizer {
    fn recognize(&mut self, clip: &AudioClip) -> Result<Recognized, String> {
        let pcm = clip.mono_16k()?;
        let text = self
            .fixtures
            .iter()
            .find(|(known, _)| *known == pcm)
            .map(|(_, text)| text.clone())
            .ok_or("Unknown test PCM")?;
        Ok(Recognized {
            text,
            confidence: 0.99,
            latency_ms: 0,
            alternatives: Vec::new(),
        })
    }
}
fn call(s: &mut Session, name: &str, arguments: Value) -> Value {
    let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name.replace('.',"_"),"arguments":arguments}});
    let response: Value = serde_json::from_str(&mcp::handle_line(s, &request.to_string()).unwrap()).unwrap();
    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(response["result"]["isError"], false, "{response}");
    response["result"]["structuredContent"].clone()
}
fn complete(s: &mut Session, event: &Value) {
    let duration = event["duration_s"].as_f64().unwrap();
    call(
        s,
        "sim.step",
        json!({"steps":((duration + 0.6) * 120.).ceil() as u32}),
    );
}
fn pilot(s: &mut Session, recognizer: &mut impl Recognizer, file: &str) {
    let rows: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(fixture_root().join("manifest.json")).unwrap())
            .unwrap();
    let mut transmitted = false;
    for name in [file.to_owned(), format!("{file}-retry")] {
        let path = fixture_root().join(format!("{name}.wav"));
        if !path.exists() {
            break;
        }
        let clip = AudioClip::from_wave(&std::fs::read(path).unwrap()).unwrap();
        recognizer.set_context(&recognition_context(s, false)).unwrap();
        if let Some(directory) = std::env::var_os("FLIGHTCRAFT_SPEECH_EXPORT") {
            let directory = Path::new(&directory);
            std::fs::create_dir_all(directory).unwrap();
            let reference = rows.iter().find(|r| r["file"] == format!("{name}.wav")).unwrap()["text"]
                .as_str()
                .unwrap();
            let (normalized, _) = flightcraft_control::normalize_speech(reference, false, s);
            let expected = flightcraft_control::parse_pilot(&normalized, &s.comms_state().callsign);
            let case = json!({"wav":fixture_root().join(format!("{name}.wav")).canonicalize().unwrap(),"snapshot":s.screen_snapshot(),"regions":s.screen_regions(),"reference":reference,"expected":expected,"expected_normalized":normalized});
            std::fs::write(
                directory.join(format!("case-{name}.json")),
                serde_json::to_vec_pretty(&case).unwrap(),
            )
            .unwrap();
        }
        let recognized = constrain_recognition(recognizer.recognize(&clip).unwrap(), s, false);
        let draft = call(
            s,
            "comms.recognize",
            json!({"text":recognized.text,"confidence":recognized.confidence}),
        );
        assert!(s.flight_record().events.last().unwrap().grade.is_none());
        eprintln!(
            "{name}: {} ms; confidence {:.3}; retry {}; {}",
            recognized.latency_ms, recognized.confidence, draft["retry"], draft["recognized"]
        );
        if draft["retry"] == true {
            let before = s.atc_state().compliance.len();
            assert!(
                s.execute("comms.confirm_speech", json!({"id":draft["id"]}))
                    .is_err()
            );
            assert_eq!(s.atc_state().compliance.len(), before);
            call(s, "comms.discard_speech", json!({"id":draft["id"]}));
            continue;
        }
        let reference = rows.iter().find(|r| r["file"] == format!("{name}.wav")).unwrap()["text"]
            .as_str()
            .unwrap();
        // Assertion only: the reference never reaches recognition or normalization.
        assert!(
            same_typed_call(reference, draft["recognized"].as_str().unwrap(), s),
            "{name}: {draft}"
        );
        let event = call(s, "comms.confirm_speech", json!({"id":draft["id"]}));
        complete(s, &event);
        transmitted = true;
        break;
    }
    assert!(transmitted, "No intelligible recorded attempt for {file}");
}

fn ai(s: &mut Session, tower: &Value, instruction: Value) -> Value {
    let event = call(
        s,
        "comms.atc",
        json!({"facility":tower["id"],"channel_khz":tower["channel_khz"],"role":"tower","instruction":{"kind":"controller","instruction":instruction}}),
    );
    assert_eq!(event["source"], "ai");
    complete(s, &event);
    let mut heard = false;
    for _ in 0..100 {
        let next = call(s, "comms.listen", json!({}));
        if next["transmission"].is_null() {
            break;
        }
        if next["transmission"]["id"] == event["id"] {
            heard = true;
            assert_eq!(next["transmission"]["reception"], "heard");
        }
    }
    assert!(
        heard,
        "AI transmission must be heard through the MCP listen queue"
    );
    event
}
fn rehearsal(recognizer: &mut impl Recognizer) -> Session {
    let mut s = atc_session("KABE");
    call(
        &mut s,
        "atc.configure",
        json!({"airport":"KABE","external_controller":true}),
    );
    let facilities = call(&mut s, "comms.facilities", json!({}));
    let tower = facilities["stations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["airport"] == "KABE" && v["kind"] == "tower")
        .unwrap()
        .clone();
    call(
        &mut s,
        "comms.tune",
        json!({"radio":1,"channel_khz":tower["channel_khz"]}),
    );
    pilot(&mut s, recognizer, "ready");
    ai(
        &mut s,
        &tower,
        json!({"kind":"takeoff","runway":"06","closed_traffic":true}),
    );
    pilot(&mut s, recognizer, "takeoff");
    assert!(
        call(&mut s, "atc.state", json!({}))["pending_instructions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    pilot(&mut s, recognizer, "crosswind");
    pilot(&mut s, recognizer, "downwind");
    ai(
        &mut s,
        &tower,
        json!({"kind":"pattern","runway":"06","entry":"left_downwind","number":1,"follow":null}),
    );
    pilot(&mut s, recognizer, "pattern");
    pilot(&mut s, recognizer, "base");
    ai(&mut s, &tower, json!({"kind":"land","runway":"06"}));
    pilot(&mut s, recognizer, "land");
    pilot(&mut s, recognizer, "final");
    pilot(&mut s, recognizer, "clear");
    let state = call(&mut s, "atc.state", json!({}));
    assert!(state["pending_instructions"].as_array().unwrap().is_empty());
    assert_eq!(
        state["compliance"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["item"] == "readback" && v["passed"] == true)
            .count(),
        3
    );
    assert!(
        s.flight_record()
            .events
            .iter()
            .filter(|e| e.source == EventSource::Ai)
            .all(|e| e.reception == Reception::Heard && e.is_simulated_atc())
    );
    let pilot_events: Vec<_> = s
        .flight_record()
        .events
        .iter()
        .filter(|e| e.speaker == Speaker::Pilot)
        .collect();
    let previews = pilot_events
        .iter()
        .filter(|e| matches!(e.meaning, Meaning::Recognition { .. }))
        .count();
    assert!((9..=18).contains(&previews));
    assert_eq!(pilot_events.len() - previews, 9);
    s
}
#[test]
fn recorded_wavs_through_recognizer_normalizer_review_and_mcp_kabe_pattern() {
    rehearsal(&mut FixtureRecognizer::new());
}
#[test]
fn invalid_wave_and_unknown_audio_are_clean_errors() {
    for bytes in [&[][..], b"RIFF", b"RIFF1234WAVEfmt "] {
        assert!(AudioClip::from_wave(bytes).is_err());
    }
    let clip = AudioClip {
        samples: vec![0.; 16000],
        rate: 16000,
    };
    assert!(FixtureRecognizer::new().recognize(&clip).is_err());
    assert!(
        AudioClip {
            samples: vec![f32::NAN],
            rate: 16000
        }
        .mono_16k()
        .is_err()
    );
    assert!(
        AudioClip {
            samples: vec![0.; 16000 * 31],
            rate: 16000
        }
        .mono_16k()
        .is_err()
    );
}
#[cfg(feature = "whisper")]
fn kabe_context() -> String {
    let mut s = atc_session("KABE");
    let facilities = call(&mut s, "comms.facilities", json!({}));
    let tower = facilities["stations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|st| st["airport"] == "KABE" && st["kind"] == "tower")
        .unwrap();
    call(
        &mut s,
        "comms.tune",
        json!({"radio":1,"channel_khz":tower["channel_khz"]}),
    );
    recognition_context(&s, false)
}
#[cfg(feature = "whisper")]
struct LoggedWhisper {
    inner: super::whisper::Whisper,
    known: Vec<(Vec<f32>, String)>,
    calls: Vec<(String, Recognized)>,
}
#[cfg(feature = "whisper")]
impl Recognizer for LoggedWhisper {
    fn set_context(&mut self, context: &str) -> Result<(), String> {
        self.inner.set_context(context)
    }
    fn recognize(&mut self, clip: &AudioClip) -> Result<Recognized, String> {
        let result = self.inner.recognize(clip)?;
        let pcm = clip.mono_16k()?;
        // Label receipts after inference; no reference text or filename enters Whisper.
        let file = self
            .known
            .iter()
            .find(|(audio, _)| *audio == pcm)
            .map(|(_, name)| name.clone())
            .ok_or("Unknown test audio")?;
        self.calls.push((file, result.clone()));
        Ok(result)
    }
}
#[cfg(feature = "whisper")]
#[test]
fn real_whisper_wav_pipeline_and_latency() {
    if std::env::var_os("FLIGHTCRAFT_WHISPER_DIR").is_none() {
        return;
    }
    let mut inner = super::whisper::Whisper::load("tiny.en").unwrap();
    inner.set_context(&kabe_context()).unwrap();
    let rows: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(fixture_root().join("manifest.json")).unwrap())
            .unwrap();
    let known = rows
        .iter()
        .map(|row| {
            let file = row["file"].as_str().unwrap();
            let bytes = std::fs::read(fixture_root().join(file)).unwrap();
            assert_eq!(
                format!("{:x}", Sha256::digest(&bytes)),
                row["sha256"].as_str().unwrap()
            );
            (
                AudioClip::from_wave(&bytes).unwrap().mono_16k().unwrap(),
                file.to_owned(),
            )
        })
        .collect();
    let mut model = LoggedWhisper {
        inner,
        known,
        calls: Vec::new(),
    };
    let start = std::time::Instant::now();
    let s = rehearsal(&mut model);
    let pipeline_ms = start.elapsed().as_millis();
    let previews: Vec<_> = s
        .flight_record()
        .events
        .iter()
        .filter(|e| matches!(e.meaning, Meaning::Recognition { .. }))
        .collect();
    assert_eq!(previews.len(), model.calls.len());
    let mut errors = 0;
    let mut words = 0;
    let mut correct = 0;
    let mut retries = 0;
    let mut results = Vec::new();
    for ((file, raw), event) in model.calls.iter().zip(&previews) {
        let row = rows.iter().find(|row| row["file"] == *file).unwrap();
        let Meaning::Recognition { normalized, retry } = &event.meaning else {
            continue;
        };
        let (wrong, count) = word_errors(row["text"].as_str().unwrap(), &event.text);
        let typed_correct = !retry && same_typed_call(row["text"].as_str().unwrap(), normalized, &s);
        if row.get("retry_of").is_none() {
            errors += wrong;
            words += count;
            correct += usize::from(typed_correct);
        } else {
            retries += 1;
        }
        results.push(json!({"file":file,"reference":row["text"],"recognized":event.text,"normalized":normalized,"retry":retry,"typed_correct":typed_correct,"latency_ms":raw.latency_ms,"decoder_first":raw.text,"alternatives":raw.alternatives,"word_errors":wrong,"reference_words":count}));
    }
    let output = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/speech");
    std::fs::create_dir_all(&output).unwrap();
    let report = json!({"model":"tiny.en","profile":if cfg!(debug_assertions) {"debug"} else {"release"},"pipeline_ms":pipeline_ms,"first_attempts":9,"first_attempt_correct_typed_calls":correct,"first_attempt_word_errors":errors,"first_attempt_reference_words":words,"first_attempt_wer":errors as f64/words as f64,"retry_wavs":retries,"confirmed_calls":9,"readbacks_passed":3,"calls":results});
    std::fs::write(
        output.join("real-pipeline.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    eprintln!(
        "Real Whisper KABE: {correct}/9 first attempts typed correctly; {errors}/{words} raw word errors; {retries} retries; 9 confirmed calls and 3 readbacks in {pipeline_ms} ms"
    );
    check_real_prompt_negative_audio(&mut model.inner);
    write_rehearsal(
        &s,
        "Every pilot line came from original TTS WAV PCM through **real CPU Candle Whisper tiny.en**, the constrained post-pass, review, and MCP. Rejected first attempts remain ungraded; clearer retry WAVs complete the call. This is a radio rehearsal, not a physically flown circuit.",
    );
    if std::env::var_os("FLIGHTCRAFT_SPEECH_PREVIEW").is_some() {
        let (_, raw) = model
            .calls
            .iter()
            .find(|(file, _)| file == "crosswind.wav")
            .unwrap();
        caption_preview(constrain_recognition(raw.clone(), &s, false));
    }
}
#[test]
#[ignore = "writes offscreen GPU speech previews and explicitly labeled stub-recognizer transcript"]
fn render_speech_previews() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/speech");
    std::fs::create_dir_all(&root).unwrap();
    let mut input = SpeechInput::default();
    input.preferences.enabled = true;
    input.level = 0.43;
    input.devices = vec!["Training headset".into()];
    input.preferences.microphone = "Training headset".into();
    gpu_image(800, 650, |ui| {
        ui.heading("Preferences · Microphone");
        ui.label("Preview · sample microphone level");
        input.preferences_ui(ui);
    })
    .save(root.join("preferences-microphone.png"))
    .unwrap();
    let recognized = FixtureRecognizer::new()
        .recognize(
            &AudioClip::from_wave(&std::fs::read(fixture_root().join("downwind.wav")).unwrap()).unwrap(),
        )
        .unwrap();
    caption_preview(recognized);
    let s = rehearsal(&mut FixtureRecognizer::new());
    write_rehearsal(
        &s,
        "Every pilot call was decoded from an original TTS WAV through a **fixture recognizer test double**. This is not Whisper accuracy or a flown circuit. Recognition previews precede every transmitted call.",
    );
}

fn caption_preview(recognized: Recognized) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/speech");
    std::fs::create_dir_all(&root).unwrap();
    let mut input = SpeechInput::default();
    let mut s = atc_session("KABE");
    input.latency_ms = Some(recognized.latency_ms);
    input.draft = Some(
        s.run(CommsRecognize {
            request_id: None,
            text: recognized.text,
            confidence: recognized.confidence,
            intercom: false,
        })
        .unwrap(),
    );
    gpu_image(1000, 240, |ui| {
        ui.heading("Push-to-talk · recognized call");
        input.caption(ui, &mut s);
    })
    .save(root.join("ptt-recognized.png"))
    .unwrap();
}

fn write_rehearsal(s: &Session, description: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/speech");
    std::fs::create_dir_all(&root).unwrap();
    let mut text = format!("# KABE radio pattern rehearsal\n\n{description}\n\n```text\n");
    let stations = s.comms_state().stations;
    for event in &s.flight_record().events {
        if let Some(line) = event.transcript_line(&stations) {
            text.push_str(&line);
            text.push('\n');
        }
    }
    text.push_str("```\n");
    std::fs::write(root.join("kabe-transcript.md"), text).unwrap();
    std::fs::write(
        root.join("kabe-record.json"),
        serde_json::to_vec_pretty(s.flight_record()).unwrap(),
    )
    .unwrap();
}

fn gpu_image(width: u32, height: u32, mut paint: impl FnMut(&mut egui::Ui)) -> image::RgbaImage {
    pollster::block_on(async {
        let mut capture = crate::test_capture::Capture::new([width, height]).await;
        for _ in 0..3 {
            capture.draw(&mut paint);
        }
        capture.validate().await;
        capture.image()
    })
}

#[cfg(feature = "whisper")]
fn word_errors(reference: &str, recognized: &str) -> (usize, usize) {
    fn words(text: &str) -> Vec<String> {
        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect()
    }
    let a = words(reference);
    let b = words(recognized);
    let mut previous: Vec<_> = (0..=b.len()).collect();
    for (i, word) in a.iter().enumerate() {
        let mut next = vec![i + 1];
        for (j, other) in b.iter().enumerate() {
            next.push(
                (previous[j] + usize::from(word != other))
                    .min(previous[j + 1] + 1)
                    .min(next[j] + 1),
            );
        }
        previous = next;
    }
    (previous[b.len()], a.len())
}

fn same_typed_call(reference: &str, recognized: &str, s: &Session) -> bool {
    use flightcraft_control::{normalize_speech, parse_pilot};
    let (reference, _) = normalize_speech(reference, false, s);
    let (recognized, valid) = normalize_speech(recognized, false, s);
    let identity = s.comms_state().callsign;
    let mut expected = parse_pilot(&reference, &identity);
    let mut actual = parse_pilot(&recognized, &identity);
    expected.normalized.clear();
    actual.normalized.clear();
    valid
        && expected == actual
        && [
            "cleared for takeoff",
            "left closed traffic",
            "enter left downwind",
            "cleared to land",
            "left crosswind",
            "left downwind",
            "left base",
            "final",
            "clear of runway",
            "request closed traffic",
        ]
        .iter()
        .all(|phrase| reference.contains(phrase) == recognized.contains(phrase))
}

#[cfg(feature = "whisper")]
#[test]
#[ignore = "manual accuracy/latency diagnostic requiring both verified model directories"]
fn measure_real_whisper_models() {
    if std::env::var_os("FLIGHTCRAFT_WHISPER_DIR").is_none() {
        return;
    }
    let output = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/speech");
    std::fs::create_dir_all(&output).unwrap();
    let rows: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(fixture_root().join("manifest.json")).unwrap())
            .unwrap();
    let clip = AudioClip::from_wave(&std::fs::read(fixture_root().join("latency.wav")).unwrap()).unwrap();
    let mut reports = Vec::new();
    for id in ["tiny.en", "base.en"] {
        let start = std::time::Instant::now();
        let mut model = super::whisper::Whisper::load(id).unwrap();
        let load_ms = start.elapsed().as_millis();
        let context = if std::env::var_os("FLIGHTCRAFT_WHISPER_NO_CONTEXT").is_none() {
            kabe_context()
        } else {
            String::new()
        };
        model.set_context(&context).unwrap();
        let mut timings = Vec::new();
        let s = atc_session("KABE");
        for _ in 0..3 {
            let r = constrain_recognition(model.recognize(&clip).unwrap(), &s, false);
            eprintln!(
                "{id}: load {load_ms} ms; {} ms / {:.3} s audio; confidence {:.3}: {}",
                r.latency_ms,
                clip.samples.len() as f64 / f64::from(clip.rate),
                r.confidence,
                r.text
            );
            timings.push(json!({"latency_ms":r.latency_ms,"confidence":r.confidence,"text":r.text}));
        }
        let mut results = Vec::new();
        let mut errors = 0;
        let mut count = 0;
        let mut correct = 0;
        for row in &rows {
            let file = row["file"].as_str().unwrap();
            if row.get("benchmark_only").is_some()
                || file == "around.wav"
                || row.get("retry_of").is_some()
                || std::env::var("FLIGHTCRAFT_WHISPER_FILES")
                    .is_ok_and(|files| !files.split(',').any(|v| v == file))
            {
                continue;
            }
            let audio = AudioClip::from_wave(&std::fs::read(fixture_root().join(file)).unwrap()).unwrap();
            let raw = model.recognize(&audio).unwrap();
            let decoder_first = raw.text.clone();
            let hypotheses = raw.alternatives.clone();
            let result = constrain_recognition(raw, &s, false);
            let (wrong, words) = word_errors(row["text"].as_str().unwrap(), &result.text);
            errors += wrong;
            count += words;
            let (normalized, valid) = flightcraft_control::normalize_speech(&result.text, false, &s);
            let typed_correct =
                result.confidence >= 0.65 && same_typed_call(row["text"].as_str().unwrap(), &result.text, &s);
            correct += usize::from(typed_correct);
            eprintln!(
                "{id} {file}: {} ms, confidence {:.3}, grammar {valid}, {wrong}/{words} errors: {} => {normalized}",
                result.latency_ms, result.confidence, result.text
            );
            results.push(json!({"file":file,"reference":row["text"],"recognized":result.text,"decoder_first":decoder_first,"alternatives":hypotheses,"confidence":result.confidence,"latency_ms":result.latency_ms,"normalized":normalized,"grammar_valid":valid,"typed_correct":typed_correct,"word_errors":wrong,"reference_words":words}));
        }
        reports.push(json!({"model":id,"context":context,"first_load_ms":load_ms,"benchmark_audio":"latency.wav","benchmark_duration_s":clip.samples.len() as f64/f64::from(clip.rate),"benchmark":timings,"word_errors":errors,"reference_words":count,"typed_correct_calls":correct,"total_calls":results.len(),"wer":errors as f64/count as f64,"calls":results}));
        std::fs::write(output.join("recognition-measurements.json"), serde_json::to_vec_pretty(&json!({"profile":if cfg!(debug_assertions) {"debug"} else {"release"},"device":"CPU","rayon_num_threads":std::env::var("RAYON_NUM_THREADS").ok(),"models":reports})).unwrap()).unwrap();
    }
}

#[test]
fn acoustic_alternatives_use_grammar_without_selecting_expected_numbers() {
    let mut s = atc_session("KABE");
    call(
        &mut s,
        "atc.configure",
        json!({"airport":"KABE","external_controller":true}),
    );
    let facilities = call(&mut s, "comms.facilities", json!({}));
    let tower = facilities["stations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["airport"] == "KABE" && v["kind"] == "tower")
        .unwrap();
    call(
        &mut s,
        "comms.tune",
        json!({"radio":1,"channel_khz":tower["channel_khz"]}),
    );
    ai(&mut s, tower, json!({"kind":"land","runway":"06"}));
    let wrong = "Runway 24 cleared to land Sling 371 Foxtrot Charlie";
    let expected = "Runway 06 cleared to land Sling 371 Foxtrot Charlie";
    let raw = Recognized {
        text: "Runway nearly six cleared to land Sling 371 Foxtrot Charlie".into(),
        confidence: 0.95,
        latency_ms: 0,
        alternatives: vec![(wrong.into(), 0.9), (expected.into(), 0.8)],
    };
    let selected = constrain_recognition(raw, &s, false);
    assert_eq!(selected.text, wrong);
    let low = Recognized {
        text: expected.into(),
        confidence: 0.2,
        latency_ms: 0,
        alternatives: Vec::new(),
    };
    let selected = constrain_recognition(low, &s, false);
    let draft = call(
        &mut s,
        "comms.recognize",
        json!({"text":selected.text,"confidence":selected.confidence}),
    );
    assert_eq!(draft["retry"], true);
}

#[test]
#[ignore = "renders the saved real-Whisper KABE flight record, after the gated pipeline test"]
fn render_real_speech_transcript() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/speech");
    let receipt: Value =
        serde_json::from_slice(&std::fs::read(root.join("real-pipeline.json")).unwrap()).unwrap();
    assert_eq!(receipt["model"], "tiny.en");
    assert_eq!(receipt["confirmed_calls"], 9);
    let record: flightcraft_control::FlightRecord =
        serde_json::from_slice(&std::fs::read(root.join("kabe-record.json")).unwrap()).unwrap();
    let s = atc_session("KABE");
    gpu_image(1400, 640, |ui| {
        ui.heading("KABE · Radio pattern rehearsal");
        ui.label("Every pilot line came from WAV audio through CPU Candle Whisper tiny.en.");
        ui.label("Nine confirmed calls · three accepted readbacks · rejected recognition remains ungraded");
        crate::radio::record_transcript(ui, &s, &record);
    })
    .save(root.join("kabe-transcript.png"))
    .unwrap();
}

#[cfg(feature = "whisper")]
#[test]
#[ignore = "manual CPU benchmark requiring both verified model directories"]
fn benchmark_real_whisper_latency() {
    if std::env::var_os("FLIGHTCRAFT_WHISPER_DIR").is_none() {
        return;
    }
    let clip = AudioClip::from_wave(&std::fs::read(fixture_root().join("latency.wav")).unwrap()).unwrap();
    let duration = clip.samples.len() as f64 / f64::from(clip.rate);
    assert!((3.0..=5.0).contains(&duration));
    let s = atc_session("KABE");
    let mut models = Vec::new();
    for id in ["tiny.en", "base.en"] {
        let start = std::time::Instant::now();
        let mut model = super::whisper::Whisper::load(id).unwrap();
        let first_load_ms = start.elapsed().as_millis();
        model.set_context(&kabe_context()).unwrap();
        let mut calls = Vec::new();
        for _ in 0..3 {
            let result = constrain_recognition(model.recognize(&clip).unwrap(), &s, false);
            assert!(same_typed_call(
                "Runway zero six cleared to land Sling three seven one foxtrot charlie",
                &result.text,
                &s
            ));
            eprintln!(
                "{id}: first load {first_load_ms} ms; recognition {} ms / {duration:.3} s audio",
                result.latency_ms
            );
            calls.push(json!({"latency_ms":result.latency_ms,"confidence":result.confidence,"recognized":result.text}));
        }
        models.push(json!({"model":id,"first_load_ms":first_load_ms,"calls":calls}));
    }
    let cpu = std::fs::read_to_string("/proc/cpuinfo").ok().and_then(|info| {
        info.lines().find_map(|line| {
            line.strip_prefix("model name")
                .and_then(|line| line.split_once(':'))
                .map(|(_, value)| value.trim().to_owned())
        })
    });
    let report = json!({"cpu":cpu,"device":"CPU","profile":if cfg!(debug_assertions) {"debug"} else {"release"},"threads":std::env::var("RAYON_NUM_THREADS").ok(),"beam_width":1,"audio":"latency.wav","duration_s":duration,"first_load_includes":"SHA-256 verification, config/tokenizer loading, tensor loading, and frontend initialization; OS file cache warm; context setup excluded","models":models});
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/speech");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("latency.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}

#[test]
fn first_ptt_offers_setup_without_enabling_capture_or_downloading() {
    let mut input = SpeechInput::default();
    assert!(input.offer_first_use());
    assert!(!input.preferences.enabled);
    assert!(input.worker.is_none());
    input.setup_open = false;
    assert!(!input.offer_first_use());
    assert!(!input.preferences.enabled);
}

#[test]
fn speech_request_ids_survive_javascript_number_transport() {
    let mut input = SpeechInput::default();
    input.user_gesture();
    let seed = input.next_request;
    assert!((1..(1 << 52)).contains(&seed));
    input.user_gesture();
    assert_eq!(input.next_request, seed);
    assert_eq!((seed as f64) as u64, seed);
}

#[cfg(all(not(target_arch = "wasm32"), feature = "desktop-audio"))]
#[test]
fn vox_keeps_a_recognized_command_available_for_review() {
    let mut s = atc_session("KABE");
    call(&mut s, "comms.audio_panel", json!({"intercom":true}));
    let draft = s
        .run(CommsRecognize {
            request_id: None,
            text: "your controls".into(),
            confidence: 0.95,
            intercom: true,
        })
        .unwrap();
    let mut input = SpeechInput::default();
    input.preferences.enabled = true;
    input.preferences.vox = true;
    input.generation = Some(s.flight_generation());
    input.draft = Some(draft.clone());
    input.user_gesture();
    let ctx = egui::Context::default();
    let _ = ctx.run(
        egui::RawInput {
            focused: true,
            time: Some(1.),
            ..Default::default()
        },
        |ctx| input.observe(ctx, &mut s),
    );
    assert_eq!(input.draft.as_ref().map(|d| d.id), Some(draft.id));
    assert!(!input.held, "a pending VOX draft must remain confirmable");
    assert!(input.capture.is_none());
    assert!(input.worker.is_none());
    assert!(input.error.is_none());
}

#[test]
fn highest_supported_rate_resamples_within_the_wasm32_length_bound() {
    let clip = AudioClip {
        samples: vec![0.25; 192000 * 30],
        rate: 192000,
    };
    let pcm = clip.mono_16k().unwrap();
    assert_eq!(pcm.len(), 16000 * 30);
    assert!(pcm.iter().all(|v| v.is_finite()));
}

#[cfg(feature = "whisper")]
#[test]
fn speech_prompt_uses_callsign_atis_runway_and_only_heard_instructions() {
    let mut s = atc_session("KABE");
    let prompt = recognition_context(&s, false);
    assert!(
        prompt.contains("N371FC") && prompt.contains("Sling Three Seven One Foxtrot Charlie"),
        "{prompt}"
    );
    assert!(prompt.to_lowercase().contains("information alpha."), "{prompt}");
    let facilities = call(&mut s, "comms.facilities", json!({}));
    let tower = facilities["stations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["airport"] == "KABE" && v["kind"] == "tower")
        .unwrap()
        .clone();
    call(
        &mut s,
        "comms.tune",
        json!({"radio":1,"channel_khz":tower["channel_khz"]}),
    );
    call(
        &mut s,
        "atc.configure",
        json!({"airport":"KABE","external_controller":true}),
    );
    let event = call(
        &mut s,
        "comms.atc",
        json!({"facility":tower["id"],"channel_khz":tower["channel_khz"],"role":"tower","instruction":{"kind":"controller","instruction":{"kind":"land","runway":"06"}}}),
    );
    assert!(
        !recognition_context(&s, false)
            .to_lowercase()
            .contains("cleared to land")
    );
    complete(&mut s, &event);
    let prompt = recognition_context(&s, false).to_lowercase();
    assert!(
        prompt.contains("allentown tower")
            && prompt.contains("runway zero six")
            && prompt.contains("cleared to land"),
        "{prompt}"
    );
}

#[cfg(feature = "whisper")]
fn check_real_prompt_negative_audio(model: &mut super::whisper::Whisper) {
    let mut s = atc_session("KABE");
    call(
        &mut s,
        "atc.configure",
        json!({"airport":"KABE","external_controller":true}),
    );
    let facilities = call(&mut s, "comms.facilities", json!({}));
    let tower = facilities["stations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["airport"] == "KABE" && v["kind"] == "tower")
        .unwrap()
        .clone();
    call(
        &mut s,
        "comms.tune",
        json!({"radio":1,"channel_khz":tower["channel_khz"]}),
    );
    ai(&mut s, &tower, json!({"kind":"land","runway":"06"}));
    let rows: Vec<Value> =
        serde_json::from_slice(&std::fs::read(fixture_root().join("negative-manifest.json")).unwrap())
            .unwrap();
    let mut results = Vec::new();
    for row in rows {
        let bytes = std::fs::read(fixture_root().join(row["file"].as_str().unwrap())).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), row["sha256"]);
        model.set_context(&recognition_context(&s, false)).unwrap();
        let raw = model.recognize(&AudioClip::from_wave(&bytes).unwrap()).unwrap();
        let draft = call(
            &mut s,
            "comms.recognize",
            json!({"text":raw.text,"confidence":raw.confidence}),
        );
        assert!(s.flight_record().events.last().unwrap().grade.is_none());
        if row["file"] == "wrong-runway.wav" {
            assert_eq!(draft["message"]["runway"], "24", "{draft}");
        } else {
            assert_eq!(draft["message"]["callsign_present"], false, "{draft}");
            assert_eq!(draft["retry"], true, "{draft}");
        }
        results.push(json!({"file":row["file"],"recognized":raw.text,"confidence":raw.confidence,"draft":draft,"stages":model.timings}));
        call(&mut s, "comms.discard_speech", json!({"id":draft["id"]}));
    }
    let output = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/speech");
    std::fs::create_dir_all(&output).unwrap();
    std::fs::write(
        output.join("negative-prompt.json"),
        serde_json::to_vec_pretty(&results).unwrap(),
    )
    .unwrap();
}
