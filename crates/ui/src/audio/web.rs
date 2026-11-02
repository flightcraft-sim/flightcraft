use super::Synth;
use web_sys::{AudioContext, GainNode};
pub(super) struct Output {
    context: AudioContext,
    gain: GainNode,
}
impl Output {
    pub fn new() -> Result<Self, String> {
        let context = AudioContext::new().map_err(|_| "Audio output unavailable")?;
        let gain = context.create_gain().map_err(|_| "Cannot create audio gain")?;
        gain.connect_with_audio_node(&context.destination())
            .map_err(|_| "Cannot connect audio output")?;
        Ok(Self { context, gain })
    }
    pub fn resume(&self) {
        let _ = self.context.resume();
    }
    pub fn volume(&mut self, gain: f32) {
        self.gain.gain().set_value(gain);
    }
    pub fn play(&mut self) -> Result<(), String> {
        self.resume();
        let mut synth = Synth::default();
        synth.restart();
        let samples: Vec<f32> = (0..44100).map(|_| synth.sample(44100.0)).collect();
        let buffer = self
            .context
            .create_buffer(1, 44100, 44100.0)
            .map_err(|_| "Cannot create warning buffer")?;
        buffer
            .copy_to_channel(&samples, 0)
            .map_err(|_| "Cannot fill warning buffer")?;
        let source = self
            .context
            .create_buffer_source()
            .map_err(|_| "Cannot create warning source")?;
        source.set_buffer(Some(&buffer));
        source
            .connect_with_audio_node(&self.gain)
            .map_err(|_| "Cannot connect warning")?;
        source.start().map_err(|_| "Cannot play warning".into())
    }
}
