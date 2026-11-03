//! Original geometric NAV receivers. Deviations are suppressed outside usable signal sectors.
use crate::{
    SimError, State,
    nav_source::{NavGuidance, NavKind, NavSource, ToFrom},
};
use flightcraft_world::{GeoPoint, World, navdata::NavaidKind, procedures::Localizer};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
const NM: f64 = 1852.;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CdiSource {
    #[default]
    Gps,
    Nav1,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ServiceVolume {
    Terminal,
    #[default]
    Low,
    High,
}
impl ServiceVolume {
    pub fn contains(self, distance_m: f64, height_m: f64) -> bool {
        let ft = height_m / 0.3048;
        let range = match self {
            Self::Terminal if (1000. ..=12000.).contains(&ft) => 25.,
            Self::Low if (1000. ..=18000.).contains(&ft) => 40.,
            Self::High if (18000. ..=45000.).contains(&ft) => 130.,
            Self::High if (14500. ..=60000.).contains(&ft) => 100.,
            Self::High if (1000. ..=14500.).contains(&ft) => 40.,
            _ => 0.,
        };
        distance_m.is_finite() && height_m.is_finite() && distance_m <= range * NM && range > 0.
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NavRadio {
    pub active_hz: u32,
    pub standby_hz: u32,
    pub course_deg: f64,
    pub back_course: bool,
    pub ident_enabled: bool,
    pub service_volume: ServiceVolume,
}
impl Default for NavRadio {
    fn default() -> Self {
        Self {
            active_hz: 110300000,
            standby_hz: 112900000,
            course_deg: 229.1,
            back_course: false,
            ident_enabled: false,
            service_volume: ServiceVolume::Low,
        }
    }
}
#[derive(Clone, Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RadioSetting {
    Standby { radio: u8, frequency_hz: u32 },
    Swap { radio: u8 },
    Course { radio: u8, degrees: f64 },
    BackCourse { radio: u8, on: bool },
    Ident { radio: u8, on: bool },
    ServiceVolume { radio: u8, class: ServiceVolume },
}
#[derive(Clone, Debug, Default, Serialize, JsonSchema, serde::Deserialize)]
pub struct RadioState {
    /// Published magnetic localizer course or the pilot's magnetic OBS selection.
    pub course_magnetic_deg: Option<f64>,
    pub ident: Option<String>,
    pub signal_valid: bool,
    pub to_from: Option<ToFrom>,
    pub radial_magnetic_deg: Option<f64>,
    pub dme_nm: Option<f64>,
    pub cone_of_confusion: bool,
    /// Full scale is 2.5 dots. Positive needle displacement commands right/down.
    pub lateral_dots: Option<f64>,
    pub vertical_dots: Option<f64>,
    pub guidance: Option<NavGuidance>,
    pub ident_tone_requested: bool,
}
impl NavSource for RadioState {
    fn guidance(&self, _north_m: f64, _east_m: f64) -> Option<NavGuidance> {
        self.guidance
    }
}
#[derive(Clone, Debug, Default, Serialize, JsonSchema, serde::Deserialize)]
pub struct Radios {
    pub source: CdiSource,
    pub receivers: [NavRadio; 2],
}
impl Radios {
    pub fn set(&mut self, setting: RadioSetting) -> Result<(), SimError> {
        let index = match setting {
            RadioSetting::Standby { radio, .. }
            | RadioSetting::Swap { radio }
            | RadioSetting::Course { radio, .. }
            | RadioSetting::BackCourse { radio, .. }
            | RadioSetting::Ident { radio, .. }
            | RadioSetting::ServiceVolume { radio, .. } => radio,
        };
        let receiver = self
            .receivers
            .get_mut(usize::from(
                index.checked_sub(1).ok_or_else(|| invalid("radio is 1 or 2"))?,
            ))
            .ok_or_else(|| invalid("radio is 1 or 2"))?;
        match setting {
            RadioSetting::Standby { frequency_hz, .. } => {
                if !(108000000..=117950000).contains(&frequency_hz) || !frequency_hz.is_multiple_of(50000) {
                    return Err(invalid(
                        "NAV frequency must be a 50 kHz channel in 108–117.95 MHz",
                    ));
                }
                receiver.standby_hz = frequency_hz;
            }
            RadioSetting::Swap { .. } => std::mem::swap(&mut receiver.active_hz, &mut receiver.standby_hz),
            RadioSetting::Course { degrees, .. } => {
                if !degrees.is_finite() || !(0. ..=360.).contains(&degrees) {
                    return Err(invalid("OBS outside 0–360 degrees"));
                }
                receiver.course_deg = degrees.rem_euclid(360.);
            }
            RadioSetting::BackCourse { on, .. } => receiver.back_course = on,
            RadioSetting::Ident { on, .. } => receiver.ident_enabled = on,
            RadioSetting::ServiceVolume { class, .. } => receiver.service_volume = class,
        }
        Ok(())
    }
    pub fn sample(&self, world: &World, state: State, powered: bool) -> [RadioState; 2] {
        self.sample_on_date(world, state, powered, Default::default())
    }
    pub(crate) fn sample_on_date(
        &self,
        world: &World,
        state: State,
        powered: bool,
        date: flightcraft_world::geomag::SimulationDate,
    ) -> [RadioState; 2] {
        self.receivers
            .each_ref()
            .map(|r| r.sample_on_date(world, state, powered, date))
    }
}
fn invalid(s: &str) -> SimError {
    SimError::InvalidSystems(s.into())
}
fn wrap(a: f64) -> f64 {
    (a + 180.).rem_euclid(360.) - 180.
}
fn frame(world: &World, state: State) -> Option<(GeoPoint, f64)> {
    let [n, e, _] = state.position_m;
    let [latitude_deg, longitude_deg] = world.geodetic(n, e)?;
    let p = GeoPoint {
        latitude_deg,
        longitude_deg,
    };
    let [latitude_deg, longitude_deg] = world.geodetic(n + 10., e)?;
    Some((
        p,
        p.inverse(GeoPoint {
            latitude_deg,
            longitude_deg,
        })
        .1
        .to_radians(),
    ))
}
impl NavRadio {
    pub fn sample(&self, world: &World, state: State, powered: bool) -> RadioState {
        self.sample_on_date(world, state, powered, Default::default())
    }
    fn sample_on_date(
        &self,
        world: &World,
        state: State,
        powered: bool,
        date: flightcraft_world::geomag::SimulationDate,
    ) -> RadioState {
        let mut result = RadioState::default();
        if !powered {
            return result;
        }
        let Some((position, convergence)) = frame(world, state) else {
            return result;
        };
        let [_, _, down] = state.position_m;
        let altitude = -down;
        if let Some(loc) = world.procedures().and_then(|d| {
            d.localizers
                .iter()
                .find(|l| (l.frequency_hz - f64::from(self.active_hz)).abs() < 100.)
        }) {
            return self.localizer(world, loc, position, altitude, convergence);
        }
        let Some(data) = world.navdata() else {
            return result;
        };
        // A sidecar localizer can operate without a CIFP catalogue. No angle is invented for its GS.
        if let Some(loc) = data.ils.iter().find(|l| {
            l.type_code == 7
                && l.frequency_hz
                    .is_some_and(|f| (f - f64::from(self.active_hz)).abs() < 100.)
        }) && let Some(course) = loc.magnetic_bearing_deg
        {
            let localizer = Localizer {
                airport: String::new(),
                ident: loc.ident.clone().unwrap_or_default(),
                runway: String::new(),
                frequency_hz: f64::from(self.active_hz),
                position: loc.position,
                course_magnetic_deg: course,
                magnetic_variation_deg: loc.magnetic_variation_deg.unwrap_or(0.),
                course_width_deg: None,
                gs_position: None,
                gs_angle_deg: None,
                gs_elevation_m: None,
                threshold_crossing_height_m: None,
            };
            return self.localizer(world, &localizer, position, altitude, convergence);
        }
        let Some((station, component)) = data
            .navaids
            .iter()
            .filter(|n| {
                matches!(
                    n.kind,
                    NavaidKind::Vor | NavaidKind::VorDme | NavaidKind::Vortac | NavaidKind::Dme
                )
            })
            .find_map(|n| {
                n.components
                    .iter()
                    .find(|c| {
                        c.frequency_hz
                            .is_some_and(|f| (f - f64::from(self.active_hz)).abs() < 100.)
                    })
                    .map(|c| (n, c))
            })
        else {
            return result;
        };
        let (distance, radial) = component.position.inverse(position);
        // ADDS has no elevation in this sidecar. Use local terrain until station elevation is supplied.
        let elevation = world
            .region()
            .and_then(|r| {
                flightcraft_world::geo::LocalProjection::new(r.origin.latitude_deg, r.origin.longitude_deg)
                    .ok()
            })
            .and_then(|p| {
                p.project(component.position.latitude_deg, component.position.longitude_deg)
                    .ok()
            })
            .map_or(0., |[n, e]| world.ground(n, e).height_msl_m);
        let height = altitude - elevation;
        let has_dme = matches!(
            station.kind,
            NavaidKind::VorDme | NavaidKind::Vortac | NavaidKind::Dme
        );
        if self.service_volume.contains(distance, height) && has_dme {
            result.dme_nm = Some(libm::hypot(distance, height) / NM);
        }
        result.ident = Some(station.ident.clone());
        result.ident_tone_requested = false;
        result.cone_of_confusion = distance < height.max(0.) * libm::tan(40_f64.to_radians());
        if result.cone_of_confusion
            || !self.service_volume.contains(distance, height)
            || matches!(station.kind, NavaidKind::Dme)
        {
            return result;
        }
        let variation = component
            .magnetic_variation_deg
            .or_else(|| {
                flightcraft_world::geomag::MagneticModel::wmm2025()
                    .and_then(|m| {
                        m.declination(
                            component.position.latitude_deg,
                            component.position.longitude_deg,
                            date,
                        )
                    })
                    .ok()
            })
            .unwrap_or(0.);
        let radial = (radial - variation).rem_euclid(360.);
        let course = (self.course_deg + variation).to_radians();
        let from = wrap(radial - self.course_deg).abs() < 90.;
        let cross = distance * libm::sin(wrap(radial - self.course_deg).to_radians());
        let to_from = if from { ToFrom::From } else { ToFrom::To };
        result.radial_magnetic_deg = Some(radial);
        result.to_from = Some(to_from);
        result.signal_valid = true;
        result.course_magnetic_deg = Some(self.course_deg);
        result.ident_tone_requested = self.ident_enabled;
        result.lateral_dots = Some(
            (-libm::asin((cross / distance.max(1.)).clamp(-1., 1.)).to_degrees() / 10. * 2.5)
                .clamp(-2.5, 2.5),
        );
        result.guidance = Some(NavGuidance {
            desired_track_rad: (course - convergence).rem_euclid(core::f64::consts::TAU),
            cross_track_m: cross,
            to_from,
            kind: NavKind::Gps,
            vertical_deviation_m: None,
            path_angle_rad: None,
        });
        result
    }
    fn localizer(
        &self,
        world: &World,
        loc: &Localizer,
        position: GeoPoint,
        altitude: f64,
        convergence: f64,
    ) -> RadioState {
        let mut result = RadioState {
            ident: Some(loc.ident.clone()),
            ident_tone_requested: false,
            ..Default::default()
        };
        let inbound = loc.course_magnetic_deg + loc.magnetic_variation_deg;
        let (distance, bearing) = loc.position.inverse(position);
        let desired = if self.back_course { inbound + 180. } else { inbound };
        let offset = wrap(bearing - (desired + 180.));
        let usable = if offset.abs() <= 10. {
            distance <= 18. * NM
        } else {
            offset.abs() <= 35. && distance <= 10. * NM
        };
        if !usable || distance < 10. {
            return result;
        }
        let threshold = world.procedures().and_then(|d| {
            d.points.iter().find(|p| {
                p.airport.as_deref() == Some(loc.airport.as_str())
                    && p.ident == loc.runway
                    && p.section == "PG"
            })
        });
        let width = loc.course_width_deg.unwrap_or_else(|| {
            let antenna_distance = threshold.map_or(1500., |p| loc.position.inverse(p.position).0);
            (2. * libm::atan2(700. * 0.3048 * 0.5, antenna_distance.max(100.)))
                .to_degrees()
                .clamp(0.5, 10.)
        });
        result.course_magnetic_deg =
            Some((loc.course_magnetic_deg + if self.back_course { 180. } else { 0. }).rem_euclid(360.));
        let cross = -distance * libm::sin(offset.to_radians());
        result.signal_valid = true;
        result.ident_tone_requested = self.ident_enabled;
        result.to_from = Some(ToFrom::To);
        result.lateral_dots = Some((offset / (width * 0.5) * 2.5).clamp(-2.5, 2.5));
        let mut guidance = NavGuidance {
            desired_track_rad: (desired.to_radians() - convergence).rem_euclid(core::f64::consts::TAU),
            cross_track_m: cross,
            to_from: ToFrom::To,
            kind: if self.back_course {
                NavKind::BackCourse
            } else {
                NavKind::Localizer
            },
            vertical_deviation_m: None,
            path_angle_rad: None,
        };
        if !self.back_course
            && let (Some(gs), Some(angle), Some(elev)) =
                (loc.gs_position, loc.gs_angle_deg, loc.gs_elevation_m)
        {
            let (gs_distance, gs_bearing) = gs.inverse(position);
            let gs_offset = wrap(gs_bearing - (inbound + 180.));
            let along = gs_distance * libm::cos(gs_offset.to_radians());
            let actual = libm::atan2(altitude - elev, along).to_degrees();
            if gs_distance <= 10. * NM
                && along > 30.
                && gs_offset.abs() <= 8.
                && (0.45 * angle..=1.75 * angle).contains(&actual)
            {
                let deviation = altitude - (elev + along * libm::tan(angle.to_radians()));
                guidance.vertical_deviation_m = Some(deviation);
                guidance.path_angle_rad = Some(-angle.to_radians());
                result.vertical_dots = Some(((actual - angle) / (0.7 * angle / 3.) * 2.5).clamp(-2.5, 2.5));
            }
        }
        if guidance.validate().is_ok() {
            result.guidance = Some(guidance);
        }
        result
    }
}
