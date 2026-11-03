//! Small navigation boundary shared by GPS, standby, glass, and future radio receivers.
use crate::SimError;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ToFrom {
    To,
    From,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NavKind {
    Gps,
    Localizer,
    BackCourse,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NavGuidance {
    pub desired_track_rad: f64,
    /// Positive to the right of the desired course, in metres.
    pub cross_track_m: f64,
    pub to_from: ToFrom,
    pub kind: NavKind,
    /// Positive above path, metres; absent until a receiver supplies a path.
    pub vertical_deviation_m: Option<f64>,
    pub path_angle_rad: Option<f64>,
}
impl NavGuidance {
    pub fn validate(&self) -> Result<(), SimError> {
        if !self.desired_track_rad.is_finite()
            || !(0.0..=core::f64::consts::TAU).contains(&self.desired_track_rad)
            || !self.cross_track_m.is_finite()
            || self.cross_track_m.abs() > 1e6
            || self
                .vertical_deviation_m
                .is_some_and(|v| !v.is_finite() || v.abs() > 20000.0)
            || self
                .path_angle_rad
                .is_some_and(|v| !v.is_finite() || !(-0.2..=0.0).contains(&v))
        {
            return Err(SimError::InvalidAutopilot("invalid navigation guidance".into()));
        }
        Ok(())
    }
}

pub trait NavSource {
    /// `None` means invalid, lost, or unavailable guidance. Called once per fixed step.
    fn guidance(&self, north_m: f64, east_m: f64) -> Option<NavGuidance>;
}

#[derive(Clone, Copy, Debug, Serialize, serde::Deserialize)]
pub struct DirectTo {
    origin: [f64; 2],
    destination: [f64; 2],
    track: f64,
    length: f64,
}
impl DirectTo {
    pub fn new(origin: [f64; 2], destination: [f64; 2]) -> Result<Self, SimError> {
        if !origin
            .into_iter()
            .chain(destination)
            .all(|v| v.is_finite() && v.abs() <= 250000.0)
        {
            return Err(SimError::InvalidAutopilot(
                "direct-to coordinates outside region".into(),
            ));
        }
        let [n0, e0] = origin;
        let [n1, e1] = destination;
        let length = libm::hypot(n1 - n0, e1 - e0);
        if length < 1.0 {
            return Err(SimError::InvalidAutopilot("direct-to leg too short".into()));
        }
        Ok(Self {
            origin,
            destination,
            track: libm::atan2(e1 - e0, n1 - n0).rem_euclid(core::f64::consts::TAU),
            length,
        })
    }
}
impl NavSource for DirectTo {
    fn guidance(&self, north_m: f64, east_m: f64) -> Option<NavGuidance> {
        let [n, e] = self.origin;
        let [dn, de] = self.destination;
        let along = ((north_m - n) * (dn - n) + (east_m - e) * (de - e)) / self.length;
        let guidance = NavGuidance {
            desired_track_rad: self.track,
            cross_track_m: ((east_m - e) * (dn - n) - (north_m - n) * (de - e)) / self.length,
            to_from: if along <= self.length {
                ToFrom::To
            } else {
                ToFrom::From
            },
            kind: NavKind::Gps,
            vertical_deviation_m: None,
            path_angle_rad: None,
        };
        guidance.validate().ok().map(|()| guidance)
    }
}

/// Adapter for a shared GPS receiver's computed course and signed cross-track output.
/// Receiver geodesy owns the course calculation; this boundary only converts units and frames.
#[derive(Clone, Copy, Debug)]
pub struct GpsNavSource {
    sample: Option<NavGuidance>,
}
impl GpsNavSource {
    pub fn from_receiver(
        valid: bool,
        desired_track_true_rad: f64,
        cross_track_nm: f64,
        to_from: ToFrom,
        grid_north_true_rad: f64,
    ) -> Result<Self, SimError> {
        if !valid {
            return Ok(Self { sample: None });
        }
        if !grid_north_true_rad.is_finite()
            || grid_north_true_rad.abs() > core::f64::consts::TAU
            || !desired_track_true_rad.is_finite()
            || !(0.0..=core::f64::consts::TAU).contains(&desired_track_true_rad)
        {
            return Err(SimError::InvalidAutopilot("invalid GPS track frame".into()));
        }
        let sample = NavGuidance {
            desired_track_rad: (desired_track_true_rad - grid_north_true_rad)
                .rem_euclid(core::f64::consts::TAU),
            cross_track_m: cross_track_nm * 1852.0,
            to_from,
            kind: NavKind::Gps,
            vertical_deviation_m: None,
            path_angle_rad: None,
        };
        sample.validate()?;
        Ok(Self { sample: Some(sample) })
    }
}
impl NavSource for GpsNavSource {
    fn guidance(&self, _north_m: f64, _east_m: f64) -> Option<NavGuidance> {
        self.sample
    }
}

/// A receiver's geographic leg, evaluated by the shared GPS geodesy module each fixed step.
#[derive(Clone, Copy, Debug, Serialize, serde::Deserialize)]
pub struct ReceiverLeg {
    pub start_deg: [f64; 2],
    pub end_deg: [f64; 2],
    pub projection_origin_deg: [f64; 2],
}
impl NavSource for ReceiverLeg {
    fn guidance(&self, north_m: f64, east_m: f64) -> Option<NavGuidance> {
        use crate::avionics::navigation;
        let [lat, lon] = self.projection_origin_deg;
        let projection = flightcraft_world::geo::LocalProjection::new(lat, lon).ok()?;
        let position = projection.unproject(north_m, east_m).ok()?;
        let north = projection.unproject(north_m + 10.0, east_m).ok()?;
        let convergence = navigation::inverse(position, north).1;
        let (cross, along) = navigation::track(self.start_deg, self.end_deg, position);
        let length = navigation::inverse(self.start_deg, self.end_deg).0;
        let sample = NavGuidance {
            desired_track_rad: (navigation::desired_track(self.start_deg, self.end_deg, position)
                - convergence)
                .rem_euclid(core::f64::consts::TAU),
            cross_track_m: cross,
            to_from: if along <= length { ToFrom::To } else { ToFrom::From },
            kind: NavKind::Gps,
            vertical_deviation_m: None,
            path_angle_rad: None,
        };
        sample.validate().ok().map(|()| sample)
    }
}
