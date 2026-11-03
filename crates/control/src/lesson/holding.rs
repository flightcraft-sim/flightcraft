use super::*;
use flightcraft_sim::avionics::procedures::{HoldEntry, hold_entry};
use flightcraft_world::procedures::{PathTerminator, Turn};
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct PublishedHold {
    pub fix: String,
    pub position: GeoPoint,
    pub inbound_deg: f64,
    pub inbound_true_deg: f64,
    pub turn: Turn,
    pub time_s: Option<f64>,
    pub distance_nm: Option<f64>,
    pub altitude_m: f64,
}
impl PublishedHold {
    pub fn find(s: &Session, airport: &str, procedure: &str, fix: &str) -> Result<Self, CommandError> {
        let d = s
            .sim()
            .world()
            .procedures()
            .ok_or_else(|| invalid("hold", "region has no procedures"))?;
        d.procedures
            .iter()
            .filter(|p| p.airport == airport && p.ident == procedure)
            .flat_map(|p| &p.routes)
            .flat_map(|r| &r.legs)
            .find_map(|l| {
                let point = d.point(l.fix.as_deref()?)?;
                (point.ident == fix
                    && matches!(
                        l.path,
                        PathTerminator::HA | PathTerminator::HF | PathTerminator::HM
                    ))
                .then(|| Self {
                    fix: point.ident.clone(),
                    position: point.position,
                    inbound_deg: l.course_deg.unwrap_or(0.),
                    inbound_true_deg: l.course_deg.unwrap_or(0.)
                        + s.sim().readout().magnetic_declination_rad.to_degrees(),
                    turn: l.turn.unwrap_or(Turn::R),
                    time_s: l.time_s,
                    distance_nm: l.distance_nm,
                    altitude_m: l.altitude1_m.unwrap_or(914.4),
                })
            })
            .ok_or_else(|| invalid("hold", "published hold fix unavailable"))
    }
}
/// Classify what was flown after crossing the fix, independent of the GPS entry annunciation.
pub fn actual_hold_entry(
    inbound_deg: f64,
    outbound_track_deg: f64,
    first_turn_deg: f64,
    turn: Turn,
) -> HoldEntry {
    let sign = if turn == Turn::R { 1. } else { -1. };
    let offset = wrap_deg(outbound_track_deg - inbound_deg - 180.) * sign;
    if (-50. ..=-10.).contains(&offset) {
        HoldEntry::Teardrop
    } else if first_turn_deg * sign < -20. {
        HoldEntry::Parallel
    } else {
        HoldEntry::Direct
    }
}
pub fn wrap_deg(deg: f64) -> f64 {
    (deg + 180.).rem_euclid(360.) - 180.
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct HoldObservation {
    pub expected_entry: Option<HoldEntry>,
    pub actual_entry: Option<HoldEntry>,
    pub inbound_time_s: Option<f64>,
    pub outbound_distance_nm: f64,
    pub inbound_cross_track_nm: f64,
    pub wind_correction_deg: f64,
    pub laps: u32,
    pub reasons: Vec<String>,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct HoldMonitor {
    pub spec: Option<PublishedHold>,
    last_position: Option<GeoPoint>,
    last_track: Option<f64>,
    crossed_s: Option<f64>,
    turn_sum: f64,
    straight_s: f64,
    entry_outbound_track: Option<f64>,
    inbound_started_s: Option<f64>,
    previous_along: Option<f64>,
    outbound: bool,
    entry_circuit_complete: bool,
    max_distance: f64,
}
impl HoldMonitor {
    pub fn sample(&mut self, position: GeoPoint, heading: f64, now: f64, dt: f64, out: &mut HoldObservation) {
        let Some(spec) = &self.spec else {
            return;
        };
        let Some(previous) = self.last_position.replace(position) else {
            return;
        };
        let (_, track) = previous.inverse(position);
        let (distance, bearing) = spec.position.inverse(position);
        let rel = (bearing - spec.inbound_true_deg).to_radians();
        let along = distance * libm::cos(rel);
        let cross = distance * libm::sin(rel) / 1852.;
        let crossed = self.previous_along.is_some_and(|a| a < 0. && along >= 0.) && distance < 0.5 * 1852.;
        self.previous_along = Some(along);
        if crossed && self.crossed_s.is_none() {
            self.crossed_s = Some(now);
            out.expected_entry = Some(hold_entry(spec.inbound_true_deg, track, spec.turn));
        } else if crossed && self.outbound && self.inbound_started_s.is_some() {
            out.inbound_time_s = self.inbound_started_s.map(|t| now - t);
            out.outbound_distance_nm = self.max_distance;
            let entry_circuit = !self.entry_circuit_complete && out.actual_entry != Some(HoldEntry::Direct);
            self.entry_circuit_complete = true;
            if !entry_circuit {
                out.laps = out.laps.saturating_add(1);
                if let Some(expected) = spec.time_s
                    && out.inbound_time_s.is_some_and(|t| (t - expected).abs() > 10.)
                {
                    add_reason(out, "Inbound timing outside the instructor's 10-second margin.");
                }
                if let Some(expected) = spec.distance_nm
                    && (self.max_distance - expected).abs() > 0.5
                {
                    add_reason(out, "Leg length outside the instructor's 0.5 NM margin.");
                }
            }
            self.crossed_s = Some(now);
            self.inbound_started_s = None;
            self.outbound = false;
            self.max_distance = 0.;
        }
        if let Some(start) = self.crossed_s {
            let delta = self.last_track.map_or(0., |last| wrap_deg(track - last));
            if out.actual_entry.is_none() {
                self.turn_sum += delta;
                if delta.abs() < 1.5 && now - start > 8. {
                    self.straight_s += dt;
                } else {
                    self.straight_s = 0.;
                }
                if self.straight_s > 12. && wrap_deg(track - spec.inbound_true_deg).abs() > 100. {
                    self.entry_outbound_track.get_or_insert(track);
                }
                if let Some(outbound_track) = self.entry_outbound_track {
                    let candidate =
                        actual_hold_entry(spec.inbound_true_deg, outbound_track, self.turn_sum, spec.turn);
                    // An arrival already near the outbound heading needs the subsequent
                    // reversal to distinguish a parallel entry from a direct entry.
                    if candidate == HoldEntry::Teardrop || self.turn_sum.abs() > 20. {
                        out.actual_entry = Some(candidate);
                        if out.actual_entry != out.expected_entry {
                            add_reason(out, "Actual hold entry did not match the arrival sector.");
                        }
                    }
                }
            }
            if wrap_deg(track - spec.inbound_true_deg).abs() > 120. {
                self.outbound = true;
            }
            if self.outbound {
                self.max_distance = self.max_distance.max(distance / 1852.);
            }
            if self.outbound && wrap_deg(track - spec.inbound_true_deg).abs() < 10. {
                self.inbound_started_s.get_or_insert(now);
                out.inbound_cross_track_nm = cross;
                out.wind_correction_deg =
                    wrap_deg(heading - (track - spec.inbound_true_deg + spec.inbound_deg));
                if cross.abs() > 0.225 {
                    add_reason(
                        out,
                        "Inbound wind correction did not maintain the holding course.",
                    );
                }
            }
        }
        self.last_track = Some(track);
    }
}
fn add_reason(out: &mut HoldObservation, reason: &str) {
    if !out.reasons.iter().any(|r| r == reason) && out.reasons.len() < 16 {
        out.reasons.push(reason.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn feed(m: &mut HoldMonitor, out: &mut HoldObservation, time_s: f64, north: f64, east: f64) {
        let origin = GeoPoint {
            latitude_deg: 0.,
            longitude_deg: 0.,
        };
        let p = origin.destination(libm::atan2(east, north).to_degrees(), libm::hypot(north, east));
        let heading = m.last_position.map_or(0., |previous| previous.inverse(p).1);
        m.sample(p, heading, time_s, 1., out);
    }
    #[test]
    fn parallel_entry_return_is_not_graded_as_a_one_minute_holding_leg() {
        let mut m = HoldMonitor {
            spec: Some(PublishedHold {
                fix: "TEST".into(),
                position: GeoPoint {
                    latitude_deg: 0.,
                    longitude_deg: 0.,
                },
                inbound_deg: 0.,
                inbound_true_deg: 0.,
                turn: Turn::R,
                time_s: Some(60.),
                distance_nm: None,
                altitude_m: 914.4,
            }),
            ..Default::default()
        };
        let mut out = HoldObservation::default();
        feed(&mut m, &mut out, 0., -2., 200.);
        feed(&mut m, &mut out, 1., -1., 100.);
        feed(&mut m, &mut out, 2., 1., -1.);
        for second in 3..=32 {
            feed(
                &mut m,
                &mut out,
                f64::from(second),
                1. - 40. * f64::from(second - 2),
                -1.,
            );
        }
        feed(&mut m, &mut out, 47., -800., 0.);
        for second in 48..=67 {
            feed(
                &mut m,
                &mut out,
                f64::from(second),
                -800. + 40. * f64::from(second - 47),
                0.,
            );
        }
        assert_eq!(out.expected_entry, Some(HoldEntry::Parallel));
        assert_eq!(out.actual_entry, Some(HoldEntry::Parallel));
        assert_eq!(out.laps, 0);
        assert!(out.reasons.is_empty(), "{:?}", out.reasons);
        // One complete standard-rate racetrack, with a sixty-second straight leg.
        let radius = 40. / 3_f64.to_radians();
        for second in 1..=60 {
            let a = f64::from(second) * core::f64::consts::PI / 60.;
            feed(
                &mut m,
                &mut out,
                67. + f64::from(second),
                radius * libm::sin(a),
                radius * (1. - libm::cos(a)),
            );
        }
        for second in 1..=60 {
            feed(
                &mut m,
                &mut out,
                127. + f64::from(second),
                -40. * f64::from(second),
                2. * radius,
            );
        }
        for second in 1..=60 {
            let a = f64::from(second) * core::f64::consts::PI / 60.;
            feed(
                &mut m,
                &mut out,
                187. + f64::from(second),
                -2400. - radius * libm::sin(a),
                radius * (1. + libm::cos(a)),
            );
        }
        for second in 1..=61 {
            feed(
                &mut m,
                &mut out,
                247. + f64::from(second),
                -2400. + 40. * f64::from(second),
                0.,
            );
        }
        assert_eq!(out.laps, 1);
        assert!(out.inbound_time_s.is_some_and(|t| (t - 60.).abs() <= 10.));
        assert!(out.reasons.is_empty(), "{:?}", out.reasons);
    }
}
