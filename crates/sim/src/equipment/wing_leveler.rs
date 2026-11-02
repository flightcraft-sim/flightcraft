//! Electric rate-sensing roll servo with a bounded turn knob.
use super::Loop;
use crate::DT_S;

pub(super) fn roll(loop_: &mut Loop, sensed_turn: f64, desired_turn: f64) -> f64 {
    loop_.pi(desired_turn - sensed_turn, 3.0, 0.3, 0.8)
}
pub(super) fn slew(previous: f64, target: f64, rate: f64) -> f64 {
    previous + (target - previous).clamp(-rate * DT_S, rate * DT_S)
}
