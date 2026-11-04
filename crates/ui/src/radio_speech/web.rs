use super::dsp::{RadioParams, blocked_garble, radio_effect};
use super::*;
use flightcraft_control::{Gender, VoiceProfile};
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
    /// Plays mono samples at the context rate, keeping a bounded list so `stop` can silence them.
    fn play(&mut self, samples: &[f32], rate: u32) -> Result<(), String> {
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
            if let Some(tone) = self.tones.first() {
                let _ = tone.unchecked_ref::<web_sys::AudioScheduledSourceNode>().stop();
            }
            self.tones.remove(0);
        }
        self.tones.push(source);
        Ok(())
    }
    /// Two carriers keyed together: the heterodyne squeal.
    pub fn garble(&mut self, event: &flightcraft_control::Transmission, volume: f32) -> Result<(), String> {
        let rate = self.audio.sample_rate() as u32;
        let samples = blocked_garble(event.duration_s as f32, rate, event.id as u32)
            .into_iter()
            .map(|v| v * volume)
            .collect::<Vec<_>>();
        self.play(&samples, rate)
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
            return self.play(&samples, rate);
        }
        let utterance = SpeechSynthesisUtterance::new_with_text(&event.text)
            .map_err(|_| "Cannot create speech utterance")?;
        let voices = self.speech.get_voices();
        let english: Vec<web_sys::SpeechSynthesisVoice> = (0..voices.length())
            .filter_map(|i| voices.get(i).dyn_into::<web_sys::SpeechSynthesisVoice>().ok())
            .filter(|v| v.lang().to_lowercase().starts_with("en"))
            .collect();
        // The browser does not say which installed voices are male or female, so spread the profile's
        // gender, timbre, and pitch over the English voices; the pitch and rate below do the rest.
        if !english.is_empty() {
            let slot = usize::from(voice.variant) * 2
                + usize::from(voice.gender == Gender::Female)
                + usize::from(voice.pitch) / 20;
            utterance.set_voice(english.get(slot % english.len()));
        }
        let intercom = event.channel == flightcraft_control::Channel::Intercom;
        // Browser rate 1.0 is about 175 words a minute.
        utterance.set_rate((f32::from(voice.wpm) / 175.0).clamp(0.6, 1.6));
        utterance.set_pitch(if intercom {
            1.05
        } else {
            (0.55 + f32::from(voice.pitch) / 100.0 * 0.95).clamp(0.5, 1.6)
        });
        utterance.set_volume(volume);
        self.speech.speak(&utterance);
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
            self.play(&bed, rate)?;
        }
        Ok(())
    }
}
