//! Optional output devices consume an original synthesised warning; headless simulation has no audio.
use serde::{Deserialize, Serialize};
#[cfg(all(not(target_arch = "wasm32"), feature = "desktop-audio"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Preferences {
    pub muted: bool,
    pub volume: f32,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            muted: false,
            volume: 0.4,
        }
    }
}
impl Preferences {
    pub fn gain(self) -> f32 {
        if self.muted || !self.volume.is_finite() {
            0.0
        } else {
            self.volume.clamp(0.0, 1.0)
        }
    }
}
/// A one-second wailing two-tone with tapered ends, generated without recorded assets.
#[cfg(any(test, target_arch = "wasm32", feature = "desktop-audio"))]
#[derive(Default)]
pub(super) struct Synth {
    time: f64,
    phase: f64,
}
#[cfg(any(test, target_arch = "wasm32", feature = "desktop-audio"))]
impl Synth {
    fn restart(&mut self) {
        self.time = 0.0;
        self.phase = 0.0;
    }
    fn sample(&mut self, rate: f64) -> f32 {
        if !rate.is_finite() || rate < 1.0 || self.time >= 1.0 {
            return 0.0;
        }
        let base = if (libm::floor(self.time / 0.18) as u32).is_multiple_of(2) {
            880.0
        } else {
            660.0
        };
        let frequency = base + 90.0 * libm::sin(self.time * core::f64::consts::TAU * 5.0);
        self.phase =
            (self.phase + frequency * core::f64::consts::TAU / rate).rem_euclid(core::f64::consts::TAU);
        let envelope = (self.time / 0.015).min(1.0) * ((1.0 - self.time) / 0.08).clamp(0.0, 1.0);
        self.time += 1.0 / rate;
        (libm::sin(self.phase) * envelope * 0.3) as f32
    }
}
#[derive(Default)]
pub struct Audio {
    pub preferences: Preferences,
    pub error: Option<String>,
    last_sequence: u64,
    generation: Option<u64>,
    #[cfg(all(not(target_arch = "wasm32"), feature = "desktop-audio"))]
    backend: Option<native::Output>,
    #[cfg(target_arch = "wasm32")]
    backend: Option<web::Output>,
}
impl Audio {
    pub fn with_preferences(preferences: Preferences) -> Self {
        Self {
            preferences,
            ..Default::default()
        }
    }
    /// Browsers require output activation while processing a user gesture.
    pub fn user_gesture(&mut self) {
        #[cfg(target_arch = "wasm32")]
        if self.preferences.gain() > 0.0 {
            if self.backend.is_none() {
                match web::Output::new() {
                    Ok(output) => self.backend = Some(output),
                    Err(error) => self.error = Some(error),
                }
            }
            if let Some(output) = &self.backend {
                output.resume();
            }
        }
    }
    pub fn observe(&mut self, generation: u64, sequence: u64) {
        if self.generation != Some(generation) {
            self.generation = Some(generation);
            self.last_sequence = 0;
        }
        let changed = sequence != self.last_sequence;
        self.last_sequence = sequence;
        let gain = self.preferences.gain();
        #[cfg(any(target_arch = "wasm32", feature = "desktop-audio"))]
        {
            if changed && gain > 0.0 && self.backend.is_none() {
                #[cfg(target_arch = "wasm32")]
                let result = web::Output::new();
                #[cfg(all(not(target_arch = "wasm32"), feature = "desktop-audio"))]
                let result = native::Output::new();
                match result {
                    Ok(output) => {
                        self.backend = Some(output);
                        self.error = None;
                    }
                    Err(e) => self.error = Some(e),
                }
            }
            if let Some(output) = &mut self.backend {
                output.volume(gain);
                if changed
                    && gain > 0.0
                    && let Err(e) = output.play()
                {
                    self.error = Some(e);
                }
            }
        }
        #[cfg(not(any(target_arch = "wasm32", feature = "desktop-audio")))]
        {
            let _ = (changed, gain);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn warning_is_finite_tapered_and_silent_after_one_second() {
        let mut synth = Synth::default();
        synth.restart();
        assert_eq!(synth.sample(48000.0), 0.0);
        let mut peak = 0.0_f32;
        for _ in 0..48000 {
            let sample = synth.sample(48000.0);
            assert!(sample.is_finite());
            peak = peak.max(sample.abs());
        }
        assert!(peak > 0.25 && peak <= 0.301);
        assert_eq!(synth.sample(48000.0), 0.0);
        assert_eq!(
            Preferences {
                muted: true,
                volume: 1.0
            }
            .gain(),
            0.0
        );
        assert_eq!(
            Preferences {
                muted: false,
                volume: f32::NAN
            }
            .gain(),
            0.0
        );
    }
}
