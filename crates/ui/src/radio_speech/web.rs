use super::*;
use wasm_bindgen::JsCast;
use web_sys::{AudioContext, SpeechSynthesis, SpeechSynthesisUtterance};
pub(super) struct Output {
    speech: SpeechSynthesis,
    audio: AudioContext,
    tones: Vec<web_sys::AudioBufferSourceNode>,
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
        })
    }
    pub fn stop(&mut self) {
        self.speech.cancel();
        for tone in self.tones.drain(..) {
            let _ = tone.unchecked_ref::<web_sys::AudioScheduledSourceNode>().stop();
        }
    }
    pub fn poll(&mut self, _: bool, _: &mut Option<String>) {}
    pub fn say(&mut self, event: &flightcraft_control::Transmission, volume: f32) -> Result<(), String> {
        if let Meaning::Ident { morse, .. } = &event.meaning {
            let rate = self.audio.sample_rate() as u32;
            let samples = morse_samples(morse, rate)
                .into_iter()
                .map(|v| v * volume)
                .collect::<Vec<_>>();
            let buffer = self
                .audio
                .create_buffer(1, samples.len() as u32, rate as f32)
                .map_err(|_| "Cannot create ident audio")?;
            buffer
                .copy_to_channel(&samples, 0)
                .map_err(|_| "Cannot fill ident audio")?;
            let source = self
                .audio
                .create_buffer_source()
                .map_err(|_| "Cannot create ident source")?;
            source.set_buffer(Some(&buffer));
            source
                .connect_with_audio_node(&self.audio.destination())
                .map_err(|_| "Cannot connect ident audio")?;
            let _ = self.audio.resume();
            source.start().map_err(|_| "Cannot play ident audio")?;
            if self.tones.len() >= 16 {
                if let Some(tone) = self.tones.first() {
                    let _ = tone.unchecked_ref::<web_sys::AudioScheduledSourceNode>().stop();
                }
                self.tones.remove(0);
            }
            self.tones.push(source);
            return Ok(());
        }
        let utterance = SpeechSynthesisUtterance::new_with_text(&event.text)
            .map_err(|_| "Cannot create speech utterance")?;
        let number = role(&event.speaker);
        let voices = self.speech.get_voices();
        if voices.length() > 0
            && let Ok(voice) = voices
                .get(number as u32 % voices.length())
                .dyn_into::<web_sys::SpeechSynthesisVoice>()
        {
            utterance.set_voice(Some(&voice));
        }
        utterance.set_rate(0.9 + (number % 4) as f32 * 0.045);
        utterance.set_pitch(0.8 + (number % 5) as f32 * 0.1);
        utterance.set_volume(volume);
        self.speech.speak(&utterance);
        Ok(())
    }
}
