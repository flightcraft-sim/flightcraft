//! Speech input is presentation I/O; only reviewed text enters control commands.
#[cfg(all(not(target_arch = "wasm32"), feature = "whisper"))]
pub mod benchmark;
#[cfg(all(not(target_arch = "wasm32"), feature = "desktop-audio"))]
mod capture;
#[cfg(all(not(target_arch = "wasm32"), feature = "whisper"))]
mod fft;
#[cfg(all(not(target_arch = "wasm32"), feature = "whisper"))]
mod frontend;
#[cfg(all(not(target_arch = "wasm32"), feature = "whisper"))]
mod incremental;
#[cfg(not(target_arch = "wasm32"))]
mod models;
pub mod pipeline;
#[cfg(test)]
mod tests;
#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(all(not(target_arch = "wasm32"), feature = "whisper"))]
mod whisper;
use crate::theme;
use eframe::egui;
use flightcraft_control::{CommsConfirmSpeech, CommsDiscardSpeech, CommsRecognize, Session, SpeechDraft};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub enabled: bool,
    pub setup_offered: bool,
    pub microphone: String,
    pub model: String,
    pub vox: bool,
    pub vox_threshold: f32,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            enabled: false,
            setup_offered: false,
            microphone: String::new(),
            model: "tiny.en".into(),
            vox: false,
            vox_threshold: 0.05,
        }
    }
}
#[derive(Default)]
pub struct SpeechInput {
    pub preferences: Preferences,
    setup_open: bool,
    pub draft: Option<SpeechDraft>,
    pub error: Option<String>,
    pub latency_ms: Option<u64>,
    held: bool,
    intercom: bool,
    gesture: bool,
    generation: Option<u64>,
    next_request: u64,
    #[cfg(all(not(target_arch = "wasm32"), feature = "desktop-audio"))]
    last_voice: f64,
    #[cfg(all(not(target_arch = "wasm32"), feature = "desktop-audio"))]
    started: f64,
    #[cfg(all(not(target_arch = "wasm32"), feature = "desktop-audio"))]
    capture_route: Option<(u8, u32)>,
    #[cfg(not(target_arch = "wasm32"))]
    pending_route: Option<(u8, u32)>,
    #[cfg(not(target_arch = "wasm32"))]
    recognizing: bool,
    pub level: f32,
    #[cfg(all(not(target_arch = "wasm32"), feature = "desktop-audio"))]
    capture: Option<capture::Capture>,
    #[cfg(not(target_arch = "wasm32"))]
    worker: Option<models::Worker>,
    #[cfg(not(target_arch = "wasm32"))]
    devices: Vec<String>,
    #[cfg(target_arch = "wasm32")]
    browser: web::Browser,
    #[cfg(target_arch = "wasm32")]
    browser_route: Option<(u8, u32)>,
}
impl SpeechInput {
    pub fn with_preferences(preferences: Preferences) -> Self {
        Self {
            preferences,
            ..Default::default()
        }
    }
    pub fn user_gesture(&mut self) {
        self.gesture = true;
        if self.next_request == 0 {
            self.next_request = speech_request_seed();
        }
    }
    /// Only UI PTT gestures offer setup; control commands never open dialogs.
    pub fn offer_first_use(&mut self) -> bool {
        if !self.preferences.setup_offered {
            self.preferences.setup_offered = true;
            self.setup_open = true;
        }
        self.setup_open
    }
    pub fn open_setup(&mut self) {
        self.preferences.setup_offered = true;
        self.setup_open = true;
    }
    pub fn setup_ui(&mut self, ctx: &egui::Context) {
        if !self.setup_open {
            return;
        }
        let mut open = true;
        egui::Window::new("Set up speech input").open(&mut open).default_width(520.).show(ctx, |ui| {
            ui.label("Practice your radio calls out loud. Choose a microphone and speech model, then hold push-to-talk.");
            self.preferences_ui(ui);
            if ui.button("Done").clicked() { self.setup_open = false; }
        });
        self.setup_open &= open;
    }
    pub fn preferences_ui(&mut self, ui: &mut egui::Ui) {
        self.preferences.setup_offered = true;
        ui.label(egui::RichText::new("MICROPHONE · SPEECH INPUT").color(theme::ACCENT));
        ui.checkbox(&mut self.preferences.enabled, "Enable speech input");
        #[cfg(not(target_arch = "wasm32"))]
        {
            ui.label("Offline recognition on this computer. Audio is never uploaded.");
            #[cfg(feature = "desktop-audio")]
            if ui.button("Refresh microphones").clicked() {
                match capture::devices() {
                    Ok(v) => self.devices = v,
                    Err(e) => self.error = Some(e),
                }
            }
            egui::ComboBox::from_id_salt("speech-microphone")
                .selected_text(if self.preferences.microphone.is_empty() {
                    "System default"
                } else {
                    &self.preferences.microphone
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.preferences.microphone, String::new(), "System default");
                    for device in &self.devices {
                        ui.selectable_value(&mut self.preferences.microphone, device.clone(), device);
                    }
                });
            egui::ComboBox::from_id_salt("speech-model")
                .selected_text(&self.preferences.model)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.preferences.model, "tiny.en".into(), "tiny.en · faster");
                    ui.selectable_value(
                        &mut self.preferences.model,
                        "base.en".into(),
                        "base.en · more accurate",
                    );
                });
            ui.label(
                "Model weights are a separate MIT-licensed download. Download only when you choose below.",
            );
            if ui.button("Download / resume selected model").clicked() {
                self.ensure_worker();
                if let Some(worker) = &self.worker
                    && let Err(e) = worker.download(&self.preferences.model)
                {
                    self.error = Some(e);
                }
            }
            if let Some(worker) = &self.worker {
                let (done, total) = worker.progress();
                if total > 0 {
                    ui.add(egui::ProgressBar::new(done as f32 / total as f32).text(format!(
                        "Model download · {:.1} / {:.1} MB",
                        done as f64 / 1e6,
                        total as f64 / 1e6
                    )));
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            ui.label("Browser speech recognition may send audio to your browser provider. It may require an internet connection. Enable only if you agree.");
            ui.label(
                "Input: browser's default microphone. Choose it in the browser's site microphone settings.",
            );
        }
        ui.add(egui::ProgressBar::new(self.level.clamp(0., 1.)).text("Microphone level · hold PTT to test"));
        ui.checkbox(&mut self.preferences.vox, "Intercom voice activation (VOX)");
        ui.label("VOX keeps the microphone open for intercom commands only. COM always requires PTT.");
        #[cfg(not(target_arch = "wasm32"))]
        ui.add(egui::Slider::new(&mut self.preferences.vox_threshold, 0.01..=0.5).text("VOX threshold"));
        if let Some(ms) = self.latency_ms {
            ui.label(format!("Last recognition: {ms} ms"));
        }
        if let Some(error) = &self.error {
            ui.colored_label(theme::AMBER, error);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn ensure_worker(&mut self) {
        if self.worker.is_none() {
            match models::Worker::new() {
                Ok(w) => self.worker = Some(w),
                Err(e) => self.error = Some(e),
            }
        }
    }
    pub fn observe(&mut self, ctx: &egui::Context, s: &mut Session) {
        let generation = s.flight_generation();
        if self.generation != Some(generation) {
            self.cancel();
            self.draft = None;
            self.generation = Some(generation);
        }
        let state = s.comms_state();
        let focused = ctx.input(|i| i.focused);
        let active = self.preferences.enabled && focused && self.gesture;
        let ptt = state.panel.ptt || state.panel.intercom_ptt;
        let vox = active && self.preferences.vox && state.panel.intercom && !state.panel.ptt;
        let intercom = state.panel.intercom_ptt || vox;
        // VOX must leave a completed draft visible until reviewed. Explicit
        // PTT can start a retry; automatic VOX cannot discard that draft.
        let capture_requested = active && (ptt || (vox && self.draft.is_none()));
        let now = ctx.input(|i| i.time);
        if !active {
            self.cancel();
        }
        #[cfg(all(not(target_arch = "wasm32"), feature = "desktop-audio"))]
        {
            // Switching between VOX/intercom and COM starts a new capture. An
            // unfinished segment cannot cross the channel boundary.
            if self.capture.is_some() && self.intercom != intercom {
                self.capture = None;
                self.held = false;
                self.level = 0.;
            }
            if capture_requested && self.capture.is_none() && !self.held && !self.recognizing {
                #[cfg(feature = "whisper")]
                {
                    // Verify/load during the held call, before the release latency budget.
                    self.ensure_worker();
                    if let Some(worker) = &self.worker {
                        worker.prepare(&self.preferences.model);
                    }
                }
                self.capture_route = Some((
                    state.panel.transmit,
                    if state.panel.transmit == 1 {
                        state.com1.active_khz
                    } else {
                        state.com2.active_khz
                    },
                ));
                self.intercom = intercom;
                self.started = now;
                self.last_voice = now;
                self.held = true;
                match capture::Capture::start(&self.preferences.microphone) {
                    Ok(c) => {
                        self.capture = Some(c);
                        if let Some(draft) = self.draft.take() {
                            let _ = s.run(CommsDiscardSpeech { id: draft.id });
                        }
                    }
                    Err(e) => self.error = Some(e),
                }
            }
            if let Some(capture) = &self.capture {
                self.level = capture.level();
                if self.level >= self.preferences.vox_threshold {
                    self.last_voice = now;
                }
            }
            let ended = !ptt && !vox || self.intercom != intercom || now - self.started >= 29.;
            let vox_end =
                vox && !state.panel.intercom_ptt && now - self.last_voice >= 0.8 && now - self.started >= 1.;
            if (ended || vox_end)
                && let Some(mut capture) = self.capture.take()
            {
                let result = if vox_end { capture.take() } else { capture.finish() };
                self.level = 0.;
                if self.last_voice > self.started || !vox {
                    match result {
                        Ok(clip) => {
                            self.ensure_worker();
                            if let Some(w) = &self.worker {
                                match w.recognize(
                                    clip,
                                    self.intercom,
                                    generation,
                                    &self.preferences.model,
                                    recognition_context(s, self.intercom),
                                ) {
                                    Ok(()) => {
                                        self.pending_route = self.capture_route;
                                        self.recognizing = true;
                                    }
                                    Err(e) => self.error = Some(e),
                                }
                            }
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
                // A new PTT edge is required after a duration limit. VOX starts a fresh segment.
                if vox {
                    self.held = false;
                }
            }
            if !ptt && !vox {
                self.held = false;
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(w) = &self.worker
            && let Some((epoch, ics, result)) = w.poll()
        {
            if epoch == u64::MAX {
                self.error = result.err();
                return;
            }
            self.recognizing = false;
            if epoch != generation || !active {
                return;
            }
            let route = (
                state.panel.transmit,
                if state.panel.transmit == 1 {
                    state.com1.active_khz
                } else {
                    state.com2.active_khz
                },
            );
            if !ics && self.pending_route != Some(route) {
                self.error =
                    Some("COM selection changed during recognition. Say again on the selected COM.".into());
                return;
            }
            match result {
                Ok(recognition) => {
                    let recognition = constrain_recognition(recognition, s, ics);
                    self.latency_ms = Some(recognition.latency_ms);
                    self.next_request = self.next_request.wrapping_add(1);
                    match s.run(CommsRecognize {
                        request_id: Some(self.next_request),
                        text: recognition.text,
                        confidence: recognition.confidence,
                        intercom: ics,
                    }) {
                        Ok(draft) => {
                            self.draft = Some(draft);
                            self.error = None;
                        }
                        Err(e) => self.error = Some(command_message(e)),
                    }
                }
                Err(e) => self.error = Some(e),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.held = active && ptt;
            self.intercom = intercom;
            let route = (
                state.panel.transmit,
                if state.panel.transmit == 1 {
                    state.com1.active_khz
                } else {
                    state.com2.active_khz
                },
            );
            if capture_requested && !self.browser.requested() {
                self.browser_route = Some(route);
                if let Some(draft) = self.draft.take() {
                    let _ = s.run(CommsDiscardSpeech { id: draft.id });
                }
            }
            self.browser.observe(capture_requested, intercom, vox, generation);
            self.level = self.browser.level();
            if let Some((epoch, ics, result)) = self.browser.poll()
                && epoch == generation
            {
                if !ics && self.browser_route != Some(route) {
                    self.error = Some(
                        "COM selection changed during recognition. Say again on the selected COM.".into(),
                    );
                    return;
                }
                match result {
                    Ok(r) => {
                        let r = constrain_recognition(r, s, ics);
                        self.latency_ms = Some(r.latency_ms);
                        self.next_request = self.next_request.wrapping_add(1);
                        match s.run(CommsRecognize {
                            request_id: Some(self.next_request),
                            text: r.text,
                            confidence: r.confidence,
                            intercom: ics,
                        }) {
                            Ok(d) => self.draft = Some(d),
                            Err(e) => self.error = Some(command_message(e)),
                        }
                    }
                    Err(e) => self.error = Some(e),
                }
            }
        }
        // These state fields are also used by the native capture feature.
        let _ = (now, intercom, ptt, vox, capture_requested);
        #[cfg(not(target_arch = "wasm32"))]
        if self.recognizing || self.worker.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        if active && (ptt || vox) {
            ctx.request_repaint_after(std::time::Duration::from_millis(25));
        }
    }
    fn cancel(&mut self) {
        #[cfg(all(not(target_arch = "wasm32"), feature = "desktop-audio"))]
        {
            self.capture = None;
        }
        #[cfg(target_arch = "wasm32")]
        self.browser.cancel();
        self.held = false;
        self.level = 0.;
    }
    pub fn caption(&mut self, ui: &mut egui::Ui, s: &mut Session) {
        #[cfg(not(target_arch = "wasm32"))]
        if self.recognizing {
            ui.label("Recognizing your call. Release push-to-talk and wait.");
        }
        #[cfg(target_arch = "wasm32")]
        let listening = self.held || self.browser.requested();
        #[cfg(not(target_arch = "wasm32"))]
        let listening = self.held;
        if listening {
            ui.colored_label(
                theme::ACCENT,
                if self.intercom {
                    "Listening · intercom"
                } else {
                    "Listening · selected COM"
                },
            );
        }
        if let Some(draft) = &self.draft {
            ui.label(egui::RichText::new(format!("You said: {}", draft.recognized)).color(theme::TEXT));
            ui.label(&draft.prompt);
            let mut dismiss = false;
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !draft.retry && !self.held,
                        egui::Button::new("Transmit recognized call"),
                    )
                    .clicked()
                {
                    match s.run(CommsConfirmSpeech { id: draft.id }) {
                        Ok(_) => {
                            dismiss = true;
                            self.error = None;
                        }
                        Err(e) => self.error = Some(command_message(e)),
                    }
                }
                if ui.button("Retry").clicked() {
                    let _ = s.run(CommsDiscardSpeech { id: draft.id });
                    dismiss = true;
                }
            });
            if dismiss {
                self.draft = None;
            }
        }
        if let Some(error) = &self.error {
            ui.colored_label(theme::AMBER, error);
        }
    }
}

fn command_message(error: flightcraft_control::CommandError) -> String {
    match error {
        flightcraft_control::CommandError::InvalidInput { message, .. } => message,
        flightcraft_control::CommandError::Failed(message) => message,
        _ => "Unable to send this call. Check the audio panel and try again.".into(),
    }
}

#[cfg(all(not(target_arch = "wasm32"), any(feature = "desktop-audio", test)))]
fn recognition_context(s: &Session, intercom: bool) -> String {
    if intercom {
        return "Your controls. My controls. I have the controls. Go around.".into();
    }
    let state = s.comms_state();
    let khz = if state.panel.transmit == 1 {
        state.com1.active_khz
    } else {
        state.com2.active_khz
    };
    use flightcraft_control::comms::phraseology as p;
    let mut names: Vec<_> = state
        .stations
        .iter()
        .filter(|st| st.channel_khz == khz)
        .take(4)
        .map(p::facility)
        .collect();
    names.push(format!(
        "{} ({})",
        state.callsign.full, state.callsign.registration
    ));
    let atc = s.atc_state();
    if let Some(atis) = atc.atis {
        names.push(format!(
            "Runway {}. Information {}",
            p::runway(&atis.runway),
            p::normal_case(&atis.information)
        ));
    }
    // Only instructions actually heard on the radio enter previous-context tokens.
    for pending in atc
        .pending_instructions
        .iter()
        .filter(|p| p.heard && !p.readback)
        .take(2)
    {
        names.push(pending.phrase.clone());
    }
    format!(
        "{}. Radio: takeoff, closed traffic, crosswind, downwind, base, final, clear of runway.",
        names.join(". ")
    )
}

/// Choose only an actual acoustic hypothesis accepted by the grammar. Numeric
/// equality to a pending instruction is deliberately not part of this choice.
fn constrain_recognition(
    mut result: pipeline::Recognized,
    s: &Session,
    intercom: bool,
) -> pipeline::Recognized {
    let mut choices = vec![(result.text.clone(), result.confidence)];
    choices.extend(result.alternatives.iter().cloned());
    if let Some((text, confidence)) = choices
        .into_iter()
        .filter(|(text, confidence)| {
            *confidence >= 0.65 && flightcraft_control::normalize_speech(text, intercom, s).1
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
    {
        result.text = text;
        result.confidence = confidence;
    }
    result
}

/// Separate browser/native windows must not reuse each other's preview IDs.
/// Keep IDs below 2^52, leaving room for a counter within JavaScript's 53-bit
/// exact integer range when screen commands cross a SharedWorker boundary.
fn speech_request_seed() -> u64 {
    const MASK: u64 = (1 << 52) - 1;
    #[cfg(not(target_arch = "wasm32"))]
    let seed = {
        use std::sync::atomic::{AtomicU64, Ordering};
        static FALLBACK: AtomicU64 = AtomicU64::new(1);
        getrandom::u64().unwrap_or_else(|_| {
            (u64::from(std::process::id()) << 20) | FALLBACK.fetch_add(1, Ordering::Relaxed)
        }) & MASK
    };
    #[cfg(target_arch = "wasm32")]
    let seed = (js_sys::Math::random() * MASK as f64) as u64;
    seed.max(1)
}
