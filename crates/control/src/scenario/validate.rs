use super::*;
fn faa_stations(s: &Session) -> Vec<flightcraft_world::ComStation> {
    s.sim.world().navdata().map_or_else(Vec::new, |n| n.comms.clone())
}
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ScenarioValidation {
    pub valid: bool,
    pub errors: Vec<String>,
}

pub(super) fn reference(s: &Session, id: &str) -> Option<GeoPoint> {
    if let Ok(a) = crate::regions::airport(s.sim.world(), id) {
        return Some(GeoPoint {
            latitude_deg: a.latitude_deg,
            longitude_deg: a.longitude_deg,
        });
    }
    let n = s.sim.world().navdata()?;
    n.fixes
        .iter()
        .find(|f| f.ident == id)
        .map(|f| f.position)
        .or_else(|| n.navaids.iter().find(|f| f.ident == id).map(|f| f.position))
}
pub(super) fn runway<'a>(
    s: &'a Session,
    airport: &str,
    end: &str,
) -> Option<&'a flightcraft_world::RunwayEnd> {
    crate::regions::airport(s.sim.world(), airport)
        .ok()?
        .runways
        .iter()
        .flat_map(|r| &r.ends)
        .find(|e| e.ident == end)
}
pub(super) fn has_airspace(s: &Session, id: &str) -> bool {
    s.sim
        .world()
        .airspace()
        .is_some_and(|a| a.volumes.iter().any(|v| v.info.id == id))
}
pub fn validate(doc: &Scenario, s: &Session) -> ScenarioValidation {
    let mut errors = Vec::new();
    if doc.version != 1
        || !text_ok(&doc.id)
        || !text_ok(&doc.title)
        || doc.steps.is_empty()
        || doc.steps.len() > 512
        || doc.route.len() < 2
        || doc.route.len() > 256
        || doc.procedures.len() > 64
    {
        errors.push("document: version 1, 2–256 route points, and 1–512 steps required".into());
        return ScenarioValidation { valid: false, errors };
    }
    let positions: Vec<_> = doc
        .route
        .iter()
        .filter_map(|p| reference(s, &p.reference))
        .collect();
    for p in &doc.route {
        if reference(s, &p.reference).is_none() {
            errors.push(format!("route: unknown airport/fix {}", p.reference));
        }
        if !p.altitude_msl_m.is_finite() || !(-500.0..=15000.0).contains(&p.altitude_msl_m) {
            errors.push("route: invalid altitude".into());
        }
        if p.clearances.len() > 64 {
            errors.push("route: at most 64 clearances per point".into());
        }
        for id in p.clearances.iter().take(64) {
            if !has_airspace(s, id) {
                errors.push(format!("route: unknown airspace clearance {id}"));
            }
        }
    }
    for pair in doc.route.windows(2) {
        if let [a, b] = pair
            && let (Some(start), Some(end)) = (reference(s, &a.reference), reference(s, &b.reference))
        {
            let (distance, bearing) = start.inverse(end);
            let count = (libm::ceil(distance / 250.0) as usize).clamp(1, 4000);
            for i in 0..=count {
                let t = i as f64 / count as f64;
                let p = start.destination(bearing, distance * t);
                let Some([n, e]) = s.radio_project(p.latitude_deg, p.longitude_deg) else {
                    errors.push("route: region projection unavailable".into());
                    break;
                };
                let height = a.altitude_msl_m + (b.altitude_msl_m - a.altitude_msl_m) * t;
                let world = s.sim.world();
                if world
                    .region()
                    .is_none_or(|r| flightcraft_world::inside(r, [n, e]).is_err())
                {
                    errors.push("route: terrain coverage missing".into());
                    break;
                }
                if height < world.ground(n, e).height_msl_m + 304.8 {
                    errors.push("route: below 1,000 ft terrain clearance".into());
                    break;
                }
                if let Ok(q) = world.airspace_at(n, e, height) {
                    for v in q.containing {
                        if matches!(
                            v.class,
                            Some(
                                flightcraft_world::AirspaceClass::B
                                    | flightcraft_world::AirspaceClass::C
                                    | flightcraft_world::AirspaceClass::D
                            )
                        ) && !a.clearances.contains(&v.id)
                        {
                            errors.push(format!("route: clearance required for {}", v.id));
                        }
                    }
                }
            }
        }
    }
    for id in &doc.procedures {
        if !s.sim.world().procedures().is_some_and(|p| {
            p.procedures
                .iter()
                .any(|p| format!("{}:{}", p.airport, p.ident) == *id)
        }) {
            errors.push(format!("unknown FAA procedure {id}"));
        }
    }
    let planned_clearances = doc
        .route
        .iter()
        .flat_map(|p| p.clearances.iter().cloned())
        .collect();
    let mut ids = std::collections::BTreeSet::new();
    for (index, step) in doc.steps.iter().enumerate() {
        if !text_ok(&step.id)
            || !ids.insert(&step.id)
            || step.when.len() > 32
            || step.actions.is_empty()
            || step.actions.len() > 32
        {
            errors.push(format!("step {index}: invalid id, conditions, or actions"));
        }
        for t in &step.when {
            validate_trigger(t, s, &positions, &mut errors);
        }
        for action in &step.actions {
            match action {
                ScenarioAction::Transmission {
                    facility,
                    channel_khz,
                    role,
                    instruction,
                } => errors.extend(validate_transmission(
                    s,
                    facility,
                    *channel_khz,
                    *role,
                    instruction,
                    &planned_clearances,
                )),
                ScenarioAction::Weather { conditions } => {
                    if let Err(e) = conditions.validate() {
                        errors.push(e.to_string());
                    }
                }
                ScenarioAction::FailureSet { failure } => match failure {
                    Failure::CarbIcing {}
                        if s.aircraft_configuration().is_some_and(|c| {
                            c.engine.induction != flightcraft_aero::Induction::Carburettor
                        }) =>
                    {
                        errors.push("carb icing requires a carburetted engine".into())
                    }
                    Failure::VacuumPump {} if !s.sim.systems().vacuum_installed => {
                        errors.push("vacuum failure requires installed vacuum instruments".into())
                    }
                    Failure::PartialPower { fraction }
                        if !fraction.is_finite() || !(0.0..=1.0).contains(fraction) =>
                    {
                        errors.push("invalid power fraction".into())
                    }
                    Failure::Jam { surface, position } => {
                        let mut f = flightcraft_sim::Failures::default();
                        if f.jam(*surface, *position).is_err() {
                            errors.push("invalid surface jam".into());
                        }
                    }
                    _ => (),
                },
                ScenarioAction::TrafficSpawn {
                    airport,
                    runway: end,
                    count,
                } => {
                    if runway(s, airport, end).is_none() || *count > 3 {
                        errors.push("invalid traffic runway/count".into());
                    }
                    let stations = faa_stations(s);
                    if !stations
                        .iter()
                        .any(|st| st.airport == *airport && st.kind == FacilityKind::Ctaf)
                        || stations
                            .iter()
                            .any(|st| st.airport == *airport && st.kind == FacilityKind::Tower)
                    {
                        errors.push("traffic requires an untowered FAA CTAF".into());
                    }
                }
                ScenarioAction::Cue { text } | ScenarioAction::EmitEvent { name: text } => {
                    if !text_ok(text) {
                        errors.push("invalid cue/event text".into());
                    }
                }
                ScenarioAction::Wait { condition } => validate_trigger(condition, s, &positions, &mut errors),
                ScenarioAction::Branch {
                    condition,
                    then_step,
                    else_step,
                } => {
                    validate_trigger(condition, s, &positions, &mut errors);
                    for target in [then_step, else_step] {
                        if !doc
                            .steps
                            .iter()
                            .enumerate()
                            .any(|(i, st)| i > index && st.id == *target)
                        {
                            errors.push(format!("branch target {target} must exist after current step"));
                        }
                    }
                }
                _ => (),
            }
        }
    }
    errors.sort();
    errors.dedup();
    ScenarioValidation {
        valid: errors.is_empty(),
        errors,
    }
}
fn validate_trigger(t: &Trigger, s: &Session, route: &[GeoPoint], errors: &mut Vec<String>) {
    let bounds =
        |a: f64, b: f64, lo: f64, hi: f64| a.is_finite() && b.is_finite() && a >= lo && a <= b && b <= hi;
    let valid = match t {
        Trigger::Distance {
            reference: id,
            within_nm,
        } => reference(s, id).is_some_and(|p| {
            *within_nm > 0.0 && *within_nm <= 200.0 && reachable(route, p, *within_nm * 1852.0)
        }),
        Trigger::DepartingRunway {
            airport,
            runway: end,
            within_nm,
        }
        | Trigger::OnFinal {
            airport,
            runway: end,
            within_nm,
        } => runway(s, airport, end).is_some_and(|r| {
            *within_nm > 0.0
                && *within_nm <= 30.0
                && reachable(
                    route,
                    GeoPoint {
                        latitude_deg: r.latitude_deg,
                        longitude_deg: r.longitude_deg,
                    },
                    *within_nm * 1852.0,
                )
        }),
        Trigger::CrossingRadial {
            reference: id,
            radial_deg,
        } => reference(s, id).is_some_and(|origin| {
            bounds(*radial_deg, *radial_deg, 0.0, 360.0)
                && route.windows(2).any(|pair| {
                    if let [a, b] = pair {
                        let signed = |p| {
                            let v = (origin.inverse(p).1 - *radial_deg).to_radians();
                            (v + core::f64::consts::PI).rem_euclid(core::f64::consts::TAU)
                                - core::f64::consts::PI
                        };
                        let old = signed(*a);
                        let new = signed(*b);
                        old * new <= 0. && (old - new).abs() < core::f64::consts::PI
                    } else {
                        false
                    }
                })
        }),
        Trigger::EnteringAirspace { id } => s
            .sim
            .world()
            .airspace()
            .and_then(|a| a.volumes.iter().find(|v| v.info.id == *id))
            .is_some_and(|volume| {
                route.windows(2).any(|pair| {
                    if let [a, b] = pair {
                        let (distance, bearing) = a.inverse(*b);
                        let samples = (libm::ceil(distance / 250.) as usize).clamp(1, 4000);
                        (0..=samples).any(|i| {
                            let p = a.destination(bearing, distance * i as f64 / samples as f64);
                            s.radio_project(p.latitude_deg, p.longitude_deg)
                                .is_some_and(|p| volume.contains_lateral(p))
                        })
                    } else {
                        false
                    }
                })
            }),
        Trigger::HeightAgl { min_m, max_m } => bounds(*min_m, *max_m, 0.0, 15000.0),
        Trigger::Airspeed { min_mps, max_mps } => bounds(*min_mps, *max_mps, 0.0, 300.0),
        Trigger::Configuration { min_flaps, max_flaps } => bounds(*min_flaps, *max_flaps, 0.0, 1.0),
        Trigger::Time { seconds, .. } => bounds(*seconds, *seconds, 0.0, 86400.0),
        Trigger::RandomWindow { earliest_s, latest_s } => bounds(*earliest_s, *latest_s, 0.0, 86400.0),
        Trigger::TunedFrequency { channel_khz } => {
            faa_stations(s).iter().any(|st| st.channel_khz == *channel_khz) || *channel_khz == 121500
        }
        Trigger::PilotCall { contains }
        | Trigger::Checklist { item: contains }
        | Trigger::Event { name: contains } => text_ok(contains),
        Trigger::Readback {
            contains, timeout_s, ..
        } => text_ok(contains) && bounds(*timeout_s, *timeout_s, 0.0, 600.0),
        Trigger::Phase { .. } => true,
    };
    if !valid {
        errors.push(format!("unreachable or invalid trigger: {t:?}"));
    }
}
fn reachable(route: &[GeoPoint], p: GeoPoint, tolerance: f64) -> bool {
    route.windows(2).any(|pair| {
        if let [a, b] = pair {
            let (d, h) = a.inverse(*b);
            let n = (libm::ceil(d / 500.) as usize).clamp(1, 2000);
            (0..=n).any(|i| a.destination(h, d * i as f64 / n as f64).inverse(p).0 <= tolerance + 250.)
        } else {
            false
        }
    })
}
pub(super) fn validate_transmission(
    s: &Session,
    facility: &str,
    khz: u32,
    role: FacilityKind,
    i: &Instruction,
    clearances: &std::collections::BTreeSet<String>,
) -> Vec<String> {
    let mut errors = Vec::new();
    let station = faa_stations(s)
        .into_iter()
        .find(|st| st.id == facility && st.kind == role && st.channel_khz == khz);
    if station.is_none() {
        errors.push("facility, role, and frequency must match an FAA station".into());
    }
    match i {
        Instruction::Controller { instruction } => {
            errors.extend(s.validate_controller(facility, khz, role, instruction));
        }
        Instruction::Land { airport, runway: end } | Instruction::GoAround { airport, runway: end } => {
            if runway(s, airport, end).is_none()
                || role != FacilityKind::Tower
                || station.as_ref().is_none_or(|st| st.airport != *airport)
            {
                errors.push("landing/go-around requires matching FAA tower and runway".into());
            }
        }
        Instruction::Altitude { altitude_msl_m, at } => {
            if !altitude_msl_m.is_finite()
                || !(0.0..=15000.0).contains(altitude_msl_m)
                || reference(s, at)
                    .and_then(|p| s.radio_project(p.latitude_deg, p.longitude_deg))
                    .is_none_or(|[n, e]| *altitude_msl_m < s.sim.world().ground(n, e).height_msl_m + 304.8)
            {
                errors.push("altitude instruction below terrain clearance or unknown fix".into());
            }
            if let Some([n, e]) =
                reference(s, at).and_then(|p| s.radio_project(p.latitude_deg, p.longitude_deg))
                && let Ok(query) = s.sim.world().airspace_at(n, e, *altitude_msl_m)
            {
                for volume in query.containing {
                    if matches!(
                        volume.class,
                        Some(
                            flightcraft_world::AirspaceClass::B
                                | flightcraft_world::AirspaceClass::C
                                | flightcraft_world::AirspaceClass::D
                        )
                    ) && !clearances.contains(&volume.id)
                    {
                        errors.push(format!(
                            "altitude instruction requires clearance for {}",
                            volume.id
                        ));
                    }
                }
            }
        }
        Instruction::AirspaceClearance { id } => {
            if !has_airspace(s, id)
                || !matches!(
                    role,
                    FacilityKind::Tower
                        | FacilityKind::Approach
                        | FacilityKind::Departure
                        | FacilityKind::Clearance
                )
            {
                errors.push("unknown airspace or invalid clearance facility role".into());
            }
        }
        Instruction::Advisory { text } | Instruction::RequestReadback { text } => {
            if !text_ok(text) {
                errors.push("invalid transmission text".into());
            }
            if matches!(
                i,
                Instruction::Advisory { .. } | Instruction::RequestReadback { .. }
            ) {
                let words = text.to_lowercase();
                if words.split(|c: char| !c.is_alphanumeric()).any(|w| {
                    matches!(
                        w,
                        "cleared"
                            | "contact"
                            | "frequency"
                            | "squawk"
                            | "descend"
                            | "climb"
                            | "maintain"
                            | "turn"
                            | "enter"
                            | "taxi"
                            | "hold"
                            | "land"
                            | "fly"
                    )
                }) {
                    errors
                        .push("free-form text cannot carry control instructions; use a typed meaning".into());
                }
            }
        }
    }
    errors
}
