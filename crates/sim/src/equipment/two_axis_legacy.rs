//! Turn-rate lateral control and pressure-rate pitch control; no attitude feedback.
use super::Loop;

pub(super) fn bank_from_rate(turn_rate: f64, speed: f64) -> f64 {
    libm::atan(turn_rate * speed.max(20.0) / flightcraft_world::GRAVITY_MPS2)
}
pub(super) fn elevator(loop_: &mut Loop, desired_vs: f64, sensed_vs: f64, acceleration: f64) -> f64 {
    loop_.pi(desired_vs - sensed_vs, 0.03, 0.006, 0.7) - 0.025 * acceleration
}
