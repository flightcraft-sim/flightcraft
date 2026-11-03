//! A native input stream exists only during PTT or explicitly enabled intercom VOX.
use super::pipeline::AudioClip;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};
#[derive(Default)]
struct Buffer {
    samples: Vec<f32>,
    level: f32,
    error: Option<String>,
    overflow: bool,
}
pub struct Capture {
    stream: cpal::Stream,
    buffer: Arc<Mutex<Buffer>>,
    rate: u32,
}
pub fn devices() -> Result<Vec<String>, String> {
    cpal::default_host()
        .input_devices()
        .map_err(|e| e.to_string())?
        .map(|d| d.name().map_err(|e| e.to_string()))
        .collect()
}
impl Capture {
    pub fn start(selected: &str) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = if selected.is_empty() {
            host.default_input_device()
        } else {
            host.input_devices()
                .map_err(|e| e.to_string())?
                .find(|d| d.name().ok().as_deref() == Some(selected))
        }
        .ok_or("Microphone unavailable. Select an input in Preferences.")?;
        let config = device.default_input_config().map_err(|e| e.to_string())?;
        let rate = config.sample_rate().0;
        let buffer = Arc::new(Mutex::new(Buffer::default()));
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => build::<f32>(&device, &config.into(), buffer.clone()),
            cpal::SampleFormat::I16 => build::<i16>(&device, &config.into(), buffer.clone()),
            cpal::SampleFormat::U16 => build::<u16>(&device, &config.into(), buffer.clone()),
            _ => Err("The microphone sample format is unsupported.".into()),
        }?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Self { stream, buffer, rate })
    }
    pub fn level(&self) -> f32 {
        self.buffer.lock().map_or(0., |b| b.level)
    }
    pub fn take(&mut self) -> Result<AudioClip, String> {
        let mut buffer = self.buffer.lock().map_err(|_| "Microphone buffer unavailable.")?;
        if let Some(error) = buffer.error.take() {
            return Err(error);
        }
        if buffer.overflow {
            return Err("Recording exceeded 30 seconds. Release PTT and retry a shorter call.".into());
        }
        Ok(AudioClip {
            samples: std::mem::take(&mut buffer.samples),
            rate: self.rate,
        })
    }
    pub fn finish(mut self) -> Result<AudioClip, String> {
        // Drop the device before handing audio to the recognizer.
        let _ = self.stream.pause();
        self.take()
    }
}
fn build<T: cpal::SizedSample + cpal::Sample>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    buffer: Arc<Mutex<Buffer>>,
) -> Result<cpal::Stream, String>
where
    f32: cpal::FromSample<T>,
{
    let channels = usize::from(config.channels).max(1);
    let max = config.sample_rate.0 as usize * 30;
    let errors = buffer.clone();
    device
        .build_input_stream(
            config,
            move |input: &[T], _| {
                if let Ok(mut b) = buffer.lock() {
                    b.level = 0.;
                    for frame in input.chunks_exact(channels) {
                        let sample =
                            frame.iter().map(|v| v.to_sample::<f32>()).sum::<f32>() / channels as f32;
                        if sample.is_finite() {
                            b.level = b.level.max(sample.abs());
                            if b.samples.len() < max {
                                b.samples.push(sample.clamp(-1., 1.));
                            } else {
                                b.overflow = true;
                            }
                        }
                    }
                }
            },
            move |error| {
                if let Ok(mut b) = errors.lock() {
                    b.error = Some(format!("Microphone disconnected: {error}"));
                }
            },
            None,
        )
        .map_err(|e| e.to_string())
}
