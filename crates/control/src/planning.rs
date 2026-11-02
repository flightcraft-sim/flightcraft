//! Bounded flight plans, wind triangles, terrain/airspace profiles, and portable XML interchange.
use crate::{
    Command, CommandError, GpsWaypoint, Session, WaypointKind,
    navigation::{failed, valid_point},
};
use flightcraft_world::{GeoPoint, geomag::MagneticModel, navdata::text_ok};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
const MAX_POINTS: usize = 64;
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanWaypoint {
    pub waypoint: GpsWaypoint,
    #[schemars(range(min=-500,max=20000))]
    pub altitude_m: f64,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CruisePerformance {
    #[schemars(range(min = 10, max = 300))]
    pub airspeed_mps: f64,
    #[schemars(range(min = 0, max = 1))]
    pub fuel_flow_kg_s: f64,
}
impl CruisePerformance {
    fn validate(self) -> Result<(), CommandError> {
        if !self.airspeed_mps.is_finite()
            || !(10.0..=300.0).contains(&self.airspeed_mps)
            || !self.fuel_flow_kg_s.is_finite()
            || !(0.0..=1.0).contains(&self.fuel_flow_kg_s)
        {
            return Err(failed("invalid cruise performance"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlightPlan {
    #[schemars(range(min = 1, max = 1))]
    pub version: u8,
    #[schemars(length(min = 1, max = 256))]
    pub name: String,
    #[schemars(length(max = 64))]
    pub waypoints: Vec<PlanWaypoint>,
    #[schemars(range(min = 0, max = 86400))]
    pub reserve_s: f64,
    pub cruise: Option<CruisePerformance>,
}
impl Default for FlightPlan {
    fn default() -> Self {
        Self {
            version: 1,
            name: "Flight plan".into(),
            waypoints: Vec::new(),
            reserve_s: 2700.0,
            cruise: None,
        }
    }
}
impl FlightPlan {
    pub fn validate(&self) -> Result<(), CommandError> {
        if self.version != 1
            || !text_ok(&self.name, 256)
            || self.waypoints.len() > MAX_POINTS
            || !self.reserve_s.is_finite()
            || !(0.0..=86400.0).contains(&self.reserve_s)
        {
            return Err(failed("invalid plan metadata/count"));
        }
        if let Some(c) = self.cruise {
            c.validate()?;
        }
        for w in &self.waypoints {
            valid_point(w.waypoint.position)?;
            if !text_ok(&w.waypoint.ident, 64)
                || !w.altitude_m.is_finite()
                || !(-500.0..=20000.0).contains(&w.altitude_m)
            {
                return Err(failed("invalid plan waypoint"));
            }
        }
        Ok(())
    }
    pub fn from_json(text: &str) -> Result<Self, CommandError> {
        if text.len() > 1024 * 1024 {
            return Err(failed("plan exceeds 1 MiB"));
        }
        let p: Self = serde_json::from_str(text).map_err(|e| failed(e.to_string()))?;
        p.validate()?;
        Ok(p)
    }
}
macro_rules! edit {
    ($ty:ident,$name:literal,$desc:literal,$body:expr) => {
        impl Command for $ty {
            const NAME: &'static str = $name;
            const DESCRIPTION: &'static str = $desc;
            type Output = FlightPlan;
            fn run(self, s: &mut Session) -> Result<FlightPlan, CommandError> {
                let mut p = s.navigation.plan.clone();
                let result: Result<(), CommandError> = ($body)(self, &mut p);
                result?;
                p.validate()?;
                s.navigation.plan = p.clone();
                Ok(p)
            }
        }
    };
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanNew {
    pub name: String,
    #[serde(default)]
    pub cruise: Option<CruisePerformance>,
    #[serde(default = "reserve")]
    pub reserve_s: f64,
}
fn reserve() -> f64 {
    2700.0
}
edit!(
    PlanNew,
    "plan.new",
    "Create an empty plan with cruise performance and a default 45-minute reserve.",
    |a: PlanNew, p: &mut FlightPlan| {
        *p = FlightPlan {
            name: a.name,
            cruise: a.cruise,
            reserve_s: a.reserve_s,
            ..Default::default()
        };
        Ok::<_, CommandError>(())
    }
);
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanAdd {
    pub point: PlanWaypoint,
}
edit!(
    PlanAdd,
    "plan.add",
    "Append a resolved waypoint to the plan.",
    |a: PlanAdd, p: &mut FlightPlan| {
        p.waypoints.push(a.point);
        Ok::<_, CommandError>(())
    }
);
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanInsert {
    pub index: usize,
    pub point: PlanWaypoint,
}
edit!(
    PlanInsert,
    "plan.insert",
    "Insert a waypoint at a zero-based position.",
    |a: PlanInsert, p: &mut FlightPlan| {
        if a.index > p.waypoints.len() {
            return Err(failed("waypoint index out of range"));
        }
        p.waypoints.insert(a.index, a.point);
        Ok(())
    }
);
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanMove {
    pub from: usize,
    pub to: usize,
}
edit!(
    PlanMove,
    "plan.move",
    "Reorder a waypoint; to is its final zero-based position.",
    |a: PlanMove, p: &mut FlightPlan| {
        if a.from >= p.waypoints.len() || a.to >= p.waypoints.len() {
            return Err(failed("waypoint index out of range"));
        }
        let w = p.waypoints.remove(a.from);
        p.waypoints.insert(a.to, w);
        Ok(())
    }
);
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanRemove {
    pub index: usize,
}
edit!(
    PlanRemove,
    "plan.remove",
    "Remove a waypoint at a zero-based index.",
    |a: PlanRemove, p: &mut FlightPlan| {
        if a.index >= p.waypoints.len() {
            return Err(failed("waypoint index out of range"));
        }
        p.waypoints.remove(a.index);
        Ok(())
    }
);
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanSetAltitude {
    pub index: usize,
    pub altitude_m: f64,
}
edit!(
    PlanSetAltitude,
    "plan.set_altitude",
    "Set a waypoint's planned MSL altitude; profiles interpolate between endpoints.",
    |a: PlanSetAltitude, p: &mut FlightPlan| {
        let w = p
            .waypoints
            .get_mut(a.index)
            .ok_or_else(|| failed("waypoint index out of range"))?;
        w.altitude_m = a.altitude_m;
        Ok(())
    }
);
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct PlanConflict {
    pub leg: usize,
    pub kind: String,
    pub detail: String,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct PlanLeg {
    pub from: String,
    pub to: String,
    pub distance_m: f64,
    pub true_course_deg: f64,
    pub magnetic_course_deg: f64,
    pub ground_speed_mps: Option<f64>,
    pub ete_s: Option<f64>,
    pub fuel_kg: Option<f64>,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct PlanSummary {
    pub legs: Vec<PlanLeg>,
    pub distance_m: f64,
    pub ete_s: Option<f64>,
    pub trip_fuel_kg: Option<f64>,
    pub reserve_fuel_kg: f64,
    pub required_fuel_kg: Option<f64>,
    pub cruise: CruisePerformance,
    pub performance_source: String,
    pub conflicts: Vec<PlanConflict>,
}
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ProfileAirspace {
    pub id: String,
    pub class: Option<flightcraft_world::AirspaceClass>,
    pub floor_m: f64,
    pub ceiling_m: f64,
}
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ProfileSample {
    pub leg: usize,
    pub distance_m: f64,
    pub position: GeoPoint,
    pub altitude_m: f64,
    pub terrain_m: Option<f64>,
    pub airspace: Vec<ProfileAirspace>,
    pub outside_region: bool,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct PlanProfile {
    pub samples: Vec<ProfileSample>,
    pub conflicts: Vec<PlanConflict>,
    pub spacing_m: f64,
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfilePlan {
    #[serde(default = "profile_spacing")]
    pub spacing_m: f64,
}
fn profile_spacing() -> f64 {
    1000.0
}
fn profile(s: &Session, spacing: f64) -> Result<PlanProfile, CommandError> {
    if !spacing.is_finite() || !(100.0..=10000.0).contains(&spacing) {
        return Err(failed("profile spacing must be 100–10000 m"));
    }
    let world = s.sim().world();
    let projection = world
        .region()
        .map(|r| flightcraft_world::geo::LocalProjection::new(r.origin.latitude_deg, r.origin.longitude_deg))
        .transpose()
        .map_err(|e| failed(e.to_string()))?;
    let mut samples = Vec::new();
    let mut conflicts = std::collections::BTreeMap::<(usize, String, String), PlanConflict>::new();
    let mut cumulative = 0.0;
    for (leg, pair) in s.navigation.plan.waypoints.windows(2).enumerate() {
        let [a, b] = pair else { continue };
        let origin = a.waypoint.position;
        let dest = b.waypoint.position;
        let (distance, bearing) = origin.inverse(dest);
        let intervals = (libm::ceil(distance / spacing) as usize).max(1);
        if intervals > 20000 || samples.len().saturating_add(intervals + 1) > 20000 {
            return Err(failed("profile exceeds 20000 samples; use larger spacing"));
        }
        for i in 0..=intervals {
            let fraction = i as f64 / intervals as f64;
            let position = if i == intervals {
                dest
            } else {
                origin.destination(bearing, distance * fraction)
            };
            let altitude = a.altitude_m + (b.altitude_m - a.altitude_m) * fraction;
            let local = projection
                .as_ref()
                .and_then(|p| p.project(position.latitude_deg, position.longitude_deg).ok());
            let outside = local.is_none_or(|p| {
                world
                    .region()
                    .is_none_or(|r| flightcraft_world::inside(r, p).is_err())
            });
            let mut airspace = Vec::new();
            let mut terrain = None;
            let mut add = |kind: &str, detail: String| {
                conflicts
                    .entry((leg, kind.into(), detail.clone()))
                    .or_insert(PlanConflict {
                        leg,
                        kind: kind.into(),
                        detail,
                    });
            };
            if outside {
                add(
                    "beyond_region",
                    "Terrain and airspace coverage unavailable beyond the region".into(),
                );
            } else if let Some([n, e]) = local {
                let height = world.ground(n, e).height_msl_m;
                terrain = Some(height);
                if altitude - height < 304.8 {
                    add("terrain", "Less than 1000 ft above terrain".into());
                }
                if let Some(data) = world.airspace() {
                    for v in &data.volumes {
                        if v.contains_lateral([n, e]) {
                            let floor = v.info.floor.msl_m(height);
                            let ceiling = v.info.ceiling.msl_m(height);
                            airspace.push(ProfileAirspace {
                                id: v.info.id.clone(),
                                class: v.info.class,
                                floor_m: floor,
                                ceiling_m: ceiling,
                            });
                            if matches!(
                                v.info.class,
                                Some(
                                    flightcraft_world::AirspaceClass::B
                                        | flightcraft_world::AirspaceClass::C
                                        | flightcraft_world::AirspaceClass::D
                                )
                            ) && altitude >= floor
                                && altitude < ceiling
                            {
                                add("airspace", format!("Enters {} below its ceiling", v.info.name));
                            }
                        }
                    }
                }
            }
            samples.push(ProfileSample {
                leg,
                distance_m: cumulative + distance * fraction,
                position,
                altitude_m: altitude,
                terrain_m: terrain,
                airspace,
                outside_region: outside,
            });
        }
        cumulative += distance;
    }
    Ok(PlanProfile {
        samples,
        conflicts: conflicts.into_values().collect(),
        spacing_m: spacing,
    })
}
impl Command for ProfilePlan {
    const NAME: &'static str = "plan.profile";
    const DESCRIPTION: &'static str = "Sample planned altitude, terrain, and lateral airspace floors/ceilings; report terrain, airspace, and coverage conflicts.";
    type Output = PlanProfile;
    fn run(self, s: &mut Session) -> Result<PlanProfile, CommandError> {
        profile(s, self.spacing_m)
    }
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SummaryPlan {}
impl Command for SummaryPlan {
    const NAME: &'static str = "plan.summary";
    const DESCRIPTION: &'static str = "Compute WMM magnetic courses, geodesic legs, wind-corrected ETE, fuel, reserve, and sampled conflicts.";
    type Output = PlanSummary;
    fn run(self, s: &mut Session) -> Result<PlanSummary, CommandError> {
        let p = &s.navigation.plan;
        let (cruise, source) = if let Some(c) = p.cruise {
            (c, "Plan cruise override".to_owned())
        } else if let Some(c) = s.aircraft_configuration() {
            (
                CruisePerformance {
                    airspeed_mps: c.variant.v_speeds.vno_mps * 0.8,
                    fuel_flow_kg_s: flightcraft_aero::interpolate(&c.engine.fuel_flow_kg_s.points, 0.65),
                },
                "Aircraft Vno × 0.8 and engine flow at 65% power; planning estimate".into(),
            )
        } else {
            return Err(failed("supply cruise performance for this aircraft"));
        };
        cruise.validate()?;
        let model = MagneticModel::wmm2025().map_err(|e| failed(e.to_string()))?;
        let mut legs = Vec::new();
        let (mut total_distance, mut total_time) = (0.0, Some(0.0));
        for pair in p.waypoints.windows(2) {
            let [a, b] = pair else { continue };
            let from = a.waypoint.position;
            let to = b.waypoint.position;
            let (distance, bearing_rad) = flightcraft_sim::avionics::navigation::inverse(
                [from.latitude_deg, from.longitude_deg],
                [to.latitude_deg, to.longitude_deg],
            );
            let bearing = bearing_rad.to_degrees();
            let variation = model
                .declination(from.latitude_deg, from.longitude_deg, s.navigation.date)
                .map_err(|e| failed(e.to_string()))?;
            let [wn, we, _] =
                s.sim()
                    .weather()
                    .mean_wind_mps([0.0, 0.0, -((a.altitude_m + b.altitude_m) * 0.5)]);
            let (sin, cos) = libm::sincos(bearing.to_radians());
            let cross = -wn * sin + we * cos;
            let along = wn * cos + we * sin;
            let speed = if cross.abs() >= cruise.airspeed_mps {
                None
            } else {
                let v = libm::sqrt(cruise.airspeed_mps * cruise.airspeed_mps - cross * cross) + along;
                (v > 1.0).then_some(v)
            };
            let time = speed.map(|v| distance / v);
            total_time = total_time.zip(time).map(|(a, b)| a + b);
            total_distance += distance;
            legs.push(PlanLeg {
                from: a.waypoint.ident.clone(),
                to: b.waypoint.ident.clone(),
                distance_m: distance,
                true_course_deg: bearing,
                magnetic_course_deg: (bearing - variation).rem_euclid(360.0),
                ground_speed_mps: speed,
                ete_s: time,
                fuel_kg: time.map(|t| t * cruise.fuel_flow_kg_s),
            });
        }
        let reserve = p.reserve_s * cruise.fuel_flow_kg_s;
        let trip = total_time.map(|t| t * cruise.fuel_flow_kg_s);
        let mut conflicts = profile(s, 1000.0)?.conflicts;
        for (leg, l) in legs.iter().enumerate() {
            if l.ete_s.is_none() {
                conflicts.push(PlanConflict {
                    leg,
                    kind: "wind".into(),
                    detail: "Cruise airspeed cannot maintain the leg in current wind".into(),
                });
            }
        }
        Ok(PlanSummary {
            legs,
            distance_m: total_distance,
            ete_s: total_time,
            trip_fuel_kg: trip,
            reserve_fuel_kg: reserve,
            required_fuel_kg: trip.map(|v| v + reserve),
            cruise,
            performance_source: source,
            conflicts,
        })
    }
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanSave {
    pub name: String,
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanLoad {
    pub name: String,
}
fn plan_filename(name: &str) -> Result<String, CommandError> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err(failed(
            "plan name must contain 1–64 ASCII letters, digits, hyphens, or underscores",
        ));
    }
    Ok(format!("plan-{name}.json"))
}
impl Command for PlanSave {
    const NAME: &'static str = "plan.save";
    const DESCRIPTION: &'static str = "Save a named plan in local app storage.";
    type Output = FlightPlan;
    fn run(self, s: &mut Session) -> Result<FlightPlan, CommandError> {
        let name = plan_filename(&self.name)?;
        let text = serde_json::to_string_pretty(&s.navigation.plan).map_err(|e| failed(e.to_string()))?;
        #[cfg(not(target_arch = "wasm32"))]
        {
            let dir = s
                .navigation
                .storage
                .as_ref()
                .ok_or_else(|| failed("local storage not configured; use plan.export"))?;
            std::fs::create_dir_all(dir).map_err(|e| failed(e.to_string()))?;
            let tmp = dir.join(format!("{name}.tmp"));
            std::fs::write(&tmp, text)
                .and_then(|()| std::fs::rename(&tmp, dir.join(name)))
                .map_err(|e| failed(e.to_string()))?;
        }
        #[cfg(target_arch = "wasm32")]
        {
            browser_storage()?
                .set_item(&name, &text)
                .map_err(|_| failed("browser storage denied or full"))?;
        }
        Ok(s.navigation.plan.clone())
    }
}
impl Command for PlanLoad {
    const NAME: &'static str = "plan.load";
    const DESCRIPTION: &'static str = "Load and validate a named plan from local app storage.";
    type Output = FlightPlan;
    fn run(self, s: &mut Session) -> Result<FlightPlan, CommandError> {
        let name = plan_filename(&self.name)?;
        #[cfg(not(target_arch = "wasm32"))]
        let text = {
            use std::io::Read;
            let dir = s
                .navigation
                .storage
                .as_ref()
                .ok_or_else(|| failed("local storage not configured; use plan.import"))?;
            let mut text = String::new();
            std::fs::File::open(dir.join(name))
                .map_err(|e| failed(e.to_string()))?
                .take(1024 * 1024 + 1)
                .read_to_string(&mut text)
                .map_err(|e| failed(e.to_string()))?;
            text
        };
        #[cfg(target_arch = "wasm32")]
        let text = browser_storage()?
            .get_item(&name)
            .map_err(|_| failed("browser storage denied"))?
            .ok_or_else(|| failed("saved plan not found"))?;
        let p = FlightPlan::from_json(&text)?;
        s.navigation.plan = p.clone();
        Ok(p)
    }
}
#[cfg(target_arch = "wasm32")]
fn browser_storage() -> Result<web_sys::Storage, CommandError> {
    web_sys::window()
        .ok_or_else(|| failed("window unavailable"))?
        .local_storage()
        .map_err(|_| failed("browser storage denied"))?
        .ok_or_else(|| failed("browser storage unavailable"))
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanFormat {
    Json,
    Gpx,
    Fpl,
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanExport {
    pub format: PlanFormat,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct PlanDocument {
    pub format: PlanFormat,
    pub text: String,
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanImport {
    pub format: PlanFormat,
    pub text: String,
}
impl Command for PlanExport {
    const NAME: &'static str = "plan.export";
    const DESCRIPTION: &'static str =
        "Export FlightCraft JSON, GPX 1.1 route XML, or Garmin flight-plan v1 XML.";
    type Output = PlanDocument;
    fn run(self, s: &mut Session) -> Result<PlanDocument, CommandError> {
        Ok(PlanDocument {
            format: self.format,
            text: export_plan(&s.navigation.plan, self.format)?,
        })
    }
}
impl Command for PlanImport {
    const NAME: &'static str = "plan.import";
    const DESCRIPTION: &'static str =
        "Import bounded JSON, GPX route/track points, or Garmin flight-plan v1 XML.";
    type Output = FlightPlan;
    fn run(self, s: &mut Session) -> Result<FlightPlan, CommandError> {
        let p = import_plan(&self.text, self.format)?;
        s.navigation.plan = p.clone();
        Ok(p)
    }
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanActivate {}
#[derive(Debug, Serialize, JsonSchema)]
pub struct PlanActivated {
    pub waypoints: usize,
    pub target: String,
}
impl Command for PlanActivate {
    const NAME: &'static str = "plan.activate";
    const DESCRIPTION: &'static str =
        "Send resolved airports, navaids, fixes, pins, and coordinates to the glass GPS flight plan.";
    type Output = PlanActivated;
    fn run(self, s: &mut Session) -> Result<PlanActivated, CommandError> {
        let plan = s.navigation.plan.clone();
        plan.validate()?;
        if plan.waypoints.is_empty() {
            return Err(failed("plan is empty"));
        }
        s.set_gps_waypoints(plan.waypoints.iter().map(|p| p.waypoint.clone()).collect())?;
        s.navigation.active_plan = Some(plan.clone());
        Ok(PlanActivated {
            waypoints: plan.waypoints.len(),
            target: "glass GPS flight plan".into(),
        })
    }
}
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
pub fn export_plan(p: &FlightPlan, format: PlanFormat) -> Result<String, CommandError> {
    p.validate()?;
    match format {
        PlanFormat::Json => serde_json::to_string_pretty(p).map_err(|e| failed(e.to_string())),
        PlanFormat::Gpx => {
            let mut x = format!(
                "<?xml version=\"1.0\"?><gpx version=\"1.1\" creator=\"FlightCraft\" xmlns=\"http://www.topografix.com/GPX/1/1\"><rte><name>{}</name>",
                escape(&p.name)
            );
            for w in &p.waypoints {
                x.push_str(&format!(
                    "<rtept lat=\"{}\" lon=\"{}\"><ele>{}</ele><name>{}</name></rtept>",
                    w.waypoint.position.latitude_deg,
                    w.waypoint.position.longitude_deg,
                    w.altitude_m,
                    escape(&w.waypoint.ident)
                ));
            }
            x.push_str("</rte></gpx>");
            Ok(x)
        }
        PlanFormat::Fpl => {
            let mut x = String::from(
                "<?xml version=\"1.0\"?><flight-plan xmlns=\"http://www8.garmin.com/xmlschemas/FlightPlan/v1\"><waypoint-table>",
            );
            for (i, w) in p.waypoints.iter().enumerate() {
                x.push_str(&format!("<waypoint><identifier>W{i:03}</identifier><type>USER WAYPOINT</type><country-code></country-code><lat>{}</lat><lon>{}</lon><comment>{}</comment></waypoint>",w.waypoint.position.latitude_deg,w.waypoint.position.longitude_deg,escape(&w.waypoint.ident)));
            }
            x.push_str(&format!(
                "</waypoint-table><route><route-name>{}</route-name><flight-plan-index>1</flight-plan-index>",
                escape(&p.name)
            ));
            for (i, _) in p.waypoints.iter().enumerate() {
                x.push_str(&format!("<route-point><waypoint-identifier>W{i:03}</waypoint-identifier><waypoint-type>USER WAYPOINT</waypoint-type><waypoint-country-code></waypoint-country-code></route-point>"));
            }
            x.push_str("</route></flight-plan>");
            Ok(x)
        }
    }
}
pub fn import_plan(text: &str, format: PlanFormat) -> Result<FlightPlan, CommandError> {
    if text.len() > 1024 * 1024 {
        return Err(failed("plan exceeds 1 MiB"));
    }
    if matches!(format, PlanFormat::Json) {
        return FlightPlan::from_json(text);
    }
    let doc = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            nodes_limit: 10000,
            ..Default::default()
        },
    )
    .map_err(|e| failed(e.to_string()))?;
    let root = doc.root_element();
    let mut p = FlightPlan::default();
    let child = |n: roxmltree::Node<'_, '_>, key: &str| {
        n.children()
            .find(|n| n.has_tag_name(key))
            .and_then(|n| n.text())
            .map(str::to_owned)
    };
    let number = |n: roxmltree::Node<'_, '_>, key: &str| -> Result<f64, CommandError> {
        child(n, key)
            .ok_or_else(|| failed(format!("missing {key}")))?
            .parse()
            .map_err(|_| failed(format!("invalid {key}")))
    };
    match format {
        PlanFormat::Gpx => {
            if root.tag_name().name() != "gpx" {
                return Err(failed("expected GPX root"));
            }
            let routes: Vec<_> = root
                .children()
                .filter(|n| n.has_tag_name("rte") || n.has_tag_name("trk"))
                .collect();
            let [route] = routes.as_slice() else {
                return Err(failed("GPX must have exactly one route or track"));
            };
            p.name = child(*route, "name").unwrap_or(p.name);
            for n in route
                .descendants()
                .filter(|n| n.has_tag_name("rtept") || n.has_tag_name("trkpt"))
            {
                let attr = |k: &str| {
                    n.attribute(k)
                        .ok_or_else(|| failed("missing GPX coordinate"))?
                        .parse::<f64>()
                        .map_err(|_| failed("invalid GPX coordinate"))
                };
                p.waypoints.push(PlanWaypoint {
                    waypoint: GpsWaypoint {
                        ident: child(n, "name").unwrap_or_else(|| format!("W{}", p.waypoints.len() + 1)),
                        kind: WaypointKind::Coordinate,
                        position: GeoPoint {
                            latitude_deg: attr("lat")?,
                            longitude_deg: attr("lon")?,
                        },
                    },
                    altitude_m: if child(n, "ele").is_some() {
                        number(n, "ele")?
                    } else {
                        1000.0
                    },
                });
                if p.waypoints.len() > MAX_POINTS {
                    return Err(failed("too many GPX points"));
                }
            }
        }
        PlanFormat::Fpl => {
            if root.tag_name().name() != "flight-plan" {
                return Err(failed("expected flight-plan root"));
            }
            let mut table = std::collections::BTreeMap::new();
            for n in root.descendants().filter(|n| n.has_tag_name("waypoint")) {
                let id = child(n, "identifier").ok_or_else(|| failed("missing waypoint identifier"))?;
                let typ = child(n, "type").unwrap_or_default();
                let country = child(n, "country-code").unwrap_or_default();
                let w = PlanWaypoint {
                    waypoint: GpsWaypoint {
                        ident: child(n, "comment")
                            .filter(|s| text_ok(s, 64))
                            .unwrap_or_else(|| id.clone()),
                        kind: match typ.as_str() {
                            "AIRPORT" => WaypointKind::Airport,
                            "VOR" | "NDB" => WaypointKind::Navaid,
                            "INT" => WaypointKind::Fix,
                            _ => WaypointKind::Coordinate,
                        },
                        position: GeoPoint {
                            latitude_deg: number(n, "lat")?,
                            longitude_deg: number(n, "lon")?,
                        },
                    },
                    altitude_m: if child(n, "elevation").is_some() {
                        number(n, "elevation")?
                    } else {
                        1000.0
                    },
                };
                if table.insert((id, typ, country), w).is_some() {
                    return Err(failed("duplicate FPL waypoint"));
                }
            }
            let routes: Vec<_> = root.children().filter(|n| n.has_tag_name("route")).collect();
            let [route] = routes.as_slice() else {
                return Err(failed("FPL must have one route"));
            };
            p.name = child(*route, "route-name").unwrap_or(p.name);
            for n in route.children().filter(|n| n.has_tag_name("route-point")) {
                let key = (
                    child(n, "waypoint-identifier").ok_or_else(|| failed("missing route identifier"))?,
                    child(n, "waypoint-type").unwrap_or_default(),
                    child(n, "waypoint-country-code").unwrap_or_default(),
                );
                p.waypoints.push(
                    table
                        .get(&key)
                        .ok_or_else(|| failed("unresolved FPL waypoint"))?
                        .clone(),
                );
            }
        }
        PlanFormat::Json => {}
    }
    p.validate()?;
    Ok(p)
}
