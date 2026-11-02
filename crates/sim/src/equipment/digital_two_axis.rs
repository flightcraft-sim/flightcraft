//! Attitude flight director, pitch servo, and envelope commands.
use super::Loop;

pub(super) fn elevator(loop_: &mut Loop, desired_pitch: f64, pitch: f64, pitch_rate: f64) -> f64 {
    loop_.pi(desired_pitch - pitch, 2.2, 0.15, 0.7) - 0.8 * pitch_rate
}
/// Envelope protection overrides vertical guidance while retaining the selected mode.
pub(super) fn protected_vs(ias: f64, minimum: f64, maximum: f64, vs: f64) -> (f64, bool, bool) {
    if ias < minimum {
        ((ias - minimum) * 0.9 - 1.0, true, false)
    } else if ias > maximum {
        ((ias - maximum) * 0.9 + 1.0, false, true)
    } else {
        (vs, false, false)
    }
}
