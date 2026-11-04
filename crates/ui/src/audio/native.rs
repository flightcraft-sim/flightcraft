use super::{AirflowNoise, Synth};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{
    Arc,
    atomic::{AtomicU32, AtomicU64, Ordering},
};

pub(super) struct Output {
    _stream: cpal::Stream,
    trigger: Arc<AtomicU64>,
    gain: Arc<AtomicU32>,
    noise: Arc<AtomicU32>,
}
impl Output {
    pub fn new() -> Result<Self, String> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("No audio output device")?;
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let trigger = Arc::new(AtomicU64::new(0));
        let gain = Arc::new(AtomicU32::new(0));
        let noise = Arc::new(AtomicU32::new(0));
        let stream = match format {
            cpal::SampleFormat::F32 => {
                stream::<f32>(&device, &config, trigger.clone(), gain.clone(), noise.clone())
            }
            cpal::SampleFormat::I16 => {
                stream::<i16>(&device, &config, trigger.clone(), gain.clone(), noise.clone())
            }
            cpal::SampleFormat::U16 => {
                stream::<u16>(&device, &config, trigger.clone(), gain.clone(), noise.clone())
            }
            _ => return Err("Unsupported audio output format".into()),
        }?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Self {
            _stream: stream,
            trigger,
            gain,
            noise,
        })
    }
    pub fn noise(&mut self, level: f32) {
        self.noise.store(level.to_bits(), Ordering::Relaxed);
    }
    pub fn volume(&mut self, gain: f32) {
        self.gain.store(gain.to_bits(), Ordering::Relaxed);
    }
    pub fn play(&mut self) -> Result<(), String> {
        self.trigger.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}
fn stream<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    trigger: Arc<AtomicU64>,
    gain: Arc<AtomicU32>,
    noise: Arc<AtomicU32>,
) -> Result<cpal::Stream, String> {
    let channels = usize::from(config.channels).max(1);
    let rate = f64::from(config.sample_rate.0);
    let mut synth = Synth {
        time: 1.0,
        phase: 0.0,
    };
    let mut sequence = 0;
    let mut airflow = AirflowNoise::default();
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                let current = trigger.load(Ordering::Relaxed);
                if current != sequence {
                    sequence = current;
                    synth.restart();
                }
                let volume = f32::from_bits(gain.load(Ordering::Relaxed));
                for frame in data.chunks_mut(channels) {
                    let sample = (synth.sample(rate)
                        + airflow.sample() * f32::from_bits(noise.load(Ordering::Relaxed)))
                        * volume;
                    for value in frame {
                        *value = T::from_sample(sample);
                    }
                }
            },
            |_| {},
            None,
        )
        .map_err(|e| e.to_string())
}
