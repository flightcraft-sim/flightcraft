//! Deterministic advisory glide geometry and aircraft-data checklist grading.
use super::*;
use flightcraft_aero::emergencies::EmergencyProcedure;
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct GlideField {
    pub id: String,
    pub position_m: [f64; 2],
    pub terrain_msl_m: f64,
    pub airport: bool,
    #[serde(default)]
    pub length_m: f64,
    #[serde(default)]
    pub width_m: f64,
    #[serde(default)]
    pub slope_percent: f64,
    #[serde(default)]
    pub heading_grid_rad: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct GlideFootprint {
    pub world: flightcraft_world::WorldChoice,
    pub origin_m: [f64; 3],
    pub boundary_m: Vec<[f64; 2]>,
    pub wind_ned_mps: [f64; 3],
    pub glide_ratio: f64,
    pub best_glide_mps: f64,
    pub fields: Vec<GlideField>,
    pub terrain_coverage_limited: bool,
}
/// 72 radial ground tracks, 100 m terrain steps, 30 km maximum. Wind correction
/// includes crosswind crab; unknown region coverage terminates a ray conservatively.
pub fn glide_footprint(
    world: &flightcraft_world::World,
    origin: [f64; 3],
    wind: [f64; 3],
    ratio: f64,
    speed: f64,
) -> Result<GlideFootprint, CommandError> {
    if origin
        .iter()
        .chain(&wind)
        .any(|v| !v.is_finite() || v.abs() > 1e7)
        || !ratio.is_finite()
        || !(2.0..=40.0).contains(&ratio)
        || !speed.is_finite()
        || !(10.0..=150.0).contains(&speed)
    {
        return Err(invalid("emergency.glide", "invalid geometry input"));
    }
    let [n, e, d] = origin;
    let [wn, we, wd] = wind;
    let mut result = GlideFootprint {
        world: world.choice(),
        origin_m: origin,
        boundary_m: Vec::new(),
        wind_ned_mps: wind,
        glide_ratio: ratio,
        best_glide_mps: speed,
        fields: Vec::new(),
        terrain_coverage_limited: false,
    };
    for ray in 0..72 {
        let angle = f64::from(ray) * core::f64::consts::TAU / 72.;
        let (dn, de) = (libm::cos(angle), libm::sin(angle));
        let along = wn * dn + we * de;
        let cross = we * dn - wn * de;
        let ground_speed = libm::sqrt((speed * speed - cross * cross).max(0.)) + along;
        let mut endpoint = [n, e];
        if ground_speed > 1. && cross.abs() < speed {
            for step in 1..=300 {
                let distance = f64::from(step) * 100.;
                let p = [n + dn * distance, e + de * distance];
                if world
                    .region()
                    .is_some_and(|r| flightcraft_world::inside(r, p).is_err())
                {
                    result.terrain_coverage_limited = true;
                    break;
                }
                let altitude = -d - distance * (speed / ratio + wd).max(0.1) / ground_speed;
                let [pn, pe] = p;
                if altitude < world.ground(pn, pe).height_msl_m + 15. {
                    break;
                }
                endpoint = p;
            }
        }
        result.boundary_m.push(endpoint);
    }
    if let (Some(region), Some(airports)) = (world.region(), world.airports()) {
        let projection = flightcraft_world::geo::LocalProjection::new(
            region.origin.latitude_deg,
            region.origin.longitude_deg,
        )
        .map_err(|e| invalid("emergency.glide", e.to_string()))?;
        for airport in &airports.airports {
            for (runway, end) in airport
                .runways
                .iter()
                .flat_map(|r| r.ends.iter().map(move |e| (r, e)))
            {
                let p = projection
                    .project(end.latitude_deg, end.longitude_deg)
                    .map_err(|e| invalid("emergency.glide", e.to_string()))?;
                if contains(&result.boundary_m, p) {
                    result.fields.push(GlideField {
                        id: format!(
                            "{}:{}",
                            airport.icao.as_deref().unwrap_or(&airport.ident),
                            end.ident
                        ),
                        position_m: p,
                        terrain_msl_m: end.elevation_m,
                        airport: true,
                        length_m: runway.length_m,
                        width_m: runway.width_m,
                        slope_percent: 0.,
                        heading_grid_rad: {
                            let next = flightcraft_world::GeoPoint {
                                latitude_deg: end.latitude_deg,
                                longitude_deg: end.longitude_deg,
                            }
                            .destination(end.heading_deg, 100.);
                            let [nn, ne] = projection
                                .project(next.latitude_deg, next.longitude_deg)
                                .map_err(|e| invalid("emergency.glide", e.to_string()))?;
                            {
                                let [pn, pe] = p;
                                libm::atan2(ne - pe, nn - pn)
                            }
                        },
                    });
                }
            }
        }
    }
    result
        .fields
        .extend(super::landing_sites::candidates(world, &result));
    Ok(result)
}
pub(super) fn contains(polygon: &[[f64; 2]], p: [f64; 2]) -> bool {
    let [x, y] = p;
    let mut inside = false;
    for (a, b) in polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
    {
        let [ax, ay] = *a;
        let [bx, by] = *b;
        if (ay > y) != (by > y) && x < (bx - ax) * (y - ay) / (by - ay) + ax {
            inside = !inside;
        }
    }
    inside
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EmergencyTiming {
    pub item: String,
    pub elapsed_s: f64,
    pub in_order: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EmergencyDebrief {
    pub failure_time_s: f64,
    #[serde(default)]
    pub failure_agl_m: f64,
    #[serde(default)]
    pub failure_location: String,
    #[serde(default)]
    pub track_m: Vec<[f64; 3]>,
    #[serde(default)]
    pub touchdown_m: Option<[f64; 3]>,
    pub source: EventSource,
    pub procedure: EmergencyProcedure,
    pub glide: GlideFootprint,
    pub recognition_s: Option<f64>,
    pub steps: Vec<EmergencyTiming>,
    pub chosen_field: Option<String>,
    pub field_reachable: bool,
    pub speed_samples: u64,
    pub speed_in_tolerance: u64,
    pub emergency_call: bool,
    pub frequency_at_failure_khz: u32,
    pub landed_at_chosen_field: bool,
    pub squawk_7700: bool,
    pub outcome: String,
    pub has_been_airborne: bool,
    pub last_sample_time_s: f64,
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EmergencyObservation {
    Recognized,
    Checklist { item: String },
    ChooseField { id: String },
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EmergencyRecord {
    pub observation: EmergencyObservation,
}
impl Command for EmergencyRecord {
    const NAME: &'static str = "emergency.record";
    const DESCRIPTION: &'static str = "Record recognition, aircraft checklist completion, or a field decision at the current simulation time.";
    type Output = ScenarioState;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        let d = s
            .scenario
            .state
            .emergency
            .as_mut()
            .ok_or_else(|| invalid(Self::NAME, "no active emergency"))?;
        let elapsed = s.sim.state().time_s - d.failure_time_s;
        match self.observation {
            EmergencyObservation::Recognized => {
                if d.recognition_s.is_none() {
                    d.recognition_s = Some(elapsed);
                }
            }
            EmergencyObservation::Checklist { item } => {
                if !d.procedure.items.iter().any(|i| i.id == item) {
                    return Err(invalid(Self::NAME, "item not in aircraft emergency procedure"));
                }
                if d.steps.iter().any(|i| i.item == item) {
                    return Err(invalid(Self::NAME, "item already completed"));
                }
                let expected = d
                    .procedure
                    .items
                    .iter()
                    .find(|i| !d.steps.iter().any(|done| done.item == i.id));
                let in_order = expected.is_some_and(|i| i.id == item);
                d.steps.push(EmergencyTiming {
                    item: item.clone(),
                    elapsed_s: elapsed,
                    in_order,
                });
                s.scenario.checklist.insert(item);
            }
            EmergencyObservation::ChooseField { id } => {
                if !text_ok(&id) {
                    return Err(invalid(Self::NAME, "invalid field id"));
                }
                d.field_reachable = d.glide.fields.iter().any(|f| f.id == id);
                d.chosen_field = Some(id);
            }
        }
        s.sequence_scenario();
        Ok(s.scenario.state.clone())
    }
}
impl Session {
    pub fn scenario_state(&self) -> &ScenarioState {
        &self.scenario.state
    }
    pub(crate) fn begin_emergency(&mut self, source: EventSource) {
        if self.scenario.state.emergency.is_some() {
            return;
        }
        let f = self.sim.failures();
        let id = if f.engine_fire {
            "engine_fire"
        } else if f.electrical_fire {
            "electrical_fire"
        } else if f.cabin_smoke {
            "cabin_smoke"
        } else if f.door_open {
            "door_open"
        } else if f.carb_icing {
            "carb_icing"
        } else if f.airframe_icing {
            "airframe_icing"
        } else if f.flap_asymmetry {
            "flap_asymmetry"
        } else if f.trim_runaway {
            "trim_runaway"
        } else if f.trim_stuck {
            "trim_stuck"
        } else if f.radio_failure {
            "radio_failure"
        } else if f.vacuum_pump {
            "vacuum_pump"
        } else if f.engine_power_fraction < 1. {
            "engine_failure"
        } else {
            return;
        };
        let Some(procedure) = self
            .aircraft_configuration()
            .and_then(|c| c.installed_systems.emergencies.iter().find(|p| p.id == id))
            .cloned()
        else {
            return;
        };
        let r = self.sim.readout();
        let Ok(glide) = glide_footprint(
            self.sim.world(),
            self.sim.state().position_m,
            r.wind_ned_mps,
            procedure.glide_ratio,
            procedure.best_glide_mps,
        ) else {
            return;
        };
        self.scenario.state.emergency = Some(EmergencyDebrief {
            failure_time_s: r.time_s,
            failure_agl_m: r.height_agl_m,
            failure_location: failure_location(self.sim.world(), glide.origin_m),
            track_m: vec![glide.origin_m],
            touchdown_m: None,
            source,
            procedure,
            glide,
            recognition_s: None,
            steps: Vec::new(),
            chosen_field: None,
            field_reachable: false,
            speed_samples: 0,
            speed_in_tolerance: 0,
            emergency_call: false,
            frequency_at_failure_khz: self.selected_com_frequency(),
            landed_at_chosen_field: false,
            squawk_7700: false,
            outcome: "in_progress".into(),
            has_been_airborne: !r.on_ground,
            last_sample_time_s: r.time_s,
        });
    }
    pub(crate) fn update_emergency(&mut self) {
        if self.scenario.state.emergency.is_none() {
            return;
        }
        let r = self.sim.readout();
        let Some(active) = &self.scenario.state.emergency else {
            return;
        };
        let emergency_call = self
            .flight_record()
            .events
            .iter()
            .filter(|e| {
                e.time_s >= active.failure_time_s
                    && e.speaker == Speaker::Pilot
                    && e.channel == Channel::Com
                    && e.time_s + e.duration_s <= r.time_s
                    && !matches!(e.reception, Reception::Blocked | Reception::NotReceived)
            })
            .filter(|e| {
                e.frequency_hz.is_some_and(|hz| {
                    (hz - 121500000.).abs() < 1.
                        || flightcraft_world::comms::carrier_thirds_hz(active.frequency_at_failure_khz)
                            .is_some_and(|carrier| (hz - f64::from(carrier) / 3.).abs() < 1.)
                })
            })
            .any(|e| contains_phrase(&e.text, "mayday") || contains_phrase(&e.text, "pan pan"));
        // The existing transponder is part of avionics; read its actual selected code.
        let squawk = self.glass.transponder_code == "7700"
            && matches!(
                self.glass.transponder_mode,
                crate::TransponderMode::On | crate::TransponderMode::Alt
            );
        if let Some(d) = &mut self.scenario.state.emergency {
            let position = self.sim.state().position_m;
            let [pn, pe, _] = position;
            if d.outcome == "in_progress"
                && d.track_m.len() < 4096
                && d.track_m
                    .last()
                    .is_none_or(|[n, e, _]| libm::hypot(pn - n, pe - e) >= 25. || r.on_ground || r.crashed)
            {
                d.track_m.push(position);
            }
            if d.outcome == "in_progress" && (r.on_ground || r.crashed) && d.has_been_airborne {
                d.touchdown_m = Some(position);
            }
            d.has_been_airborne |= !r.on_ground;
            if r.time_s > d.last_sample_time_s && !r.on_ground && d.outcome == "in_progress" {
                d.speed_samples += 1;
                if (r.indicated_airspeed_mps - d.procedure.best_glide_mps).abs()
                    <= d.procedure.speed_tolerance_mps
                {
                    d.speed_in_tolerance += 1;
                }
            }
            d.last_sample_time_s = r.time_s;
            d.emergency_call |= emergency_call;
            d.squawk_7700 |= squawk;
            if r.crashed {
                d.outcome = "crashed".into();
            } else if r.on_ground
                && d.has_been_airborne
                && r.time_s > d.failure_time_s + 1.
                && d.outcome == "in_progress"
            {
                d.outcome = "landed".into();
                let [n, e, _] = self.sim.state().position_m;
                d.landed_at_chosen_field = d
                    .chosen_field
                    .as_ref()
                    .and_then(|id| d.glide.fields.iter().find(|f| f.id == *id))
                    .is_some_and(|f| {
                        let [fnorth, feast] = f.position_m;
                        libm::hypot(n - fnorth, e - feast) <= 500.
                    });
            }
        }
    }
}

impl EmergencyDebrief {
    pub fn validate(&self) -> Result<(), CommandError> {
        let finite = |v: f64| v.is_finite() && (0.0..=86400.0).contains(&v);
        self.procedure
            .validate()
            .map_err(|e| invalid("flight.events", e.to_string()))?;
        let mut items = std::collections::BTreeSet::new();
        if !finite(self.last_sample_time_s)
            || self.last_sample_time_s < self.failure_time_s
            || !finite(self.failure_time_s)
            || self.recognition_s.is_some_and(|v| !finite(v))
            || self.steps.len() > 64
            || self.steps.iter().any(|t| {
                !finite(t.elapsed_s)
                    || !items.insert(&t.item)
                    || !self.procedure.items.iter().any(|i| i.id == t.item)
            })
            || !self.failure_agl_m.is_finite()
            || self.failure_agl_m.abs() > 1e5
            || self.failure_location.len() > 256
            || self.track_m.len() > 4096
            || self
                .track_m
                .iter()
                .flatten()
                .chain(self.touchdown_m.iter().flatten())
                .any(|v| !v.is_finite() || v.abs() > 1e7)
            || self.glide.boundary_m.len() != 72
            || self.glide.fields.len() > 16384
            || self
                .glide
                .origin_m
                .iter()
                .chain(&self.glide.wind_ned_mps)
                .chain(self.glide.boundary_m.iter().flatten())
                .any(|v| !v.is_finite() || v.abs() > 1e7)
            || self.glide.fields.iter().any(|f| {
                !text_ok(&f.id)
                    || !f.terrain_msl_m.is_finite()
                    || [f.length_m, f.width_m, f.slope_percent, f.heading_grid_rad]
                        .iter()
                        .any(|v| !v.is_finite() || v.abs() > 1e5)
                    || f.position_m.iter().any(|v| !v.is_finite() || v.abs() > 1e7)
            })
            || !self.glide.glide_ratio.is_finite()
            || !(2.0..=40.0).contains(&self.glide.glide_ratio)
            || !self.glide.best_glide_mps.is_finite()
            || !(10.0..=150.0).contains(&self.glide.best_glide_mps)
            || self.speed_in_tolerance > self.speed_samples
            || !matches!(self.outcome.as_str(), "in_progress" | "landed" | "crashed")
            || self.chosen_field.as_ref().is_some_and(|id| !text_ok(id))
        {
            return Err(invalid("flight.events", "invalid emergency debrief"));
        }
        Ok(())
    }
}

pub fn compass(bearing_deg: f64) -> &'static str {
    match ((bearing_deg.rem_euclid(360.) + 22.5) / 45.) as u32 % 8 {
        0 => "N",
        1 => "NE",
        2 => "E",
        3 => "SE",
        4 => "S",
        5 => "SW",
        6 => "W",
        _ => "NW",
    }
}
fn failure_location(world: &flightcraft_world::World, [n, e, _]: [f64; 3]) -> String {
    let Some([latitude_deg, longitude_deg]) = world.geodetic(n, e) else {
        return String::new();
    };
    let failure = flightcraft_world::GeoPoint {
        latitude_deg,
        longitude_deg,
    };
    world
        .airports()
        .into_iter()
        .flat_map(|a| &a.airports)
        .map(|a| {
            let (distance, bearing) = flightcraft_world::GeoPoint {
                latitude_deg: a.latitude_deg,
                longitude_deg: a.longitude_deg,
            }
            .inverse(failure);
            (distance, bearing, a.icao.as_deref().unwrap_or(&a.ident))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map_or(String::new(), |(d, b, id)| {
            format!("{:.1} nm {} of {id}", d / 1852., compass(b))
        })
}
fn grouped(value: f64) -> String {
    let digits = format!("{:.0}", value.max(0.));
    let len = digits.len();
    digits
        .chars()
        .enumerate()
        .flat_map(|(i, c)| {
            if i > 0 && (len - i).is_multiple_of(3) {
                vec![',', c]
            } else {
                vec![c]
            }
        })
        .collect()
}
impl GlideField {
    pub fn description(&self, origin: [f64; 3]) -> String {
        if self.airport {
            return format!(
                "{} · {} ft runway",
                self.id.replace(':', " runway "),
                grouped(self.length_m / 0.3048)
            );
        }
        let [n, e] = self.position_m;
        let [on, oe, _] = origin;
        format!(
            "open field {:.1} nm {}, {} ft, slope {:.1}% · unverified surface",
            libm::hypot(n - on, e - oe) / 1852.,
            compass(libm::atan2(e - oe, n - on).to_degrees()),
            grouped(self.length_m / 0.3048),
            self.slope_percent
        )
    }
}
impl EmergencyDebrief {
    pub fn summary(&self) -> String {
        let title = if self.procedure.id == "engine_failure" {
            "Engine failure"
        } else {
            &self.procedure.title
        };
        let recognition = self
            .recognition_s
            .map_or("not yet recognized".into(), |s| format!("recognized in {s:.1} s"));
        let speed = if self.speed_samples == 0 {
            "best glide not yet sampled".into()
        } else {
            format!(
                "best glide {:.0} kt held {:.0}% of the time (target ±{:.0} kt)",
                self.procedure.best_glide_mps / 0.514444,
                100. * self.speed_in_tolerance as f64 / self.speed_samples as f64,
                self.procedure.speed_tolerance_mps / 0.514444
            )
        };
        let outcome = match (
            self.outcome.as_str(),
            self.touchdown_m,
            self.chosen_field
                .as_ref()
                .and_then(|id| self.glide.fields.iter().find(|f| f.id == *id)),
        ) {
            ("landed" | "crashed", Some([n, e, _]), Some(field)) => {
                let [fnorth, feast] = field.position_m;
                let dn = n - fnorth;
                let de = e - feast;
                let along = dn * libm::cos(field.heading_grid_rad) + de * libm::sin(field.heading_grid_rad);
                let cross = de * libm::cos(field.heading_grid_rad) - dn * libm::sin(field.heading_grid_rad);
                let verb = if self.outcome == "crashed" {
                    "impact"
                } else {
                    "landed"
                };
                let target = field.id.replace(':', " runway ");
                if field.airport && cross.abs() < 250. && along < 0. {
                    format!("{verb} {:.1} nm short of {target}", -along / 1852.)
                } else if field.airport && cross.abs() <= field.width_m / 2. && along <= field.length_m {
                    format!("{verb} on {target}")
                } else {
                    format!(
                        "{verb} {:.1} nm {} of {target}",
                        libm::hypot(dn, de) / 1852.,
                        compass(libm::atan2(de, dn).to_degrees())
                    )
                }
            }
            ("landed", _, _) if self.chosen_field.is_some() => {
                "landed; selected field has no recorded geometry".into()
            }
            ("crashed", _, _) if self.chosen_field.is_some() => {
                "impact; selected field has no recorded geometry".into()
            }
            ("landed", _, _) => "landed; no field selected".into(),
            ("crashed", _, _) => "impact; no field selected".into(),
            _ => "flight in progress".into(),
        };
        format!(
            "{title} at {} ft AGL{} · {recognition} · {speed} · {} · {} · {outcome}",
            grouped(self.failure_agl_m / 0.3048),
            if self.failure_location.is_empty() {
                String::new()
            } else {
                format!(", {}", self.failure_location)
            },
            if self.emergency_call {
                "emergency call made"
            } else {
                "no Mayday call"
            },
            if self.squawk_7700 {
                "7700 selected"
            } else {
                "no 7700"
            }
        )
    }
}
