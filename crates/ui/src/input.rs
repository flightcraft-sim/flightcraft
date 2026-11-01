//! Keyboard and gilrs feed the same named action values and edge events.
use crate::keymap::{Action, Keymap};
use eframe::egui;
use flightcraft_sim::Controls;
use gilrs::{Axis, Button, EventType, Gilrs};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub struct ActionFrame {
    pub held: BTreeMap<Action, f64>,
    pub pressed: BTreeSet<Action>,
    pub axes: BTreeMap<Action, f64>,
}

impl ActionFrame {
    fn value(&self, action: Action) -> f64 {
        self.held.get(&action).copied().unwrap_or_default()
    }

    pub fn cockpit_look(&self) -> [f64; 2] {
        [
            self.value(Action::LookRight) - self.value(Action::LookLeft),
            self.value(Action::LookUp),
        ]
    }

    pub fn pressed(&self, action: Action) -> bool {
        self.pressed.contains(&action)
    }
}

pub struct Input {
    gamepad: Option<Gilrs>,
    pub gamepad_status: String,
    pub trim: f64,
}

impl Default for Input {
    fn default() -> Self {
        let (gamepad, gamepad_status) = match Gilrs::new() {
            Ok(pad) => (Some(pad), "No gamepad connected".to_owned()),
            Err(e) => (None, format!("Gamepad unavailable: {e}")),
        };
        Self {
            gamepad,
            gamepad_status,
            trim: 0.0,
        }
    }
}

fn dead_zone(value: f32) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    let value = f64::from(value);
    if value.abs() < 0.08 {
        0.0
    } else {
        value.signum() * ((value.abs() - 0.08) / 0.92).min(1.0)
    }
}

impl Input {
    pub fn collect(&mut self, ctx: &egui::Context, keymap: &Keymap) -> ActionFrame {
        let mut frame = ActionFrame::default();
        let focused = ctx.input(|i| i.focused);
        if focused && !ctx.wants_keyboard_input() {
            ctx.input(|input| {
                for action in Action::ALL {
                    if keymap.down(input, action) {
                        frame.held.insert(action, 1.0);
                    }
                    if keymap.pressed(input, action) {
                        frame.pressed.insert(action);
                    }
                }
            });
        }
        if let Some(gilrs) = &mut self.gamepad {
            while let Some(event) = gilrs.next_event() {
                if focused && let EventType::ButtonPressed(button, _) = event.event {
                    for (action, buttons) in &keymap.gamepad_bindings {
                        if buttons.contains(&button) {
                            frame.pressed.insert(*action);
                        }
                    }
                }
            }
            if let Some((_, pad)) = gilrs.gamepads().next() {
                self.gamepad_status = pad.name().to_owned();
                if focused {
                    for (axis, positive, negative) in [
                        (Axis::LeftStickY, Action::ElevatorDown, Action::ElevatorUp),
                        (Axis::LeftStickX, Action::AileronRight, Action::AileronLeft),
                        (Axis::RightStickX, Action::RudderRight, Action::RudderLeft),
                    ] {
                        let value = dead_zone(pad.value(axis));
                        if value != 0.0 {
                            frame.axes.insert(positive, value.max(0.0));
                            frame.axes.insert(negative, (-value).max(0.0));
                        }
                    }
                    for (action, buttons) in &keymap.gamepad_bindings {
                        if buttons.iter().any(|button| pad.is_pressed(*button)) {
                            frame.held.insert(*action, 1.0);
                        }
                    }
                    // gilrs exposes browser and native triggers as either axes or analog buttons.
                    for (axis, button, action) in [
                        (Axis::RightZ, Button::RightTrigger2, Action::ThrottleUp),
                        (Axis::LeftZ, Button::LeftTrigger2, Action::Brakes),
                    ] {
                        let value = pad
                            .axis_data(axis)
                            .map(|v| f64::from(v.value()).clamp(0.0, 1.0))
                            .or_else(|| pad.button_data(button).map(|v| f64::from(v.value())))
                            .unwrap_or_default();
                        if value > 0.02 {
                            let entry = frame.held.entry(action).or_default();
                            *entry = entry.max(value);
                        }
                    }
                }
            } else {
                self.gamepad_status = "No gamepad connected".to_owned();
            }
        }
        frame
    }

    pub fn controls(&mut self, frame: &ActionFrame, current: Controls, dt: f64) -> Controls {
        let dt = dt.clamp(0.0, 0.25);
        let axis = |value: f64, positive, negative| {
            if frame.axes.contains_key(&positive) {
                return frame.axes.get(&positive).copied().unwrap_or_default()
                    - frame.axes.get(&negative).copied().unwrap_or_default();
            }
            let target = frame.value(positive) - frame.value(negative);
            value + (target - value).clamp(-2.5 * dt, 2.5 * dt)
        };
        let old_trim = self.trim;
        self.trim = (self.trim + (frame.value(Action::TrimUp) - frame.value(Action::TrimDown)) * dt * 0.15)
            .clamp(-0.3, 0.3);
        Controls {
            elevator: axis(
                current.elevator - old_trim,
                Action::ElevatorUp,
                Action::ElevatorDown,
            ) + self.trim,
            aileron: axis(current.aileron, Action::AileronRight, Action::AileronLeft),
            rudder: axis(current.rudder, Action::RudderRight, Action::RudderLeft),
            throttle: current.throttle
                + (frame.value(Action::ThrottleUp) - frame.value(Action::ThrottleDown)) * dt * 0.35,
            flaps: if frame.pressed(Action::Flaps) {
                if current.flaps >= 0.99 {
                    0.0
                } else {
                    (current.flaps + 0.25).min(1.0)
                }
            } else {
                current.flaps
            },
            brake: frame.value(Action::Brakes),
        }
        .clamped()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> Input {
        Input {
            gamepad: None,
            gamepad_status: String::new(),
            trim: 0.0,
        }
    }

    #[test]
    fn keyboard_ramps_recenters_and_holds_throttle() {
        let mut input = input();
        let mut frame = ActionFrame::default();
        frame.held.insert(Action::ElevatorUp, 1.0);
        frame.held.insert(Action::ThrottleUp, 1.0);
        let c = input.controls(&frame, Controls::default(), 0.1);
        assert!((c.elevator - 0.25).abs() < 1e-9);
        assert!(c.throttle > 0.0);
        let neutral = input.controls(&ActionFrame::default(), c, 0.1);
        assert_eq!(neutral.elevator, 0.0);
        assert_eq!(neutral.throttle, c.throttle);
    }

    #[test]
    fn glance_actions_release_and_cancel_without_changing_flight_controls() {
        let mut frame = ActionFrame::default();
        frame.held.insert(Action::LookLeft, 1.0);
        frame.held.insert(Action::LookUp, 1.0);
        assert_eq!(frame.cockpit_look(), [-1.0, 1.0]);
        let current = Controls::default();
        assert_eq!(input().controls(&frame, current, 0.1), current);
        frame.held.insert(Action::LookRight, 1.0);
        assert_eq!(frame.cockpit_look(), [0.0, 1.0]);
        assert_eq!(ActionFrame::default().cockpit_look(), [0.0, 0.0]);
    }

    #[test]
    fn analog_sticks_and_triggers_use_the_same_controls() {
        let mut input = input();
        let mut frame = ActionFrame::default();
        frame.axes.insert(Action::AileronRight, 0.7);
        frame.held.insert(Action::Brakes, 0.4);
        frame.pressed.insert(Action::Flaps);
        let c = input.controls(&frame, Controls::default(), 0.1);
        assert_eq!(c.aileron, 0.7);
        assert_eq!(c.brake, 0.4);
        assert_eq!(c.flaps, 0.25);
    }
}
