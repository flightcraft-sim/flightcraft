//! A fixed-step clock that discards stalled time and never accumulates paused time.
use flightcraft_sim::DT_S;

#[derive(Default)]
pub struct FixedStep {
    remainder: f64,
}

impl FixedStep {
    pub fn advance(&mut self, elapsed_s: f64, paused: bool) -> u32 {
        if paused {
            self.remainder = 0.0;
            return 0;
        }
        if !elapsed_s.is_finite() || elapsed_s < 0.0 {
            return 0;
        }
        self.remainder += elapsed_s.min(0.25);
        let steps = ((self.remainder + 1e-12) / DT_S).floor() as u32;
        self.remainder = (self.remainder - f64::from(steps) * DT_S).max(0.0);
        steps
    }

    pub fn reset(&mut self) {
        self.remainder = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractional_frames_preserve_time() {
        let mut clock = FixedStep::default();
        let total: u32 = (0..144).map(|_| clock.advance(1.0 / 144.0, false)).sum();
        assert_eq!(total, 120);
        assert!(clock.remainder < 1e-10);
    }

    #[test]
    fn stalls_are_capped_and_not_replayed() {
        let mut clock = FixedStep::default();
        assert_eq!(clock.advance(20.0, false), 30);
        assert_eq!(clock.advance(0.0, false), 0);
    }

    #[test]
    fn pause_and_reset_discard_fractional_time() {
        let mut clock = FixedStep::default();
        assert_eq!(clock.advance(DT_S * 0.75, false), 0);
        assert_eq!(clock.advance(10.0, true), 0);
        assert_eq!(clock.advance(DT_S * 0.5, false), 0);
        clock.reset();
        assert_eq!(clock.advance(DT_S * 0.5, false), 0);
    }

    #[test]
    fn invalid_deltas_do_not_poison_clock() {
        let mut clock = FixedStep::default();
        for elapsed in [f64::NAN, f64::INFINITY, -1.0] {
            assert_eq!(clock.advance(elapsed, false), 0);
        }
        assert_eq!(clock.advance(DT_S, false), 1);
    }
}
