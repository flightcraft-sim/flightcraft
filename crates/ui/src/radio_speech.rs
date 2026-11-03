//! Presentation-only speech and original radio DSP; never called by headless simulation.
#[cfg(all(not(target_arch = "wasm32"), feature = "desktop-audio"))]
use flightcraft_control::Channel;
#[cfg(any(target_arch = "wasm32", feature = "desktop-audio"))]
use flightcraft_control::Meaning;
use flightcraft_control::{Reception, Session, Speaker};
#[cfg(all(not(target_arch = "wasm32"), feature = "desktop-audio"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;
#[derive(Default)]
pub struct Speech {
    generation: Option<u64>,
    last: u64,
    error: Option<String>,
    #[cfg(all(not(target_arch = "wasm32"), feature = "desktop-audio"))]
    output: Option<native::Output>,
    #[cfg(target_arch = "wasm32")]
    output: Option<web::Output>,
}
impl Speech {
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn observe(&mut self, session: &Session, master_gain: f32) {
        let state = session.comms_state();
        let changed_flight = self.generation != Some(session.flight_generation());
        if changed_flight {
            self.generation = Some(session.flight_generation());
            self.last = 0;
        }
        let active = master_gain.is_finite()
            && master_gain > 0.0
            && state.panel.speech
            && state.panel.volume > 0.0
            && state.powered;
        #[cfg(any(target_arch = "wasm32", feature = "desktop-audio"))]
        {
            if let Some(output) = &mut self.output {
                if !active || changed_flight {
                    output.stop();
                }
                output.poll(active, &mut self.error);
            }
        }
        for event in session
            .flight_record()
            .events
            .iter()
            .take_while(|e| e.reception != Reception::Pending)
            .filter(|e| e.id > self.last)
        {
            if !event.is_spoken() {
                continue;
            }
            if !active || event.reception != Reception::Heard || event.speaker == Speaker::Pilot {
                continue;
            }
            #[cfg(any(target_arch = "wasm32", feature = "desktop-audio"))]
            {
                if self.output.is_none() {
                    #[cfg(target_arch = "wasm32")]
                    let result = web::Output::new();
                    #[cfg(all(not(target_arch = "wasm32"), feature = "desktop-audio"))]
                    let result = native::Output::new();
                    match result {
                        Ok(output) => self.output = Some(output),
                        Err(error) => {
                            self.error = Some(error);
                            continue;
                        }
                    }
                }
                if let Some(output) = &mut self.output
                    && let Err(error) =
                        output.say(event, state.panel.volume as f32 * master_gain.clamp(0.0, 1.0))
                {
                    self.error = Some(error);
                }
            }
            #[cfg(not(any(target_arch = "wasm32", feature = "desktop-audio")))]
            {
                self.error =
                    Some("Speech output needs the desktop audio feature; captions remain available.".into());
            }
        }
        // A later short event may complete before an earlier long one. Only advance a contiguous cursor.
        self.last = session
            .flight_record()
            .events
            .iter()
            .take_while(|e| e.reception != Reception::Pending)
            .last()
            .map_or(self.last, |e| e.id);
    }
}
#[cfg(any(test, target_arch = "wasm32", feature = "desktop-audio"))]
fn role(speaker: &Speaker) -> usize {
    match speaker {
        Speaker::Pilot => 0,
        Speaker::Instructor => 1,
        Speaker::Facility { role, .. } => match role {
            flightcraft_world::FacilityKind::Tower => 2,
            flightcraft_world::FacilityKind::Ground => 3,
            flightcraft_world::FacilityKind::Approach | flightcraft_world::FacilityKind::Departure => 4,
            _ => 5,
        },
        Speaker::Traffic { callsign } => {
            6 + callsign
                .bytes()
                .fold(0_usize, |n, b| n.wrapping_add(usize::from(b)))
                % 3
        }
        Speaker::Nav { .. } => 9,
    }
}
/// One-pole high/low cut, soft compression, deterministic quiet static, and a 120 ms squelch tail.
#[cfg(any(test, all(not(target_arch = "wasm32"), feature = "desktop-audio")))]
fn radio_effect(samples: &mut Vec<f32>, rate: u32) {
    let r = f64::from(rate.max(8000));
    let high = libm::exp(-core::f64::consts::TAU * 300.0 / r) as f32;
    let low = 1.0 - libm::exp(-core::f64::consts::TAU * 3000.0 / r) as f32;
    let mut hp = 0.0;
    let mut lp = 0.0;
    let mut previous = 0.0;
    for (i, value) in samples.iter_mut().enumerate() {
        let input = if value.is_finite() { *value } else { 0.0 };
        hp = high * (hp + input - previous);
        previous = input;
        lp += low * (hp - lp);
        let compressed = lp * 2.5 / (1.0 + (lp * 2.5).abs());
        let noise = static_sample(i) * 0.006;
        *value = (compressed + noise).clamp(-1.0, 1.0);
    }
    for i in 0..rate * 12 / 100 {
        samples.push(static_sample(i as usize) * 0.025 * (1.0 - i as f32 / (rate as f32 * 0.12)).max(0.0));
    }
}
#[cfg(any(test, all(not(target_arch = "wasm32"), feature = "desktop-audio")))]
fn static_sample(index: usize) -> f32 {
    let mut bits = (index as u32).wrapping_mul(0x9e3779b9).wrapping_add(0x7f4a7c15);
    bits ^= bits >> 16;
    bits = bits.wrapping_mul(0x85ebca6b);
    bits ^= bits >> 13;
    (bits & 65535) as f32 / 32767.5 - 1.0
}
#[cfg(any(test, target_arch = "wasm32", feature = "desktop-audio"))]
fn morse_samples(code: &str, rate: u32) -> Vec<f32> {
    let mut result = Vec::new();
    for symbol in code.chars() {
        let units = match symbol {
            '.' => 1,
            '-' => 3,
            ' ' => 2,
            _ => 0,
        };
        let len = rate * units / 10;
        for i in 0..len {
            let envelope = (i as f32 / (rate as f32 * 0.004)).min(1.0)
                * ((len - i) as f32 / (rate as f32 * 0.004)).min(1.0);
            result.push(if symbol == ' ' {
                0.0
            } else {
                (libm::sin(f64::from(i) / f64::from(rate) * core::f64::consts::TAU * 1020.0) as f32)
                    * envelope
                    * 0.25
            });
        }
        if symbol != ' ' {
            result.extend(std::iter::repeat_n(0.0, (rate / 10) as usize));
        }
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    fn rms(v: &[f32]) -> f64 {
        libm::sqrt(v.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / v.len().max(1) as f64)
    }
    #[test]
    fn effect_rejects_dc_and_out_of_band_and_adds_bounded_tail() {
        let make = |hz: f64| {
            (0..22050)
                .map(|i| (libm::sin(f64::from(i) / 22050.0 * core::f64::consts::TAU * hz) * 0.1) as f32)
                .collect::<Vec<_>>()
        };
        let mut low = make(50.0);
        let mut mid = make(1000.0);
        let mut high = make(9000.0);
        radio_effect(&mut low, 22050);
        radio_effect(&mut mid, 22050);
        radio_effect(&mut high, 22050);
        assert!(rms(&mid) > rms(&low) * 2.0);
        assert!(rms(&mid) > rms(&high) * 1.7);
        assert_eq!(mid.len(), 22050 + 2646);
        assert!(mid.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
        let morse = morse_samples("... -... .---", 22050);
        assert!(morse.iter().any(|v| v.abs() > 0.2));
        assert!(
            role(&Speaker::Instructor)
                != role(&Speaker::Traffic {
                    callsign: "Trainer 1".into()
                })
        );
    }
}
