use super::*;
use flightcraft_world::{AirspaceClass, geo::LocalProjection};
fn fail(message: impl Into<String>) -> CommandError {
    invalid(CrewIntent::NAME, message)
}
fn projection(s: &Session) -> Result<LocalProjection, CommandError> {
    let r = s
        .sim
        .world()
        .region()
        .ok_or_else(|| fail("this intent needs installed region data"))?;
    LocalProjection::new(r.origin.latitude_deg, r.origin.longitude_deg).map_err(|e| fail(e.to_string()))
}
pub(super) fn geographic(s: &Session, [n, e, _]: [f64; 3]) -> Option<GeoPoint> {
    let [latitude_deg, longitude_deg] = projection(s).ok()?.unproject(n, e).ok()?;
    Some(GeoPoint {
        latitude_deg,
        longitude_deg,
    })
}
pub(super) fn local(s: &Session, point: GeoPoint, altitude: f64) -> Result<[f64; 3], CommandError> {
    crate::navigation::valid_point(point)?;
    let [n, e] = projection(s)?
        .project(point.latitude_deg, point.longitude_deg)
        .map_err(|e| fail(e.to_string()))?;
    Ok([n, e, -altitude])
}
pub(super) fn runway(s: &Session, airport: &str, end: &str) -> Result<CrewRunway, CommandError> {
    if airport == "default" && s.sim.world().region().is_none() {
        let r = s.sim.world().runway();
        return Ok(CrewRunway {
            airport: "Corvid Field".into(),
            runway: "36".into(),
            threshold_m: r.threshold_ned_m,
            heading_grid_rad: r.heading_deg.to_radians(),
            length_m: r.length_m,
            width_m: r.width_m,
            pattern_altitude_m: 1000. * 0.3048,
        });
    }
    let a = s
        .sim
        .world()
        .airports()
        .and_then(|d| {
            d.airports.iter().find(|a| {
                a.ident.eq_ignore_ascii_case(airport)
                    || a.icao.as_ref().is_some_and(|i| i.eq_ignore_ascii_case(airport))
            })
        })
        .ok_or_else(|| fail(format!("airport {airport} is not in the installed catalogue")))?;
    let (strip, threshold) = a
        .runways
        .iter()
        .find_map(|r| r.ends.iter().find(|e| e.ident == end).map(|e| (r, e)))
        .ok_or_else(|| fail(format!("runway {end} is not in {airport}")))?;
    // The regional projection's axes are true north/east. Runway headings are surveyed true.
    let threshold_m = local(
        s,
        GeoPoint {
            latitude_deg: threshold.latitude_deg,
            longitude_deg: threshold.longitude_deg,
        },
        threshold.elevation_m,
    )?;
    Ok(CrewRunway {
        airport: a.icao.clone().unwrap_or_else(|| a.ident.clone()),
        runway: end.into(),
        threshold_m,
        heading_grid_rad: threshold.heading_deg.to_radians(),
        length_m: strip.length_m,
        width_m: strip.width_m,
        pattern_altitude_m: a.traffic_pattern_altitude_ft.map_or_else(
            || ((a.elevation_m / 0.3048 + 1000.) / 100.).round() * 100.,
            f64::from,
        ) * 0.3048,
    })
}
fn nearest_runway(s: &Session) -> Result<CrewRunway, CommandError> {
    if s.sim.world().region().is_none() {
        return runway(s, "default", "36");
    }
    let r = s.sim.readout();
    let mut nearest: Option<(f64, CrewRunway)> = None;
    if let Some(data) = s.sim.world().airports() {
        for a in &data.airports {
            for strip in &a.runways {
                for end in &strip.ends {
                    let candidate = runway(s, &a.ident, &end.ident)?;
                    let [n, e, _] = candidate.threshold_m;
                    let [pn, pe, _] = s.sim.state().position_m;
                    let score = libm::hypot(n - pn, e - pe)
                        + flightcraft_sim::autopilot::wrap(candidate.heading_grid_rad - r.heading_grid_rad)
                            .abs()
                            * 1000.0;
                    if nearest.as_ref().is_none_or(|(best, _)| score < *best) {
                        nearest = Some((score, candidate));
                    }
                }
            }
        }
    }
    nearest
        .map(|(_, r)| r)
        .ok_or_else(|| fail("no runway data available"))
}
pub(super) fn leg(s: &Session, name: &str, point: [f64; 3]) -> CrewLeg {
    CrewLeg {
        name: name.into(),
        position_m: point,
        position: geographic(s, point),
    }
}
pub(super) fn arrival_legs(s: &Session, r: &CrewRunway, straight: bool) -> Vec<CrewLeg> {
    if straight {
        return vec![leg(
            s,
            "Straight-in intercept",
            r.point(-4000., 0., r.pattern_altitude_m + r.threshold_m[2]),
        )];
    }
    vec![
        leg(
            s,
            "45-degree downwind entry",
            r.point(800., -3000., r.pattern_altitude_m + r.threshold_m[2]),
        ),
        leg(
            s,
            "Join left downwind",
            r.point(-700., -1500., r.pattern_altitude_m + r.threshold_m[2]),
        ),
    ]
}
pub(super) fn validate_path(s: &mut Session, plan: &mut CrewPlan) -> Result<(), CommandError> {
    if s.sim.world().region().is_none() {
        plan.checks
            .push("Original flat-world runway; no regional terrain or airspace catalogue".into());
        return Ok(());
    }
    let start = leg(s, "Current position", s.sim.state().position_m);
    let mut points = vec![start];
    if let Some(d) = &plan.departure {
        // Check the climb segment at actual endpoint heights, allowing the departure corridor.
        points.push(leg(s, "Climb-out", d.point(3000., 0., 315.)));
    }
    points.extend(plan.legs.clone());
    let waypoints = points
        .iter()
        .filter_map(|l| {
            l.position.map(|position| PlanWaypoint {
                waypoint: GpsWaypoint {
                    ident: l.name.clone(),
                    position,
                    kind: WaypointKind::Coordinate,
                },
                altitude_m: -l.position_m.get(2).copied().unwrap_or(0.),
            })
        })
        .collect();
    let saved = std::mem::replace(
        &mut s.navigation.plan,
        FlightPlan {
            waypoints,
            ..Default::default()
        },
    );
    let result = ProfilePlan { spacing_m: 100. }.run(s);
    s.navigation.plan = saved;
    let profile = result?;
    let new_york_bravo_clearance = s.atc_state().bravo_clearance;
    for sample in &profile.samples {
        if sample.outside_region {
            return Err(fail("route leaves installed terrain and airspace coverage"));
        }
        if sample.airspace.iter().any(|a| {
            a.class == Some(AirspaceClass::B)
                && sample.altitude_m >= a.floor_m
                && sample.altitude_m < a.ceiling_m
                && !(new_york_bravo_clearance
                    && s.sim.world().airspace().is_some_and(|data| {
                        data.volumes.iter().any(|v| {
                            v.info.id == a.id && v.info.name.eq_ignore_ascii_case("NEW YORK CLASS B")
                        })
                    }))
        }) {
            return Err(fail(
                "route enters Class B without applicable clearance; choose another route or altitude",
            ));
        }
        if let Some(height) = sample.terrain_m {
            // Arrival/departure corridors are checked for collision rather than cruise clearance.
            let corridor = plan.arrival.iter().chain(plan.departure.iter()).any(|r| {
                let [n, e, _] = r.threshold_m;
                local(s, sample.position, sample.altitude_m)
                    .is_ok_and(|[pn, pe, _]| libm::hypot(pn - n, pe - e) < 6500.)
            });
            let clearance = if corridor { 0.0 } else { 304.8 };
            if sample.altitude_m < height + clearance - 2.0 {
                return Err(fail(format!(
                    "terrain clearance insufficient near {:.4}, {:.4}",
                    sample.position.latitude_deg, sample.position.longitude_deg
                )));
            }
        }
    }
    plan.checks.push(format!(
        "{} route-profile samples at 100 m: terrain coverage and Class B checked",
        profile.samples.len()
    ));
    plan.checks
        .push("Radio position reports do not confer controlled-airspace or landing clearance".into());
    Ok(())
}
pub(super) fn plan(s: &mut Session, intent: &PilotIntent) -> Result<(CrewPlan, PilotPhase), CommandError> {
    if s.crew.state.mode == CrewMode::Solo {
        return Err(fail("select watching or demonstrating mode first"));
    }
    if s.sim.readout().crashed {
        return Err(fail("reset the crashed aircraft first"));
    }
    let speeds = s
        .aircraft_configuration()
        .map(|a| &a.variant.v_speeds)
        .ok_or_else(|| fail("aircraft needs standard V-speed performance data"))?;
    let vy = speeds.vy_mps;
    let r = s.sim.readout();
    let grounded = r.on_ground || r.height_agl_m < 2.;
    let mut p = CrewPlan {
        briefing: String::new(),
        legs: vec![],
        departure: None,
        arrival: None,
        altitude_m: r.altitude_msl_m,
        checks: vec![format!(
            "Aircraft performance: Vx {:.1}, Vy {:.1}, rotation {:.1}, approach 1.3 Vso {:.1} m/s",
            speeds.vx_mps,
            vy,
            speeds.vr_mps,
            1.3 * speeds.vso_mps
        )],
    };
    let mut phase = if grounded {
        PilotPhase::BeforeTakeoff
    } else {
        PilotPhase::Cruise
    };
    match intent {
        PilotIntent::Approach { .. } | PilotIntent::HoldAtFix { .. } => {
            return super::instrument::plan(s, intent);
        }
        PilotIntent::Hold {
            altitude_m,
            heading_rad,
        } => {
            range(CrewIntent::NAME, "altitude_m", *altitude_m, 2., 10000.)?;
            range(
                CrewIntent::NAME,
                "heading_rad",
                *heading_rad,
                0.,
                core::f64::consts::TAU,
            )?;
            if grounded {
                return Err(fail("hold requires an airborne aircraft"));
            }
            p.altitude_m = *altitude_m;
            phase = PilotPhase::Hold;
            let [n, e, _] = s.sim.state().position_m;
            p.legs.push(leg(s, "Hold target", [n, e, -altitude_m]));
        }
        PilotIntent::Orbit {
            position,
            altitude_m,
            radius_m,
        } => {
            if grounded {
                return Err(fail("orbit requires an airborne aircraft; take off first"));
            }
            range(CrewIntent::NAME, "radius_m", *radius_m, 500., 10000.)?;
            p.altitude_m = *altitude_m;
            let centre = local(s, *position, *altitude_m)?;
            // Check the complete circle, not just its centre.
            for i in 0..=32 {
                let a = f64::from(i) * core::f64::consts::TAU / 32.;
                let (sin, cos) = libm::sincos(a);
                let [n, e, d] = centre;
                p.legs.push(leg(
                    s,
                    "Scenic orbit",
                    [n + cos * radius_m, e + sin * radius_m, d],
                ));
            }
            phase = PilotPhase::Orbit;
        }
        PilotIntent::GoAround {} => {
            let arrival = s
                .crew
                .state
                .plan
                .as_ref()
                .and_then(|p| p.arrival.clone())
                .ok_or_else(|| fail("go-around requires a planned runway"))?;
            if grounded {
                return Err(fail("go-around requires an airborne aircraft"));
            }
            p.altitude_m = arrival.pattern_altitude_m;
            p.arrival = Some(arrival);
            phase = PilotPhase::GoAround;
        }
        PilotIntent::TakeOff { airport, runway: end } => {
            if !grounded {
                return Err(fail("takeoff requires an aircraft on the runway"));
            }
            let departure = runway(s, airport, end)?;
            p.altitude_m = departure.pattern_altitude_m;
            p.departure = Some(departure);
        }
        PilotIntent::JoinPattern { airport, runway: end }
        | PilotIntent::LandRunway { airport, runway: end }
        | PilotIntent::FlyToAirport {
            airport, runway: end, ..
        } => {
            let arrival = runway(s, airport, end)?;
            p.altitude_m = match intent {
                PilotIntent::FlyToAirport { altitude_m, .. } => *altitude_m,
                _ => arrival.pattern_altitude_m,
            };
            let (along, cross) = arrival.relative(s.sim.state().position_m);
            let straight = !grounded
                && along < -2500.
                && cross.abs() < 500.
                && flightcraft_sim::autopilot::wrap(r.heading_grid_rad - arrival.heading_grid_rad).abs()
                    < 0.35;
            p.legs = arrival_legs(s, &arrival, straight);
            if let Some(entry) = p.legs.first() {
                let [en, ee, _] = entry.position_m;
                let [n, e, _] = s.sim.state().position_m;
                let distance = libm::hypot(n - en, e - ee);
                if distance > 22000.0 {
                    let before = 18000.0 / distance;
                    let cruise = [en + (n - en) * before, ee + (e - ee) * before, -p.altitude_m];
                    p.legs.insert(0, leg(s, &format!("Cruise to {airport}"), cruise));
                }
            }
            if straight {
                phase = PilotPhase::Final;
            }
            p.arrival = Some(arrival);
        }
        PilotIntent::FlyToCoordinate { position, altitude_m } => {
            p.altitude_m = *altitude_m;
            p.legs
                .push(leg(s, "Requested coordinate", local(s, *position, *altitude_m)?));
        }
        PilotIntent::FlightPlan {} => {
            if s.navigation.plan.waypoints.is_empty() {
                return Err(fail("the flight plan is empty"));
            }
            for w in &s.navigation.plan.waypoints {
                p.legs.push(leg(
                    s,
                    &w.waypoint.ident,
                    local(s, w.waypoint.position, w.altitude_m)?,
                ));
            }
            p.altitude_m = p
                .legs
                .first()
                .and_then(|l| l.position_m.get(2))
                .map_or(r.altitude_msl_m, |d| -d);
        }
        PilotIntent::Scenic {
            preset,
            airport,
            runway: end,
        } => {
            let (name, coords): (&str, &[(f64, f64)]) = match preset {
                ScenicPreset::DelawareWaterGap => ("Delaware Water Gap", &[]),
                ScenicPreset::PoconoPlateau => (
                    "Pocono plateau",
                    &[(41.08, -75.23), (41.16, -75.34), (41.08, -75.23)],
                ),
                ScenicPreset::HudsonCorridor => {
                    return Err(fail(
                        "Hudson corridor preset unavailable: a verified Class B exclusion route and current special-flight-rules procedures are required",
                    ));
                }
            };
            let scenic_points: Vec<(f64, f64)> = if matches!(preset, ScenicPreset::DelawareWaterGap) {
                let landmarks = s.sim.world().landmarks().ok_or_else(|| {
                    fail("install the regional GNIS landmarks for the Water Gap scenic flight")
                })?;
                [
                    ("Mount Tammany", "Summit"),
                    ("Delaware Water Gap", "Gap"),
                    ("Tannersville", "Populated Place"),
                    ("Mount Tammany", "Summit"),
                ]
                .iter()
                .map(|(name, class)| {
                    let f = landmarks
                        .features
                        .iter()
                        .find(|f| f.name == *name && f.feature_class == *class)
                        .ok_or_else(|| fail(format!("regional GNIS feature missing: {name}")))?;
                    Ok((f.position.latitude_deg, f.position.longitude_deg))
                })
                .collect::<Result<_, CommandError>>()?
            } else {
                coords.to_vec()
            };
            p.altitude_m = 1250.;
            for (lat, lon) in &scenic_points {
                p.legs.push(leg(
                    s,
                    name,
                    local(
                        s,
                        GeoPoint {
                            latitude_deg: *lat,
                            longitude_deg: *lon,
                        },
                        p.altitude_m,
                    )?,
                ));
            }
            let arrival = runway(s, airport, end)?;
            p.legs.extend(arrival_legs(s, &arrival, false));
            p.arrival = Some(arrival);
        }
    }
    range(CrewIntent::NAME, "altitude_m", p.altitude_m, 2., 10000.)?;
    if grounded {
        if p.departure.is_none() {
            p.departure = Some(nearest_runway(s)?);
        }
        if let Some(d) = &p.departure {
            let (along, cross) = d.relative(s.sim.state().position_m);
            if !(-10. ..d.length_m * 0.5).contains(&along)
                || cross.abs() > d.width_m * 0.4
                || flightcraft_sim::autopilot::wrap(r.heading_grid_rad - d.heading_grid_rad).abs() > 0.2
            {
                return Err(fail(
                    "line up on the selected departure runway before asking for takeoff",
                ));
            }
        }
    }
    if !matches!(intent, PilotIntent::Hold { .. }) {
        p.altitude_m = (p.altitude_m / 0.3048 / 100.).round() * 100. * 0.3048;
    }
    let mut previous = s.sim.state().position_m;
    for target in &mut p.legs {
        let [n, e, _] = previous;
        let [tn, te, td] = target.position_m;
        if !target.name.contains("downwind")
            && !target.name.contains("intercept")
            && !matches!(intent, PilotIntent::Hold { .. } | PilotIntent::GoAround { .. })
        {
            let course = s
                .sim
                .heading_reference()
                .grid_to_magnetic(libm::atan2(te - e, tn - n));
            let ground = (0..=20)
                .map(|i| {
                    let f = f64::from(i) / 20.;
                    s.sim
                        .world()
                        .ground(n + (tn - n) * f, e + (te - e) * f)
                        .height_msl_m
                })
                .fold(f64::INFINITY, f64::min);
            target.position_m[2] = -vfr_altitude(-td, ground, course);
            p.altitude_m = p.altitude_m.max(-target.position_m[2]);
        }
        previous = target.position_m;
    }
    if let Some(first) = p
        .legs
        .iter()
        .find(|l| !l.name.contains("downwind") && !l.name.contains("intercept"))
        && !matches!(intent, PilotIntent::Hold { .. } | PilotIntent::Orbit { .. })
    {
        p.altitude_m = -first.position_m[2];
    }
    if let (Some(departure), Some(first)) = (&p.departure, p.legs.first()) {
        let [n, e, d] = departure.point(3000., 0., 315.);
        let [tn, te, _] = first.position_m;
        let distance = libm::hypot(tn - n, te - e);
        let climb_distance = ((p.altitude_m + d) / 3.0 * vy).max(0.0);
        if climb_distance > 1000.0 && distance > climb_distance + 1000.0 {
            let fraction = climb_distance / distance;
            p.legs.insert(
                0,
                leg(
                    s,
                    "Climb to cruise altitude",
                    [n + (tn - n) * fraction, e + (te - e) * fraction, -p.altitude_m],
                ),
            );
        }
    }
    p.checks.push(
        "Crosswind envelope: 5.2 m/s (engineering assumption; aircraft has no demonstrated-limit field)"
            .into(),
    );
    // The aircraft format does not yet publish demonstrated crosswind; use a conservative
    // engineering envelope and explicitly report it instead of claiming certification data.
    for a in p.departure.iter().chain(p.arrival.iter()) {
        let [wn, we, _] = s.sim.weather().mean_wind_mps(a.threshold_m);
        let (sin, cos) = libm::sincos(a.heading_grid_rad);
        if (-wn * sin + we * cos).abs() > 5.2 {
            return Err(fail("crosswind exceeds the crew's ten-knot operating envelope"));
        }
    }
    validate_path(s, &mut p)?;
    let purpose = match intent {
        PilotIntent::FlyToAirport { airport, runway, .. } | PilotIntent::LandRunway { airport, runway } => {
            format!(
                "We'll fly to {} for runway {}. At a towered field, we'll follow the assigned runway and land only when cleared.",
                airport_name(s, airport),
                crate::comms::phraseology::runway(runway)
            )
        }
        PilotIntent::JoinPattern { airport, runway } => {
            format!(
                "We'll join the pattern at {} for runway {}.",
                airport_name(s, airport),
                crate::comms::phraseology::runway(runway)
            )
        }
        PilotIntent::TakeOff { runway, .. } => {
            format!(
                "We'll take off on runway {}, climb, and hold the departure heading.",
                crate::comms::phraseology::runway(runway)
            )
        }
        PilotIntent::FlyToCoordinate { .. } => {
            "We'll fly to your selected point, then hold our heading and altitude.".into()
        }
        PilotIntent::FlightPlan {} => "We'll follow the flight plan, then hold at its end.".into(),
        PilotIntent::Scenic { preset, .. } => {
            let name = match preset {
                ScenicPreset::DelawareWaterGap => "Delaware Water Gap",
                ScenicPreset::PoconoPlateau => "Pocono plateau",
                ScenicPreset::HudsonCorridor => "Hudson corridor",
            };
            format!("We'll follow the {name} scenic route and return for landing.")
        }
        PilotIntent::Hold { .. } => "I'll hold your selected altitude and heading.".into(),
        PilotIntent::Orbit { .. } => "We'll make a gentle orbit around your selected point.".into(),
        PilotIntent::Approach { .. } | PilotIntent::HoldAtFix { .. } => String::new(),
        PilotIntent::GoAround {} => "Going around. Full power, then a new pattern.".into(),
    };
    p.briefing = format!(
        "{purpose} Best-rate climb is {} knots. Cruise is {} feet. We'll use a forty-five-degree entry to left downwind, or a straight-in when already aligned. I'll keep the controls until we confirm an exchange.",
        crate::comms::phraseology::digits((vy / 0.514444).round() as u32, 1),
        crate::comms::phraseology::altitude((p.altitude_m / 0.3048).round() as u32)
    );
    Ok((p, phase))
}

fn airport_name(s: &Session, ident: &str) -> String {
    s.sim
        .world()
        .airports()
        .and_then(|d| {
            d.airports.iter().find(|a| {
                a.ident.eq_ignore_ascii_case(ident)
                    || a.icao.as_ref().is_some_and(|i| i.eq_ignore_ascii_case(ident))
            })
        })
        .map_or_else(
            || ident.to_owned(),
            |a| crate::comms::phraseology::normal_case(&a.name),
        )
}

/// 14 CFR 91.159: above 3,000 ft AGL, magnetic course 000–179 uses odd + 500;
/// 180–359 uses even + 500. Keep at least the requested altitude, including terrain margin.
pub(super) fn vfr_altitude(requested_m: f64, ground_m: f64, magnetic_course: f64) -> f64 {
    let feet = (requested_m / 0.3048 / 100.).round() * 100.;
    if (requested_m - ground_m) / 0.3048 <= 3000. + 1e-6 {
        return feet * 0.3048;
    }
    let base = if magnetic_course.rem_euclid(core::f64::consts::TAU) < core::f64::consts::PI {
        1500.
    } else {
        2500.
    };
    (base + ((feet - base) / 2000.).ceil().max(0.) * 2000.) * 0.3048
}

#[cfg(test)]
mod altitude_tests {
    use super::*;
    #[test]
    fn magnetic_hemispheres_and_three_thousand_agl_threshold() {
        let feet = |m| m / 0.3048;
        assert_eq!(
            feet(vfr_altitude(4100. * 0.3048, 0., 90_f64.to_radians())).round(),
            5500.
        );
        assert_eq!(
            feet(vfr_altitude(4100. * 0.3048, 0., 270_f64.to_radians())).round(),
            4500.
        );
        assert_eq!(
            feet(vfr_altitude(2801. * 0.3048, 0., 90_f64.to_radians())).round(),
            2800.
        );
        assert_eq!(
            feet(vfr_altitude(4100. * 0.3048, 2000. * 0.3048, 90_f64.to_radians())).round(),
            4100.
        );
    }
}
