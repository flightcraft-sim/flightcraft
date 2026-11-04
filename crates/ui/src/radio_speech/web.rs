use super::dsp::{RadioParams, blocked_garble, radio_effect};
use super::*;
use flightcraft_control::{Gender, VoiceProfile};
use wasm_bindgen::JsCast;
use web_sys::{AudioContext, SpeechSynthesis, SpeechSynthesisUtterance};
/// A call being spoken by `speechSynthesis`, kept so it can be re-issued when another call is cut.
struct Spoken {
    event: u64,
    text: String,
    voice: VoiceProfile,
    intercom: bool,
    volume: f32,
    /// Wall-clock millisecond the (remaining) text was handed to the engine.
    started_ms: f64,
    /// Wall-clock millisecond at which the voice must be silent: the end of its carrier. `speechSynthesis`
    /// cannot be time-fitted, so it is cut here instead of running into the next call.
    deadline_ms: Option<f64>,
}
impl Spoken {
    /// Browser rate 1.0 is about 175 words a minute.
    fn rate(&self) -> f32 {
        (f32::from(self.voice.wpm) / 175.0).clamp(0.6, 1.6)
    }
    /// The text not yet spoken `now_ms`, estimated from elapsed time at the utterance's rate. The browser
    /// reports no progress for a whole utterance, so this is only an estimate.
    fn remaining_text(&self, now_ms: f64) -> String {
        let words_per_s = f64::from(self.rate()) * 175.0 / 60.0;
        let spoken = ((now_ms - self.started_ms).max(0.0) / 1000.0 * words_per_s) as usize;
        self.text
            .split_whitespace()
            .skip(spoken)
            .collect::<Vec<_>>()
            .join(" ")
    }
}
pub(super) struct Output {
    speech: SpeechSynthesis,
    audio: AudioContext,
    /// Radio-bed and tone sources, with the call each belongs to.
    tones: Vec<(u64, web_sys::AudioBufferSourceNode)>,
    spoken: Vec<Spoken>,
}
impl Output {
    pub fn new() -> Result<Self, String> {
        let speech = web_sys::window()
            .ok_or("No browser window")?
            .speech_synthesis()
            .map_err(|_| "Browser speech synthesis unavailable")?;
        let audio = AudioContext::new().map_err(|_| "Browser audio unavailable")?;
        Ok(Self {
            speech,
            audio,
            tones: Vec::new(),
            spoken: Vec::new(),
        })
    }
    pub fn stop(&mut self) {
        self.speech.cancel();
        self.spoken.clear();
        for (_, tone) in self.tones.drain(..) {
            let _ = tone.unchecked_ref::<web_sys::AudioScheduledSourceNode>().stop();
        }
    }
    /// Silences one call. `speechSynthesis` cannot cancel a single utterance, so any other call still
    /// being spoken is cancelled with it and re-issued with the text it has left (estimated from elapsed
    /// time); it may repeat or skip a word at the join. The call's radio bed is stopped exactly.
    pub fn stop_call(&mut self, event: u64) {
        self.tones.retain(|(id, tone)| {
            if *id == event {
                let _ = tone.unchecked_ref::<web_sys::AudioScheduledSourceNode>().stop();
            }
            *id != event
        });
        if !self.spoken.iter().any(|s| s.event == event) {
            return;
        }
        self.spoken.retain(|s| s.event != event);
        self.speech.cancel();
        let now = js_sys::Date::now();
        let others = std::mem::take(&mut self.spoken);
        for mut other in others {
            if other.deadline_ms.is_some_and(|at| at <= now) {
                continue;
            }
            let text = other.remaining_text(now);
            if text.is_empty() {
                continue;
            }
            other.text = text;
            other.started_ms = now;
            if self.speak(&other).is_ok() {
                self.spoken.push(other);
            }
        }
    }
    pub fn poll(&mut self, _: bool, _: &mut Option<String>) {
        let now = js_sys::Date::now();
        if self
            .spoken
            .iter()
            .any(|s| s.deadline_ms.is_some_and(|at| now >= at))
        {
            self.spoken.retain(|s| s.deadline_ms.is_none_or(|at| now < at));
            // A single utterance cannot be cut while another is speaking; with nothing else live, cut.
            if self.spoken.is_empty() {
                self.speech.cancel();
            }
        }
    }
    /// Hands one utterance to the engine.
    fn speak(&self, spoken: &Spoken) -> Result<(), String> {
        let utterance = SpeechSynthesisUtterance::new_with_text(&spoken.text)
            .map_err(|_| "Cannot create speech utterance")?;
        let voices = self.speech.get_voices();
        let english: Vec<web_sys::SpeechSynthesisVoice> = (0..voices.length())
            .filter_map(|i| voices.get(i).dyn_into::<web_sys::SpeechSynthesisVoice>().ok())
            .filter(|v| v.lang().to_lowercase().starts_with("en"))
            .collect();
        // The browser does not say which installed voices are male or female, so spread the profile's
        // gender, timbre, and pitch over the English voices; the pitch and rate below do the rest.
        let voice = &spoken.voice;
        if !english.is_empty() {
            let slot = usize::from(voice.variant) * 2
                + usize::from(voice.gender == Gender::Female)
                + usize::from(voice.pitch) / 20;
            utterance.set_voice(english.get(slot % english.len()));
        }
        utterance.set_rate(spoken.rate());
        utterance.set_pitch(if spoken.intercom {
            1.05
        } else {
            (0.55 + f32::from(voice.pitch) / 100.0 * 0.95).clamp(0.5, 1.6)
        });
        utterance.set_volume(spoken.volume);
        self.speech.speak(&utterance);
        Ok(())
    }
    /// Plays mono samples at the context rate, keeping a bounded list so `stop` can silence them.
    fn play(&mut self, event: u64, samples: &[f32], rate: u32) -> Result<(), String> {
        let buffer = self
            .audio
            .create_buffer(1, samples.len() as u32, rate as f32)
            .map_err(|_| "Cannot create radio audio")?;
        buffer
            .copy_to_channel(samples, 0)
            .map_err(|_| "Cannot fill radio audio")?;
        let source = self
            .audio
            .create_buffer_source()
            .map_err(|_| "Cannot create radio source")?;
        source.set_buffer(Some(&buffer));
        source
            .connect_with_audio_node(&self.audio.destination())
            .map_err(|_| "Cannot connect radio audio")?;
        let _ = self.audio.resume();
        source.start().map_err(|_| "Cannot play radio audio")?;
        if self.tones.len() >= 16 {
            if let Some((_, tone)) = self.tones.first() {
                let _ = tone.unchecked_ref::<web_sys::AudioScheduledSourceNode>().stop();
            }
            self.tones.remove(0);
        }
        self.tones.push((event, source));
        Ok(())
    }
    /// Two carriers keyed together: the heterodyne squeal.
    pub fn garble(
        &mut self,
        event: &flightcraft_control::Transmission,
        seconds: f32,
        volume: f32,
    ) -> Result<(), String> {
        let rate = self.audio.sample_rate() as u32;
        let samples = blocked_garble(seconds, rate, event.id as u32)
            .into_iter()
            .map(|v| v * volume)
            .collect::<Vec<_>>();
        self.play(event.id, &samples, rate)
    }
    pub fn say(
        &mut self,
        event: &flightcraft_control::Transmission,
        volume: f32,
        voice: VoiceProfile,
        distance: f32,
    ) -> Result<(), String> {
        let rate = self.audio.sample_rate() as u32;
        if let Meaning::Ident { morse, .. } = &event.meaning {
            let samples = morse_samples(morse, rate)
                .into_iter()
                .map(|v| v * volume)
                .collect::<Vec<_>>();
            return self.play(event.id, &samples, rate);
        }
        let spoken = Spoken {
            event: event.id,
            text: event.text.clone(),
            voice,
            intercom: event.channel == flightcraft_control::Channel::Intercom,
            volume,
            started_ms: js_sys::Date::now(),
            // A radio call is on the air now; its voice must be silent when the carrier drops.
            deadline_ms: (event.channel == flightcraft_control::Channel::Com)
                .then(|| js_sys::Date::now() + event.duration_s * 1000.0),
        };
        self.speak(&spoken)?;
        self.spoken.push(spoken);
        if event.channel == flightcraft_control::Channel::Com {
            // The browser keeps speech out of WebAudio, so the voice itself is unfiltered. The radio's
            // squelch break, hiss rising with distance, and squelch tail play alongside it from the same
            // parameters as the desktop, timed to the call's duration.
            let mut bed = vec![0.0; (event.duration_s as f32 * rate as f32) as usize];
            radio_effect(
                &mut bed,
                rate,
                &RadioParams {
                    radio: voice.radio,
                    distance,
                    seed: event.id as u32,
                },
            );
            for v in &mut bed {
                *v *= volume * 0.6;
            }
            self.play(event.id, &bed, rate)?;
        }
        Ok(())
    }
}
