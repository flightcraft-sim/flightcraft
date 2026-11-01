//! The only module that knows physical keyboard keys. Bindings persist as JSON.
use eframe::egui::{Event, InputState, Key};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Action {
    ElevatorUp,
    ElevatorDown,
    AileronLeft,
    AileronRight,
    RudderLeft,
    RudderRight,
    ThrottleUp,
    ThrottleDown,
    Flaps,
    Brakes,
    Pause,
    Reset,
    Camera,
    TrimUp,
    TrimDown,
}

impl Action {
    pub const ALL: [Self; 15] = [
        Self::ElevatorUp,
        Self::ElevatorDown,
        Self::AileronLeft,
        Self::AileronRight,
        Self::RudderLeft,
        Self::RudderRight,
        Self::ThrottleUp,
        Self::ThrottleDown,
        Self::Flaps,
        Self::Brakes,
        Self::Pause,
        Self::Reset,
        Self::Camera,
        Self::TrimUp,
        Self::TrimDown,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::ElevatorUp => "Pitch up",
            Self::ElevatorDown => "Pitch down",
            Self::AileronLeft => "Roll left",
            Self::AileronRight => "Roll right",
            Self::RudderLeft => "Yaw left",
            Self::RudderRight => "Yaw right",
            Self::ThrottleUp => "Throttle up",
            Self::ThrottleDown => "Throttle down",
            Self::Flaps => "Cycle flaps",
            Self::Brakes => "Hold brakes",
            Self::Pause => "Pause / resume",
            Self::Reset => "Reset flight",
            Self::Camera => "Change camera",
            Self::TrimUp => "Trim up",
            Self::TrimDown => "Trim down",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Keymap {
    pub bindings: BTreeMap<Action, Vec<Key>>,
}

impl Default for Keymap {
    fn default() -> Self {
        use Action::*;
        Self {
            bindings: [
                (ElevatorUp, vec![Key::ArrowDown, Key::S]),
                (ElevatorDown, vec![Key::ArrowUp, Key::W]),
                (AileronLeft, vec![Key::ArrowLeft, Key::A]),
                (AileronRight, vec![Key::ArrowRight, Key::D]),
                (RudderLeft, vec![Key::Q]),
                (RudderRight, vec![Key::E]),
                (ThrottleUp, vec![Key::PageUp]),
                (ThrottleDown, vec![Key::PageDown]),
                (Flaps, vec![Key::F]),
                (Brakes, vec![Key::B]),
                (Pause, vec![Key::Space, Key::P]),
                (Reset, vec![Key::R]),
                (Camera, vec![Key::C]),
                (TrimUp, vec![Key::Home]),
                (TrimDown, vec![Key::End]),
            ]
            .into_iter()
            .collect(),
        }
    }
}

impl Keymap {
    pub fn down(&self, input: &InputState, action: Action) -> bool {
        self.bindings
            .get(&action)
            .is_some_and(|keys| keys.iter().any(|&key| input.key_down(key)))
    }

    pub fn pressed(&self, input: &InputState, action: Action) -> bool {
        self.bindings
            .get(&action)
            .is_some_and(|keys| keys.iter().any(|key| input.events.iter().any(|event| {
            matches!(event, Event::Key { key: event_key, pressed: true, repeat: false, .. } if event_key == key)
        })))
    }

    pub fn label(&self, action: Action) -> String {
        self.bindings
            .get(&action)
            .map(|keys| keys.iter().map(|key| key.name()).collect::<Vec<_>>().join(" / "))
            .unwrap_or_default()
    }

    pub fn json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Require every action, at least one key each, and no accidental cross-action collisions.
    pub fn from_json(json: &str) -> Result<Self, String> {
        let value: Self = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let mut assigned = Vec::new();
        for action in Action::ALL {
            let keys = value
                .bindings
                .get(&action)
                .ok_or_else(|| format!("Missing {}", action.label()))?;
            if keys.is_empty() {
                return Err(format!("{} needs a key", action.label()));
            }
            for key in keys {
                if assigned.contains(key) {
                    return Err(format!("{} is assigned more than once", key.name()));
                }
                assigned.push(*key);
            }
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_cover_every_action_without_conflicts() {
        let map = Keymap::default();
        assert_eq!(map.bindings.len(), Action::ALL.len());
        let json = map.json().expect("serialize defaults");
        assert_eq!(Keymap::from_json(&json).expect("validate defaults"), map);
    }

    #[test]
    fn custom_bindings_round_trip() {
        let mut map = Keymap::default();
        map.bindings.insert(Action::Camera, vec![Key::V]);
        let restored = Keymap::from_json(&map.json().expect("serialize")).expect("parse");
        assert_eq!(restored, map);
    }

    #[test]
    fn one_shot_actions_ignore_keyboard_repeat() {
        let map = Keymap::default();
        let event = |repeat| Event::Key {
            key: Key::F,
            physical_key: None,
            pressed: true,
            repeat,
            modifiers: Default::default(),
        };
        let mut input = InputState::default();
        input.events = vec![event(true)];
        assert!(!map.pressed(&input, Action::Flaps));
        input.events = vec![event(false)];
        assert!(map.pressed(&input, Action::Flaps));
    }

    #[test]
    fn reject_incomplete_duplicate_and_malformed_maps() {
        let mut map = Keymap::default();
        map.bindings.remove(&Action::Reset);
        assert!(Keymap::from_json(&map.json().expect("serialize")).is_err());
        map.bindings.insert(Action::Reset, vec![Key::F]);
        assert!(Keymap::from_json(&map.json().expect("serialize")).is_err());
        assert!(Keymap::from_json("not JSON").is_err());
    }
}
