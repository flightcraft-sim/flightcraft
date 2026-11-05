//! Integrated glass behaviour. The same snapshot drives public skins and command clients.
use crate::{CommandError, Session, TrafficClass, invalid, range};
use flightcraft_sim::avionics::navigation::{desired_track, inverse, track};
use schemars::JsonSchema;
use serde::Serialize;
const NM: f64 = 1852.0;

pub use flightcraft_sim::avionics::glass::*;
#[derive(Clone, Debug, Serialize, JsonSchema, serde::Deserialize)]
pub struct GlassSettings {
    pub synthetic_vision: bool,
    pub heading_bug_rad: f64,
    pub course_rad: f64,
    pub course_manual: bool,
    pub selected_altitude_m: f64,
    pub pressure_hpa: bool,
    pub bearing_pointer: BearingPointer,
    pub transponder_code: String,
    pub transponder_mode: TransponderMode,
    /// Seconds of IDENT left (the reply carries the ident flag); 0 when not identing.
    #[serde(default)]
    pub ident_remaining_s: f64,
    /// Simulation step at which the ident flag drops; internal.
    #[serde(skip)]
    pub ident_until_step: u64,
    /// Simulation time of the last IDENT press, for the controller's compliance check.
    #[serde(skip)]
    pub last_ident_time_s: Option<f64>,
    pub range_nm: f64,
    pub orientation: MapOrientation,
    pub page: GlassPage,
    pub flight_plan: Vec<String>,
    pub waypoints: Vec<GpsWaypoint>,
    pub active_leg: usize,
    /// The waypoint selected on the FPL page, for Insert, Remove, and Activate Leg.
    #[serde(default)]
    pub fpl_cursor: usize,
    pub leg_origin_deg: Option<[f64; 2]>,
    /// Traffic on the MFD map (needs an ADS-B In installation).
    #[serde(default = "traffic_shown")]
    pub traffic_shown: bool,
    /// The altitude window traffic is shown in.
    #[serde(default)]
    pub traffic_filter: TrafficFilter,
    #[serde(skip)]
    pub generation: u64,
}
fn traffic_shown() -> bool {
    true
}
impl Default for GlassSettings {
    fn default() -> Self {
        Self {
            synthetic_vision: false,
            heading_bug_rad: 0.0,
            course_rad: 0.0,
            course_manual: false,
            selected_altitude_m: 914.4,
            pressure_hpa: false,
            bearing_pointer: BearingPointer::Off,
            transponder_code: "1200".into(),
            transponder_mode: TransponderMode::Alt,
            ident_remaining_s: 0.0,
            ident_until_step: 0,
            last_ident_time_s: None,
            range_nm: 40.0,
            orientation: MapOrientation::NorthUp,
            page: GlassPage::Map,
            flight_plan: Vec::new(),
            waypoints: Vec::new(),
            active_leg: 0,
            fpl_cursor: 0,
            leg_origin_deg: None,
            traffic_shown: true,
            traffic_filter: TrafficFilter::Normal,
            generation: 0,
        }
    }
}
#[derive(Clone, Debug, Serialize, JsonSchema, serde::Deserialize)]
pub struct NavigationLeg {
    pub airport: String,
    pub distance_nm: f64,
    pub bearing_rad: f64,
    pub ete_s: Option<f64>,
}
#[derive(Clone, Debug, Serialize, JsonSchema, serde::Deserialize)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema, serde::Deserialize)]
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
#[derive(Clone, Debug, Default, Serialize, JsonSchema, serde::Deserialize)]
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
#[derive(Clone, Debug, Serialize, JsonSchema, serde::Deserialize)]
pub struct MapOutline {
    pub id: String,
    pub class: Option<flightcraft_world::AirspaceClass>,
    pub rings: Vec<Vec<[f64; 2]>>,
}
#[derive(Clone, Debug, Serialize, JsonSchema, serde::Deserialize)]
pub struct MapAirport {
    pub ident: String,
    pub position: [f64; 2],
    pub runways: Vec<[[f64; 2]; 2]>,
}
#[derive(Clone, Debug, Serialize, JsonSchema, serde::Deserialize)]
pub struct EngineValue {
    pub label: String,
    pub value: f64,
    pub unit: String,
    pub min: f64,
    pub max: f64,
    pub normal: [f64; 2],
}
#[derive(Clone, Debug, Serialize, JsonSchema, serde::Deserialize)]
pub struct MapTraffic {
    pub callsign: String,
    /// Written registration, the label glass units draw.
    #[serde(default)]
    pub registration: String,
    pub position: [f64; 2],
    pub relative_altitude_m: f64,
    #[serde(default)]
    pub address: u32,
    #[serde(default)]
    pub class: TrafficClass,
    /// Direction of travel on the chart, radians clockwise from the screen top.
    #[serde(default)]
    pub chart_track_rad: f64,
    #[serde(default)]
    pub vertical_rate_fpm: f64,
    #[serde(default)]
    pub ground_speed_kt: f64,
    #[serde(default)]
    pub range_nm: f64,
    /// A controller's call named this aircraft within the last 20 seconds.
    #[serde(default)]
    pub highlighted: bool,
}
#[derive(Clone, Debug, Serialize, JsonSchema, serde::Deserialize)]
pub struct GlassState {
    pub comms: crate::CommsSnapshot,
    pub traffic: Vec<MapTraffic>,
    /// An ADS-B In (or TIS-B) receiver feeds this aircraft's glass displays.
    #[serde(default)]
    pub traffic_installed: bool,
    /// A traffic alert is active: the PFD shows the TRAFFIC annunciation.
    #[serde(default)]
    pub traffic_alert: bool,
    pub touch: crate::TouchState,
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
    /// Nearest VORs, NDBs, intersections, and airspace. Filled only while a display shows the
    /// matching nearest page; empty otherwise.
    #[serde(default)]
    pub nearest_vors: Vec<crate::touch_nearest::NearestFacility>,
    #[serde(default)]
    pub nearest_ndbs: Vec<crate::touch_nearest::NearestFacility>,
    #[serde(default)]
    pub nearest_intersections: Vec<crate::touch_nearest::NearestFacility>,
    #[serde(default)]
    pub nearest_airspace: Vec<crate::touch_nearest::NearestAirspace>,
    pub engine: Vec<EngineValue>,
    /// Map coordinates are fractions of a square spanning twice the selected range.
    pub terrain: TerrainRaster,
    pub outlines: Vec<MapOutline>,
    pub airports: Vec<MapAirport>,
    pub region: Option<String>,
    pub aircraft_heading_relative_rad: f64,
    /// The crew alerting messages, for the CAS list page.
    #[serde(default)]
    pub cas: crate::touch_cas::CasWindow,
}
impl Session {
    pub fn set_glass(&mut self, setting: GlassSetting) -> Result<(), CommandError> {
        self.reset_glass_generation();
        const NAME: &str = "avionics.set";
        match setting {
            GlassSetting::SyntheticVision { enabled } => self.glass.synthetic_vision = enabled,
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
                if !matches!(mode, TransponderMode::On | TransponderMode::Alt) {
                    self.glass.ident_until_step = 0;
                }
            }
            GlassSetting::Ident {} => {
                if !matches!(
                    self.glass.transponder_mode,
                    TransponderMode::On | TransponderMode::Alt
                ) {
                    return Err(invalid(NAME, "transponder must be on or altitude to ident"));
                }
                let steps = (IDENT_DURATION_S / flightcraft_sim::DT_S).round() as u64;
                self.glass.ident_until_step = self.sim.steps() + steps;
                self.glass.last_ident_time_s = Some(self.sim.state().time_s);
            }
            GlassSetting::CasAcknowledge { index } => {
                let failures = self
                    .sim
                    .installed_autopilot()
                    .map(|a| a.failures.clone())
                    .unwrap_or_default();
                let list = crate::touch_cas::window(self.sim.systems().modern.as_ref(), &failures).list();
                let message = list
                    .get(index)
                    .ok_or_else(|| invalid(NAME, "no such CAS message"))?;
                if let Some(i) = message.annunciation
                    && !message.acknowledged
                {
                    self.sim
                        .set_systems(flightcraft_sim::systems::SystemSetting::Equipment {
                            setting: flightcraft_sim::modern::ModernSetting::AcknowledgeMessage { index: i },
                        })?;
                }
            }
            GlassSetting::MapRange { range_nm } => {
                range(
                    NAME,
                    "range_nm",
                    range_nm,
                    crate::touch_efis::MAP_RANGE_MIN_NM * 0.99,
                    crate::touch_efis::MAP_RANGE_MAX_NM,
                )?;
                self.glass.range_nm = range_nm;
                self.map.range_nm = range_nm;
            }
            GlassSetting::Orientation { orientation } => {
                self.glass.orientation = orientation;
                self.map.orientation = orientation.into();
            }
            GlassSetting::Page { page } => self.glass.page = page,
            GlassSetting::DisplayPage { display, page } => {
                self.set_equipment(crate::EquipmentSetting::Display {
                    id: display,
                    action: crate::DisplayAction::Layout { layout: page },
                })?
            }
            GlassSetting::DisplayData { display, area } => {
                self.set_equipment(crate::EquipmentSetting::Display {
                    id: display,
                    action: crate::DisplayAction::DataArea { area },
                })?
            }
            GlassSetting::PilotSeat { seat } => self.set_pilot_seat(seat),
            GlassSetting::DisplayReversion { enabled } => self.set_display_reversion(enabled),
            GlassSetting::Traffic { shown } => self.glass.traffic_shown = shown,
            GlassSetting::TrafficFilter { filter } => self.glass.traffic_filter = filter,
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
        self.glass.fpl_cursor = 0;
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
        self.glass.fpl_cursor = 0;
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
        if let Some(state) = &self.presentation_glass {
            return state.clone();
        }
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
        settings.ident_remaining_s =
            settings.ident_until_step.saturating_sub(sim.steps()) as f64 * flightcraft_sim::DT_S;
        settings.range_nm = self.map.range_nm.max(crate::touch_efis::MAP_RANGE_MIN_NM * 0.99);
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
        nearest.retain(|l| l.distance_nm <= crate::touch_nearest::SEARCH_RADIUS_NM);
        nearest.truncate(crate::touch_nearest::MAX_ROWS);
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
        let touch = self.touch_state();
        let mut wanted = std::collections::BTreeSet::new();
        for d in touch.displays.iter().filter(|d| d.powered && !d.failed) {
            if crate::touch_efis::effective_page(d.layout, d.mfd_page)
                == flightcraft_aero::modern::MfdPage::Nearest
                && matches!(
                    crate::touch_efis::ScreenMode::of(d.layout),
                    crate::touch_efis::ScreenMode::FullMfd | crate::touch_efis::ScreenMode::Split
                )
            {
                wanted.insert(d.nearest_page);
            }
        }
        let near = self.nearest_lists(&wanted, position);
        let installed_power = |id: &str| sys.electrical.loads.iter().any(|l| l.id == id && l.powered);
        let pfd_failed = computers.pfd_failed || !installed_power("glass_pfd");
        let mfd_failed = computers.mfd_failed || !installed_power("glass_mfd");
        let [wind_n, wind_e, _] = r.wind_ned_mps;
        let traffic_installed = self.adsb_in_installed();
        let targets = if traffic_installed && !pfd_failed {
            self.traffic_targets()
        } else {
            Vec::new()
        };
        let traffic_alert = targets.iter().any(|t| t.class == TrafficClass::Alert);
        let traffic_map: Vec<MapTraffic> = targets
            .into_iter()
            .filter(|t| {
                !mfd_failed
                    && settings.traffic_shown
                    && (settings.traffic_filter.shows(t.relative_altitude_ft)
                        || t.class == TrafficClass::Alert)
            })
            .map(|t| {
                let [north, east, _] = t.position_m;
                MapTraffic {
                    position: project([north, east]),
                    relative_altitude_m: t.relative_altitude_ft * 0.3048,
                    chart_track_rad: t.track_grid_rad - angle,
                    address: t.address,
                    class: t.class,
                    vertical_rate_fpm: t.vertical_rate_fpm,
                    ground_speed_kt: t.ground_speed_kt,
                    range_nm: t.range_nm,
                    highlighted: t.highlighted,
                    registration: t.registration,
                    callsign: t.callsign,
                }
            })
            .collect();
        GlassState {
            comms: self.comms_state(),
            traffic: traffic_map,
            traffic_installed,
            traffic_alert,
            touch,
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
            nearest_vors: near.vors,
            nearest_ndbs: near.ndbs,
            nearest_intersections: near.intersections,
            nearest_airspace: near.airspace,
            engine: engine(self),
            terrain,
            outlines,
            airports,
            region: world.region().map(|r| r.name.clone()),
            aircraft_heading_relative_rad: r.heading_grid_rad - angle,
            cas: crate::touch_cas::window(
                sys.modern.as_ref(),
                sim.installed_autopilot()
                    .map_or(&[][..], |a| a.failures.as_slice()),
            ),
        }
    }
}
#[derive(Default)]
struct NearLists {
    vors: Vec<crate::touch_nearest::NearestFacility>,
    ndbs: Vec<crate::touch_nearest::NearestFacility>,
    intersections: Vec<crate::touch_nearest::NearestFacility>,
    airspace: Vec<crate::touch_nearest::NearestAirspace>,
}
impl Session {
    /// The nearest-page lists the open pages need; empty lists for pages nobody is showing.
    fn nearest_lists(
        &self,
        wanted: &std::collections::BTreeSet<flightcraft_aero::modern::NearestPage>,
        position: Option<[f64; 2]>,
    ) -> NearLists {
        use crate::touch_nearest as near;
        use flightcraft_aero::modern::NearestPage as P;
        use flightcraft_world::navdata::NavaidKind;
        let mut out = NearLists::default();
        if wanted.is_empty() {
            return out;
        }
        let world = self.sim.world();
        let reference = self.sim.heading_reference();
        if let (Some(p), Some(nav)) = (position, world.navdata()) {
            // A box test first: the national data holds many thousands of fixes.
            let dlat = near::SEARCH_RADIUS_NM / 60.0 * 1.05;
            let dlon = dlat / libm::cos(p[0].to_radians()).abs().max(0.05);
            let in_box = |lat: f64, lon: f64| (lat - p[0]).abs() <= dlat && (lon - p[1]).abs() <= dlon;
            let facility = |ident: &str, name: &str, at: [f64; 2], khz: Option<f64>, detail: &str| {
                let (d, b) = inverse(p, at);
                near::NearestFacility {
                    ident: ident.to_owned(),
                    name: name.to_owned(),
                    bearing_rad: reference.true_to_magnetic(b),
                    distance_nm: d / NM,
                    frequency_khz: khz,
                    detail: detail.to_owned(),
                }
            };
            if wanted.contains(&P::Vors) || wanted.contains(&P::Ndbs) {
                for n in nav
                    .navaids
                    .iter()
                    .filter(|n| in_box(n.position.latitude_deg, n.position.longitude_deg))
                {
                    let khz = n
                        .components
                        .iter()
                        .find_map(|c| c.frequency_hz)
                        .map(|hz| hz / 1000.0);
                    let at = [n.position.latitude_deg, n.position.longitude_deg];
                    match n.kind {
                        NavaidKind::Vor | NavaidKind::VorDme | NavaidKind::Vortac
                            if wanted.contains(&P::Vors) =>
                        {
                            let detail = match n.kind {
                                NavaidKind::VorDme => "VOR/DME",
                                NavaidKind::Vortac => "VORTAC",
                                _ => "VOR",
                            };
                            out.vors.push(facility(&n.ident, &n.name, at, khz, detail));
                        }
                        NavaidKind::Ndb if wanted.contains(&P::Ndbs) => {
                            out.ndbs.push(facility(&n.ident, &n.name, at, khz, "NDB"));
                        }
                        _ => {}
                    }
                }
            }
            if wanted.contains(&P::Intersections) {
                for f in nav
                    .fixes
                    .iter()
                    .filter(|f| in_box(f.position.latitude_deg, f.position.longitude_deg))
                {
                    out.intersections.push(facility(
                        &f.ident,
                        &f.name,
                        [f.position.latitude_deg, f.position.longitude_deg],
                        None,
                        "",
                    ));
                }
            }
            out.vors = near::keep_nearest(out.vors);
            out.ndbs = near::keep_nearest(out.ndbs);
            out.intersections = near::keep_nearest(out.intersections);
        }
        if wanted.contains(&P::Airspace) {
            let [n, e, _] = self.sim.state().position_m;
            let [vn, ve, _] = self.sim.state().velocity_mps;
            let altitude = self.sim.readout().altitude_msl_m;
            let samples: Vec<Vec<near::SampledVolume>> = (0..=near::LOOKAHEAD_S / near::LOOKAHEAD_STEP_S)
                .map(|i| {
                    let t = f64::from(i * near::LOOKAHEAD_STEP_S);
                    world
                        .airspace_at(n + vn * t, e + ve * t, altitude)
                        .map(|q| {
                            q.containing
                                .into_iter()
                                .filter(|v| v.class.is_some())
                                .map(|v| (v.id, v.name, v.class.map(|c| format!("{c:?}"))))
                                .collect()
                        })
                        .unwrap_or_default()
                })
                .collect();
            out.airspace = near::airspace_entries(&samples);
        }
        out
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
    let max_rpm =
        session.sim.aircraft().propulsion.max_rpm * configuration.map_or(1., |c| c.engine.gearbox_ratio);
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
    if let Some(m) = &s.modern
        && m.electronic
    {
        for (label, value, unit, max, normal) in [
            ("COOLANT", m.coolant_temperature_c, "C", 150., [60., 110.]),
            (
                "MANIFOLD",
                m.manifold_pressure_pa / 1000.,
                "kPa",
                120.,
                [20., 105.],
            ),
            ("FUEL P", m.fuel_pressure_pa / 1000., "kPa", 500., [200., 400.]),
            (
                "PROP",
                m.propeller_rpm,
                "rpm",
                session.sim.aircraft().propulsion.max_rpm,
                [0., session.sim.aircraft().propulsion.max_rpm],
            ),
        ] {
            values.push(EngineValue {
                label: label.into(),
                value,
                unit: unit.into(),
                min: 0.,
                max,
                normal,
            });
        }
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
