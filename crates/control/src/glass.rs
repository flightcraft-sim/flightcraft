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
    pub bearing_rad: f64,
    pub desired_track_rad: f64,
    pub cross_track_nm: f64,
    pub cdi_full_scale_nm: f64,
    /// Positive needle displacement commands a correction to the right.
    pub cdi_deviation: f64,
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
            }
            GlassSetting::Orientation { orientation } => self.glass.orientation = orientation,
            GlassSetting::Page { page } => self.glass.page = page,
            GlassSetting::DirectTo { airport } => self.set_glass_plan(vec![airport])?,
            GlassSetting::FlightPlan { airports } => self.set_glass_plan(airports)?,
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
        self.glass.flight_plan = canonical;
        self.glass.course_manual = false;
        self.glass.active_leg = 0;
        self.glass.leg_origin_deg = origin;
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
        let [n, e, _] = self.sim.state().position_m;
        let Some(position) = self.sim.world().geodetic(n, e) else {
            return;
        };
        while self.glass.active_leg + 1 < self.glass.flight_plan.len() {
            let Some(id) = self.glass.flight_plan.get(self.glass.active_leg) else {
                break;
            };
            let Ok(destination) = crate::regions::airport(self.sim.world(), id) else {
                break;
            };
            if inverse(position, [destination.latitude_deg, destination.longitude_deg]).0 > 0.3 * NM {
                break;
            }
            self.glass.active_leg += 1;
        }
    }
    pub fn glass_state(&self) -> GlassState {
        let sim = &self.sim;
        let world = sim.world();
        let r = sim.readout();
        let sys = sim.systems();
        let computers = sim.glass_computers();
        let mut settings = if self.glass.generation == self.flight_generation {
            self.glass.clone()
        } else {
            GlassSettings::default()
        };
        settings.heading_bug_rad = sys.controls.heading_bug_rad;
        let [n, e, _] = sim.state().position_m;
        let position = world.geodetic(n, e);
        let [vn, ve, _] = sim.state().velocity_mps;
        let ground_track = if r.ground_speed_mps > 0.5 {
            libm::atan2(ve, vn).rem_euclid(core::f64::consts::TAU)
        } else {
            r.heading_rad
        };
        let gps = gps(world, &settings, position, r.ground_speed_mps);
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
                                bearing_rad: b,
                                ete_s: ete(d, r.ground_speed_mps),
                            }
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        nearest.sort_by(|a, b| a.distance_nm.total_cmp(&b.distance_nm));
        nearest.truncate(8);
        let angle = if settings.orientation == MapOrientation::TrackUp {
            ground_track
        } else {
            0.0
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
            ground_track_rad: ground_track,
            wind_from_rad: libm::atan2(-wind_e, -wind_n).rem_euclid(core::f64::consts::TAU),
            wind_speed_mps: libm::hypot(wind_n, wind_e),
            gps,
            nearest,
            engine: engine(self),
            terrain,
            outlines,
            airports,
            region: world.region().map(|r| r.name.clone()),
            aircraft_heading_relative_rad: r.heading_rad - angle,
        }
    }
}
fn ete(distance: f64, speed: f64) -> Option<f64> {
    (speed > 1.0).then_some(distance / speed.max(1.0))
}
fn gps(
    world: &flightcraft_world::World,
    s: &GlassSettings,
    position: Option<[f64; 2]>,
    speed: f64,
) -> GpsState {
    let mut legs = Vec::new();
    let mut previous = s.leg_origin_deg;
    for id in &s.flight_plan {
        if let Ok(a) = crate::regions::airport(world, id) {
            let dest = [a.latitude_deg, a.longitude_deg];
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
        bearing_rad: 0.0,
        desired_track_rad: 0.0,
        cross_track_nm: 0.0,
        cdi_full_scale_nm: 2.0,
        cdi_deviation: 0.0,
        ete_s: None,
        legs,
    };
    let Some(position) = position else {
        return result;
    };
    let Some(id) = s.flight_plan.get(s.active_leg) else {
        return result;
    };
    let Ok(a) = crate::regions::airport(world, id) else {
        return result;
    };
    let dest = [a.latitude_deg, a.longitude_deg];
    let (d, b) = inverse(position, dest);
    let start = if s.active_leg == 0 {
        s.leg_origin_deg
    } else {
        s.flight_plan
            .get(s.active_leg - 1)
            .and_then(|id| crate::regions::airport(world, id).ok())
            .map(|a| [a.latitude_deg, a.longitude_deg])
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
    result.bearing_rad = b;
    result.desired_track_rad = course;
    result.cross_track_nm = xt / NM;
    result.cdi_full_scale_nm = scale;
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
