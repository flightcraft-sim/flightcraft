//! Presentation-only speech and original radio DSP; never called by headless simulation.
#[cfg(any(target_arch = "wasm32", feature = "desktop-audio"))]
use flightcraft_control::{Channel, Meaning};
use flightcraft_control::{Reception, Session, Speaker};
#[cfg(any(test, target_arch = "wasm32", feature = "desktop-audio"))]
mod dsp;
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
            // Two carriers keyed together are heard as a squeal, not as either voice. The user's own
            // overlapping call is never voiced.
            #[cfg(any(target_arch = "wasm32", feature = "desktop-audio"))]
            let blocked = event.reception == Reception::Blocked
                && event.channel == Channel::Com
                && event.speaker != Speaker::Pilot;
            #[cfg(not(any(target_arch = "wasm32", feature = "desktop-audio")))]
            let blocked = false;
            if !active
                || (event.reception != Reception::Heard && !blocked)
                || (event.speaker == Speaker::Pilot
                    && event.source != flightcraft_control::EventSource::Rules)
            {
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
                if matches!(event.meaning, Meaning::Crew { critical: true })
                    && let Some(output) = &mut self.output
                {
                    output.stop();
                }
                let volume = state.panel.volume as f32 * master_gain.clamp(0.0, 1.0);
                let voice = flightcraft_control::speaker_voice(&event.speaker, &event.meaning)
                    .unwrap_or_else(|| flightcraft_control::pilot_voice(0, 0));
                let distance = session.transmission_distance_fraction(event);
                if let Some(output) = &mut self.output {
                    let result = if blocked {
                        output.garble(event, volume)
                    } else {
                        output.say(event, volume, voice, distance)
                    };
                    if let Err(error) = result {
                        self.error = Some(error);
                    }
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
    #[test]
    fn morse_is_audible_tone() {
        let morse = morse_samples("... -... .---", 22050);
        assert!(morse.iter().any(|v| v.abs() > 0.2));
        assert!(morse.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
    }
}
