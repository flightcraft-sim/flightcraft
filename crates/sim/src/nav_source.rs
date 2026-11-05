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
    /// A vertical navigation path along this leg, from the flight-plan altitude constraints. It
    /// is left out of the serialised state when absent, so a leg without one hashes as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vnav: Option<crate::avionics::vnav::LegPath>,
}
impl ReceiverLeg {
    /// Guidance for an aircraft at `altitude_m`: the lateral guidance of [`NavSource::guidance`],
    /// and, when the leg carries a vertical path, the deviation above it (positive above) and
    /// the path angle the autopilot's GP mode tracks.
    pub fn guidance_at(&self, north_m: f64, east_m: f64, altitude_m: f64) -> Option<NavGuidance> {
        use crate::avionics::navigation;
        let mut sample = self.guidance(north_m, east_m)?;
        if let Some(path) = &self.vnav {
            let [lat, lon] = self.projection_origin_deg;
            let projection = flightcraft_world::geo::LocalProjection::new(lat, lon).ok()?;
            let position = projection.unproject(north_m, east_m).ok()?;
            let (_, along) = navigation::track(self.start_deg, self.end_deg, position);
            let remaining = navigation::inverse(self.start_deg, self.end_deg).0 - along;
            let (path_altitude, angle) = path.at(remaining);
            sample.vertical_deviation_m = Some(altitude_m - path_altitude);
            sample.path_angle_rad = Some(angle);
        }
        sample.validate().ok().map(|()| sample)
    }
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::avionics::vnav::LegPath;

    fn leg(vnav: Option<LegPath>) -> ReceiverLeg {
        ReceiverLeg {
            start_deg: [40.0, -74.0],
            end_deg: [40.0, -73.8],
            projection_origin_deg: [40.0, -74.0],
            vnav,
        }
    }

    #[test]
    fn a_leg_without_a_vertical_path_gives_no_vertical_guidance_and_hashes_as_before() {
        let g = leg(None).guidance_at(0., 0., 1500.).unwrap();
        assert_eq!((g.vertical_deviation_m, g.path_angle_rad), (None, None));
        let json = serde_json::to_string(&leg(None)).unwrap();
        assert!(!json.contains("vnav"), "{json}");
    }

    #[test]
    fn a_vertical_path_gives_a_deviation_above_it_and_the_path_angle() {
        let angle = 3f64.to_radians();
        let path = LegPath {
            target_altitude_m: 600.,
            beyond_m: 0.,
            angle_rad: angle,
        };
        let l = leg(Some(path));
        let length = crate::avionics::navigation::inverse(l.start_deg, l.end_deg).0;
        // At the start of the leg the path stands `length` of descent above the target.
        let on_path = 600. + length * libm::tan(angle);
        let above = l.guidance_at(0., 0., on_path + 100.).unwrap();
        let below = l.guidance_at(0., 0., on_path - 100.).unwrap();
        assert!((above.vertical_deviation_m.unwrap() - 100.).abs() < 2.);
        assert!((below.vertical_deviation_m.unwrap() + 100.).abs() < 2.);
        assert!((above.path_angle_rad.unwrap() + angle).abs() < 1e-12);
        // Past the end of the leg the path is level at the target.
        let [lat, lon] = l.projection_origin_deg;
        let projection = flightcraft_world::geo::LocalProjection::new(lat, lon).unwrap();
        let [north, east] = projection.project(40.0, -73.79).unwrap();
        let past = l.guidance_at(north, east, 640.).unwrap();
        assert!((past.vertical_deviation_m.unwrap() - 40.).abs() < 1.0);
        assert_eq!(past.path_angle_rad, Some(0.0));
        // A leg with a path serialises it.
        assert!(serde_json::to_string(&l).unwrap().contains("vnav"));
    }
}
