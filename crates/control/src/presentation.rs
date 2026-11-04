//! Bounded, versioned display snapshots. Remote copies have no simulation clock.
use crate::{CommsSnapshot, GlassState, ScenarioState, Session, screens::ScreenSet};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Snapshot {
    pub version: u32,
    pub generation: u64,
    pub state: flightcraft_sim::State,
    pub readout: flightcraft_sim::Readout,
    pub systems: flightcraft_sim::systems::SystemsState,
    pub conditions: flightcraft_world::Conditions,
    pub hood: bool,
    pub camera: Option<crate::CameraSettings>,
    pub paused: bool,
    pub rate: f64,
    pub configuration: Option<flightcraft_aero::ResolvedAircraft>,
    pub world: flightcraft_world::WorldChoice,
    pub glass: GlassState,
    pub map: crate::MapState,
    pub plan: crate::FlightPlan,
    pub pins: crate::PinsFile,
    pub date: flightcraft_world::geomag::SimulationDate,
    pub scenario: ScenarioState,
    #[serde(default)]
    pub lesson: crate::LessonDebrief,
    pub comms: CommsSnapshot,
    #[serde(default)]
    pub callsigns: crate::CallsignPreferences,
    pub atc: crate::comms::AtcState,
    pub record: crate::comms::FlightRecord,
    #[serde(default)]
    pub live_weather: crate::live_weather::runtime::Catalog,
    pub screens: ScreenSet,
    #[serde(default)]
    pub crew: crate::CrewState,
    /// The traffic display list, nearest first and at most [`MAX_SNAPSHOT_TRAFFIC`] aircraft, with each
    /// controller highlight already applied. A remote screen has no fleet, so this is its only source.
    #[serde(default)]
    pub traffic: Vec<crate::TrafficTarget>,
}
/// The most traffic targets a screen snapshot carries; the nearest are kept.
pub const MAX_SNAPSHOT_TRAFFIC: usize = 32;
impl Session {
    pub fn screen_snapshot(&self) -> Snapshot {
        let mut record = self.flight_record().clone();
        record.inputs.clear();
        record.lesson = None;
        if record.events.len() > 200 {
            record.events.drain(..record.events.len() - 200);
        }
        Snapshot {
            version: 1,
            generation: self.flight_generation,
            state: *self.sim.state(),
            readout: self.sim.readout(),
            systems: self.sim.systems().clone(),
            conditions: self.sim.conditions().clone(),
            hood: self.sim.hood(),
            camera: self.camera,
            paused: self.paused,
            rate: self.rate,
            configuration: self.active_configuration.clone(),
            world: self.sim.world().choice(),
            glass: self.glass_state(),
            map: self.map.clone(),
            plan: self.navigation.plan.clone(),
            pins: self.navigation.pins.clone(),
            date: self.navigation.date,
            scenario: self.scenario.state.clone(),
            lesson: self.lesson.debrief.clone(),
            comms: self.comms_state(),
            callsigns: self.callsign_preferences().clone(),
            atc: self.atc_state(),
            record,
            live_weather: crate::live_weather::runtime::Catalog {
                saved_sources: self.live_weather.saved_sources.values().cloned().collect(),
                sources: self.live_weather.entries.clone(),
                nearby: self.live_weather.nearby.clone(),
                following: self.live_weather.following.clone(),
            },
            screens: self.screens.clone(),
            crew: self.crew_state(),
            traffic: self
                .traffic_targets()
                .into_iter()
                .take(MAX_SNAPSHOT_TRAFFIC)
                .collect(),
        }
    }
    pub fn apply_screen_snapshot(&mut self, snapshot: Snapshot) -> Result<(), String> {
        if snapshot.version != 1 || !snapshot.state.time_s.is_finite() {
            return Err("unsupported or invalid screen snapshot".into());
        }
        snapshot.systems.canopy.validate().map_err(|e| e.to_string())?;
        snapshot.screens.validate()?;
        snapshot.lesson.validate().map_err(|e| e.to_string())?;
        snapshot.live_weather.validate()?;
        let identity = snapshot
            .configuration
            .as_ref()
            .map(|c| c.aircraft.identity.id.as_str());
        if self.sim.world().choice() != snapshot.world
            || identity.is_some_and(|id| id != self.sim.aircraft().identity.id)
            || self.flight_generation != snapshot.generation
        {
            let world = self
                .regions
                .values()
                .find(|w| w.choice() == snapshot.world)
                .cloned()
                .map(Ok)
                .unwrap_or_else(|| flightcraft_world::World::load(snapshot.world.clone()))
                .map_err(|e| e.to_string())?;
            let aircraft = snapshot
                .configuration
                .as_ref()
                .map(|c| c.aircraft.clone())
                .unwrap_or_else(|| self.selected_aircraft.clone());
            self.sim = flightcraft_sim::Sim::new_with_aircraft_weather_world(
                &flightcraft_sim::Start::Runway,
                aircraft,
                snapshot.readout.weather,
                world,
            )
            .map_err(|e| e.to_string())?;
            if let Some(configuration) = &snapshot.configuration {
                self.sim
                    .configure_systems(configuration)
                    .map_err(|e| e.to_string())?;
            }
        }
        self.sim
            .set_conditions(snapshot.conditions)
            .map_err(|e| e.to_string())?;
        self.sim.set_hood(snapshot.hood);
        self.sim
            .install_presentation(snapshot.state, snapshot.readout, snapshot.systems);
        self.camera = snapshot.camera;
        self.paused = snapshot.paused;
        self.rate = snapshot.rate;
        self.flight_generation = snapshot.generation;
        self.active_configuration = snapshot.configuration;
        self.presentation_glass = Some(snapshot.glass);
        self.restore_callsign_preferences(snapshot.callsigns)
            .map_err(|e| e.to_string())?;
        self.restore_presented_callsign(&snapshot.comms.callsign)
            .map_err(|e| e.to_string())?;
        self.presentation_comms = Some(snapshot.comms);
        self.presentation_atc = Some(snapshot.atc);
        let mut traffic = snapshot.traffic;
        traffic.truncate(MAX_SNAPSHOT_TRAFFIC);
        self.presentation_traffic = Some(traffic);
        self.map = snapshot.map;
        self.navigation.plan = snapshot.plan;
        self.navigation.pins = snapshot.pins;
        self.navigation.date = snapshot.date;
        self.scenario.state = snapshot.scenario;
        self.lesson.debrief = snapshot.lesson;
        self.comms.record = snapshot.record;
        self.live_weather.saved_sources = snapshot
            .live_weather
            .saved_sources
            .into_iter()
            .map(|s| (s.id.clone(), s))
            .collect();
        self.live_weather.entries = snapshot.live_weather.sources;
        self.live_weather.nearby = snapshot.live_weather.nearby;
        self.live_weather.following = snapshot.live_weather.following;
        self.live_weather.loaded = true;
        self.screens = snapshot.screens;
        self.crew.state = snapshot.crew;
        self.presentation_commands = Some(Vec::new());
        Ok(())
    }
    pub fn take_screen_commands(&mut self) -> Vec<(String, serde_json::Value)> {
        self.presentation_commands
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegionTransfer {
    pub region: flightcraft_world::Region,
    pub tiles: Vec<String>,
    pub airports: Option<flightcraft_world::Airports>,
    pub navdata: Option<flightcraft_world::Navdata>,
    pub procedures: Option<flightcraft_world::Procedures>,
    pub airspace: Option<flightcraft_world::AirspaceFile>,
    pub landmarks: Option<flightcraft_world::Landmarks>,
}
impl RegionTransfer {
    fn from_world(world: &flightcraft_world::World) -> Option<Self> {
        use base64::Engine;
        let region = world.region()?.clone();
        Some(Self {
            region,
            tiles: world
                .tiles()
                .iter()
                .map(|t| base64::engine::general_purpose::STANDARD.encode(t.write()))
                .collect(),
            airports: world.airports().cloned(),
            navdata: world.navdata().cloned(),
            landmarks: world.landmarks().cloned(),
            procedures: world.procedures().cloned(),
            airspace: world.airspace().map(|a| flightcraft_world::AirspaceFile {
                attribution: a.attribution.clone(),
                volumes: a.volumes.iter().map(|v| v.info.clone()).collect(),
            }),
        })
    }
    pub fn into_world(self) -> Result<flightcraft_world::World, String> {
        use base64::Engine;
        if self.tiles.len() > 4096 || self.tiles.iter().map(String::len).sum::<usize>() > 192 * 1024 * 1024 {
            return Err("region transfer exceeds budget".into());
        }
        let tiles = self
            .tiles
            .iter()
            .map(|s| {
                base64::engine::general_purpose::STANDARD
                    .decode(s)
                    .map_err(|e| e.to_string())
                    .and_then(|b| flightcraft_world::HeightTile::read(&b).map_err(|e| e.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut world =
            flightcraft_world::World::from_region(self.region, tiles).map_err(|e| e.to_string())?;
        if let Some(data) = self.landmarks {
            world = world.with_landmarks(data).map_err(|e| e.to_string())?;
        }
        if let Some(data) = self.airports {
            world = world.with_airports(data).map_err(|e| e.to_string())?;
        }
        if let Some(data) = self.navdata {
            world = world.with_navdata(data).map_err(|e| e.to_string())?;
        }
        if let Some(data) = self.procedures {
            world = world.with_procedures(data).map_err(|e| e.to_string())?;
        }
        if let Some(data) = self.airspace {
            world = world.with_airspace(data).map_err(|e| e.to_string())?;
        }
        Ok(world)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkerSeed {
    #[serde(default)]
    pub callsigns: crate::CallsignPreferences,
    pub weather_sources: Vec<crate::WeatherSource>,
    pub regions: Vec<RegionTransfer>,
    pub world: flightcraft_world::WorldChoice,
    pub configuration: Option<flightcraft_aero::ResolvedAircraft>,
    pub screens: ScreenSet,
}
impl Session {
    pub fn worker_seed(&self) -> WorkerSeed {
        WorkerSeed {
            callsigns: self.callsign_preferences().clone(),
            weather_sources: crate::live_weather::user_sources_for_seed(self),
            regions: self
                .regions
                .values()
                .filter_map(RegionTransfer::from_world)
                .collect(),
            world: self.sim.world().choice(),
            configuration: self.active_configuration.clone(),
            screens: self.screens.clone(),
        }
    }
    pub fn from_worker_seed(seed: WorkerSeed) -> Result<Self, String> {
        seed.screens.validate()?;
        if seed.regions.len() > 64 {
            return Err("too many regions".into());
        }
        let mut session = Session::new().map_err(|e| e.to_string())?;
        session.load_weather_sources();
        if seed.weather_sources.len() > 128 {
            return Err("Too many saved weather sources".into());
        }
        for source in seed.weather_sources {
            source.validate()?;
            session
                .live_weather
                .saved_sources
                .insert(source.id.clone(), source.clone());
            session.live_weather.insert(source)?;
        }
        for region in seed.regions {
            session
                .install_region(region.into_world()?)
                .map_err(|e| e.to_string())?;
        }
        if let Some(configuration) = seed.configuration {
            configuration.aircraft.validate().map_err(|e| e.to_string())?;
            session.selected_aircraft = configuration.aircraft.clone();
            session.selected_configuration = Some(configuration.clone());
            session.active_configuration = Some(configuration);
        }
        session
            .execute(
                "sim.reset",
                serde_json::json!({"world":seed.world,"start":{"kind":"runway"},"paused":false}),
            )
            .map_err(|e| e.to_string())?;
        session
            .restore_callsign_preferences(seed.callsigns)
            .map_err(|e| e.to_string())?;
        session.screens = seed.screens;
        session.enable_app();
        Ok(session)
    }
    pub fn screen_regions(&self) -> Vec<RegionTransfer> {
        RegionTransfer::from_world(self.sim.world()).into_iter().collect()
    }
    pub fn install_screen_regions(&mut self, regions: Vec<RegionTransfer>) -> Result<(), String> {
        for r in regions {
            if !self.regions.contains_key(&r.region.id) {
                self.install_region(r.into_world()?).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{SetAutopilot, SetControls, SetWeather};
    use serde_json::json;

    #[test]
    fn typed_patches_round_trip_over_the_screen_command_wire() {
        let weather = serde_json::to_value(SetWeather {
            preset: Some(flightcraft_world::WeatherPreset::Night),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(weather, json!({"preset":"night"}));
        assert!(serde_json::from_value::<SetWeather>(weather).is_ok());
        let controls = serde_json::to_value(SetControls {
            throttle: Some(0.5),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(controls, json!({"throttle":0.5}));
        assert!(serde_json::from_value::<SetControls>(controls).is_ok());
        // This patch deliberately distinguishes omission from an explicit null (clear the hold).
        let hold = serde_json::to_value(SetAutopilot {
            wings_level: Some(None),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(hold, json!({"wings_level":null}));
        let round_trip: SetAutopilot = serde_json::from_value(hold).unwrap();
        assert_eq!(round_trip.wings_level, Some(None));
    }
}
