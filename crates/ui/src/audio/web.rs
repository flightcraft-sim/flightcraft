use super::{AirflowNoise, Synth};
use web_sys::{AudioContext, GainNode};
pub(super) struct Output {
    context: AudioContext,
    gain: GainNode,
    noise_gain: GainNode,
    _noise_source: web_sys::AudioBufferSourceNode,
}
impl Output {
    pub fn new() -> Result<Self, String> {
        let context = AudioContext::new().map_err(|_| "Audio output unavailable")?;
        let gain = context.create_gain().map_err(|_| "Cannot create audio gain")?;
        gain.connect_with_audio_node(&context.destination())
            .map_err(|_| "Cannot connect audio output")?;
        let noise_gain = context.create_gain().map_err(|_| "Cannot create airflow gain")?;
        noise_gain.gain().set_value(0.);
        noise_gain
            .connect_with_audio_node(&gain)
            .map_err(|_| "Cannot connect airflow")?;
        let mut noise = AirflowNoise::default();
        let samples: Vec<f32> = (0..44100).map(|_| noise.sample()).collect();
        let buffer = context
            .create_buffer(1, 44100, 44100.)
            .map_err(|_| "Cannot create airflow buffer")?;
        buffer
            .copy_to_channel(&samples, 0)
            .map_err(|_| "Cannot fill airflow buffer")?;
        let source = context
            .create_buffer_source()
            .map_err(|_| "Cannot create airflow source")?;
        source.set_buffer(Some(&buffer));
        source.set_loop(true);
        source
            .connect_with_audio_node(&noise_gain)
            .map_err(|_| "Cannot connect airflow source")?;
        source.start().map_err(|_| "Cannot start airflow")?;
        Ok(Self {
            context,
            gain,
            noise_gain,
            _noise_source: source,
        })
    }
    pub fn resume(&self) {
        let _ = self.context.resume();
    }
    pub fn noise(&mut self, level: f32) {
        self.noise_gain.gain().set_value(level);
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
