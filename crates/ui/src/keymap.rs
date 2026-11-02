//! The only module that knows physical keyboard keys. Bindings persist as JSON.
use eframe::egui::{Event, InputState, Key};
use gilrs::Button;
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
    Hood,
    Airspace,
    Search,
    PlanningMap,
    MapZoomIn,
    MapZoomOut,
    TrimUp,
    TrimDown,
    LookLeft,
    LookRight,
    LookUp,
    Master,
    Avionics,
    Alternator,
    FuelPump,
    Lights,
    PitotHeat,
    CarbHeat,
    AlternateStatic,
    Starter,
    Magnetos,
    FuelSelector,
    MixtureRich,
    MixtureLean,
    Primer,
    AltimeterUp,
    AltimeterDown,
    HeadingBugLeft,
    HeadingBugRight,
    AutopilotEngage,
    AutopilotDisconnect,
    ControlWheelSteering,
    AutopilotHeading,
    AutopilotAltitude,
    AutopilotLevel,
    HeadingBugSync,
    StandbyCourseLeft,
    StandbyCourseRight,
    StandbyAltitudeUp,
    StandbyAltitudeDown,
    StandbyAttitudePower,
    StandbyHsiPower,
    StandbyReversion,
    StandbyDirectTo,
    GlassRangeIn,
    GlassRangeOut,
    GlassPage,
    GlassOrientation,
    GlassAltitudeUp,
    GlassAltitudeDown,
    GlassCourseLeft,
    GlassCourseRight,
}

impl Action {
    pub const ALL: [Self; 65] = [
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
        Self::Hood,
        Self::Airspace,
        Self::Search,
        Self::PlanningMap,
        Self::MapZoomIn,
        Self::MapZoomOut,
        Self::TrimUp,
        Self::TrimDown,
        Self::LookLeft,
        Self::LookRight,
        Self::LookUp,
        Self::Master,
        Self::Avionics,
        Self::Alternator,
        Self::FuelPump,
        Self::Lights,
        Self::PitotHeat,
        Self::CarbHeat,
        Self::AlternateStatic,
        Self::Starter,
        Self::Magnetos,
        Self::FuelSelector,
        Self::MixtureRich,
        Self::MixtureLean,
        Self::Primer,
        Self::AltimeterUp,
        Self::AltimeterDown,
        Self::HeadingBugLeft,
        Self::HeadingBugRight,
        Self::AutopilotEngage,
        Self::AutopilotDisconnect,
        Self::ControlWheelSteering,
        Self::AutopilotHeading,
        Self::AutopilotAltitude,
        Self::AutopilotLevel,
        Self::HeadingBugSync,
        Self::StandbyCourseLeft,
        Self::StandbyCourseRight,
        Self::StandbyAltitudeUp,
        Self::StandbyAltitudeDown,
        Self::StandbyAttitudePower,
        Self::StandbyHsiPower,
        Self::StandbyReversion,
        Self::StandbyDirectTo,
        Self::GlassRangeIn,
        Self::GlassRangeOut,
        Self::GlassPage,
        Self::GlassOrientation,
        Self::GlassAltitudeUp,
        Self::GlassAltitudeDown,
        Self::GlassCourseLeft,
        Self::GlassCourseRight,
    ];
    /// Systems shortcuts are optional, and can be bound in the same persisted keymap.
    pub const SYSTEMS: [Self; 18] = [
        Self::Master,
        Self::Avionics,
        Self::Alternator,
        Self::FuelPump,
        Self::Lights,
        Self::PitotHeat,
        Self::CarbHeat,
        Self::AlternateStatic,
        Self::Starter,
        Self::Magnetos,
        Self::FuelSelector,
        Self::MixtureRich,
        Self::MixtureLean,
        Self::Primer,
        Self::AltimeterUp,
        Self::AltimeterDown,
        Self::HeadingBugLeft,
        Self::HeadingBugRight,
    ];
    /// Equipment controls are optional bindings, separate from airframe systems.
    pub const AUTOPILOT: [Self; 6] = [
        Self::AutopilotEngage,
        Self::AutopilotDisconnect,
        Self::ControlWheelSteering,
        Self::AutopilotHeading,
        Self::AutopilotAltitude,
        Self::AutopilotLevel,
    ];

    pub const AVIONICS: [Self; 13] = [
        Self::HeadingBugSync,
        Self::HeadingBugLeft,
        Self::HeadingBugRight,
        Self::AltimeterUp,
        Self::AltimeterDown,
        Self::StandbyCourseLeft,
        Self::StandbyCourseRight,
        Self::StandbyAltitudeUp,
        Self::StandbyAltitudeDown,
        Self::StandbyAttitudePower,
        Self::StandbyHsiPower,
        Self::StandbyReversion,
        Self::StandbyDirectTo,
    ];

    pub const GLASS: [Self; 10] = [
        Self::GlassRangeIn,
        Self::GlassRangeOut,
        Self::GlassPage,
        Self::GlassOrientation,
        Self::GlassAltitudeUp,
        Self::GlassAltitudeDown,
        Self::GlassCourseLeft,
        Self::GlassCourseRight,
        Self::HeadingBugLeft,
        Self::HeadingBugRight,
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
            Self::Hood => "View-limiting hood",
            Self::Camera => "Change camera",
            Self::PlanningMap => "Show / hide planning map",
            Self::MapZoomIn => "Map zoom in",
            Self::MapZoomOut => "Map zoom out",
            Self::Search => "Focus search",
            Self::Airspace => "Show / hide airspace",
            Self::TrimUp => "Trim up",
            Self::TrimDown => "Trim down",
            Self::LookLeft => "Glance left",
            Self::LookRight => "Glance right",
            Self::LookUp => "Glance up",
            Self::Master => "Toggle master",
            Self::Avionics => "Toggle avionics",
            Self::Alternator => "Toggle alternator",
            Self::FuelPump => "Toggle fuel pump",
            Self::Lights => "Toggle lights",
            Self::PitotHeat => "Toggle pitot heat",
            Self::CarbHeat => "Toggle carb heat",
            Self::AlternateStatic => "Toggle alternate static",
            Self::Starter => "Hold starter",
            Self::Magnetos => "Cycle magnetos",
            Self::FuelSelector => "Cycle fuel selector",
            Self::MixtureRich => "Mixture richer",
            Self::MixtureLean => "Mixture leaner",
            Self::Primer => "Add primer charge",
            Self::AltimeterUp => "Altimeter setting up",
            Self::AltimeterDown => "Altimeter setting down",
            Self::HeadingBugLeft => "Heading bug left",
            Self::StandbyCourseLeft => "Standby course left",
            Self::StandbyCourseRight => "Standby course right",
            Self::StandbyAltitudeUp => "Selected altitude up",
            Self::StandbyAltitudeDown => "Selected altitude down",
            Self::StandbyAttitudePower => "Toggle attitude power",
            Self::StandbyHsiPower => "Toggle navigation power",
            Self::StandbyReversion => "Toggle attitude reversion",
            Self::StandbyDirectTo => "Open direct to",
            Self::HeadingBugRight => "Heading bug right",
            Self::AutopilotEngage => "Autopilot engage",
            Self::AutopilotDisconnect => "Yoke disconnect",
            Self::ControlWheelSteering => "Hold control wheel steering",
            Self::AutopilotHeading => "Autopilot HDG",
            Self::AutopilotAltitude => "Autopilot ALT",
            Self::AutopilotLevel => "Autopilot LVL",
            Self::HeadingBugSync => "Push heading knob / sync heading",
            Self::GlassRangeIn => "Glass map zoom in",
            Self::GlassRangeOut => "Glass map zoom out",
            Self::GlassPage => "Glass next page",
            Self::GlassOrientation => "Glass north / track up",
            Self::GlassAltitudeUp => "Glass selected altitude up",
            Self::GlassAltitudeDown => "Glass selected altitude down",
            Self::GlassCourseLeft => "Glass course left",
            Self::GlassCourseRight => "Glass course right",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Keymap {
    pub bindings: BTreeMap<Action, Vec<Key>>,
    #[serde(default = "default_gamepad_bindings")]
    pub gamepad_bindings: BTreeMap<Action, Vec<Button>>,
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
                (Hood, vec![Key::F8]),
                (Airspace, vec![Key::U]),
                (Search, vec![Key::Slash]),
                (PlanningMap, vec![Key::M]),
                (MapZoomIn, vec![Key::Plus]),
                (MapZoomOut, vec![Key::Minus]),
                (TrimUp, vec![Key::Home]),
                (TrimDown, vec![Key::End]),
                (LookLeft, vec![Key::J]),
                (LookRight, vec![Key::L]),
                (LookUp, vec![Key::I]),
            ]
            .into_iter()
            .chain(
                Action::SYSTEMS
                    .into_iter()
                    .chain(Action::AUTOPILOT)
                    .chain(Action::AVIONICS)
                    .chain(Action::GLASS)
                    .map(|action| (action, Vec::new())),
            )
            .chain([
                (HeadingBugLeft, vec![Key::Comma]),
                (HeadingBugRight, vec![Key::Period]),
                (HeadingBugSync, vec![Key::H]),
            ])
            .collect(),
            gamepad_bindings: default_gamepad_bindings(),
        }
    }
}

fn default_gamepad_bindings() -> BTreeMap<Action, Vec<Button>> {
    [
        (Action::Pause, Button::Start),
        (Action::Reset, Button::Select),
        (Action::Camera, Button::North),
        (Action::Flaps, Button::South),
        (Action::Brakes, Button::East),
        (Action::ThrottleUp, Button::DPadUp),
        (Action::ThrottleDown, Button::DPadDown),
        (Action::LookLeft, Button::LeftTrigger),
        (Action::LookRight, Button::RightTrigger),
        (Action::LookUp, Button::West),
    ]
    .into_iter()
    .map(|(action, button)| (action, vec![button]))
    .collect()
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
        let mut value: Self = serde_json::from_str(json).map_err(|e| e.to_string())?;
        // Upgrade saved maps from before head look, preserving every existing custom binding.
        let looks = [Action::LookLeft, Action::LookRight, Action::LookUp];
        if looks.iter().all(|a| !value.bindings.contains_key(a)) {
            for action in looks {
                let defaults = Self::default().bindings.remove(&action).unwrap_or_default();
                let available = defaults
                    .into_iter()
                    .filter(|key| !value.bindings.values().any(|keys| keys.contains(key)))
                    .collect::<Vec<_>>();
                if available.is_empty() {
                    return Err(format!("Assign a key for {}", action.label()));
                }
                value.bindings.insert(action, available);
            }
        }
        if !value.bindings.contains_key(&Action::Airspace) {
            let key = [Key::U, Key::O, Key::V]
                .into_iter()
                .find(|key| !value.bindings.values().any(|keys| keys.contains(key)))
                .ok_or("Assign a key for airspace")?;
            value.bindings.insert(Action::Airspace, vec![key]);
        }
        for action in Action::SYSTEMS
            .into_iter()
            .chain(Action::AUTOPILOT)
            .chain(Action::AVIONICS)
            .chain(Action::GLASS)
        {
            value.bindings.entry(action).or_default();
        }
        for action in [Action::PlanningMap, Action::MapZoomIn, Action::MapZoomOut] {
            if !value.bindings.contains_key(&action) {
                let keys = Self::default()
                    .bindings
                    .remove(&action)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|key| !value.bindings.values().any(|keys| keys.contains(key)))
                    .collect();
                value.bindings.insert(action, keys);
            }
        }
        let mut buttons = Vec::new();
        for assigned in value.gamepad_bindings.values() {
            for button in assigned {
                if buttons.contains(button) {
                    return Err(format!("{button:?} is assigned more than once"));
                }
                buttons.push(*button);
            }
        }
        let mut assigned = Vec::new();
        for action in Action::ALL {
            let keys = value
                .bindings
                .get(&action)
                .ok_or_else(|| format!("Missing {}", action.label()))?;
            if keys.is_empty()
                && !Action::SYSTEMS.contains(&action)
                && !Action::AUTOPILOT.contains(&action)
                && !Action::AVIONICS.contains(&action)
                && ![
                    Action::Search,
                    Action::PlanningMap,
                    Action::MapZoomIn,
                    Action::MapZoomOut,
                ]
                .contains(&action)
                && !Action::GLASS.contains(&action)
            {
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
    fn saved_maps_gain_glances_and_gamepad_buttons_are_configurable() {
        let mut legacy = Keymap::default();
        legacy.bindings.insert(Action::Camera, vec![Key::V]);
        for action in [
            Action::LookLeft,
            Action::LookRight,
            Action::LookUp,
            Action::Airspace,
        ] {
            legacy.bindings.remove(&action);
        }
        let mut json = serde_json::to_value(&legacy).expect("serialize");
        json.as_object_mut().expect("object").remove("gamepad_bindings");
        let mut upgraded = Keymap::from_json(&json.to_string()).expect("upgrade");
        assert_eq!(upgraded.bindings.get(&Action::Camera), Some(&vec![Key::V]));
        assert_eq!(upgraded.bindings.len(), Action::ALL.len());
        upgraded
            .gamepad_bindings
            .insert(Action::LookUp, vec![Button::DPadLeft]);
        assert_eq!(
            Keymap::from_json(&upgraded.json().expect("serialize")).expect("parse"),
            upgraded
        );
        upgraded
            .gamepad_bindings
            .insert(Action::LookLeft, vec![Button::DPadLeft]);
        assert!(Keymap::from_json(&upgraded.json().expect("serialize")).is_err());
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
