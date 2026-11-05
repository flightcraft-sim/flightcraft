//! Behaviour of the landscape touch EFIS primary flight display: the autopilot status box, the
//! flight director bug and course-sync knob actions, the knob-function hints, and the rules for a
//! lost primary flight display (reversion).
//!
//! Pure functions over the autopilot's state; nothing here draws or reads a session.
use flightcraft_sim::equipment::{EquipmentState, LateralMode, PanelSetting, VerticalMode};

const KNOTS: f64 = 1.943_844_492_440_6;
const FEET: f64 = 1. / 0.3048;

/// How an annunciation is coloured: white when armed, green when active, flashing yellow when the
/// mode was lost or the autopilot disconnected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfcsTone {
    Armed,
    Active,
    /// Flashing yellow; `visible` is the current phase of the flash.
    Caution {
        visible: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AfcsCell {
    pub text: String,
    pub tone: AfcsTone,
}

/// What the AFCS status box shows: autopilot and yaw damper at the far left, lateral modes in the
/// centre, vertical modes on the right.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AfcsBox {
    pub autopilot: Option<AfcsCell>,
    pub yaw_damper: Option<AfcsCell>,
    pub lateral_active: Option<AfcsCell>,
    pub lateral_armed: Option<AfcsCell>,
    pub vertical_active: Option<AfcsCell>,
    pub vertical_armed: Option<AfcsCell>,
    /// The reference of the active vertical mode (altitude held, vertical speed, airspeed).
    pub reference: Option<String>,
    /// The flight director is on: command bars are drawn.
    pub flight_director: bool,
    /// Command bars are hollow while the pilot hand-flies with the director alone.
    pub hand_flying: bool,
}

fn cell(text: &str, tone: AfcsTone) -> Option<AfcsCell> {
    Some(AfcsCell {
        text: text.to_owned(),
        tone,
    })
}

/// Lateral mode annunciation. Navigation modes read as the selected navigation source.
pub fn lateral_label(mode: LateralMode, nav_source: &str) -> String {
    match mode {
        LateralMode::Rol => "ROL".into(),
        LateralMode::Hdg => "HDG".into(),
        LateralMode::Trk => "TRK".into(),
        LateralMode::Nav | LateralMode::Apr => nav_source.to_ascii_uppercase(),
        LateralMode::Rev | LateralMode::LocBc => "BC".into(),
    }
}

pub fn vertical_label(mode: VerticalMode) -> &'static str {
    match mode {
        VerticalMode::Pit => "PIT",
        VerticalMode::Alt => "ALT",
        VerticalMode::Vs => "VS",
        VerticalMode::Ias => "IAS",
        VerticalMode::Gp => "GP",
        VerticalMode::Gs => "GS",
    }
}

/// The AFCS status box for an autopilot state and the selected navigation source ("GPS", "VOR",
/// "LOC"). No autopilot installed gives `None` at the call site, not an empty box.
pub fn afcs_box(s: &EquipmentState, nav_source: &str) -> AfcsBox {
    let mut b = AfcsBox {
        flight_director: s.flight_director,
        hand_flying: s.flight_director && !s.engaged,
        ..AfcsBox::default()
    };
    let caution = |visible| AfcsTone::Caution { visible };
    b.autopilot = if s.engaged {
        cell("AP", AfcsTone::Active)
    } else if s.ap_flash {
        // A disconnect flashes the annunciation yellow for a few seconds.
        cell("AP", caution(s.ap_flash_visible))
    } else {
        None
    };
    b.yaw_damper = s.yaw_damper.then(|| cell("YD", AfcsTone::Active)).flatten();
    if s.flight_director || s.engaged {
        b.lateral_active = s
            .active_lateral
            .and_then(|m| cell(&lateral_label(m, nav_source), AfcsTone::Active));
        b.lateral_armed = s
            .armed_lateral
            .and_then(|m| cell(&lateral_label(m, nav_source), AfcsTone::Armed));
        b.vertical_active = s
            .active_vertical
            .and_then(|m| cell(vertical_label(m), AfcsTone::Active));
        b.vertical_armed = if s.alts_armed {
            cell("ALTS", AfcsTone::Armed)
        } else {
            s.armed_vertical
                .and_then(|m| cell(vertical_label(m), AfcsTone::Armed))
        };
        b.reference = match s.active_vertical {
            Some(VerticalMode::Alt) => Some(format!("{:.0}ft", s.captured_altitude_m * FEET)),
            Some(VerticalMode::Vs) => Some(format!("{:+.0}fpm", s.vertical_speed_mps * FEET * 60.)),
            Some(VerticalMode::Ias) => Some(format!("{:.0}kt", s.airspeed_mps * KNOTS)),
            _ => None,
        };
    }
    b
}

/// The reference change per knob detent for the active vertical mode, from the vertical modes
/// table: pitch 0.5 degree, vertical speed 100 fpm, indicated airspeed 1 knot. Altitude hold's
/// 10 ft needs a hold-altitude setting the engine's autopilot does not have, so that mode is not
/// steppable here.
pub fn fd_bug_step(s: &EquipmentState, detents: i32) -> Option<PanelSetting> {
    let n = f64::from(detents);
    match s.active_vertical? {
        VerticalMode::Pit => Some(PanelSetting::Pitch {
            pitch_rad: (s.pitch_rad + n * 0.5_f64.to_radians()).clamp(-0.25, 0.35),
        }),
        VerticalMode::Vs => Some(PanelSetting::VerticalSpeed {
            vertical_speed_mps: (s.vertical_speed_mps + n * 100. / FEET / 60.).clamp(-8., 8.),
        }),
        VerticalMode::Ias => Some(PanelSetting::Airspeed {
            airspeed_mps: (s.airspeed_mps + n / KNOTS).clamp(20., 90.),
        }),
        VerticalMode::Alt | VerticalMode::Gp | VerticalMode::Gs => None,
    }
}

/// Pushing the flight director bug knob sets the pitch reference to the current pitch.
pub fn fd_bug_sync(current_pitch_rad: f64) -> PanelSetting {
    PanelSetting::Pitch {
        pitch_rad: current_pitch_rad.clamp(-0.25, 0.35),
    }
}

/// The course a course-knob push syncs to: the active leg's desired track when guidance is
/// valid, else the current heading, rounded to a degree.
pub fn course_sync_target(guidance_valid: bool, desired_track_rad: f64, heading_rad: f64) -> f64 {
    let source = if guidance_valid {
        desired_track_rad
    } else {
        heading_rad
    };
    let degrees = libm::round(source.to_degrees()).rem_euclid(360.);
    degrees.to_radians()
}

/// A button of the on-screen AFCS controls (the window the AFCS status box opens on a unit with no
/// separate mode controller).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfcsKey {
    Ap,
    Fd,
    Yd,
    Hdg,
    Trk,
    Nav,
    Apr,
    Alt,
    Vs,
    Ias,
    Lvl,
    NoseUp,
    NoseDown,
}

/// One button as drawn: lit green when its mode is active, white when armed, dimmed when the
/// installed autopilot lacks the function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AfcsButton {
    pub key: AfcsKey,
    pub label: &'static str,
    pub tone: Option<AfcsTone>,
    pub enabled: bool,
}

/// The window's buttons in `AfcsKey::ALL` order, and the reference line between the nose pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AfcsWindow {
    pub buttons: Vec<AfcsButton>,
    /// What the nose up and nose down pair adjusts, with its present value: "VS +500 fpm",
    /// "IAS 90 kt", "ALT 5000 ft", "PIT +3 deg", or "NO MODE".
    pub reference: String,
}

impl AfcsKey {
    pub const ALL: [Self; 13] = [
        Self::Ap,
        Self::Fd,
        Self::Yd,
        Self::Hdg,
        Self::Trk,
        Self::Nav,
        Self::Apr,
        Self::Alt,
        Self::Vs,
        Self::Ias,
        Self::Lvl,
        Self::NoseUp,
        Self::NoseDown,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Ap => "AP",
            Self::Fd => "FD",
            Self::Yd => "YD",
            Self::Hdg => "HDG",
            Self::Trk => "TRK",
            Self::Nav => "NAV",
            Self::Apr => "APR",
            Self::Alt => "ALT",
            Self::Vs => "VS",
            Self::Ias => "IAS",
            Self::Lvl => "LVL",
            Self::NoseUp => "NOSE UP",
            Self::NoseDown => "NOSE DN",
        }
    }
}

const VS_STEP_MPS: f64 = 0.508;
/// The altitude hold reference moves 20 ft a press, as the pitch wheel does in ALT.
const HOLD_STEP_M: f64 = 6.096;

/// The vertical mode the nose pair steps, if the autopilot is in one it can step.
fn nose_mode(s: &EquipmentState) -> Option<VerticalMode> {
    s.active_vertical.filter(|m| {
        matches!(
            m,
            VerticalMode::Pit | VerticalMode::Alt | VerticalMode::Vs | VerticalMode::Ias
        )
    })
}

fn nose_available(s: &EquipmentState) -> bool {
    (s.engaged || s.flight_director)
        && (nose_mode(s).is_some()
            || (s.active_vertical.is_none() && s.installation.vertical_modes.contains(&VerticalMode::Vs)))
}

fn lateral_key(key: AfcsKey) -> Option<LateralMode> {
    match key {
        AfcsKey::Hdg => Some(LateralMode::Hdg),
        AfcsKey::Trk => Some(LateralMode::Trk),
        AfcsKey::Nav => Some(LateralMode::Nav),
        AfcsKey::Apr => Some(LateralMode::Apr),
        _ => None,
    }
}

fn vertical_key(key: AfcsKey) -> Option<VerticalMode> {
    match key {
        AfcsKey::Alt => Some(VerticalMode::Alt),
        AfcsKey::Vs => Some(VerticalMode::Vs),
        AfcsKey::Ias => Some(VerticalMode::Ias),
        _ => None,
    }
}

/// The on-screen AFCS controls for an autopilot state: which buttons are lit (green for the active
/// mode, white for the armed one) and which the installed autopilot can use.
pub fn afcs_window(s: &EquipmentState) -> AfcsWindow {
    let modes_shown = s.flight_director || s.engaged;
    let buttons = AfcsKey::ALL
        .iter()
        .map(|&key| {
            let (tone, enabled) = if let Some(mode) = lateral_key(key) {
                let tone = if !modes_shown {
                    None
                } else if s.active_lateral == Some(mode) {
                    Some(AfcsTone::Active)
                } else if s.armed_lateral == Some(mode) {
                    Some(AfcsTone::Armed)
                } else {
                    None
                };
                (tone, s.installation.lateral_modes.contains(&mode))
            } else if let Some(mode) = vertical_key(key) {
                let tone = if !modes_shown {
                    None
                } else if s.active_vertical == Some(mode) {
                    Some(AfcsTone::Active)
                } else if s.armed_vertical == Some(mode) || (mode == VerticalMode::Alt && s.alts_armed) {
                    Some(AfcsTone::Armed)
                } else {
                    None
                };
                (tone, s.installation.vertical_modes.contains(&mode))
            } else {
                match key {
                    AfcsKey::Ap => (s.engaged.then_some(AfcsTone::Active), true),
                    AfcsKey::Fd => (s.flight_director.then_some(AfcsTone::Active), true),
                    AfcsKey::Yd => (
                        s.yaw_damper.then_some(AfcsTone::Active),
                        s.installation.yaw_damper,
                    ),
                    AfcsKey::Lvl => (
                        (s.engaged
                            && s.active_lateral == Some(LateralMode::Rol)
                            && matches!(s.active_vertical, Some(VerticalMode::Pit) | None))
                        .then_some(AfcsTone::Active),
                        true,
                    ),
                    _ => (None, nose_available(s)),
                }
            };
            AfcsButton {
                key,
                label: key.label(),
                tone,
                enabled,
            }
        })
        .collect();
    let reference = match nose_mode(s) {
        Some(VerticalMode::Vs) => format!("VS {:+.0} fpm", s.vertical_speed_mps * FEET * 60.),
        Some(VerticalMode::Ias) => format!("IAS {:.0} kt", s.airspeed_mps * KNOTS),
        Some(VerticalMode::Alt) => format!("ALT {:.0} ft", s.captured_altitude_m * FEET),
        Some(VerticalMode::Pit) => format!("PIT {:+.0} deg", s.pitch_rad.to_degrees()),
        _ => "NO MODE".to_owned(),
    };
    AfcsWindow { buttons, reference }
}

/// The autopilot commands one press of a button sends: the same commands the mode controller's
/// keys send. A button the installed autopilot lacks sends nothing. The nose pair steps the
/// reference of the active vertical mode (vertical speed 100 fpm, airspeed 1 kt, held altitude
/// 20 ft, pitch 1 degree); with no vertical mode it starts vertical speed first.
pub fn afcs_press(s: &EquipmentState, key: AfcsKey) -> Vec<PanelSetting> {
    let enabled = afcs_window(s).buttons.iter().any(|b| b.key == key && b.enabled);
    if !enabled {
        return Vec::new();
    }
    if let Some(mode) = lateral_key(key) {
        return vec![PanelSetting::Lateral { mode }];
    }
    if let Some(mode) = vertical_key(key) {
        return vec![PanelSetting::Vertical { mode }];
    }
    match key {
        AfcsKey::Ap => vec![PanelSetting::Engage { on: !s.engaged }],
        AfcsKey::Fd => vec![PanelSetting::FlightDirector {
            on: !s.flight_director,
        }],
        AfcsKey::Yd => vec![PanelSetting::YawDamper { on: !s.yaw_damper }],
        AfcsKey::Lvl => vec![PanelSetting::Level],
        _ => {
            let n = if key == AfcsKey::NoseUp { 1. } else { -1. };
            match nose_mode(s) {
                Some(VerticalMode::Vs) => vec![PanelSetting::VerticalSpeed {
                    vertical_speed_mps: ((s.vertical_speed_mps / VS_STEP_MPS).round() * VS_STEP_MPS
                        + n * VS_STEP_MPS)
                        .clamp(-7.62, 7.62),
                }],
                Some(VerticalMode::Ias) => vec![PanelSetting::Airspeed {
                    airspeed_mps: ((s.airspeed_mps * KNOTS).round() + n).clamp(40., 174.) / KNOTS,
                }],
                Some(VerticalMode::Alt) => vec![PanelSetting::HoldAltitude {
                    altitude_m: (s.captured_altitude_m + n * HOLD_STEP_M).clamp(2., 10_000.),
                }],
                Some(VerticalMode::Pit) => vec![PanelSetting::Pitch {
                    pitch_rad: (s.pitch_rad + n.to_radians()).clamp(-0.25, 0.35),
                }],
                _ => vec![
                    PanelSetting::Vertical {
                        mode: VerticalMode::Vs,
                    },
                    PanelSetting::VerticalSpeed {
                        vertical_speed_mps: n * VS_STEP_MPS,
                    },
                ],
            }
        }
    }
}

/// One display for the reversion rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayHealth {
    /// This display is configured as the primary flight display.
    pub primary_pfd: bool,
    /// Failed or without power.
    pub lost: bool,
}

/// A lost primary flight display disconnects the autopilot; a lost multi-function display does
/// not (the unit's reversionary-mode appendix).
pub fn primary_display_lost(displays: &[DisplayHealth]) -> bool {
    displays.iter().any(|d| d.primary_pfd && d.lost)
}

/// In reversion the director is unavailable: basic autopilot modes remain, command bars go.
pub fn command_bars_available(reversion: bool) -> bool {
    !reversion
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use flightcraft_sim::equipment::{AutopilotFamily, AutopilotInstallation};

    fn state() -> EquipmentState {
        EquipmentState {
            installation: AutopilotInstallation::new(AutopilotFamily::DigitalTwoAxis),
            engaged: false,
            flight_director: false,
            active_lateral: None,
            armed_lateral: None,
            active_vertical: None,
            armed_vertical: None,
            alts_armed: false,
            heading_rad: 0.,
            track_rad: 0.,
            altitude_m: 1524.,
            captured_altitude_m: 1524.,
            vertical_speed_mps: 0.,
            airspeed_mps: 40.,
            pitch_rad: 0.05,
            turn_rate_rps: 0.,
            command_bank_rad: 0.,
            command_pitch_rad: 0.,
            roll_servo_engaged: false,
            pitch_servo_engaged: false,
            yaw_servo_engaged: false,
            servo_deflection: [0.; 3],
            trim: 0.,
            trim_in_motion: false,
            mistrim: false,
            altitude_alert: false,
            underspeed: false,
            overspeed: false,
            stability_active: false,
            pilot_overpower: false,
            control_wheel_steering: false,
            yaw_damper: false,
            failures: vec![],
            disconnect_reason: None,
            disconnect_tone: false,
            disconnect_sequence: 0,
            ap_flash: false,
            ap_flash_visible: false,
        }
    }

    #[test]
    fn box_is_empty_with_everything_off() {
        assert_eq!(afcs_box(&state(), "GPS"), AfcsBox::default());
    }

    #[test]
    fn autopilot_modes_appear_green_active_and_white_armed_with_their_references() {
        let mut s = state();
        s.engaged = true;
        s.flight_director = true;
        s.yaw_damper = true;
        s.active_lateral = Some(LateralMode::Hdg);
        s.armed_lateral = Some(LateralMode::Nav);
        s.active_vertical = Some(VerticalMode::Alt);
        s.alts_armed = true;
        let b = afcs_box(&s, "gps");
        let text = |c: &Option<AfcsCell>| c.as_ref().map(|c| (c.text.clone(), c.tone));
        assert_eq!(text(&b.autopilot), Some(("AP".into(), AfcsTone::Active)));
        assert_eq!(text(&b.yaw_damper), Some(("YD".into(), AfcsTone::Active)));
        assert_eq!(text(&b.lateral_active), Some(("HDG".into(), AfcsTone::Active)));
        assert_eq!(text(&b.lateral_armed), Some(("GPS".into(), AfcsTone::Armed)));
        assert_eq!(text(&b.vertical_active), Some(("ALT".into(), AfcsTone::Active)));
        assert_eq!(text(&b.vertical_armed), Some(("ALTS".into(), AfcsTone::Armed)));
        assert_eq!(b.reference.as_deref(), Some("5000ft"));
        assert!(b.flight_director && !b.hand_flying);
    }

    fn tone(w: &AfcsWindow, key: AfcsKey) -> Option<AfcsTone> {
        w.buttons.iter().find(|b| b.key == key).unwrap().tone
    }

    #[test]
    fn afcs_buttons_show_active_and_armed_modes_and_dim_what_is_not_installed() {
        let mut s = state();
        s.engaged = true;
        s.flight_director = true;
        s.active_lateral = Some(LateralMode::Hdg);
        s.armed_lateral = Some(LateralMode::Nav);
        s.active_vertical = Some(VerticalMode::Vs);
        s.alts_armed = true;
        let w = afcs_window(&s);
        assert_eq!(tone(&w, AfcsKey::Ap), Some(AfcsTone::Active));
        assert_eq!(tone(&w, AfcsKey::Fd), Some(AfcsTone::Active));
        assert_eq!(tone(&w, AfcsKey::Hdg), Some(AfcsTone::Active));
        assert_eq!(tone(&w, AfcsKey::Nav), Some(AfcsTone::Armed));
        assert_eq!(tone(&w, AfcsKey::Apr), None);
        assert_eq!(tone(&w, AfcsKey::Vs), Some(AfcsTone::Active));
        assert_eq!(tone(&w, AfcsKey::Alt), Some(AfcsTone::Armed));
        assert_eq!(tone(&w, AfcsKey::Lvl), None);
        assert_eq!(w.reference, "VS +0 fpm");
        // The installation has no yaw damper: the button is dimmed and sends nothing.
        assert!(!w.buttons.iter().find(|b| b.key == AfcsKey::Yd).unwrap().enabled);
        assert!(afcs_press(&s, AfcsKey::Yd).is_empty());
        // A wing leveler has no TRK, and no vertical modes at all.
        s.installation = AutopilotInstallation::new(AutopilotFamily::WingLeveler);
        assert!(afcs_press(&s, AfcsKey::Trk).is_empty());
        assert!(afcs_press(&s, AfcsKey::Alt).is_empty());
        // With the autopilot and director off nothing is lit but the buttons stay usable.
        let off = afcs_window(&state());
        assert!(off.buttons.iter().all(|b| b.tone.is_none()));
        assert!(
            !off.buttons
                .iter()
                .find(|b| b.key == AfcsKey::NoseUp)
                .unwrap()
                .enabled
        );
    }

    #[test]
    fn afcs_presses_send_the_mode_controllers_commands() {
        let mut s = state();
        assert_eq!(
            afcs_press(&s, AfcsKey::Ap),
            vec![PanelSetting::Engage { on: true }]
        );
        assert_eq!(
            afcs_press(&s, AfcsKey::Fd),
            vec![PanelSetting::FlightDirector { on: true }]
        );
        assert_eq!(afcs_press(&s, AfcsKey::Lvl), vec![PanelSetting::Level]);
        for (key, mode) in [
            (AfcsKey::Hdg, LateralMode::Hdg),
            (AfcsKey::Trk, LateralMode::Trk),
            (AfcsKey::Nav, LateralMode::Nav),
            (AfcsKey::Apr, LateralMode::Apr),
        ] {
            assert_eq!(afcs_press(&s, key), vec![PanelSetting::Lateral { mode }]);
        }
        for (key, mode) in [
            (AfcsKey::Alt, VerticalMode::Alt),
            (AfcsKey::Vs, VerticalMode::Vs),
            (AfcsKey::Ias, VerticalMode::Ias),
        ] {
            assert_eq!(afcs_press(&s, key), vec![PanelSetting::Vertical { mode }]);
        }
        s.engaged = true;
        assert_eq!(
            afcs_press(&s, AfcsKey::Ap),
            vec![PanelSetting::Engage { on: false }]
        );
        // Nose up and down step the active mode's reference.
        s.active_vertical = Some(VerticalMode::Vs);
        s.vertical_speed_mps = 0.;
        assert_eq!(
            afcs_press(&s, AfcsKey::NoseUp),
            vec![PanelSetting::VerticalSpeed {
                vertical_speed_mps: 0.508
            }]
        );
        assert_eq!(
            afcs_press(&s, AfcsKey::NoseDown),
            vec![PanelSetting::VerticalSpeed {
                vertical_speed_mps: -0.508
            }]
        );
        s.active_vertical = Some(VerticalMode::Alt);
        s.captured_altitude_m = 1000.;
        assert_eq!(
            afcs_press(&s, AfcsKey::NoseUp),
            vec![PanelSetting::HoldAltitude { altitude_m: 1006.096 }]
        );
        s.active_vertical = Some(VerticalMode::Ias);
        s.airspeed_mps = 40.;
        let Some(PanelSetting::Airspeed { airspeed_mps }) = afcs_press(&s, AfcsKey::NoseUp).pop() else {
            panic!("airspeed step")
        };
        assert!(
            (airspeed_mps * KNOTS - 79.).abs() < 1e-6,
            "{}",
            airspeed_mps * KNOTS
        );
        s.active_vertical = Some(VerticalMode::Pit);
        s.pitch_rad = 0.;
        assert_eq!(
            afcs_press(&s, AfcsKey::NoseDown),
            vec![PanelSetting::Pitch {
                pitch_rad: -1_f64.to_radians()
            }]
        );
        // No vertical mode: the pair starts vertical speed.
        s.active_vertical = None;
        assert_eq!(
            afcs_press(&s, AfcsKey::NoseUp),
            vec![
                PanelSetting::Vertical {
                    mode: VerticalMode::Vs
                },
                PanelSetting::VerticalSpeed {
                    vertical_speed_mps: 0.508
                }
            ]
        );
    }

    #[test]
    fn every_mode_has_the_guides_annunciation() {
        for (mode, label) in [
            (LateralMode::Rol, "ROL"),
            (LateralMode::Hdg, "HDG"),
            (LateralMode::Trk, "TRK"),
            (LateralMode::LocBc, "BC"),
        ] {
            assert_eq!(lateral_label(mode, "VOR"), label);
        }
        assert_eq!(lateral_label(LateralMode::Nav, "vor"), "VOR");
        assert_eq!(lateral_label(LateralMode::Apr, "LOC"), "LOC");
        for (mode, label) in [
            (VerticalMode::Pit, "PIT"),
            (VerticalMode::Alt, "ALT"),
            (VerticalMode::Vs, "VS"),
            (VerticalMode::Ias, "IAS"),
            (VerticalMode::Gp, "GP"),
            (VerticalMode::Gs, "GS"),
        ] {
            assert_eq!(vertical_label(mode), label);
        }
    }

    #[test]
    fn references_follow_the_active_vertical_mode() {
        let mut s = state();
        s.flight_director = true;
        s.active_vertical = Some(VerticalMode::Vs);
        s.vertical_speed_mps = -500. / FEET / 60.;
        assert_eq!(afcs_box(&s, "GPS").reference.as_deref(), Some("-500fpm"));
        s.active_vertical = Some(VerticalMode::Ias);
        s.airspeed_mps = 90. / KNOTS;
        assert_eq!(afcs_box(&s, "GPS").reference.as_deref(), Some("90kt"));
        s.active_vertical = Some(VerticalMode::Pit);
        assert_eq!(afcs_box(&s, "GPS").reference, None);
    }

    #[test]
    fn flight_director_alone_is_hand_flown_and_shows_hollow_bars() {
        let mut s = state();
        s.flight_director = true;
        s.active_lateral = Some(LateralMode::Rol);
        s.active_vertical = Some(VerticalMode::Pit);
        let b = afcs_box(&s, "GPS");
        assert!(b.hand_flying && b.autopilot.is_none());
        // Roll hold and pitch hold are the director's defaults and appear active.
        assert_eq!(b.lateral_active.unwrap().text, "ROL");
        assert_eq!(b.vertical_active.unwrap().text, "PIT");
    }

    #[test]
    fn a_disconnect_flashes_the_autopilot_annunciation_yellow() {
        let mut s = state();
        s.ap_flash = true;
        s.ap_flash_visible = true;
        let on = afcs_box(&s, "GPS").autopilot.unwrap();
        assert_eq!(on.tone, AfcsTone::Caution { visible: true });
        s.ap_flash_visible = false;
        assert_eq!(
            afcs_box(&s, "GPS").autopilot.unwrap().tone,
            AfcsTone::Caution { visible: false }
        );
    }

    #[test]
    fn flight_director_bug_steps_the_active_references_by_the_tables_increments() {
        let mut s = state();
        s.active_vertical = Some(VerticalMode::Pit);
        let Some(PanelSetting::Pitch { pitch_rad }) = fd_bug_step(&s, 2) else {
            panic!("pitch");
        };
        assert!((pitch_rad - (0.05 + 1.0_f64.to_radians())).abs() < 1e-12);
        let Some(PanelSetting::Pitch { pitch_rad }) = fd_bug_step(&s, -1) else {
            panic!()
        };
        assert!((pitch_rad - (0.05 - 0.5_f64.to_radians())).abs() < 1e-12);

        s.active_vertical = Some(VerticalMode::Vs);
        s.vertical_speed_mps = 0.;
        let Some(PanelSetting::VerticalSpeed { vertical_speed_mps }) = fd_bug_step(&s, 3) else {
            panic!("vs");
        };
        assert!((vertical_speed_mps * FEET * 60. - 300.).abs() < 1e-9);

        s.active_vertical = Some(VerticalMode::Ias);
        s.airspeed_mps = 40.;
        let Some(PanelSetting::Airspeed { airspeed_mps }) = fd_bug_step(&s, 5) else {
            panic!("ias");
        };
        assert!(((airspeed_mps - 40.) * KNOTS - 5.).abs() < 1e-9);
    }

    #[test]
    fn flight_director_bug_clamps_and_does_nothing_without_a_steppable_mode() {
        let mut s = state();
        assert_eq!(fd_bug_step(&s, 1), None);
        s.active_vertical = Some(VerticalMode::Alt);
        assert_eq!(fd_bug_step(&s, 1), None);
        s.active_vertical = Some(VerticalMode::Pit);
        let Some(PanelSetting::Pitch { pitch_rad }) = fd_bug_step(&s, 100) else {
            panic!()
        };
        assert_eq!(pitch_rad, 0.35);
        s.active_vertical = Some(VerticalMode::Ias);
        let Some(PanelSetting::Airspeed { airspeed_mps }) = fd_bug_step(&s, -100) else {
            panic!()
        };
        assert_eq!(airspeed_mps, 20.);
    }

    #[test]
    fn pushing_the_flight_director_knob_syncs_pitch() {
        assert_eq!(fd_bug_sync(0.1), PanelSetting::Pitch { pitch_rad: 0.1 });
        assert_eq!(fd_bug_sync(1.0), PanelSetting::Pitch { pitch_rad: 0.35 });
    }

    #[test]
    fn course_sync_takes_the_active_leg_else_the_heading_in_whole_degrees() {
        let deg = f64::to_radians;
        let t = course_sync_target(true, deg(123.4), deg(10.));
        assert!((t - deg(123.)).abs() < 1e-12);
        let t = course_sync_target(false, deg(123.4), deg(269.6));
        assert!((t - deg(270.)).abs() < 1e-12);
        // 359.6 rounds up through north to zero.
        let t = course_sync_target(false, 0., deg(359.6));
        assert!(t.abs() < 1e-12);
    }

    #[test]
    fn only_a_lost_primary_display_disconnects_the_autopilot() {
        let pfd = |lost| DisplayHealth {
            primary_pfd: true,
            lost,
        };
        let mfd = |lost| DisplayHealth {
            primary_pfd: false,
            lost,
        };
        assert!(!primary_display_lost(&[pfd(false), mfd(false)]));
        assert!(primary_display_lost(&[pfd(true), mfd(false)]));
        assert!(!primary_display_lost(&[pfd(false), mfd(true)]));
        assert!(!primary_display_lost(&[]));
    }

    #[test]
    fn reversion_removes_the_flight_director() {
        assert!(command_bars_available(false));
        assert!(!command_bars_available(true));
    }
}
