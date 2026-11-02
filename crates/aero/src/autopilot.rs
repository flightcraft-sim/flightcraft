//! Panel-installed, brand-neutral autopilot equipment definitions.
use crate::{AircraftError, aircraft::require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AutopilotFamily {
    WingLeveler,
    TwoAxisLegacy,
    DigitalTwoAxis,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LateralMode {
    Rol,
    Hdg,
    Trk,
    Nav,
    Apr,
    Rev,
    LocBc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VerticalMode {
    Pit,
    Alt,
    Vs,
    Ias,
    Gp,
    Gs,
}

/// Normalised control force is used for servo torque: 1 is full pilot authority.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AutopilotInstallation {
    pub family: AutopilotFamily,
    pub lateral_modes: Vec<LateralMode>,
    pub vertical_modes: Vec<VerticalMode>,
    pub power_load: String,
    pub roll_sensor_load: String,
    pub bank_limit_rad: f64,
    pub pitch_rate_limit_rps: f64,
    pub max_load_g: f64,
    pub servo_torque: f64,
    pub servo_rate_per_s: f64,
    pub electric_trim: bool,
    pub altitude_preselect: bool,
    pub altitude_alert: bool,
    pub stability_protection: bool,
    pub yaw_damper: bool,
    pub minimum_ias_mps: f64,
    pub maximum_ias_mps: f64,
}

impl AutopilotInstallation {
    #[must_use]
    pub fn new(family: AutopilotFamily) -> Self {
        use AutopilotFamily::*;
        use LateralMode::*;
        use VerticalMode::*;
        Self {
            family,
            lateral_modes: match family {
                WingLeveler => vec![Rol],
                TwoAxisLegacy => vec![Rol, Hdg, Nav, Apr, Rev],
                DigitalTwoAxis => vec![Rol, Hdg, Trk, Nav, Apr, LocBc],
            },
            vertical_modes: match family {
                WingLeveler => vec![],
                TwoAxisLegacy => vec![Alt, Vs],
                DigitalTwoAxis => vec![Pit, Alt, Vs, Ias, Gp, Gs],
            },
            power_load: "avionics".into(),
            roll_sensor_load: if family == DigitalTwoAxis {
                "gyros"
            } else {
                "turn_coordinator"
            }
            .into(),
            bank_limit_rad: 25.0 * core::f64::consts::PI / 180.0,
            pitch_rate_limit_rps: 0.12,
            max_load_g: 1.5,
            servo_torque: 0.65,
            servo_rate_per_s: 0.7,
            electric_trim: family != WingLeveler,
            altitude_preselect: family != WingLeveler,
            altitude_alert: family != WingLeveler,
            stability_protection: family == DigitalTwoAxis,
            yaw_damper: false,
            minimum_ias_mps: 27.0,
            maximum_ias_mps: 70.0,
        }
    }

    pub fn validate(&self) -> Result<(), AircraftError> {
        use AutopilotFamily::*;
        let allowed = Self::new(self.family);
        require(
            self.lateral_modes.contains(&LateralMode::Rol)
                && self.lateral_modes.len() <= 7
                && self
                    .lateral_modes
                    .iter()
                    .all(|m| allowed.lateral_modes.contains(m))
                && self.vertical_modes.len() <= 6
                && self
                    .vertical_modes
                    .iter()
                    .all(|m| allowed.vertical_modes.contains(m)),
            "autopilot.modes",
            "unsupported modes for equipment family",
        )?;
        let mut lateral = Vec::new();
        for mode in &self.lateral_modes {
            require(
                !lateral.contains(mode),
                "autopilot.lateral_modes",
                "duplicate mode",
            )?;
            lateral.push(*mode);
        }
        let mut vertical = Vec::new();
        for mode in &self.vertical_modes {
            require(
                !vertical.contains(mode),
                "autopilot.vertical_modes",
                "duplicate mode",
            )?;
            vertical.push(*mode);
        }
        require(
            self.family == WingLeveler || !self.vertical_modes.is_empty(),
            "autopilot.vertical_modes",
            "two-axis equipment requires a vertical mode",
        )?;
        for (name, value, lo, hi) in [
            ("bank_limit_rad", self.bank_limit_rad, 0.1, 0.6),
            ("pitch_rate_limit_rps", self.pitch_rate_limit_rps, 0.02, 0.3),
            ("max_load_g", self.max_load_g, 1.1, 2.0),
            ("servo_torque", self.servo_torque, 0.05, 0.9),
            ("servo_rate_per_s", self.servo_rate_per_s, 0.05, 3.0),
            ("minimum_ias_mps", self.minimum_ias_mps, 15.0, 50.0),
            ("maximum_ias_mps", self.maximum_ias_mps, 40.0, 100.0),
        ] {
            require(
                value.is_finite() && (lo..=hi).contains(&value),
                name,
                "outside permitted equipment range",
            )?;
        }
        require(
            self.minimum_ias_mps < self.maximum_ias_mps,
            "autopilot.airspeed",
            "inverted envelope",
        )?;
        for load in [&self.power_load, &self.roll_sensor_load] {
            require(
                !load.is_empty()
                    && load.len() <= 64
                    && load
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
                "autopilot.sensor",
                "invalid load id",
            )?;
        }
        require(
            self.family != WingLeveler
                || !(self.electric_trim
                    || self.altitude_preselect
                    || self.altitude_alert
                    || self.stability_protection
                    || self.yaw_damper),
            "autopilot.options",
            "single-axis equipment has no vertical or protection options",
        )?;
        require(
            self.family == DigitalTwoAxis || !(self.stability_protection || self.yaw_damper),
            "autopilot.options",
            "protection and yaw damper require digital equipment",
        )
    }
}
