//! Deterministic power availability and fixed control-surface deflections.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ControlSurface {
    Elevator,
    Aileron,
    Rudder,
    Flaps,
}

/// Actual deflections override pilot/autopilot demands until cleared. Zero power
/// means a failed engine, including removal of idle shaft power.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Failures {
    #[serde(default)]
    pub vacuum_pump: bool,
    #[serde(default)]
    pub alternator: bool,
    #[serde(default)]
    pub pitot_blocked: bool,
    #[serde(default)]
    pub static_blocked: bool,
    #[schemars(range(min = 0, max = 1))]
    pub engine_power_fraction: f64,
    #[schemars(range(min = -1, max = 1))]
    pub elevator: Option<f64>,
    #[schemars(range(min = -1, max = 1))]
    pub aileron: Option<f64>,
    #[schemars(range(min = -1, max = 1))]
    pub rudder: Option<f64>,
    #[schemars(range(min = 0, max = 1))]
    pub flaps: Option<f64>,
}

impl Default for Failures {
    fn default() -> Self {
        Self {
            vacuum_pump: false,
            alternator: false,
            pitot_blocked: false,
            static_blocked: false,
            engine_power_fraction: 1.0,
            elevator: None,
            aileron: None,
            rudder: None,
            flaps: None,
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
#[error("power fraction must be finite in 0..=1; jams finite in -1..=1 (flaps 0..=1)")]
pub struct FailureError;

impl Failures {
    pub fn validate(self) -> Result<(), FailureError> {
        for (value, lo) in [
            (Some(self.engine_power_fraction), 0.0),
            (self.elevator, -1.0),
            (self.aileron, -1.0),
            (self.rudder, -1.0),
            (self.flaps, 0.0),
        ] {
            if value.is_some_and(|v| !v.is_finite() || !(lo..=1.0).contains(&v)) {
                return Err(FailureError);
            }
        }
        Ok(())
    }

    pub fn jam(&mut self, surface: ControlSurface, position: f64) -> Result<(), FailureError> {
        let mut next = *self;
        match surface {
            ControlSurface::Elevator => next.elevator = Some(position),
            ControlSurface::Aileron => next.aileron = Some(position),
            ControlSurface::Rudder => next.rudder = Some(position),
            ControlSurface::Flaps => next.flaps = Some(position),
        }
        next.validate()?;
        *self = next;
        Ok(())
    }

    #[must_use]
    pub fn apply(self, c: crate::Controls) -> crate::Controls {
        crate::Controls {
            elevator: self.elevator.unwrap_or(c.elevator),
            aileron: self.aileron.unwrap_or(c.aileron),
            rudder: self.rudder.unwrap_or(c.rudder),
            flaps: self.flaps.unwrap_or(c.flaps),
            ..c
        }
    }
}
