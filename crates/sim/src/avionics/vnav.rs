//! Vertical navigation: altitude constraints on flight-plan waypoints, the descent path computed
//! backwards from them, and the deviation from that path. Pure geometry on distances and
//! altitudes; no session, receiver, or clock.
//!
//! The model. Distances are metres along the route from the aircraft; altitudes are metres MSL.
//! Only constrained waypoints are vertices of the path. Working backwards from the last
//! constraint, each earlier constrained waypoint's path altitude is the altitude a descent at
//! the descent angle would need there to reach the next vertex, held to its own constraint
//! (an *at* constraint pins it, *at or above* raises it to at least the constraint, *at or
//! below* lowers it to at most the constraint). Between vertices the path is a straight line;
//! from the aircraft to the first vertex ahead it is the descent-angle line through that vertex.
//! Past the last vertex the path is level at that altitude.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The default descent angle, degrees.
pub const DEFAULT_ANGLE_DEG: f64 = 3.0;
/// The shallowest and steepest descent angle the pilot may set, degrees.
pub const MIN_ANGLE_DEG: f64 = 1.0;
pub const MAX_ANGLE_DEG: f64 = 6.0;
/// One dot of vertical deviation on the indicator, metres (100 ft); full scale is 2.5 dots.
pub const DOT_M: f64 = 30.48;
pub const FULL_SCALE_DOTS: f64 = 2.5;
const FEET: f64 = 1.0 / 0.3048;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintKind {
    /// Cross at this altitude.
    At,
    /// Cross at or above this altitude.
    AtOrAbove,
    /// Cross at or below this altitude.
    AtOrBelow,
}

/// An altitude restriction at a flight-plan waypoint.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AltitudeConstraint {
    pub kind: ConstraintKind,
    /// Metres MSL.
    #[schemars(range(min = 0, max = 15000))]
    pub altitude_m: f64,
}

impl AltitudeConstraint {
    pub fn new(kind: ConstraintKind, altitude_ft: f64) -> Self {
        Self {
            kind,
            altitude_m: altitude_ft * 0.3048,
        }
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.altitude_m.is_finite() && (0.0..=15_000.0).contains(&self.altitude_m) {
            Ok(())
        } else {
            Err("altitude constraint must be between 0 and 15,000 m")
        }
    }

    /// The constraint as the FPL page writes it: feet, then A (at or above), B (at or below), or
    /// nothing (at).
    pub fn text(&self) -> String {
        let feet = (self.altitude_m * FEET).round() as i64;
        match self.kind {
            ConstraintKind::At => format!("{feet}FT"),
            ConstraintKind::AtOrAbove => format!("{feet}FT A"),
            ConstraintKind::AtOrBelow => format!("{feet}FT B"),
        }
    }
}

/// A route point ahead of the aircraft.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoutePoint {
    /// Distance along the route from the aircraft, metres.
    pub distance_m: f64,
    pub constraint: Option<AltitudeConstraint>,
}

/// The vertical path to the next constrained waypoint.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct VnavSolution {
    /// Index (into the points given) of the waypoint the path leads to.
    pub target_index: usize,
    /// Path altitude at that waypoint.
    pub target_altitude_m: f64,
    /// Route distance to it.
    pub target_distance_m: f64,
    /// The path's altitude at the aircraft's position.
    pub path_altitude_m: f64,
    /// Aircraft above (positive) or below (negative) the path, metres.
    pub deviation_m: f64,
    /// The vertical speed that reaches the target altitude at the target waypoint at the current
    /// ground speed; negative descends. `None` without groundspeed.
    pub required_vs_mps: Option<f64>,
    /// Route distance to the top of descent (where the descent-angle path meets the current
    /// altitude): zero or negative when the aircraft is on or below the path's descent.
    pub top_of_descent_m: Option<f64>,
    /// The descent angle used, radians (positive).
    pub angle_rad: f64,
}

impl VnavSolution {
    /// Deviation in dots of the indicator, positive when above the path, clamped to full scale.
    pub fn dots(&self) -> f64 {
        (self.deviation_m / DOT_M).clamp(-FULL_SCALE_DOTS, FULL_SCALE_DOTS)
    }
}

/// Path altitude at each constrained point, computed backwards. Entries are
/// `(index, distance, altitude)` in route order.
pub fn vertices(points: &[RoutePoint], angle_rad: f64) -> Vec<(usize, f64, f64)> {
    let tan = libm::tan(angle_rad);
    let mut out: Vec<(usize, f64, f64)> = Vec::new();
    for (index, point) in points.iter().enumerate().rev() {
        let Some(c) = point.constraint else { continue };
        let altitude = match out.last() {
            None => c.altitude_m,
            Some(&(_, next_distance, next_altitude)) => {
                let projected = next_altitude + (next_distance - point.distance_m).max(0.0) * tan;
                match c.kind {
                    ConstraintKind::At => c.altitude_m,
                    ConstraintKind::AtOrAbove => c.altitude_m.max(projected),
                    ConstraintKind::AtOrBelow => c.altitude_m.min(projected),
                }
            }
        };
        out.push((index, point.distance_m, altitude));
    }
    out.reverse();
    out
}

/// The path to the next constraint ahead, for an aircraft at `altitude_m`. `points` are the
/// route points from the active waypoint on, nearest first, with distances from the aircraft.
/// `None` when no constraint lies ahead, the angle is outside 1 to 6 degrees, or the next
/// constraint is an *at or below* the aircraft already satisfies.
pub fn solve(
    points: &[RoutePoint],
    altitude_m: f64,
    ground_speed_mps: f64,
    angle_deg: f64,
) -> Option<VnavSolution> {
    if !(MIN_ANGLE_DEG..=MAX_ANGLE_DEG).contains(&angle_deg) || !altitude_m.is_finite() {
        return None;
    }
    if points
        .iter()
        .any(|p| !p.distance_m.is_finite() || p.distance_m < 0.0)
    {
        return None;
    }
    let angle = angle_deg.to_radians();
    let tan = libm::tan(angle);
    let path = vertices(points, angle);
    // The first vertex the aircraft still has to meet: an at-or-below constraint it already
    // satisfies is skipped.
    let &(index, distance, target) = path.iter().find(|&&(i, _, _)| {
        !matches!(points.get(i).and_then(|p| p.constraint), Some(c) if c.kind == ConstraintKind::AtOrBelow && altitude_m <= c.altitude_m)
    })?;
    let path_altitude = target + distance * tan;
    let climb_to_target = target - altitude_m;
    let required_vs =
        (ground_speed_mps > 1.0 && distance > 1.0).then(|| climb_to_target / (distance / ground_speed_mps));
    let top = (altitude_m > target).then(|| distance - (altitude_m - target) / tan);
    Some(VnavSolution {
        target_index: index,
        target_altitude_m: target,
        target_distance_m: distance,
        path_altitude_m: path_altitude,
        deviation_m: altitude_m - path_altitude,
        required_vs_mps: required_vs,
        top_of_descent_m: top,
        angle_rad: angle,
    })
}

/// A vertical path along the active leg, for the autopilot's path tracking: the path altitude at
/// a point `remaining_m` from the end of the leg is `end_altitude_m + (remaining_m + beyond_m)
/// * tan(angle)`, and level at `end_altitude_m` once the target is passed.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegPath {
    /// Path altitude at the target waypoint.
    pub target_altitude_m: f64,
    /// Route distance from the end of the active leg to the target waypoint (zero when the leg
    /// ends at it).
    pub beyond_m: f64,
    pub angle_rad: f64,
}

impl LegPath {
    /// Path altitude and the path angle (negative descends, zero level) at `remaining_m`
    /// from the end of the active leg.
    pub fn at(&self, remaining_m: f64) -> (f64, f64) {
        let to_target = remaining_m + self.beyond_m;
        if to_target <= 0.0 {
            (self.target_altitude_m, 0.0)
        } else {
            (
                self.target_altitude_m + to_target * libm::tan(self.angle_rad),
                -self.angle_rad,
            )
        }
    }
}

/// Text for the data bar: vertical speed required in feet per minute, rounded to 10.
pub fn vs_text(required_vs_mps: f64) -> String {
    format!("{:+.0}FPM", (required_vs_mps * FEET * 60.0 / 10.0).round() * 10.0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const NM: f64 = 1852.0;
    fn ft(f: f64) -> f64 {
        f * 0.3048
    }
    fn at(distance_nm: f64, kind: ConstraintKind, feet: f64) -> RoutePoint {
        RoutePoint {
            distance_m: distance_nm * NM,
            constraint: Some(AltitudeConstraint::new(kind, feet)),
        }
    }
    fn open(distance_nm: f64) -> RoutePoint {
        RoutePoint {
            distance_m: distance_nm * NM,
            constraint: None,
        }
    }

    #[test]
    fn constraints_are_validated() {
        assert!(
            AltitudeConstraint::new(ConstraintKind::At, 3000.)
                .validate()
                .is_ok()
        );
        assert!(AltitudeConstraint::new(ConstraintKind::At, 0.).validate().is_ok());
        assert!(
            AltitudeConstraint::new(ConstraintKind::At, -50.)
                .validate()
                .is_err()
        );
        assert!(
            AltitudeConstraint::new(ConstraintKind::At, 60_000.)
                .validate()
                .is_err()
        );
        let nan = AltitudeConstraint {
            kind: ConstraintKind::At,
            altitude_m: f64::NAN,
        };
        assert!(nan.validate().is_err());
        assert_eq!(
            AltitudeConstraint::new(ConstraintKind::AtOrAbove, 2500.).text(),
            "2500FT A"
        );
        assert_eq!(
            AltitudeConstraint::new(ConstraintKind::AtOrBelow, 4000.).text(),
            "4000FT B"
        );
        assert_eq!(
            AltitudeConstraint::new(ConstraintKind::At, 1500.).text(),
            "1500FT"
        );
    }

    #[test]
    fn the_path_through_one_constraint_is_a_three_degree_line_and_climbs_back_up_the_route() {
        // A fix 10 NM ahead at 2,000 ft. A 3 degree path is 0.0524 * 6076 ft = 318 ft per NM.
        let points = [open(4.), at(10., ConstraintKind::At, 2000.)];
        let s = solve(&points, ft(5000.), 60., 3.0).unwrap();
        assert_eq!(s.target_index, 1);
        assert!((s.target_altitude_m - ft(2000.)).abs() < 1e-9);
        let want = ft(2000.) + 10. * NM * libm::tan(3f64.to_radians());
        assert!((s.path_altitude_m - want).abs() < 1e-6);
        assert!(
            (want * FEET - 5183.).abs() < 5.,
            "path is near 5,183 ft here: {}",
            want * FEET
        );
        // Top of descent: where the path meets 5,000 ft is a little under 10 NM out.
        let tod = s.top_of_descent_m.unwrap() / NM;
        assert!((tod - (10. - 3000. / 318.4)).abs() < 0.05, "{tod}");
    }

    #[test]
    fn deviation_is_positive_above_the_path_and_negative_below() {
        let points = [at(10., ConstraintKind::At, 2000.)];
        let path = solve(&points, ft(3000.), 60., 3.0).unwrap().path_altitude_m;
        let above = solve(&points, path + ft(200.), 60., 3.0).unwrap();
        let below = solve(&points, path - ft(200.), 60., 3.0).unwrap();
        let on = solve(&points, path, 60., 3.0).unwrap();
        assert!((above.deviation_m - ft(200.)).abs() < 1e-6);
        assert!((below.deviation_m + ft(200.)).abs() < 1e-6);
        assert!(on.deviation_m.abs() < 1e-6);
        // Two dots is 200 ft; the indicator clamps at 2.5 dots.
        assert!((above.dots() - 200. / 100.).abs() < 0.01);
        assert!(below.dots() < 0.);
        let far_above = solve(&points, path + ft(2000.), 60., 3.0).unwrap();
        assert_eq!(far_above.dots(), FULL_SCALE_DOTS);
    }

    #[test]
    fn vertical_speed_required_reaches_the_target_altitude_at_the_target() {
        // 3,000 ft to lose in 10 NM at 120 kt: 5 minutes, so 600 fpm down.
        let gs = 120. * 1852. / 3600.;
        let s = solve(&[at(10., ConstraintKind::At, 2000.)], ft(5000.), gs, 3.0).unwrap();
        let fpm = s.required_vs_mps.unwrap() * FEET * 60.;
        assert!((fpm + 600.).abs() < 0.5, "{fpm}");
        assert_eq!(vs_text(s.required_vs_mps.unwrap()), "-600FPM");
        // Level at the target altitude needs none; below it needs a climb.
        let level = solve(&[at(10., ConstraintKind::At, 2000.)], ft(2000.), gs, 3.0).unwrap();
        assert!(level.required_vs_mps.unwrap().abs() < 1e-9);
        let low = solve(&[at(10., ConstraintKind::At, 2000.)], ft(1500.), gs, 3.0).unwrap();
        assert!(low.required_vs_mps.unwrap() > 0.);
        // Without ground speed there is no answer.
        assert_eq!(
            solve(&[at(10., ConstraintKind::At, 2000.)], ft(5000.), 0., 3.0)
                .unwrap()
                .required_vs_mps,
            None
        );
    }

    #[test]
    fn the_path_is_computed_backwards_through_several_constraints() {
        // Fixes at 8 NM (at or above 3,500), 14 NM (at 3,000), 20 NM (at 1,500).
        let points = [
            at(8., ConstraintKind::AtOrAbove, 3500.),
            at(14., ConstraintKind::At, 3000.),
            at(20., ConstraintKind::At, 1500.),
        ];
        let v = vertices(&points, 3f64.to_radians());
        assert_eq!(v.len(), 3);
        // The last is pinned; the middle is pinned by its own "at".
        assert!((v[2].2 - ft(1500.)).abs() < 1e-9);
        assert!((v[1].2 - ft(3000.)).abs() < 1e-9);
        // The first: a 3 degree descent from 3,000 ft at 14 NM needs 6 NM * 318 ft = 1,910 ft more,
        // so 4,910 ft there, above its 3,500 floor.
        let want = ft(3000.) + 6. * NM * libm::tan(3f64.to_radians());
        assert!(
            (v[0].2 - want).abs() < 1e-6,
            "{} vs {}",
            v[0].2 * FEET,
            want * FEET
        );
        // The path leads to the first vertex.
        let s = solve(&points, ft(6000.), 60., 3.0).unwrap();
        assert_eq!(s.target_index, 0);
        assert!((s.target_altitude_m - want).abs() < 1e-6);
    }

    #[test]
    fn at_or_above_and_at_or_below_hold_the_path_to_their_side() {
        let angle = 3f64.to_radians();
        // At or above 3,000 with a following fix at 2,000, 2 NM later: the 3 degree descent
        // needs only 2,000 + 637 = 2,637 ft there, so the floor of 3,000 holds.
        let floor = [
            at(10., ConstraintKind::AtOrAbove, 3000.),
            at(12., ConstraintKind::At, 2000.),
        ];
        assert!((vertices(&floor, angle)[0].2 - ft(3000.)).abs() < 1e-9);
        // At or below 2,500 over a follow-on at 2,000 only 1 NM later: the descent needs
        // 2,318 ft there, which is under the ceiling and wins.
        let ceiling = [
            at(10., ConstraintKind::AtOrBelow, 2500.),
            at(11., ConstraintKind::At, 2000.),
        ];
        let got = vertices(&ceiling, angle)[0].2;
        assert!((got - (ft(2000.) + NM * libm::tan(angle))).abs() < 1e-6);
        assert!(got < ft(2500.));
        // An at-or-below the aircraft already satisfies is not a target: the next one is.
        let skip = [
            at(5., ConstraintKind::AtOrBelow, 4000.),
            at(10., ConstraintKind::At, 2000.),
        ];
        assert_eq!(solve(&skip, ft(3500.), 60., 3.0).unwrap().target_index, 1);
        assert_eq!(solve(&skip, ft(4500.), 60., 3.0).unwrap().target_index, 0);
    }

    #[test]
    fn no_constraint_a_bad_angle_or_a_bad_distance_gives_no_path() {
        assert!(solve(&[open(5.), open(9.)], ft(3000.), 60., 3.0).is_none());
        assert!(solve(&[], ft(3000.), 60., 3.0).is_none());
        let p = [at(10., ConstraintKind::At, 2000.)];
        assert!(solve(&p, ft(3000.), 60., 0.5).is_none());
        assert!(solve(&p, ft(3000.), 60., 7.0).is_none());
        assert!(solve(&p, f64::NAN, 60., 3.0).is_none());
        assert!(solve(&[at(-1., ConstraintKind::At, 2000.)], ft(3000.), 60., 3.0).is_none());
        // A steeper angle makes a lower path at the same distance from the target.
        let shallow = solve(&p, ft(3000.), 60., 2.0).unwrap().path_altitude_m;
        let steep = solve(&p, ft(3000.), 60., 5.0).unwrap().path_altitude_m;
        assert!(steep > shallow);
    }

    #[test]
    fn the_leg_path_descends_to_the_target_then_levels() {
        let leg = LegPath {
            target_altitude_m: ft(2000.),
            beyond_m: 2. * NM,
            angle_rad: 3f64.to_radians(),
        };
        // 3 NM from the end of the leg and 2 NM more to the target: 5 NM of descent.
        let (alt, angle) = leg.at(3. * NM);
        assert!((alt - (ft(2000.) + 5. * NM * libm::tan(3f64.to_radians()))).abs() < 1e-6);
        assert!((angle + 3f64.to_radians()).abs() < 1e-12);
        // Passing the target: level at its altitude.
        let (alt, angle) = leg.at(-2.5 * NM);
        assert_eq!((alt, angle), (ft(2000.), 0.0));
        // It agrees with the whole-route solution at the same point.
        let route = [at(5., ConstraintKind::At, 2000.)];
        let s = solve(&route, ft(4000.), 60., 3.0).unwrap();
        assert!((leg.at(3. * NM).0 - s.path_altitude_m).abs() < 1e-6);
    }
}
