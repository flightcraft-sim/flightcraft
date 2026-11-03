//! Fixed-step GPS procedure sequencing shared by glass and standby installations.
use super::navigation;
use crate::{
    SimError, State,
    nav_source::{NavGuidance, NavKind, ToFrom},
};
use flightcraft_world::{
    GeoPoint, World,
    procedures::{PathTerminator as Pt, *},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
const NM: f64 = 1852.;
fn invalid(s: &str) -> SimError {
    SimError::InvalidSystems(s.into())
}
fn array(p: GeoPoint) -> [f64; 2] {
    [p.latitude_deg, p.longitude_deg]
}
fn wrap(d: f64) -> f64 {
    (d + 180.).rem_euclid(360.) - 180.
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ApproachMode {
    #[default]
    ENR,
    TERM,
    APR,
    LNAV,
    #[serde(rename = "LNAV/VNAV")]
    LnavVnav,
    LPV,
    MISSED,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HoldEntry {
    Direct,
    Parallel,
    Teardrop,
}
pub fn hold_entry(inbound_deg: f64, arrival_track_deg: f64, turn: Turn) -> HoldEntry {
    let relative = wrap((arrival_track_deg - inbound_deg) * if turn == Turn::R { 1. } else { -1. });
    if (-70. ..=110.).contains(&relative) {
        HoldEntry::Direct
    } else if relative > 110. {
        HoldEntry::Teardrop
    } else {
        HoldEntry::Parallel
    }
}
#[derive(Clone, Debug, Serialize, JsonSchema, serde::Deserialize)]
pub struct LoadedProcedure {
    pub airport: String,
    pub ident: String,
    pub kind: ProcedureKind,
    pub transition: Option<String>,
    pub plate: Option<PlateLink>,
    pub capabilities: Vec<ApproachCapability>,
}
#[derive(Clone, Debug, Serialize, JsonSchema, serde::Deserialize)]
pub struct FlightLeg {
    pub leg: ProcedureLeg,
    pub course_magnetic_deg: Option<f64>,
    pub course_true_deg: Option<f64>,
    pub procedure_kind: ProcedureKind,
    pub endpoint: Option<GeoPoint>,
    pub center: Option<GeoPoint>,
    pub navaid: Option<GeoPoint>,
    pub variation_deg: f64,
    pub fix_elevation_m: Option<f64>,
}
#[derive(Clone, Debug, Default, Serialize, JsonSchema, serde::Deserialize)]
pub struct ProcedureReceiver {
    #[serde(default)]
    pub lnav_only: bool,
    pub loaded: Vec<LoadedProcedure>,
    pub legs: Vec<FlightLeg>,
    pub active_leg: usize,
    pub approach_active: bool,
    pub mode: ApproachMode,
    pub suspended: bool,
    pub missed_approach: bool,
    pub waypoint_alert: bool,
    pub hold_entry: Option<HoldEntry>,
    pub hold_laps: u32,
    pub valid: bool,
    /// Magnetic track for pilot readouts.
    pub desired_track_rad: f64,
    pub desired_track_true_rad: f64,
    pub cross_track_nm: f64,
    pub distance_nm: f64,
    pub cdi_full_scale_nm: f64,
    pub vertical_deviation_m: Option<f64>,
    pub path_angle_rad: Option<f64>,
    pub guidance: Option<NavGuidance>,
    origin: Option<GeoPoint>,
    leg_elapsed_s: f64,
    hold_elapsed_s: f64,
    hold_entered: bool,
    exit_hold: bool,
    vectors: bool,
}
impl ProcedureReceiver {
    pub fn load(
        &mut self,
        world: &World,
        state: State,
        airport: &str,
        ident: &str,
        transition: Option<&str>,
        vectors: bool,
    ) -> Result<(), SimError> {
        if airport.len() > 8 || ident.len() > 16 || transition.is_some_and(|t| t.len() > 8) {
            return Err(invalid("procedure identifier exceeds bounds"));
        }
        let data = world
            .procedures()
            .ok_or_else(|| invalid("active region has no procedures"))?;
        let airport = world
            .airports()
            .and_then(|a| {
                a.airports
                    .iter()
                    .find(|a| a.ident == airport || a.icao.as_deref() == Some(airport))
            })
            .and_then(|a| a.icao.as_deref())
            .unwrap_or(airport);
        let p = data
            .procedures
            .iter()
            .find(|p| p.airport == airport && p.ident == ident)
            .ok_or_else(|| invalid("procedure not in active region"))?;
        let common = p
            .routes
            .iter()
            .find(|r| r.transition.is_empty())
            .or_else(|| p.routes.first())
            .ok_or_else(|| invalid("procedure has no common route"))?;
        let selected = if !vectors {
            transition
                .map(|t| {
                    p.routes
                        .iter()
                        .find(|r| r.transition == t)
                        .ok_or_else(|| invalid("unknown procedure transition"))
                })
                .transpose()?
        } else {
            None
        };
        let mut chosen = Vec::new();
        if p.kind == ProcedureKind::Departure {
            chosen.extend(common.legs.iter());
        }
        if let Some(route) = selected
            && (route.route_type != common.route_type || route.transition != common.transition)
        {
            chosen.extend(route.legs.iter());
        }
        if p.kind != ProcedureKind::Departure {
            chosen.extend(common.legs.iter());
        }
        let [n, e, _] = state.position_m;
        let mut previous = world
            .geodetic(n, e)
            .map(|[latitude_deg, longitude_deg]| GeoPoint {
                latitude_deg,
                longitude_deg,
            });
        let mut legs = Vec::new();
        for leg in chosen {
            if legs
                .last()
                .is_some_and(|previous: &FlightLeg| leg.path == Pt::IF && previous.leg.fix == leg.fix)
            {
                continue;
            }
            let point = leg.fix.as_deref().and_then(|id| data.point(id));
            let center = leg
                .center_fix
                .as_deref()
                .and_then(|id| data.point(id))
                .map(|p| p.position);
            let navaid = leg
                .recommended_navaid
                .as_deref()
                .and_then(|id| data.point(id))
                .map(|p| p.position);
            let variation = point
                .and_then(|p| p.magnetic_variation_deg)
                .or_else(|| {
                    data.points
                        .iter()
                        .find(|p| p.airport.as_deref() == Some(airport) && p.section == "PA")
                        .and_then(|p| p.magnetic_variation_deg)
                })
                .unwrap_or(0.);
            let course_true_deg = leg
                .course_deg
                .map(|c| c + if leg.course_true { 0. } else { variation })
                .or_else(|| {
                    previous
                        .zip(point.map(|p| p.position))
                        .map(|(a, b)| a.inverse(b).1)
                });
            previous = point.map(|p| p.position).or(previous);
            legs.push(FlightLeg {
                course_magnetic_deg: leg.course_deg.filter(|_| !leg.course_true),
                course_true_deg,
                leg: leg.clone(),
                procedure_kind: p.kind,
                endpoint: point.map(|p| p.position),
                center,
                navaid,
                variation_deg: variation,
                fix_elevation_m: point.and_then(|p| p.elevation_m),
            });
        }
        if legs.is_empty() || legs.len() > 256 {
            return Err(invalid("procedure legs outside bounds"));
        }
        let [n, e, _] = state.position_m;
        let [latitude_deg, longitude_deg] = world
            .geodetic(n, e)
            .ok_or_else(|| invalid("procedure requires geographic position"))?;
        let origin = GeoPoint {
            latitude_deg,
            longitude_deg,
        };
        let mut next = self.clone();
        // Replacing one procedure retains the other plan stages in departure/arrival/approach order.
        let existing_kind = next.legs.get(next.active_leg).map(|l| l.procedure_kind);
        next.legs.retain(|l| l.procedure_kind != p.kind);
        next.legs.extend(legs);
        next.legs.sort_by_key(|l| match l.procedure_kind {
            ProcedureKind::Departure => 0,
            ProcedureKind::Arrival => 1,
            ProcedureKind::Approach => 2,
        });
        next.active_leg = if p.kind == ProcedureKind::Approach
            && (vectors || existing_kind == Some(ProcedureKind::Approach))
        {
            next.legs
                .iter()
                .position(|l| l.procedure_kind == ProcedureKind::Approach)
                .unwrap_or(0)
        } else {
            next.active_leg.min(next.legs.len().saturating_sub(1))
        };
        next.origin = Some(origin);
        if next.legs.len() > 512 {
            return Err(invalid("active procedure plan exceeds 512 legs"));
        }
        next.loaded.retain(|l| l.kind != p.kind);
        next.loaded.push(LoadedProcedure {
            airport: p.airport.clone(),
            ident: p.ident.clone(),
            kind: p.kind,
            transition: transition.map(str::to_owned),
            plate: p.plate.clone(),
            capabilities: p.capabilities.clone(),
        });
        next.approach_active = false;
        next.missed_approach = false;
        next.suspended = false;
        next.vectors = vectors;
        next.leg_elapsed_s = 0.;
        next.hold_entered = false;
        next.hold_entry = None;
        next.hold_elapsed_s = 0.;
        next.exit_hold = false;
        if vectors {
            next.select_final()?;
        }
        *self = next;
        Ok(())
    }
    fn select_final(&mut self) -> Result<(), SimError> {
        let index = self
            .legs
            .iter()
            .position(|l| !l.leg.missed && l.leg.waypoint_description.ends_with('F'))
            .or_else(|| {
                self.legs
                    .iter()
                    .position(|l| !l.leg.missed && l.leg.vertical_angle_deg.is_some())
            })
            .ok_or_else(|| invalid("procedure has no final segment"))?;
        self.active_leg = index;
        self.vectors = true;
        self.suspended = false;
        self.leg_elapsed_s = 0.;
        Ok(())
    }
    pub fn activate(&mut self, vectors: bool) -> Result<(), SimError> {
        if !self.loaded.iter().any(|l| l.kind == ProcedureKind::Approach) {
            return Err(invalid("no approach loaded"));
        }
        if vectors {
            self.select_final()?;
        } else if !self
            .legs
            .get(self.active_leg)
            .is_some_and(|l| l.procedure_kind == ProcedureKind::Approach)
        {
            self.active_leg = self
                .legs
                .iter()
                .position(|l| l.procedure_kind == ProcedureKind::Approach)
                .ok_or_else(|| invalid("no approach legs"))?;
            self.leg_elapsed_s = 0.;
            self.hold_entered = false;
            self.suspended = false;
        }
        self.approach_active = true;
        Ok(())
    }
    pub fn missed(&mut self) -> Result<(), SimError> {
        let index = self
            .legs
            .iter()
            .position(|l| l.leg.missed)
            .ok_or_else(|| invalid("no missed approach loaded"))?;
        self.active_leg = index;
        self.missed_approach = true;
        self.suspended = false;
        self.approach_active = false;
        self.leg_elapsed_s = 0.;
        self.hold_entered = false;
        self.exit_hold = false;
        self.vertical_deviation_m = None;
        self.path_angle_rad = None;
        Ok(())
    }
    pub fn hold(&mut self, suspend: bool, exit: bool) -> Result<(), SimError> {
        if !self
            .legs
            .get(self.active_leg)
            .is_some_and(|l| matches!(l.leg.path, Pt::HA | Pt::HF | Pt::HM | Pt::PI))
        {
            return Err(invalid("active leg is not a hold"));
        }
        self.suspended = suspend;
        self.exit_hold = exit;
        Ok(())
    }
    pub fn sequence(&mut self) -> Result<(), SimError> {
        let next = self
            .legs
            .get(self.active_leg.saturating_add(1))
            .ok_or_else(|| invalid("no next procedure leg"))?;
        if next.leg.missed && !self.missed_approach {
            return Err(invalid("activate missed approach explicitly"));
        }
        self.origin = self
            .legs
            .get(self.active_leg)
            .and_then(|l| l.endpoint)
            .or(self.origin);
        self.active_leg = self.active_leg.saturating_add(1);
        self.leg_elapsed_s = 0.;
        self.hold_entered = false;
        self.hold_entry = None;
        self.hold_elapsed_s = 0.;
        self.hold_laps = 0;
        self.suspended = false;
        self.vectors = false;
        Ok(())
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn step(&mut self, world: &World, state: State, powered: bool) {
        let reference =
            crate::magnetic::reference_at(world, state.position_m, Default::default()).unwrap_or_default();
        self.step_in_wind(world, state, powered, [0., 0.], reference);
    }
    pub(crate) fn step_in_wind(
        &mut self,
        world: &World,
        state: State,
        powered: bool,
        wind: [f64; 2],
        reference: crate::magnetic::HeadingReference,
    ) {
        for leg in &mut self.legs {
            leg.course_magnetic_deg = leg.leg.course_deg.filter(|_| !leg.leg.course_true).or_else(|| {
                leg.course_true_deg.map(|c| {
                    if leg.leg.course_true {
                        reference.true_to_magnetic(c.to_radians()).to_degrees()
                    } else {
                        (c - leg.variation_deg).rem_euclid(360.)
                    }
                })
            });
        }
        self.valid = false;
        self.guidance = None;
        self.vertical_deviation_m = None;
        self.path_angle_rad = None;
        if !powered || self.legs.is_empty() {
            return;
        }
        let [n, e, down] = state.position_m;
        let Some([latitude_deg, longitude_deg]) = world.geodetic(n, e) else {
            return;
        };
        let position = GeoPoint {
            latitude_deg,
            longitude_deg,
        };
        let Some(north) = world.geodetic(n + 10., e) else {
            return;
        };
        let convergence = navigation::inverse(array(position), north).1;
        let altitude = -down;
        let [vn, ve, _] = state.velocity_mps;
        let speed = libm::hypot(vn, ve);
        let track = libm::atan2(ve, vn).to_degrees();
        self.leg_elapsed_s += crate::DT_S;
        for _ in 0..16 {
            let Some(active) = self.legs.get(self.active_leg) else {
                return;
            };
            let origin = self.origin.unwrap_or(position);
            let endpoint = active.endpoint;
            let course =
                (active.leg.course_deg.unwrap_or_else(|| {
                    endpoint.map_or(track, |p| origin.inverse(p).1) - active.variation_deg
                }) + if active.leg.course_true {
                    0.
                } else {
                    active.variation_deg
                })
                .rem_euclid(360.);
            // Vectors join the extension of the final segment, independent of the aircraft's
            // off-course origin. The FAF-to-MAP geometry defines that published final course.
            let course = if self.vectors {
                endpoint
                    .zip(
                        self.legs
                            .get(self.active_leg.saturating_add(1))
                            .filter(|l| !l.leg.missed)
                            .and_then(|l| l.endpoint),
                    )
                    .map_or(course, |(a, b)| a.inverse(b).1)
            } else {
                course
            };
            let distance = endpoint.map_or(0., |p| position.inverse(p).0);
            let path = active.leg.path;
            let mut complete = false;
            let mut cross;
            let mut desired = course.to_radians();
            if matches!(path, Pt::HA | Pt::HF | Pt::HM | Pt::PI) {
                let Some(fix) = endpoint else {
                    return;
                };
                if !self.hold_entered && distance > 0.3 * NM {
                    desired = navigation::desired_track(array(origin), array(fix), array(position));
                    cross = navigation::track(array(origin), array(fix), array(position)).0;
                } else {
                    if !self.hold_entered {
                        self.hold_entered = true;
                        self.hold_entry = Some(hold_entry(course, track, active.leg.turn.unwrap_or(Turn::R)));
                        self.hold_elapsed_s = 0.;
                        self.suspended = path == Pt::HM;
                    }
                    self.hold_elapsed_s += crate::DT_S;
                    let outbound = active
                        .leg
                        .time_s
                        .unwrap_or_else(|| active.leg.distance_nm.map_or(60., |d| d * NM / speed.max(20.)))
                        .clamp(20., 300.);
                    let turn = if active.leg.turn == Some(Turn::L) { -1. } else { 1. };
                    let period = 120. + 2. * outbound;
                    let phase = self.hold_elapsed_s.rem_euclid(period);
                    let entry_offset = if self.hold_laps == 0 {
                        match self.hold_entry {
                            Some(HoldEntry::Teardrop) => -30. * turn,
                            Some(HoldEntry::Parallel) => 30. * turn,
                            _ => 0.,
                        }
                    } else {
                        0.
                    };
                    let radius = speed.max(20.) / 3_f64.to_radians();
                    let turn_center = fix.destination(course + 90. * turn, radius);
                    let outbound_start = fix.destination(course + 90. * turn, 2. * radius);
                    let outbound_end =
                        outbound_start.destination(course + 180. + entry_offset, outbound * speed.max(20.));
                    if phase < 60. || (phase >= 60. + outbound && phase < 120. + outbound) {
                        let center = if phase < 60. {
                            turn_center
                        } else {
                            outbound_end.destination(course - 90. * turn, radius)
                        };
                        let (actual_radius, radial) = center.inverse(position);
                        desired = (radial + 90. * turn)
                            .to_radians()
                            .rem_euclid(core::f64::consts::TAU);
                        cross = (radius - actual_radius) * turn;
                    } else if phase < 60. + outbound {
                        desired = navigation::desired_track(
                            array(outbound_start),
                            array(outbound_end),
                            array(position),
                        );
                        cross =
                            navigation::track(array(outbound_start), array(outbound_end), array(position)).0;
                    } else {
                        let inbound_start = fix.destination(course + 180., outbound * speed.max(20.));
                        desired =
                            navigation::desired_track(array(inbound_start), array(fix), array(position));
                        cross = navigation::track(array(inbound_start), array(fix), array(position)).0;
                    }
                    if self.hold_elapsed_s >= period {
                        self.hold_elapsed_s -= period;
                        self.hold_laps = self.hold_laps.saturating_add(1);
                        complete = (path == Pt::HF
                            || path == Pt::PI
                            || self.exit_hold
                            || (path == Pt::HA && altitude_met(&active.leg, altitude)))
                            && !self.suspended;
                    }
                }
            } else if matches!(path, Pt::RF | Pt::AF) {
                let center = active.center.or(active.navaid);
                let Some(center) = center else {
                    return;
                };
                let (radius, radial) = center.inverse(position);
                let target = active
                    .leg
                    .arc_radius_nm
                    .or(active.leg.rho_nm)
                    .map(|r| r * NM)
                    .or_else(|| endpoint.map(|p| center.inverse(p).0));
                let Some(target) = target else {
                    return;
                };
                let direction = if active.leg.turn == Some(Turn::L) { -1. } else { 1. };
                desired = (radial + 90. * direction)
                    .to_radians()
                    .rem_euclid(core::f64::consts::TAU);
                cross = (target - radius) * direction;
                complete = distance < 0.2 * NM;
            } else if matches!(path, Pt::IF | Pt::TF | Pt::CF | Pt::DF) {
                let Some(end) = endpoint else {
                    return;
                };
                let start = if path == Pt::CF || self.vectors {
                    end.destination(course + 180., 50. * NM)
                } else {
                    origin
                };
                let (xt, along) = navigation::track(array(start), array(end), array(position));
                cross = xt;
                desired = navigation::desired_track(array(start), array(end), array(position));
                complete = distance < 0.15 * NM
                    || (along >= navigation::inverse(array(start), array(end)).0 && xt.abs() < 0.3 * NM);
            } else {
                // Course/heading legs retain their published course until their explicit terminator.
                let course_origin = endpoint
                    .filter(|_| matches!(path, Pt::FA | Pt::FC | Pt::FD | Pt::FM))
                    .unwrap_or(origin);
                let far = course_origin.destination(course, 100. * NM);
                cross = navigation::track(array(course_origin), array(far), array(position)).0;
                if matches!(path, Pt::VA | Pt::VD | Pt::VI | Pt::VM | Pt::VR) {
                    cross = 0.;
                    let [wn, we] = wind;
                    let airspeed = libm::hypot(vn - wn, ve - we);
                    let grid_heading = course.to_radians() - convergence;
                    desired = (libm::atan2(
                        airspeed * libm::sin(grid_heading) + we,
                        airspeed * libm::cos(grid_heading) + wn,
                    ) + convergence)
                        .rem_euclid(core::f64::consts::TAU);
                }
                complete = match path {
                    Pt::CA | Pt::VA | Pt::FA => altitude_met(&active.leg, altitude),
                    Pt::FC => active
                        .leg
                        .distance_nm
                        .is_some_and(|d| course_origin.inverse(position).0 >= d * NM),
                    Pt::CD | Pt::VD | Pt::FD => active
                        .navaid
                        .zip(active.leg.distance_nm)
                        .is_some_and(|(p, d)| (p.inverse(position).0 - d * NM).abs() < 0.1 * NM),
                    Pt::CI | Pt::VI => self
                        .legs
                        .get(self.active_leg.saturating_add(1))
                        .and_then(|next| next.endpoint.map(|end| (next, end)))
                        .is_some_and(|(next, end)| {
                            let inbound = next.leg.course_deg.unwrap_or(course) + next.variation_deg;
                            navigation::track(
                                array(end.destination(inbound + 180., 50. * NM)),
                                array(end),
                                array(position),
                            )
                            .0
                            .abs()
                                < 100.
                        }),
                    Pt::CR | Pt::VR => active.navaid.zip(active.leg.theta_deg).is_some_and(|(p, theta)| {
                        wrap(p.inverse(position).1 - (theta + active.variation_deg)).abs() < 1.
                    }),
                    _ => false,
                };
            }
            self.distance_nm = distance / NM;
            self.waypoint_alert = endpoint.is_some() && distance < speed * 20.;
            let next_missed = self
                .legs
                .get(self.active_leg.saturating_add(1))
                .is_some_and(|l| l.leg.missed)
                && !self.missed_approach;
            if complete && !self.suspended {
                if next_missed {
                    self.suspended = true;
                } else if self.active_leg.saturating_add(1) < self.legs.len() {
                    self.active_leg = self.active_leg.saturating_add(1);
                    self.origin = endpoint.or(Some(position));
                    self.leg_elapsed_s = 0.;
                    self.hold_entered = false;
                    self.hold_entry = None;
                    self.hold_elapsed_s = 0.;
                    self.hold_laps = 0;
                    self.vectors = false;
                    continue;
                }
            }
            self.mode = if self.missed_approach {
                ApproachMode::MISSED
            } else if self.approach_active {
                let p = self.loaded.iter().find(|p| p.kind == ProcedureKind::Approach);
                if p.is_some_and(|p| p.ident.starts_with('R')) {
                    ApproachMode::LNAV
                } else {
                    ApproachMode::APR
                }
            } else if distance < 30. * NM {
                ApproachMode::TERM
            } else {
                ApproachMode::ENR
            };
            self.cdi_full_scale_nm = if self.approach_active {
                0.3
            } else if self.mode == ApproachMode::TERM {
                1.
            } else {
                2.
            };
            // Published final angle is anchored to the runway/MAP altitude, never the current aircraft.
            if self.approach_active
                && !self.lnav_only
                && self.loaded.iter().any(|p| {
                    p.kind == ProcedureKind::Approach
                        && (p.capabilities.contains(&ApproachCapability::LPV)
                            || p.capabilities.contains(&ApproachCapability::LnavVnav))
                })
                && !self.missed_approach
                && let Some(final_leg) = self
                    .legs
                    .iter()
                    .rev()
                    .find(|l| !l.leg.missed && l.leg.vertical_angle_deg.is_some())
                && let (Some(angle), Some(end), Some(elevation)) = (
                    final_leg.leg.vertical_angle_deg,
                    final_leg.endpoint,
                    final_leg.leg.altitude1_m.or(final_leg.fix_elevation_m),
                )
            {
                let final_course = final_leg
                    .leg
                    .course_deg
                    .map(|c| {
                        c + if final_leg.leg.course_true {
                            0.
                        } else {
                            final_leg.variation_deg
                        }
                    })
                    .unwrap_or_else(|| {
                        self.legs
                            .iter()
                            .rev()
                            .find(|l| !l.leg.missed && l.endpoint != Some(end))
                            .and_then(|l| l.endpoint)
                            .map_or(course, |p| p.inverse(end).1)
                    });
                let (to_end, bearing) = position.inverse(end);
                let along = to_end * libm::cos(wrap(bearing - final_course).to_radians());
                if angle < 0. && along > 0. && along < 15. * NM {
                    self.vertical_deviation_m =
                        Some(altitude - (elevation + along * libm::tan(-angle.to_radians())));
                    self.path_angle_rad = Some(angle.to_radians());
                    // SBAS service is annunciated only when a matching path-point record exists.
                    let sbas = self
                        .loaded
                        .iter()
                        .find(|p| p.kind == ProcedureKind::Approach)
                        .and_then(|p| {
                            world
                                .procedures()?
                                .sbas
                                .iter()
                                .find(|s| s.airport == p.airport && s.approach == p.ident)
                        });
                    if sbas.is_some_and(|s| s.angle_deg > 0.)
                        && self
                            .loaded
                            .iter()
                            .any(|p| p.capabilities.contains(&ApproachCapability::LPV))
                    {
                        self.mode = ApproachMode::LPV;
                        self.cdi_full_scale_nm =
                            (along * libm::tan(2_f64.to_radians()) / NM).clamp(0.035, 0.3);
                    } else if self.mode == ApproachMode::LNAV {
                        self.mode = ApproachMode::LnavVnav;
                    }
                }
            }
            let guidance = NavGuidance {
                desired_track_rad: (desired - convergence).rem_euclid(core::f64::consts::TAU),
                cross_track_m: cross,
                to_from: ToFrom::To,
                kind: NavKind::Gps,
                vertical_deviation_m: self.vertical_deviation_m,
                path_angle_rad: self.path_angle_rad,
            };
            if guidance.validate().is_ok() {
                self.valid = true;
                self.guidance = Some(guidance);
                self.desired_track_true_rad = desired.rem_euclid(core::f64::consts::TAU);
                self.desired_track_rad = reference.true_to_magnetic(desired);
                self.cross_track_nm = cross / NM;
            }
            return;
        }
    }
}
fn altitude_met(leg: &ProcedureLeg, altitude: f64) -> bool {
    leg.altitude1_m
        .is_some_and(|a| match leg.altitude_descriptor.as_str() {
            "-" => altitude <= a + 3.,
            "B" => altitude >= a - 3. && leg.altitude2_m.is_none_or(|b| altitude <= b + 3.),
            _ => altitude >= a - 3.,
        })
}

impl crate::nav_source::NavSource for ProcedureReceiver {
    fn guidance(&self, _north_m: f64, _east_m: f64) -> Option<NavGuidance> {
        self.guidance
    }
}
