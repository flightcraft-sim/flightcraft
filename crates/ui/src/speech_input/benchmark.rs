//! Headless diagnostic entry point in the actual desktop executable. Never fetches weights.
use super::{
    pipeline::{AudioClip, Recognizer},
    whisper::Whisper,
};
use flightcraft_control::{Command, CommsRecognize, Session};
use serde::Deserialize;
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    time::Instant,
};
#[derive(Deserialize)]
struct Request {
    models: Vec<String>,
    #[serde(default = "repetitions")]
    repetitions: usize,
    cases: Vec<Case>,
    #[serde(default = "minimum_context")]
    audio_context_seconds: usize,
    #[serde(default)]
    prompt_mode: PromptMode,
}
fn minimum_context() -> usize {
    10
}
#[derive(Clone, Copy, Default, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum PromptMode {
    #[default]
    State,
    Legacy,
    None,
}
fn context(mode: PromptMode, case: &Case, session: &Session) -> String {
    match mode {
        PromptMode::State => super::recognition_context(session, false),
        PromptMode::None => String::new(),
        PromptMode::Legacy => {
            if !case.context.is_empty() {
                return case.context.clone();
            }
            let s = session.comms_state();
            let khz = if s.panel.transmit == 1 {
                s.com1.active_khz
            } else {
                s.com2.active_khz
            };
            let mut names = vec![s.callsign.spoken_type];
            for station in s.stations.iter().filter(|st| st.channel_khz == khz).take(4) {
                names.push(flightcraft_control::comms::phraseology::facility(station));
            }
            format!(
                "{}. Radio phraseology: runway, cleared, takeoff, closed traffic, crosswind, downwind, base, final, land.",
                names.join(". ")
            )
        }
    }
}

fn repetitions() -> usize {
    3
}
#[derive(Deserialize)]
struct Case {
    wav: PathBuf,
    #[serde(default)]
    context: String,
    snapshot: Option<flightcraft_control::presentation::Snapshot>,
    #[serde(default)]
    regions: Vec<flightcraft_control::presentation::RegionTransfer>,
}
/// Times resampling, frontend, encoder, decoder, and normalization after simulated PTT release.
/// WAV disk I/O and model installation/load are outside the per-call timer.
pub fn run(request: &Path) -> Result<String, String> {
    if std::fs::metadata(request).map_err(|e| e.to_string())?.len() > 32 * 1024 * 1024 {
        return Err("Speech benchmark request exceeds 32 MB.".into());
    }
    let request: Request = serde_json::from_slice(&std::fs::read(request).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if request.models.is_empty()
        || request.cases.is_empty()
        || request.audio_context_seconds > 30
        || request.repetitions == 0
        || request.repetitions > 20
        || request.cases.len() > 64
        || request.models.len() > 2
    {
        return Err("Use one or two models, up to 64 recordings, and one to 20 repetitions.".into());
    }
    let mut reports = Vec::new();
    for id in request.models {
        let load = Instant::now();
        let mut model = Whisper::load(&id)?;
        model.minimum_context_seconds = request.audio_context_seconds;
        let first_load_ms = load.elapsed().as_secs_f64() * 1000.;
        let mut calls = Vec::new();
        for case in &request.cases {
            let clip = AudioClip::from_wave(&std::fs::read(&case.wav).map_err(|e| e.to_string())?)?;
            for _ in 0..request.repetitions {
                let mut session = Session::new().map_err(|e| e.to_string())?;
                session.install_screen_regions(case.regions.clone())?;
                if let Some(snapshot) = &case.snapshot {
                    session.apply_screen_snapshot(snapshot.clone())?;
                } else {
                    session
                        .execute(
                            "comms.callsign",
                            json!({"registration":"N371FC","spoken_type":"Sling"}),
                        )
                        .map_err(|e| e.to_string())?;
                }
                let start = Instant::now();
                let context = context(request.prompt_mode, case, &session);
                model.set_context(&context)?;
                let result = model.recognize(&clip)?;
                let normalization = Instant::now();
                let result = super::constrain_recognition(result, &session, false);
                let draft = CommsRecognize {
                    request_id: None,
                    text: result.text,
                    confidence: result.confidence,
                    intercom: false,
                }
                .run(&mut session)
                .map_err(|e| e.to_string())?;
                let normalization_ms = normalization.elapsed().as_secs_f64() * 1000.;
                let release_to_typed_ms = start.elapsed().as_secs_f64() * 1000.;
                calls.push(json!({"wav":case.wav,"prompt":context,"duration_s":clip.samples.len() as f64/f64::from(clip.rate),"release_to_typed_ms":release_to_typed_ms,"normalization_ms":normalization_ms,"stages":model.timings,"confidence":result.confidence,"draft":draft}));
            }
        }
        reports.push(json!({"model":id,"first_load_ms":first_load_ms,"calls":calls}));
    }
    serde_json::to_string_pretty(&json!({"executable":std::env::current_exe().ok(),"minimum_audio_context_seconds":request.audio_context_seconds,"prompt_mode":request.prompt_mode,"debug_assertions":cfg!(debug_assertions),"rayon_num_threads":std::env::var("RAYON_NUM_THREADS").ok(),"models":reports})).map_err(|e| e.to_string())
}
