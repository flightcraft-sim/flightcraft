//! Aircraft equipment autopilots, independent of the idealised script assists.
mod digital_two_axis;
mod two_axis_legacy;
mod wing_leveler;

use crate::{
    Controls, DT_S, SimError,
    autopilot::wrap,
    nav_source::{NavGuidance, NavKind},
    systems::{SensedInstruments, SystemsState},
};
pub use flightcraft_aero::autopilot::{AutopilotFamily, AutopilotInstallation, LateralMode, VerticalMode};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EquipmentFailure {
    RollServo,
    PitchServo,
    Trim,
    RollSensor,
    PressureSensor,
    AttitudeSensor,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisconnectReason {
    Pilot,
    Power,
    Servo,
    RollSensor,
    PressureSensor,
    AttitudeSensor,
    Navigation,
}

/// All controls are tagged actions, including momentary yoke buttons and knob pushes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PanelSetting {
    Engage {
        on: bool,
    },
    Disconnect,
    FlightDirector {
        on: bool,
    },
    Lateral {
        mode: LateralMode,
    },
    Vertical {
        mode: VerticalMode,
    },
    Level,
    Turn {
        #[schemars(range(min = -0.05236, max = 0.05236))]
        rate_rps: f64,
    },
    Track {
        #[schemars(range(min = 0.0, max = core::f64::consts::TAU))]
        track_rad: f64,
    },
    SelectedAltitude {
        altitude_m: f64,
    },
    Altitude {
        #[schemars(range(min = 2.0, max = 10000.0))]
        altitude_m: f64,
    },
    /// Move the altitude the hold mode is holding (the flight director's reference), as the
    /// pitch wheel does in ALT. Only while altitude hold is the active vertical mode.
    HoldAltitude {
        #[schemars(range(min = 2.0, max = 10000.0))]
        altitude_m: f64,
    },
    VerticalSpeed {
        #[schemars(range(min = -8.0, max = 8.0))]
        vertical_speed_mps: f64,
    },
    Airspeed {
        #[schemars(range(min = 20.0, max = 90.0))]
        airspeed_mps: f64,
    },
    Pitch {
        #[schemars(range(min = -0.25, max = 0.35))]
        pitch_rad: f64,
    },
    YawDamper {
        on: bool,
    },
    ControlWheelSteering {
        pressed: bool,
    },
    Failure {
        failure: EquipmentFailure,
        failed: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema, serde::Deserialize)]
pub struct EquipmentState {
    pub installation: AutopilotInstallation,
    pub engaged: bool,
    pub flight_director: bool,
    pub active_lateral: Option<LateralMode>,
    pub armed_lateral: Option<LateralMode>,
    pub active_vertical: Option<VerticalMode>,
    pub armed_vertical: Option<VerticalMode>,
    pub alts_armed: bool,
    pub heading_rad: f64,
    pub track_rad: f64,
    pub altitude_m: f64,
    pub captured_altitude_m: f64,
    pub vertical_speed_mps: f64,
    pub airspeed_mps: f64,
    pub pitch_rad: f64,
    pub turn_rate_rps: f64,
    pub command_bank_rad: f64,
    pub command_pitch_rad: f64,
    pub roll_servo_engaged: bool,
    pub pitch_servo_engaged: bool,
    pub yaw_servo_engaged: bool,
    pub servo_deflection: [f64; 3],
    pub trim: f64,
    pub trim_in_motion: bool,
    pub mistrim: bool,
    pub altitude_alert: bool,
    pub underspeed: bool,
    pub overspeed: bool,
    pub stability_active: bool,
    pub pilot_overpower: bool,
    pub control_wheel_steering: bool,
    pub yaw_damper: bool,
    pub failures: Vec<EquipmentFailure>,
    pub disconnect_reason: Option<DisconnectReason>,
    pub disconnect_tone: bool,
    pub disconnect_sequence: u64,
    pub ap_flash: bool,
    pub ap_flash_visible: bool,
}

#[derive(Clone, Debug, Default, Serialize, serde::Deserialize)]
pub(super) struct Loop {
    integral: f64,
}
impl Loop {
    fn pi(&mut self, error: f64, kp: f64, ki: f64, limit: f64) -> f64 {
        let candidate = self.integral + error * ki * DT_S;
        let output = error * kp + candidate;
        if output.abs() <= limit || output * error < 0.0 {
            self.integral = candidate.clamp(-limit, limit);
        }
        (error * kp + self.integral).clamp(-limit, limit)
    }
}

#[derive(Clone, Debug, Serialize, serde::Deserialize)]
pub(crate) struct Equipment {
    pub(crate) state: EquipmentState,
    roll: Loop,
    pitch: Loop,
    climb: Loop,
    speed: Loop,
    base: Controls,
    previous_pitch: f64,
    previous_altitude: f64,
    pressure_vs: f64,
    command_vs: f64,
    previous_vs: f64,
    mistrim_s: f64,
    disconnect_s: f64,
    capture_pending: bool,
    heading_direction: f64,
    sensor_override: Option<SensedInstruments>,
    receiver_air_data_valid: bool,
}
fn invalid(message: &str) -> SimError {
    SimError::InvalidAutopilot(message.into())
}
fn range(value: f64, lo: f64, hi: f64) -> Result<(), SimError> {
    if value.is_finite() && (lo..=hi).contains(&value) {
        Ok(())
    } else {
        Err(invalid("panel value outside finite permitted range"))
    }
}

impl Equipment {
    pub(crate) fn new(
        installation: AutopilotInstallation,
        systems: &SystemsState,
        pilot: Controls,
    ) -> Result<Self, SimError> {
        installation.validate()?;
        for id in [&installation.power_load, &installation.roll_sensor_load] {
            if !systems.electrical.loads.iter().any(|l| &l.id == id) {
                return Err(invalid("autopilot references an uninstalled electrical load"));
            }
        }
        let s = systems.sensed;
        Ok(Self {
            state: EquipmentState {
                installation,
                engaged: false,
                flight_director: false,
                active_lateral: None,
                armed_lateral: None,
                active_vertical: None,
                armed_vertical: None,
                alts_armed: false,
                heading_rad: systems.controls.heading_bug_rad,
                track_rad: s.heading_rad,
                altitude_m: s.altitude_msl_m,
                captured_altitude_m: s.altitude_msl_m,
                vertical_speed_mps: 0.0,
                airspeed_mps: s.indicated_airspeed_mps,
                pitch_rad: s.pitch_rad,
                turn_rate_rps: 0.0,
                command_bank_rad: 0.0,
                command_pitch_rad: s.pitch_rad,
                roll_servo_engaged: false,
                pitch_servo_engaged: false,
                yaw_servo_engaged: false,
                servo_deflection: [0.0; 3],
                trim: 0.0,
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
            },
            roll: Loop::default(),
            pitch: Loop::default(),
            climb: Loop::default(),
            speed: Loop::default(),
            base: pilot,
            previous_pitch: s.pitch_rad,
            previous_altitude: s.altitude_msl_m,
            pressure_vs: s.vertical_speed_mps,
            command_vs: s.vertical_speed_mps,
            previous_vs: s.vertical_speed_mps,
            mistrim_s: 0.0,
            disconnect_s: 0.0,
            capture_pending: false,
            heading_direction: 0.0,
            sensor_override: None,
            receiver_air_data_valid: true,
        })
    }
    pub(crate) fn set_sensors(&mut self, sensors: Option<SensedInstruments>) -> Result<(), SimError> {
        if let Some(s) = sensors {
            for (v, lo, hi) in [
                (s.roll_rad, -core::f64::consts::PI, core::f64::consts::PI),
                (
                    s.pitch_rad,
                    -core::f64::consts::FRAC_PI_2,
                    core::f64::consts::FRAC_PI_2,
                ),
                (s.heading_rad, 0.0, core::f64::consts::TAU),
                (s.indicated_airspeed_mps, 0.0, 500.0),
                (s.altitude_msl_m, -1000.0, 20000.0),
                (s.vertical_speed_mps, -500.0, 500.0),
                (s.turn_rate_rps, -100.0, 100.0),
                (s.turn_coordinator_rate_rps, -100.0, 100.0),
                (s.lateral_acceleration_g, -100.0, 100.0),
            ] {
                range(v, lo, hi)?;
            }
        }
        self.sensor_override = sensors;
        Ok(())
    }
    pub(crate) fn set_receiver_air_data_valid(&mut self, valid: bool) {
        self.receiver_air_data_valid = valid;
    }
    fn sensors(&self, systems: &SystemsState) -> SensedInstruments {
        self.sensor_override.unwrap_or(systems.sensed)
    }
    fn failed(&self, failure: EquipmentFailure) -> bool {
        self.state.failures.contains(&failure)
    }
    fn powered(&self, systems: &SystemsState, id: &str) -> bool {
        systems.electrical.loads.iter().any(|l| l.id == id && l.powered)
    }
    fn health(&self, systems: &SystemsState) -> Option<DisconnectReason> {
        let s = &self.state;
        if !self.powered(systems, &s.installation.power_load) {
            return Some(DisconnectReason::Power);
        }
        if self.failed(EquipmentFailure::RollServo)
            || (s.installation.family != AutopilotFamily::WingLeveler
                && self.failed(EquipmentFailure::PitchServo))
        {
            return Some(DisconnectReason::Servo);
        }
        if self.failed(EquipmentFailure::RollSensor)
            || !self.powered(systems, &s.installation.roll_sensor_load)
        {
            return Some(DisconnectReason::RollSensor);
        }
        if s.installation.family != AutopilotFamily::WingLeveler
            && (!self.receiver_air_data_valid
                || self.failed(EquipmentFailure::PressureSensor)
                || (systems.failures.static_blocked && !systems.controls.alternate_static))
        {
            return Some(DisconnectReason::PressureSensor);
        }
        if s.active_lateral == Some(LateralMode::Hdg) && !self.sensors(systems).attitude_valid {
            return Some(DisconnectReason::AttitudeSensor);
        }
        if s.active_vertical == Some(VerticalMode::Ias) && systems.failures.pitot_blocked {
            return Some(DisconnectReason::PressureSensor);
        }
        if s.installation.family == AutopilotFamily::DigitalTwoAxis
            && (self.failed(EquipmentFailure::AttitudeSensor) || !self.sensors(systems).attitude_valid)
        {
            return Some(DisconnectReason::AttitudeSensor);
        }
        None
    }
    fn disconnect(&mut self, reason: DisconnectReason) {
        let s = &mut self.state;
        if s.engaged {
            s.disconnect_sequence = s.disconnect_sequence.saturating_add(1);
            self.disconnect_s = 5.0;
        }
        s.engaged = false;
        if reason != DisconnectReason::Pilot || s.installation.family != AutopilotFamily::DigitalTwoAxis {
            s.active_lateral = None;
            s.active_vertical = None;
            s.armed_lateral = None;
            s.armed_vertical = None;
            s.alts_armed = false;
            s.flight_director = false;
        }
        s.roll_servo_engaged = false;
        s.pitch_servo_engaged = false;
        s.yaw_servo_engaged = false;
        s.servo_deflection = [0.0; 3];
        s.trim_in_motion = false;
        s.disconnect_reason = Some(reason);
        s.disconnect_tone = self.disconnect_s > 4.0;
        s.ap_flash = self.disconnect_s > 0.0;
        s.ap_flash_visible = s.ap_flash;
    }
    pub(crate) fn set(
        &mut self,
        setting: PanelSetting,
        systems: &SystemsState,
        pilot: Controls,
    ) -> Result<(), SimError> {
        let sensed = self.sensors(systems);
        if self.state.installation.family == AutopilotFamily::WingLeveler
            && !matches!(
                setting,
                PanelSetting::Engage { .. }
                    | PanelSetting::Disconnect
                    | PanelSetting::Turn { .. }
                    | PanelSetting::Failure { .. }
            )
        {
            return Err(invalid("single-axis equipment has only AP and TURN controls"));
        }
        match setting {
            PanelSetting::Engage { on: false } | PanelSetting::Disconnect => {
                self.disconnect(DisconnectReason::Pilot)
            }
            PanelSetting::Engage { on: true } => {
                if self.health(systems).is_some() {
                    return Err(invalid("autopilot sensor, power, or servo unavailable"));
                }
                if sensed.roll_rad.abs() > 0.79 || sensed.pitch_rad.abs() > 0.35 {
                    return Err(invalid("outside autopilot engagement attitude limits"));
                }
                let s = &mut self.state;
                if !s.engaged {
                    s.active_lateral = s.active_lateral.or(Some(LateralMode::Rol));
                    if s.installation.family != AutopilotFamily::WingLeveler {
                        s.active_vertical = s.active_vertical.or(Some(
                            if s.installation.family == AutopilotFamily::DigitalTwoAxis {
                                VerticalMode::Pit
                            } else {
                                VerticalMode::Vs
                            },
                        ));
                    }
                    s.pitch_rad = sensed.pitch_rad;
                    s.engaged = true;
                    s.flight_director = s.installation.family == AutopilotFamily::DigitalTwoAxis;
                    s.disconnect_reason = None;
                    self.base = pilot;
                    self.roll = Loop::default();
                    self.pitch = Loop::default();
                    self.climb = Loop::default();
                    self.speed = Loop::default();
                }
            }
            PanelSetting::FlightDirector { on } => {
                if self.state.installation.family != AutopilotFamily::DigitalTwoAxis {
                    return Err(invalid("no flight director installed"));
                }
                if !on && self.state.engaged {
                    self.disconnect(DisconnectReason::Pilot);
                }
                self.state.flight_director = on;
                if !on {
                    self.state.active_lateral = None;
                    self.state.active_vertical = None;
                    self.state.armed_lateral = None;
                    self.state.armed_vertical = None;
                    self.state.alts_armed = false;
                }
                if on {
                    self.state.active_lateral = self.state.active_lateral.or(Some(LateralMode::Rol));
                    self.state.active_vertical = self.state.active_vertical.or(Some(VerticalMode::Pit));
                }
            }
            PanelSetting::Lateral { mode } => {
                if !self.state.installation.lateral_modes.contains(&mode) {
                    return Err(invalid("lateral mode not installed"));
                }
                self.heading_direction = 0.0;
                if matches!(
                    mode,
                    LateralMode::Nav | LateralMode::Apr | LateralMode::Rev | LateralMode::LocBc
                ) {
                    self.state.armed_lateral = Some(mode);
                } else {
                    self.state.active_lateral = Some(mode);
                    self.state.armed_lateral = None;
                }
            }
            PanelSetting::Vertical { mode } => {
                if !self.state.installation.vertical_modes.contains(&mode) {
                    return Err(invalid("vertical mode not installed"));
                }
                if matches!(mode, VerticalMode::Gp | VerticalMode::Gs) {
                    self.state.armed_vertical = Some(mode);
                } else {
                    self.state.active_vertical = Some(mode);
                    self.state.armed_vertical = None;
                    if mode == VerticalMode::Alt {
                        self.state.captured_altitude_m = sensed.altitude_msl_m;
                        self.state.alts_armed = false;
                    }
                    if mode == VerticalMode::Pit {
                        self.state.pitch_rad = sensed.pitch_rad;
                    }
                }
            }
            PanelSetting::Level => {
                if self.state.installation.family != AutopilotFamily::DigitalTwoAxis {
                    return Err(invalid("LVL not installed"));
                }
                self.set(PanelSetting::Engage { on: true }, systems, pilot)?;
                self.state.active_lateral = Some(LateralMode::Rol);
                self.state.active_vertical = Some(VerticalMode::Vs);
                self.state.vertical_speed_mps = 0.0;
                // Start recovery with a modest lift attitude; the pressure loop learns trim.
                self.state.pitch_rad = 0.08;
                self.climb = Loop::default();
                self.state.turn_rate_rps = 0.0;
                self.state.armed_lateral = None;
                self.state.armed_vertical = None;
                self.state.alts_armed = false;
            }
            PanelSetting::Turn { rate_rps } => {
                range(rate_rps, -0.05236, 0.05236)?;
                self.state.turn_rate_rps = rate_rps;
            }
            PanelSetting::Track { track_rad } => {
                range(track_rad, 0.0, core::f64::consts::TAU)?;
                self.state.track_rad = track_rad;
            }
            PanelSetting::SelectedAltitude { altitude_m } => {
                range(altitude_m, 2.0, 10000.0)?;
                self.state.altitude_m = altitude_m;
            }
            PanelSetting::Altitude { altitude_m } => {
                range(altitude_m, 2.0, 10000.0)?;
                if !self.state.installation.altitude_preselect {
                    return Err(invalid("altitude preselect not installed"));
                }
                self.state.altitude_m = altitude_m;
                self.state.alts_armed = true;
                self.capture_pending = true;
            }
            PanelSetting::HoldAltitude { altitude_m } => {
                range(altitude_m, 2.0, 10000.0)?;
                if self.state.active_vertical != Some(VerticalMode::Alt) {
                    return Err(invalid("altitude hold is not the active vertical mode"));
                }
                self.state.captured_altitude_m = altitude_m;
            }
            PanelSetting::VerticalSpeed { vertical_speed_mps } => {
                range(vertical_speed_mps, -8.0, 8.0)?;
                self.state.vertical_speed_mps = vertical_speed_mps;
            }
            PanelSetting::Airspeed { airspeed_mps } => {
                range(airspeed_mps, 20.0, 90.0)?;
                self.state.airspeed_mps = airspeed_mps;
            }
            PanelSetting::Pitch { pitch_rad } => {
                range(pitch_rad, -0.25, 0.35)?;
                self.state.pitch_rad = pitch_rad;
            }
            PanelSetting::YawDamper { on } => {
                if !self.state.installation.yaw_damper {
                    return Err(invalid("yaw damper not installed"));
                }
                self.state.yaw_damper = on;
            }
            PanelSetting::ControlWheelSteering { pressed } => {
                if !pressed && self.state.control_wheel_steering {
                    self.state.pitch_rad = sensed.pitch_rad;
                    if self.state.active_vertical == Some(VerticalMode::Alt) {
                        self.state.captured_altitude_m = sensed.altitude_msl_m;
                    }
                    self.state.vertical_speed_mps = sensed.vertical_speed_mps;
                    self.base = pilot;
                    self.heading_direction = 0.0;
                    self.pitch = Loop::default();
                    self.roll = Loop::default();
                    self.climb = Loop::default();
                }
                self.state.control_wheel_steering = pressed;
            }
            PanelSetting::Failure { failure, failed } => {
                self.state.failures.retain(|v| *v != failure);
                if failed {
                    self.state.failures.push(failure);
                }
            }
        }
        Ok(())
    }

    fn stabilize(&mut self, systems: &SystemsState, mut output: Controls) -> Controls {
        if self.state.installation.stability_protection && self.health(systems).is_none() {
            let sensed = self.sensors(systems);
            let roll_excess =
                (sensed.roll_rad.abs() - core::f64::consts::FRAC_PI_4).max(0.0) * sensed.roll_rad.signum();
            let pitch_excess = (sensed.pitch_rad.abs() - 0.3491).max(0.0) * sensed.pitch_rad.signum();
            self.state.stability_active = roll_excess != 0.0 || pitch_excess != 0.0;
            let torque = self.state.installation.servo_torque.min(0.2);
            output.aileron += (-roll_excess * 0.5).clamp(-torque, torque);
            output.elevator += (-pitch_excess * 0.5).clamp(-torque, torque);
        }
        output
    }

    pub(crate) fn update(
        &mut self,
        systems: &SystemsState,
        pilot: Controls,
        track: f64,
        ground_speed: f64,
        load_g: f64,
        nav: Option<NavGuidance>,
    ) -> Controls {
        let sensed = self.sensors(systems);
        self.disconnect_s = (self.disconnect_s - DT_S).max(0.0);
        self.state.disconnect_tone = self.disconnect_s > 4.0;
        self.state.ap_flash = self.disconnect_s > 0.0;
        self.state.ap_flash_visible =
            self.state.ap_flash && (libm::floor(self.disconnect_s * 4.0) as u32).is_multiple_of(2);
        if let Some(reason) = self.health(systems)
            && (self.state.engaged || self.state.flight_director)
        {
            self.disconnect(reason);
            self.state.flight_director = false;
        }
        let pitch_rate = wrap(sensed.pitch_rad - self.previous_pitch) / DT_S;
        self.previous_pitch = sensed.pitch_rad;
        let raw_vs = (sensed.altitude_msl_m - self.previous_altitude) / DT_S;
        self.previous_altitude = sensed.altitude_msl_m;
        self.pressure_vs += (raw_vs - self.pressure_vs) * (1.0 - libm::exp(-DT_S / 0.35));
        let acceleration = (self.pressure_vs - self.previous_vs) / DT_S;
        self.previous_vs = self.pressure_vs;
        let legacy = self.state.installation.family != AutopilotFamily::DigitalTwoAxis;
        let bank = if legacy {
            two_axis_legacy::bank_from_rate(sensed.turn_rate_rps, sensed.indicated_airspeed_mps)
        } else {
            sensed.roll_rad
        };
        let bug = systems.controls.heading_bug_rad;
        let heading_error = wrap(bug - sensed.heading_rad);
        if bug != self.state.heading_rad || self.heading_direction == 0.0 {
            self.heading_direction = heading_error.signum();
        }
        self.state.heading_rad = bug;
        // Retain the sampled short-way choice at the reciprocal through adverse yaw on roll-in.
        let heading_error = if heading_error.abs() > 170.0 * core::f64::consts::PI / 180.0 {
            heading_error.abs() * self.heading_direction
        } else {
            heading_error
        };
        self.state.stability_active = false;
        self.state.roll_servo_engaged = false;
        self.state.pitch_servo_engaged = false;
        self.state.yaw_servo_engaged = false;
        self.state.trim_in_motion = false;
        self.state.underspeed = false;
        self.state.overspeed = false;
        let mut output = pilot;
        output.elevator += self.state.trim;
        if !self.state.engaged && !self.state.flight_director {
            self.state.servo_deflection = [0.0; 3];
            return self.stabilize(systems, output).clamped();
        }
        let nav = nav.filter(|n| n.validate().is_ok());
        if (self.state.armed_lateral == Some(LateralMode::Apr)
            || self.state.active_lateral == Some(LateralMode::Apr))
            && self.state.armed_vertical.is_none()
            && !matches!(
                self.state.active_vertical,
                Some(VerticalMode::Gs | VerticalMode::Gp)
            )
            && let Some(n) = nav
        {
            self.state.armed_vertical = match n.kind {
                NavKind::Localizer => Some(VerticalMode::Gs),
                NavKind::Gps if !legacy && n.path_angle_rad.is_some() => Some(VerticalMode::Gp),
                _ => None,
            };
        }
        if let (Some(mode), Some(n)) = (self.state.armed_lateral, nav) {
            let compatible = match mode {
                LateralMode::Rev | LateralMode::LocBc => n.kind == NavKind::BackCourse,
                _ => n.kind != NavKind::BackCourse,
            };
            if compatible && n.cross_track_m.abs() < if mode == LateralMode::Apr { 100.0 } else { 5000.0 } {
                self.state.active_lateral = Some(mode);
                self.state.armed_lateral = None;
            }
        }
        let nav_mode = matches!(
            self.state.active_lateral,
            Some(LateralMode::Nav | LateralMode::Apr | LateralMode::Rev | LateralMode::LocBc)
        );
        let vertical_path = matches!(
            self.state.active_vertical,
            Some(VerticalMode::Gp | VerticalMode::Gs)
        );
        let missing_path = vertical_path
            && nav.is_none_or(|n| n.vertical_deviation_m.is_none() || n.path_angle_rad.is_none());
        if (nav_mode && nav.is_none()) || missing_path {
            self.disconnect(DisconnectReason::Navigation);
            self.state.flight_director = false;
            return output.clamped();
        }
        let limit = self.state.installation.bank_limit_rad;
        let desired_bank = match self.state.active_lateral {
            Some(LateralMode::Hdg) => (1.3 * heading_error).clamp(-limit, limit),
            Some(LateralMode::Trk) => (1.3 * wrap(self.state.track_rad - track)).clamp(-limit, limit),
            Some(LateralMode::Nav | LateralMode::Apr | LateralMode::Rev | LateralMode::LocBc) => {
                nav.map_or(0.0, |n| {
                    let intercept = (-n.cross_track_m * 0.003).clamp(-0.7, 0.7);
                    (1.3 * wrap(n.desired_track_rad + intercept - track)).clamp(-limit, limit)
                })
            }
            _ => two_axis_legacy::bank_from_rate(self.state.turn_rate_rps, sensed.indicated_airspeed_mps)
                .clamp(-limit, limit),
        };
        self.state.command_bank_rad = wing_leveler::slew(self.state.command_bank_rad, desired_bank, 0.22);
        let desired_turn = flightcraft_world::GRAVITY_MPS2 * libm::tan(self.state.command_bank_rad)
            / sensed.indicated_airspeed_mps.max(20.0);
        let roll_demand = if legacy {
            wing_leveler::roll(&mut self.roll, sensed.turn_coordinator_rate_rps, desired_turn)
        } else {
            self.roll.pi(self.state.command_bank_rad - bank, 1.4, 0.06, 0.8) - 0.35 * sensed.turn_rate_rps
        };
        let mut desired_vs = match self.state.active_vertical {
            Some(VerticalMode::Alt) => {
                ((self.state.captured_altitude_m - sensed.altitude_msl_m) * 0.12).clamp(-4.0, 4.0)
            }
            Some(VerticalMode::Ias) => self.speed.pi(
                sensed.indicated_airspeed_mps - self.state.airspeed_mps,
                0.9,
                0.10,
                6.0,
            ),
            Some(VerticalMode::Gp | VerticalMode::Gs) => nav
                .and_then(|n| n.vertical_deviation_m.zip(n.path_angle_rad))
                .map_or(0.0, |(d, a)| ground_speed * libm::tan(a) - d * 0.12),
            _ => self.state.vertical_speed_mps,
        };
        if let (Some(mode), Some(n)) = (self.state.armed_vertical, nav) {
            let compatible = (mode == VerticalMode::Gp && n.kind == NavKind::Gps)
                || (mode == VerticalMode::Gs && n.kind == NavKind::Localizer);
            if compatible
                && n.vertical_deviation_m.is_some_and(|v| v.abs() < 30.0)
                && n.path_angle_rad.is_some()
            {
                self.state.active_vertical = Some(mode);
                self.state.armed_vertical = None;
            }
        }
        if self.state.alts_armed
            && self.capture_pending
            && self.state.active_vertical != Some(VerticalMode::Alt)
        {
            let error = self.state.altitude_m - sensed.altitude_msl_m;
            if desired_vs * error >= 0.0 && error.abs() < (self.pressure_vs.abs() * 8.0).max(15.0) {
                self.state.active_vertical = Some(VerticalMode::Alt);
                self.state.captured_altitude_m = self.state.altitude_m;
                self.state.alts_armed = false;
                self.capture_pending = false;
                desired_vs = (error * 0.12).clamp(-4.0, 4.0);
            }
        }
        self.state.altitude_alert = self.state.installation.altitude_alert
            && (self.state.altitude_m - sensed.altitude_msl_m).abs() < 60.96;
        if !legacy {
            (desired_vs, self.state.underspeed, self.state.overspeed) = digital_two_axis::protected_vs(
                sensed.indicated_airspeed_mps,
                self.state.installation.minimum_ias_mps,
                self.state.installation.maximum_ias_mps,
                desired_vs,
            );
        }
        let desired_pitch = if self.state.active_vertical == Some(VerticalMode::Pit)
            && !self.state.underspeed
            && !self.state.overspeed
        {
            self.state.pitch_rad
        } else {
            self.climb.pi(desired_vs - self.pressure_vs, 0.018, 0.004, 0.3)
                + self.state.pitch_rad
                + desired_vs / sensed.indicated_airspeed_mps.max(25.0)
        };
        self.state.command_pitch_rad = wing_leveler::slew(
            self.state.command_pitch_rad,
            desired_pitch.clamp(-0.25, 0.35),
            self.state.installation.pitch_rate_limit_rps,
        );
        self.command_vs = wing_leveler::slew(
            self.command_vs,
            desired_vs,
            sensed.indicated_airspeed_mps.max(20.0) * self.state.installation.pitch_rate_limit_rps,
        );
        let mut pitch_demand = if legacy {
            two_axis_legacy::elevator(&mut self.pitch, self.command_vs, self.pressure_vs, acceleration)
        } else {
            digital_two_axis::elevator(
                &mut self.pitch,
                self.state.command_pitch_rad,
                sensed.pitch_rad,
                pitch_rate,
            )
        };
        if load_g > self.state.installation.max_load_g
            || (!legacy && pitch_rate > self.state.installation.pitch_rate_limit_rps)
        {
            pitch_demand = pitch_demand.min(0.0);
        }
        if load_g < 0.5 || (!legacy && pitch_rate < -self.state.installation.pitch_rate_limit_rps) {
            pitch_demand = pitch_demand.max(0.0);
        }
        if !self.state.engaged || self.state.control_wheel_steering {
            self.state.servo_deflection = [0.0; 3];
            if !self.state.engaged {
                output = self.stabilize(systems, output);
            }
            return output.clamped();
        }
        let torque = self.state.installation.servo_torque;
        let rate = self.state.installation.servo_rate_per_s;
        let [old_roll, old_pitch, old_yaw] = self.state.servo_deflection;
        let overpower_roll = (pilot.aileron - self.base.aileron).abs() > torque;
        let overpower_pitch = (pilot.elevator - self.base.elevator).abs() > torque;
        self.state.pilot_overpower = overpower_roll || overpower_pitch;
        let roll = if overpower_roll {
            0.0
        } else {
            wing_leveler::slew(old_roll, roll_demand.clamp(-torque, torque), rate)
        };
        let mut pitch = if overpower_pitch {
            0.0
        } else {
            wing_leveler::slew(
                old_pitch,
                (pitch_demand - self.state.trim).clamp(-torque, torque),
                rate,
            )
        };
        self.state.roll_servo_engaged = !overpower_roll;
        self.state.pitch_servo_engaged =
            self.state.installation.family != AutopilotFamily::WingLeveler && !overpower_pitch;
        if self.state.pitch_servo_engaged && self.state.installation.electric_trim {
            if pitch.abs() > 0.04 && !self.failed(EquipmentFailure::Trim) {
                let delta = (pitch * 0.08 * DT_S).clamp(-0.025 * DT_S, 0.025 * DT_S);
                let trim = (self.state.trim + delta).clamp(-0.5, 0.5);
                pitch -= trim - self.state.trim;
                self.state.trim_in_motion = (trim - self.state.trim).abs() > 1e-10;
                self.state.trim = trim;
            }
            if pitch.abs() > 0.12 {
                self.mistrim_s += DT_S;
            } else {
                self.mistrim_s = 0.0;
            }
            self.state.mistrim = self.mistrim_s > 3.0;
        }
        let yaw = if self.state.yaw_damper {
            wing_leveler::slew(
                old_yaw,
                (-sensed.lateral_acceleration_g * 0.8).clamp(-torque, torque),
                rate,
            )
        } else {
            0.0
        };
        self.state.yaw_servo_engaged = self.state.yaw_damper;
        self.state.servo_deflection = [
            roll,
            if self.state.pitch_servo_engaged {
                pitch
            } else {
                0.0
            },
            yaw,
        ];
        output.aileron += roll;
        if self.state.pitch_servo_engaged {
            output.elevator = pilot.elevator + self.state.trim + pitch;
        }
        output.rudder += yaw;
        output.clamped()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    fn unit(family: AutopilotFamily) -> (Equipment, SystemsState, Controls) {
        let sim = crate::Sim::new(&crate::Start::Air {
            height_m: 1000.0,
            airspeed_mps: 40.0,
            heading_rad: 0.0,
        })
        .unwrap();
        let mut installation = AutopilotInstallation::new(family);
        installation.roll_sensor_load = "turn_coordinator".into();
        let systems = sim.systems().clone();
        let pilot = sim.controls();
        (
            Equipment::new(installation, &systems, pilot).unwrap(),
            systems,
            pilot,
        )
    }
    #[test]
    fn each_family_checks_engagement_and_only_supports_its_modes() {
        for family in [
            AutopilotFamily::WingLeveler,
            AutopilotFamily::TwoAxisLegacy,
            AutopilotFamily::DigitalTwoAxis,
        ] {
            let (mut ap, mut systems, pilot) = unit(family);
            systems.sensed.roll_rad = 0.9;
            assert!(
                ap.set(PanelSetting::Engage { on: true }, &systems, pilot)
                    .is_err()
            );
            systems.sensed.roll_rad = 0.0;
            systems.sensed.pitch_rad = -0.4;
            assert!(
                ap.set(PanelSetting::Engage { on: true }, &systems, pilot)
                    .is_err()
            );
            systems.sensed.pitch_rad = 0.1;
            ap.set(PanelSetting::Engage { on: true }, &systems, pilot)
                .unwrap();
            assert!(ap.state.engaged);
            assert!(
                ap.set(PanelSetting::Turn { rate_rps: f64::NAN }, &systems, pilot)
                    .is_err()
            );
            if family == AutopilotFamily::WingLeveler {
                assert!(
                    ap.set(
                        PanelSetting::Vertical {
                            mode: VerticalMode::Alt
                        },
                        &systems,
                        pilot
                    )
                    .is_err()
                );
                assert!(!ap.state.pitch_servo_engaged);
            }
            if family == AutopilotFamily::TwoAxisLegacy {
                assert!(
                    ap.set(
                        PanelSetting::Vertical {
                            mode: VerticalMode::Ias
                        },
                        &systems,
                        pilot
                    )
                    .is_err()
                );
            }
        }
    }
    #[test]
    fn director_only_has_bars_without_servos_and_cws_releases_clutches() {
        let (mut ap, systems, pilot) = unit(AutopilotFamily::DigitalTwoAxis);
        ap.set(PanelSetting::FlightDirector { on: true }, &systems, pilot)
            .unwrap();
        let c = ap.update(&systems, pilot, 0.0, 40.0, 1.0, None);
        assert_eq!(c, pilot);
        assert!(ap.state.flight_director && !ap.state.roll_servo_engaged && !ap.state.pitch_servo_engaged);
        ap.set(PanelSetting::Engage { on: true }, &systems, pilot)
            .unwrap();
        ap.set(
            PanelSetting::ControlWheelSteering { pressed: true },
            &systems,
            pilot,
        )
        .unwrap();
        assert_eq!(ap.update(&systems, pilot, 0.0, 40.0, 1.0, None), pilot);
        assert!(!ap.state.roll_servo_engaged && !ap.state.pitch_servo_engaged);
        ap.set(
            PanelSetting::ControlWheelSteering { pressed: false },
            &systems,
            pilot,
        )
        .unwrap();
        ap.update(&systems, pilot, 0.0, 40.0, 1.0, None);
        assert!(ap.state.roll_servo_engaged && ap.state.pitch_servo_engaged);
    }
    #[test]
    fn digital_envelope_protection_and_optional_stability_and_yaw_damper() {
        let (mut ap, mut systems, pilot) = unit(AutopilotFamily::DigitalTwoAxis);
        ap.set(PanelSetting::Engage { on: true }, &systems, pilot)
            .unwrap();
        systems.sensed.indicated_airspeed_mps = 24.0;
        ap.update(&systems, pilot, 0.0, 24.0, 1.0, None);
        assert!(ap.state.underspeed && !ap.state.overspeed);
        systems.sensed.indicated_airspeed_mps = 75.0;
        ap.update(&systems, pilot, 0.0, 75.0, 1.0, None);
        assert!(ap.state.overspeed && !ap.state.underspeed);
        ap.set(PanelSetting::Disconnect, &systems, pilot).unwrap();
        ap.set(PanelSetting::FlightDirector { on: false }, &systems, pilot)
            .unwrap();
        systems.sensed.roll_rad = 0.9;
        systems.sensed.pitch_rad = -0.4;
        let c = ap.update(&systems, pilot, 0.0, 40.0, 1.0, None);
        assert!(ap.state.stability_active && c.aileron < pilot.aileron && c.elevator > pilot.elevator);
        ap.state.installation.stability_protection = false;
        let c = ap.update(&systems, pilot, 0.0, 40.0, 1.0, None);
        assert!(!ap.state.stability_active);
        assert_eq!(c, pilot);
        assert!(
            ap.set(PanelSetting::YawDamper { on: true }, &systems, pilot)
                .is_err()
        );
        ap.state.installation.yaw_damper = true;
        systems.sensed.roll_rad = 0.0;
        systems.sensed.pitch_rad = 0.1;
        ap.set(PanelSetting::Engage { on: true }, &systems, pilot)
            .unwrap();
        ap.set(PanelSetting::YawDamper { on: true }, &systems, pilot)
            .unwrap();
        systems.sensed.lateral_acceleration_g = 0.2;
        let c = ap.update(&systems, pilot, 0.0, 40.0, 1.0, None);
        assert!(ap.state.yaw_servo_engaged && c.rudder < pilot.rudder);
    }
    #[test]
    fn the_hold_altitude_steps_only_in_alt_and_moves_the_director_reference() {
        let (mut ap, systems, pilot) = unit(AutopilotFamily::DigitalTwoAxis);
        ap.set(PanelSetting::Engage { on: true }, &systems, pilot)
            .unwrap();
        // Without altitude hold active there is no reference to step.
        assert!(
            ap.set(PanelSetting::HoldAltitude { altitude_m: 1100.0 }, &systems, pilot)
                .is_err()
        );
        ap.set(
            PanelSetting::Vertical {
                mode: VerticalMode::Alt,
            },
            &systems,
            pilot,
        )
        .unwrap();
        let held = ap.state.captured_altitude_m;
        let level = ap.update(&systems, pilot, 0.0, 40.0, 1.0, None);
        // Stepping the reference up 20 ft at a time: the stored hold altitude follows, and the mode,
        // the selected altitude, and the armed capture are untouched.
        let selected = ap.state.altitude_m;
        ap.set(
            PanelSetting::HoldAltitude {
                altitude_m: held + 6.096,
            },
            &systems,
            pilot,
        )
        .unwrap();
        assert_eq!(ap.state.captured_altitude_m, held + 6.096);
        assert_eq!(ap.state.active_vertical, Some(VerticalMode::Alt));
        assert_eq!(ap.state.altitude_m, selected);
        // A higher reference commands a climb (more up pitch than level hold).
        ap.set(
            PanelSetting::HoldAltitude {
                altitude_m: held + 60.0,
            },
            &systems,
            pilot,
        )
        .unwrap();
        let climb = ap.update(&systems, pilot, 0.0, 40.0, 1.0, None);
        assert!(
            climb.elevator != level.elevator,
            "the reference changes the pitch command"
        );
        // Bad values are refused and leave the reference alone.
        let before = ap.state.captured_altitude_m;
        for bad in [f64::NAN, 1.0, 20000.0] {
            assert!(
                ap.set(PanelSetting::HoldAltitude { altitude_m: bad }, &systems, pilot)
                    .is_err()
            );
        }
        assert_eq!(ap.state.captured_altitude_m, before);
    }
    #[test]
    fn selecting_an_altitude_while_in_alt_does_not_start_a_climb() {
        let (mut ap, systems, pilot) = unit(AutopilotFamily::DigitalTwoAxis);
        ap.set(PanelSetting::Engage { on: true }, &systems, pilot)
            .unwrap();
        ap.set(
            PanelSetting::Vertical {
                mode: VerticalMode::Alt,
            },
            &systems,
            pilot,
        )
        .unwrap();
        let held = ap.state.captured_altitude_m;
        ap.set(PanelSetting::Altitude { altitude_m: 2000.0 }, &systems, pilot)
            .unwrap();
        ap.update(&systems, pilot, 0.0, 40.0, 1.0, None);
        assert_eq!(ap.state.captured_altitude_m, held);
        assert_eq!(ap.state.altitude_m, 2000.0);
        assert!(ap.state.alts_armed);
        assert_eq!(ap.state.active_vertical, Some(VerticalMode::Alt));
    }

    #[test]
    fn electronic_sensor_adapter_uses_its_validity_and_rejects_hostile_snapshots() {
        let (mut ap, mut systems, pilot) = unit(AutopilotFamily::DigitalTwoAxis);
        let electronic = systems.sensed;
        ap.set_sensors(Some(electronic)).unwrap();
        systems.sensed.attitude_valid = false;
        ap.set(PanelSetting::Engage { on: true }, &systems, pilot)
            .unwrap();
        ap.update(&systems, pilot, 0.0, 40.0, 1.0, None);
        assert!(ap.state.engaged);
        let mut bad = electronic;
        bad.pitch_rad = f64::NAN;
        assert!(ap.set_sensors(Some(bad)).is_err());
        assert_eq!(ap.sensors(&systems), electronic);
        ap.set_sensors(None).unwrap();
        ap.update(&systems, pilot, 0.0, 40.0, 1.0, None);
        assert!(!ap.state.engaged);
        assert_eq!(ap.state.disconnect_reason, Some(DisconnectReason::AttitudeSensor));
    }

    #[test]
    fn pressure_roll_and_attitude_faults_and_servo_slew_are_observable() {
        for failure in [
            EquipmentFailure::PressureSensor,
            EquipmentFailure::AttitudeSensor,
            EquipmentFailure::PitchServo,
        ] {
            let (mut ap, systems, pilot) = unit(AutopilotFamily::DigitalTwoAxis);
            ap.set(PanelSetting::Engage { on: true }, &systems, pilot)
                .unwrap();
            ap.set(
                PanelSetting::Failure {
                    failure,
                    failed: true,
                },
                &systems,
                pilot,
            )
            .unwrap();
            ap.update(&systems, pilot, 0.0, 40.0, 1.0, None);
            assert!(!ap.state.engaged && ap.state.ap_flash && ap.state.disconnect_tone);
        }
        let (mut ap, mut systems, pilot) = unit(AutopilotFamily::DigitalTwoAxis);
        ap.set(PanelSetting::Engage { on: true }, &systems, pilot)
            .unwrap();
        ap.set(
            PanelSetting::Lateral {
                mode: LateralMode::Hdg,
            },
            &systems,
            pilot,
        )
        .unwrap();
        systems.controls.heading_bug_rad = 1.5;
        for _ in 0..120 {
            let [old, _, _] = ap.state.servo_deflection;
            ap.update(&systems, pilot, 0.0, 40.0, 1.0, None);
            let [roll, _, _] = ap.state.servo_deflection;
            assert!((roll - old).abs() <= ap.state.installation.servo_rate_per_s * DT_S + 1e-10);
            assert!(roll.abs() <= ap.state.installation.servo_torque);
        }
    }
}
