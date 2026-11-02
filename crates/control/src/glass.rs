//! Integrated glass behaviour. The same snapshot drives public skins and command clients.
use crate::{CommandError, Session, invalid, range};
use flightcraft_sim::avionics::navigation::{desired_track, inverse, track};
use schemars::JsonSchema;
use serde::Serialize;
const NM: f64 = 1852.0;

pub use flightcraft_sim::avionics::glass::*;
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct GlassSettings {
    pub heading_bug_rad: f64,
    pub course_rad: f64,
    pub course_manual: bool,
    pub selected_altitude_m: f64,
    pub pressure_hpa: bool,
    pub bearing_pointer: BearingPointer,
    pub transponder_code: String,
    pub transponder_mode: TransponderMode,
    pub range_nm: f64,
    pub orientation: MapOrientation,
    pub page: GlassPage,
    pub flight_plan: Vec<String>,
    pub waypoints: Vec<GpsWaypoint>,
    pub active_leg: usize,
    pub leg_origin_deg: Option<[f64; 2]>,
    #[serde(skip)]
    pub generation: u64,
}
impl Default for GlassSettings {
    fn default() -> Self {
        Self {
            heading_bug_rad: 0.0,
            course_rad: 0.0,
            course_manual: false,
            selected_altitude_m: 914.4,
            pressure_hpa: false,
            bearing_pointer: BearingPointer::Off,
            transponder_code: "1200".into(),
            transponder_mode: TransponderMode::Alt,
            range_nm: 40.0,
            orientation: MapOrientation::NorthUp,
            page: GlassPage::Map,
            flight_plan: Vec::new(),
            waypoints: Vec::new(),
            active_leg: 0,
            leg_origin_deg: None,
            generation: 0,
        }
    }
}
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct NavigationLeg {
    pub airport: String,
    pub distance_nm: f64,
    pub bearing_rad: f64,
    pub ete_s: Option<f64>,
}
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct GpsState {
    pub valid: bool,
    pub active_leg: usize,
    pub destination: Option<String>,
    pub distance_nm: f64,
    pub distance_valid: bool,
    pub bearing_rad: f64,
    pub desired_track_rad: f64,
    pub cross_track_nm: f64,
    pub cdi_full_scale_nm: f64,
    /// Positive needle displacement commands a correction to the right.
    pub cdi_deviation: f64,
    pub source: String,
    pub annunciation: String,
    pub vertical_dots: Option<f64>,
    pub suspended: bool,
    pub waypoint_alert: bool,
    pub ete_s: Option<f64>,
    pub legs: Vec<NavigationLeg>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TerrainBand {
    Clear,
    Caution,
    Warning,
    Unknown,
}
pub fn terrain_band(aircraft_m: f64, terrain_m: f64) -> TerrainBand {
    let clearance = (aircraft_m - terrain_m) / 0.3048;
    if clearance <= 100.0 + 1e-8 {
        TerrainBand::Warning
    } else if clearance <= 1000.0 + 1e-8 {
        TerrainBand::Caution
    } else {
        TerrainBand::Clear
    }
}
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
pub struct TerrainRaster {
    pub columns: u32,
    pub rows: u32,
    /// North-up or track-up chart samples, left to right, top to bottom; null is uncovered.
    pub elevation_m: Vec<Option<f32>>,
}
impl TerrainRaster {
    pub fn len(&self) -> usize {
        self.elevation_m.len()
    }
    pub fn is_empty(&self) -> bool {
        self.elevation_m.is_empty()
    }
}
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct MapOutline {
    pub id: String,
    pub class: Option<flightcraft_world::AirspaceClass>,
    pub rings: Vec<Vec<[f64; 2]>>,
}
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct MapAirport {
    pub ident: String,
    pub position: [f64; 2],
    pub runways: Vec<[[f64; 2]; 2]>,
}
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct EngineValue {
    pub label: String,
    pub value: f64,
    pub unit: String,
    pub min: f64,
    pub max: f64,
    pub normal: [f64; 2],
}
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct GlassState {
    pub settings: GlassSettings,
    pub air_data: flightcraft_sim::avionics::sensor::AirDataComputer,
    pub standby: flightcraft_sim::avionics::sensor::AirDataComputer,
    pub standby_on_backup: bool,
    pub standby_remaining_s: f64,
    pub pfd_failed: bool,
    pub mfd_failed: bool,
    pub reversion: bool,
    pub baro_pa: f64,
    pub ground_speed_mps: f64,
    pub ground_track_rad: f64,
    pub wind_from_rad: f64,
    pub wind_speed_mps: f64,
    pub gps: GpsState,
    pub nearest: Vec<NavigationLeg>,
    pub engine: Vec<EngineValue>,
    /// Map coordinates are fractions of a square spanning twice the selected range.
    pub terrain: TerrainRaster,
    pub outlines: Vec<MapOutline>,
    pub airports: Vec<MapAirport>,
    pub region: Option<String>,
    pub aircraft_heading_relative_rad: f64,
}
impl Session {
    pub fn set_glass(&mut self, setting: GlassSetting) -> Result<(), CommandError> {
        self.reset_glass_generation();
        const NAME: &str = "avionics.set";
        match setting {
            GlassSetting::HeadingBug { heading_rad } => {
                range(NAME, "heading_rad", heading_rad, 0.0, core::f64::consts::TAU)?;
                self.sim
                    .set_avionics(flightcraft_sim::avionics::AvionicsSetting::HeadingBug {
                        degrees: heading_rad.to_degrees(),
                    })?;
            }
            GlassSetting::Course { course_rad } => {
                range(NAME, "course_rad", course_rad, 0.0, core::f64::consts::TAU)?;
                self.glass.course_rad = course_rad;
                self.glass.course_manual = true;
            }
            GlassSetting::SelectedAltitude { altitude_m } => {
                range(NAME, "altitude_m", altitude_m, -500.0, 15000.0)?;
                if self.sim.installed_autopilot().is_some() {
                    self.sim
                        .set_avionics(flightcraft_sim::avionics::AvionicsSetting::SelectedAltitude {
                            feet: altitude_m / 0.3048,
                        })?;
                }
                self.glass.selected_altitude_m = altitude_m;
            }
            GlassSetting::Baro { pressure_pa } => self
                .sim
                .set_systems(flightcraft_sim::systems::SystemSetting::Altimeter { pressure_pa })?,
            GlassSetting::PressureUnits { hpa } => self.glass.pressure_hpa = hpa,
            GlassSetting::BearingPointer { source } => self.glass.bearing_pointer = source,
            GlassSetting::Transponder { code, mode } => {
                if code.len() != 4 || !code.bytes().all(|b| (b'0'..=b'7').contains(&b)) {
                    return Err(invalid(NAME, "code must contain four octal digits"));
                }
                self.glass.transponder_code = code;
                self.glass.transponder_mode = mode;
            }
            GlassSetting::MapRange { range_nm } => {
                range(NAME, "range_nm", range_nm, 1.0, 200.0)?;
                self.glass.range_nm = range_nm;
                self.map.range_nm = range_nm;
            }
            GlassSetting::Orientation { orientation } => {
                self.glass.orientation = orientation;
                self.map.orientation = orientation.into();
            }
            GlassSetting::Page { page } => self.glass.page = page,
            GlassSetting::DirectTo { airport } => self.set_glass_plan(vec![airport])?,
            GlassSetting::FlightPlan { airports } => self.set_glass_plan(airports)?,
            GlassSetting::FlightPlanWaypoints { waypoints } => self.set_gps_waypoints(waypoints)?,
            GlassSetting::DirectToWaypoint { waypoint } => self.set_gps_waypoints(vec![waypoint])?,
            GlassSetting::ActiveLeg { index } => {
                if index >= self.glass.flight_plan.len() {
                    return Err(invalid(NAME, "active leg outside flight plan"));
                }
                self.glass.active_leg = index;
            }
            GlassSetting::Failure { target, failed } => {
                let c = self.sim.glass_computers_mut();
                *match target {
                    GlassFailure::Ahrs => &mut c.ahrs_failed,
                    GlassFailure::Adc => &mut c.adc_failed,
                    GlassFailure::Pfd => &mut c.pfd_failed,
                    GlassFailure::Mfd => &mut c.mfd_failed,
                } = failed;
            }
        }
        self.sync_receiver_leg();
        self.sim.refresh_glass_computers();
        Ok(())
    }
    fn set_glass_plan(&mut self, airports: Vec<String>) -> Result<(), CommandError> {
        if airports.len() > 64 {
            return Err(invalid("avionics.set", "flight plan exceeds 64 airports"));
        }
        let mut canonical = Vec::new();
        for ident in airports {
            canonical.push(crate::regions::airport(self.sim.world(), &ident)?.ident.clone());
        }
        let [n, e, _] = self.sim.state().position_m;
        let origin = self.sim.world().geodetic(n, e);
        self.sim
            .set_avionics(flightcraft_sim::avionics::AvionicsSetting::ClearProcedure {})?;
        self.glass.waypoints.clear();
        self.glass.flight_plan = canonical;
        self.glass.course_manual = false;
        self.glass.active_leg = 0;
        self.glass.leg_origin_deg = origin;
        Ok(())
    }
    fn sync_receiver_leg(&mut self) {
        let world = self.sim.world();
        // Resolved waypoints (navaids, fixes, pins, coordinates) when present, else airport idents.
        let point = |index: usize| -> Option<[f64; 2]> {
            if self.glass.waypoints.len() == self.glass.flight_plan.len()
                && let Some(w) = self.glass.waypoints.get(index)
            {
                return Some([w.position.latitude_deg, w.position.longitude_deg]);
            }
            let id = self.glass.flight_plan.get(index)?;
            let a = crate::regions::airport(world, id).ok()?;
            Some([a.latitude_deg, a.longitude_deg])
        };
        let leg = (|| {
            let region = world.region()?;
            let end = point(self.glass.active_leg)?;
            let start = if self.glass.active_leg == 0 {
                self.glass.leg_origin_deg?
            } else {
                point(self.glass.active_leg.saturating_sub(1))?
            };
            Some(flightcraft_sim::nav_source::ReceiverLeg {
                start_deg: start,
                end_deg: end,
                projection_origin_deg: [region.origin.latitude_deg, region.origin.longitude_deg],
            })
        })();
        self.sim.set_receiver_leg(leg);
    }

    /// Replace GPS navigation atomically with resolved airports, navaids, fixes, pins, or coordinates.
    pub fn set_gps_waypoints(&mut self, waypoints: Vec<GpsWaypoint>) -> Result<(), CommandError> {
        if waypoints.len() > 64 {
            return Err(invalid("avionics.set", "flight plan exceeds 64 waypoints"));
        }
        for w in &waypoints {
            if !flightcraft_world::navdata::text_ok(&w.ident, 64) {
                return Err(invalid("avionics.set", "invalid waypoint ident"));
            }
            w.position
                .validate()
                .map_err(|e| CommandError::Failed(e.to_string()))?;
        }
        self.sim
            .set_avionics(flightcraft_sim::avionics::AvionicsSetting::ClearProcedure {})?;
        let [n, e, _] = self.sim.state().position_m;
        self.glass.flight_plan = waypoints.iter().map(|w| w.ident.clone()).collect();
        self.glass.waypoints = waypoints;
        self.glass.active_leg = 0;
        self.glass.course_manual = false;
        self.glass.leg_origin_deg = self.sim.world().geodetic(n, e);
        self.glass.generation = self.flight_generation;
        Ok(())
    }
    pub(crate) fn reset_glass_generation(&mut self) {
        if self.glass.generation != self.flight_generation {
            self.glass = GlassSettings {
                generation: self.flight_generation,
                ..Default::default()
            };
        }
    }
    pub(crate) fn sequence_glass(&mut self) {
        self.reset_glass_generation();
        if !self.sim.procedure_receiver().loaded.is_empty() {
            return;
        }
        let [n, e, _] = self.sim.state().position_m;
        let Some(position) = self.sim.world().geodetic(n, e) else {
            return;
        };
        while self.glass.active_leg + 1 < self.glass.flight_plan.len() {
            let Some(id) = self.glass.flight_plan.get(self.glass.active_leg) else {
                break;
            };
            let Some(destination) = gps_destination(&self.glass, self.sim.world(), self.glass.active_leg)
            else {
                break;
            };
            let _ = id;
            if inverse(position, destination).0 > 0.3 * NM {
                break;
            }
            self.glass.active_leg += 1;
        }
        self.sync_receiver_leg();
    }
    pub fn glass_state(&self) -> GlassState {
        let sim = &self.sim;
        let world = sim.world();
        let r = sim.readout();
        let reference = sim.heading_reference();
        let sys = sim.systems();
        let computers = sim.glass_computers();
        let mut settings = if self.glass.generation == self.flight_generation {
            self.glass.clone()
        } else {
            GlassSettings::default()
        };
        settings.range_nm = self.map.range_nm.max(1.0);
        settings.orientation = self.map.orientation.into();
        settings.heading_bug_rad = sys.controls.heading_bug_rad;
        if let Some(ap) = sim.installed_autopilot() {
            settings.selected_altitude_m = ap.altitude_m;
        }
        let [n, e, _] = sim.state().position_m;
        let position = world.geodetic(n, e);
        let [vn, ve, _] = sim.state().velocity_mps;
        let ground_track = if r.ground_speed_mps > 0.5 {
            libm::atan2(ve, vn).rem_euclid(core::f64::consts::TAU)
        } else {
            r.heading_grid_rad
        };
        let mut gps = gps(world, &settings, position, r.ground_speed_mps);
        gps.desired_track_rad = reference.true_to_magnetic(gps.desired_track_rad);
        gps.bearing_rad = reference.true_to_magnetic(gps.bearing_rad);
        for leg in &mut gps.legs {
            leg.bearing_rad = reference.true_to_magnetic(leg.bearing_rad);
        }
        let receiver = sim.procedure_receiver();
        if !receiver.loaded.is_empty() {
            settings.flight_plan = receiver
                .legs
                .iter()
                .map(|l| {
                    l.leg
                        .fix
                        .as_deref()
                        .and_then(|id| world.procedures()?.point(id))
                        .map_or_else(|| format!("{:?}", l.leg.path), |p| p.ident.clone())
                })
                .collect();
            settings.active_leg = receiver.active_leg;
            gps.valid = receiver.valid;
            gps.active_leg = receiver.active_leg;
            gps.destination = settings.flight_plan.get(receiver.active_leg).cloned();
            gps.bearing_rad = receiver
                .legs
                .get(receiver.active_leg)
                .and_then(|l| l.endpoint)
                .zip(position)
                .map_or(0., |(end, p)| {
                    reference.true_to_magnetic(inverse(p, [end.latitude_deg, end.longitude_deg]).1)
                });
            gps.distance_nm = receiver.distance_nm;
            gps.distance_valid = receiver.valid;
            gps.desired_track_rad = receiver.desired_track_rad;
            gps.cross_track_nm = receiver.cross_track_nm;
            gps.cdi_full_scale_nm = receiver.cdi_full_scale_nm;
            gps.cdi_deviation =
                (-receiver.cross_track_nm / receiver.cdi_full_scale_nm.max(0.001)).clamp(-1., 1.);
            gps.annunciation = match receiver.mode {
                flightcraft_sim::avionics::procedures::ApproachMode::LnavVnav => "LNAV/VNAV".into(),
                mode => format!("{mode:?}"),
            };
            gps.vertical_dots = receiver
                .vertical_deviation_m
                .map(|m| (m / 30. * 2.5).clamp(-2.5, 2.5));
            gps.suspended = receiver.suspended;
            gps.waypoint_alert = receiver.waypoint_alert;
            gps.legs = receiver
                .legs
                .iter()
                .enumerate()
                .map(|(i, l)| NavigationLeg {
                    airport: settings.flight_plan.get(i).cloned().unwrap_or_default(),
                    distance_nm: l
                        .endpoint
                        .zip(position)
                        .map_or(0., |(a, b)| inverse(b, [a.latitude_deg, a.longitude_deg]).0 / NM),
                    bearing_rad: l.course_magnetic_deg.unwrap_or(0.).to_radians(),
                    ete_s: None,
                })
                .collect();
        }
        if sim.radios().source == flightcraft_sim::avionics::radio::CdiSource::Nav1
            && let Some(radio) = sim.radio_state().first()
        {
            gps.source = match radio.guidance.map(|g| g.kind) {
                Some(flightcraft_sim::nav_source::NavKind::Localizer) => "LOC",
                Some(flightcraft_sim::nav_source::NavKind::BackCourse) => "BC",
                _ => "NAV1",
            }
            .into();
            gps.distance_nm = radio.dme_nm.unwrap_or(0.);
            gps.distance_valid = radio.dme_nm.is_some();
            gps.valid = radio.signal_valid;
            gps.destination = radio.ident.clone();
            gps.cdi_deviation = radio.lateral_dots.unwrap_or(0.) / 2.5;
            gps.vertical_dots = radio.vertical_dots;
            if let Some(g) = radio.guidance {
                gps.cross_track_nm = g.cross_track_m / NM;
                gps.desired_track_rad = radio.course_magnetic_deg.map_or_else(
                    || reference.grid_to_magnetic(g.desired_track_rad),
                    f64::to_radians,
                );
            }
            gps.annunciation = if radio.vertical_dots.is_some() {
                "ILS".into()
            } else {
                "NAV1".into()
            };
        }
        let mut nearest = world
            .airports()
            .map(|db| {
                db.airports
                    .iter()
                    .filter_map(|a| {
                        position.map(|p| {
                            let (d, b) = inverse(p, [a.latitude_deg, a.longitude_deg]);
                            NavigationLeg {
                                airport: a.ident.clone(),
                                distance_nm: d / NM,
                                bearing_rad: reference.true_to_magnetic(b),
                                ete_s: ete(d, r.ground_speed_mps),
                            }
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        nearest.sort_by(|a, b| a.distance_nm.total_cmp(&b.distance_nm));
        nearest.truncate(8);
        let angle = match settings.orientation {
            MapOrientation::TrackUp => ground_track,
            MapOrientation::HeadingUp => reference.magnetic_to_grid(computers.primary.heading_rad),
            MapOrientation::NorthUp => -reference.grid_north_true_rad,
        };
        let (cos, sin) = (libm::cos(angle), libm::sin(angle));
        let span = settings.range_nm * NM * 2.0;
        let project = |[north, east]: [f64; 2]| {
            let (dn, de) = (north - n, east - e);
            [
                (de * cos - dn * sin) / span + 0.5,
                0.5 - (dn * cos + de * sin) / span,
            ]
        };
        let mut terrain = TerrainRaster::default();
        if let Some(region) = world
            .region()
            .filter(|_| sys.electrical.loads.iter().any(|l| l.id == "glass_mfd"))
        {
            let spacing = f64::from(region.spacing_m);
            let intervals = ((span / spacing).ceil() as u32).clamp(64, 256);
            terrain.columns = intervals + 1;
            terrain.rows = intervals + 1;
            let extent = f64::from(region.tile_size.saturating_sub(1)) * spacing;
            let [south, west] = region.southwest_m;
            let tiles = world
                .tiles()
                .iter()
                .map(|tile| {
                    let [tn, te] = tile.origin_m();
                    (
                        (
                            ((tn - south) / extent).round() as i64,
                            ((te - west) / extent).round() as i64,
                        ),
                        tile,
                    )
                })
                .collect::<std::collections::BTreeMap<_, _>>();
            for row in 0..=intervals {
                for col in 0..=intervals {
                    let (x, y) = (
                        f64::from(col) / f64::from(intervals),
                        f64::from(row) / f64::from(intervals),
                    );
                    let (right, forward) = ((x - 0.5) * span, (0.5 - y) * span);
                    let (north, east) = (n + forward * cos - right * sin, e + forward * sin + right * cos);
                    let key = (
                        ((north - south) / extent).floor() as i64,
                        ((east - west) / extent).floor() as i64,
                    );
                    let height = tiles
                        .get(&key)
                        .and_then(|tile| tile.sample(north, east))
                        .map(|g| g.height_msl_m as f32);
                    terrain.elevation_m.push(height);
                }
            }
        }
        let outlines = world
            .airspace()
            .map(|airspace| {
                airspace
                    .volumes
                    .iter()
                    .map(|v| MapOutline {
                        id: v.info.id.clone(),
                        class: v.info.class,
                        rings: v
                            .polygons
                            .iter()
                            .flatten()
                            .map(|ring| ring.iter().map(|p| project(*p)).collect())
                            .collect(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let projection = world.region().and_then(|reg| {
            flightcraft_world::geo::LocalProjection::new(reg.origin.latitude_deg, reg.origin.longitude_deg)
                .ok()
        });
        let airports = world
            .airports()
            .map(|db| {
                db.airports
                    .iter()
                    .filter_map(|a| {
                        let p = projection.as_ref()?;
                        let position = project(p.project(a.latitude_deg, a.longitude_deg).ok()?);
                        let runways = a
                            .runways
                            .iter()
                            .filter_map(|rw| {
                                let [a, b] = &rw.ends;
                                Some([
                                    project(p.project(a.latitude_deg, a.longitude_deg).ok()?),
                                    project(p.project(b.latitude_deg, b.longitude_deg).ok()?),
                                ])
                            })
                            .collect();
                        Some(MapAirport {
                            ident: a.ident.clone(),
                            position,
                            runways,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let installed_power = |id: &str| sys.electrical.loads.iter().any(|l| l.id == id && l.powered);
        let pfd_failed = computers.pfd_failed || !installed_power("glass_pfd");
        let mfd_failed = computers.mfd_failed || !installed_power("glass_mfd");
        let [wind_n, wind_e, _] = r.wind_ned_mps;
        GlassState {
            settings,
            air_data: computers.primary.clone(),
            standby: computers.standby.clone(),
            standby_on_backup: sys
                .electrical
                .loads
                .iter()
                .any(|l| l.id == "glass_standby" && l.on_backup),
            standby_remaining_s: sys
                .electrical
                .loads
                .iter()
                .find(|l| l.id == "glass_standby")
                .map_or(0., |l| l.backup_remaining_ah * 3600. / 0.5),
            pfd_failed,
            mfd_failed,
            reversion: mfd_failed && !pfd_failed,
            baro_pa: sys.controls.altimeter_setting_pa,
            ground_speed_mps: r.ground_speed_mps,
            ground_track_rad: reference.grid_to_magnetic(ground_track),
            wind_from_rad: if libm::hypot(wind_n, wind_e) < 0.1 {
                0.
            } else {
                reference.grid_to_magnetic(libm::atan2(-wind_e, -wind_n))
            },
            wind_speed_mps: libm::hypot(wind_n, wind_e),
            gps,
            nearest,
            engine: engine(self),
            terrain,
            outlines,
            airports,
            region: world.region().map(|r| r.name.clone()),
            aircraft_heading_relative_rad: r.heading_grid_rad - angle,
        }
    }
}
fn ete(distance: f64, speed: f64) -> Option<f64> {
    (speed > 1.0).then_some(distance / speed.max(1.0))
}
fn gps_destination(s: &GlassSettings, world: &flightcraft_world::World, index: usize) -> Option<[f64; 2]> {
    if let Some(w) = s.waypoints.get(index) {
        return Some([w.position.latitude_deg, w.position.longitude_deg]);
    }
    let id = s.flight_plan.get(index)?;
    crate::regions::airport(world, id)
        .ok()
        .map(|a| [a.latitude_deg, a.longitude_deg])
}
fn gps(
    world: &flightcraft_world::World,
    s: &GlassSettings,
    position: Option<[f64; 2]>,
    speed: f64,
) -> GpsState {
    let mut legs = Vec::new();
    let mut previous = s.leg_origin_deg;
    for (index, id) in s.flight_plan.iter().enumerate() {
        if let Some(dest) = gps_destination(s, world, index) {
            let (d, b) = previous.map_or((0.0, 0.0), |p| inverse(p, dest));
            legs.push(NavigationLeg {
                airport: id.clone(),
                distance_nm: d / NM,
                bearing_rad: b,
                ete_s: ete(d, speed),
            });
            previous = Some(dest);
        }
    }
    let mut result = GpsState {
        valid: false,
        active_leg: s.active_leg,
        destination: None,
        distance_nm: 0.0,
        distance_valid: false,
        bearing_rad: 0.0,
        desired_track_rad: 0.0,
        cross_track_nm: 0.0,
        cdi_full_scale_nm: 2.0,
        cdi_deviation: 0.0,
        source: "GPS".into(),
        annunciation: "ENR".into(),
        vertical_dots: None,
        suspended: false,
        waypoint_alert: false,
        ete_s: None,
        legs,
    };
    let Some(position) = position else {
        return result;
    };
    let Some(id) = s.flight_plan.get(s.active_leg) else {
        return result;
    };
    let Some(dest) = gps_destination(s, world, s.active_leg) else {
        return result;
    };
    let (d, b) = inverse(position, dest);
    let start = if s.active_leg == 0 {
        s.leg_origin_deg
    } else {
        gps_destination(s, world, s.active_leg - 1)
    };
    let Some(start) = start else {
        return result;
    };
    let (xt, _) = track(start, dest, position);
    let course = if inverse(start, dest).0 < 1.0 {
        b
    } else {
        desired_track(start, dest, position)
    };
    let scale = if d < 30.0 * NM { 1.0 } else { 2.0 };
    result.valid = true;
    result.destination = Some(id.clone());
    result.distance_nm = d / NM;
    result.distance_valid = true;
    result.bearing_rad = b;
    result.desired_track_rad = course;
    result.cross_track_nm = xt / NM;
    result.cdi_full_scale_nm = scale;
    result.annunciation = if scale == 1. { "TERM" } else { "ENR" }.into();
    result.cdi_deviation = (-xt / NM / scale).clamp(-1.0, 1.0);
    result.ete_s = ete(d, speed);
    result
}
fn engine(session: &Session) -> Vec<EngineValue> {
    let s = session.sim.systems();
    let configuration = session.aircraft_configuration();
    let max_rpm = session.sim.aircraft().propulsion.max_rpm;
    let flow = configuration
        .and_then(|c| c.engine.fuel_flow_kg_s.points.last())
        .map_or(30.0, |(_, flow)| flow * 3600.0);
    let mut values = Vec::new();
    for (label, value, unit, min, max, normal) in [
        (
            "RPM",
            s.engine_rpm,
            "rpm",
            0.0,
            max_rpm,
            [max_rpm * 0.6, max_rpm * 0.96],
        ),
        (
            "FLOW",
            s.fuel_flow_kg_s * 3600.0,
            "kg/h",
            0.0,
            flow * 1.3,
            [0.0, flow],
        ),
        (
            "OIL P",
            s.oil_pressure_pa / 1000.0,
            "kPa",
            0.0,
            700.0,
            [200.0, 500.0],
        ),
        ("OIL T", s.oil_temperature_c, "C", 0.0, 160.0, [50.0, 115.0]),
        ("EGT", s.egt_c, "C", 0.0, 1100.0, [400.0, 900.0]),
        ("CHT", s.cht_c, "C", 0.0, 300.0, [80.0, 220.0]),
        (
            "VOLTS",
            s.electrical.main_bus_voltage,
            "V",
            0.0,
            18.0,
            [13.0, 15.0],
        ),
        (
            "AMPS",
            -s.electrical.battery_current_a,
            "A",
            -60.0,
            60.0,
            [-5.0, 5.0],
        ),
    ] {
        values.push(EngineValue {
            label: label.into(),
            value,
            unit: unit.into(),
            min,
            max,
            normal,
        });
    }
    for tank in &s.tanks {
        values.push(EngineValue {
            label: format!("FUEL {}", tank.id),
            value: tank.indicated_kg,
            unit: "kg".into(),
            min: 0.0,
            max: tank.capacity_kg,
            normal: [tank.capacity_kg * 0.25, tank.capacity_kg],
        });
    }
    values
}
#[cfg(test)]
mod tests;
