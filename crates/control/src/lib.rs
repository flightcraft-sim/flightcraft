//! Everything is a command: each has input and output schemas, and none opens a dialog.
//! The session owns a simulation; the MCP server and the UI both drive it through commands.
//!
//! Two ways in, same code path: typed ([`Session::run`] with a [`Command`]) for Rust callers such
//! as the UI, and JSON ([`Session::execute`]) for MCP and scripts. Both go through the registry.

pub mod lesson;
pub use lesson::*;
pub mod scenario;
pub use scenario::*;
pub mod comms;
pub use comms::*;
mod modern;
pub use modern::*;
pub mod glass;
pub use glass::*;
mod avionics;
pub use avionics::*;
mod navigation;
pub use navigation::*;
pub mod map;
pub use map::*;
mod planning;
pub use planning::*;
mod geocoding;
pub use geocoding::*;
mod location;
pub use location::*;
mod aircraft;
mod airspace;
pub use airspace::*;
mod regions;
pub use regions::*;
mod packs;
pub use aircraft::*;
pub use packs::*;
pub mod efb;
pub use efb::*;
mod app;
pub mod crew;
mod holds;
#[cfg(not(target_arch = "wasm32"))]
pub mod host;
pub mod presentation;
pub mod screens;
pub use app::*;
pub use crew::*;
#[cfg(not(target_arch = "wasm32"))]
pub mod bridge;
mod systems;
pub use systems::*;
pub mod mcp;
pub use holds::*;

use flightcraft_sim::{Controls, Readout, Sim, SimError, Start, State};
pub use flightcraft_world::WorldChoice;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// Ten simulated minutes per explicit step call, at 120 Hz.
pub const MAX_STEPS: u32 = 120 * 600;

/// A command: a name, a typed input, and a typed output, both with JSON schemas.
pub trait Command: Serialize + DeserializeOwned + schemars::JsonSchema + Send + 'static {
    /// Dotted name, for example `sim.step`.
    const NAME: &'static str;
    /// One sentence for the command list and MCP tool description.
    const DESCRIPTION: &'static str;
    type Output: Serialize + schemars::JsonSchema + Send + 'static;
    fn run(self, session: &mut Session) -> Result<Self::Output, CommandError>;
}

/// What a command list entry says about a command.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct CommandInfo {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: serde_json::Value,
    pub output_schema: serde_json::Value,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum CommandError {
    #[error("unknown command: {0}")]
    Unknown(String),
    #[error("invalid input for {command}: {message}")]
    InvalidInput { command: String, message: String },
    #[error(transparent)]
    Callsign(#[from] comms::CallsignError),
    #[error(transparent)]
    Sim(#[from] SimError),
    #[error("{0}")]
    Failed(String),
    #[error("command awaits completion by the app")]
    AppPending(AppRequest),
}

/// One flight session: the simulation and the settings commands change.
#[derive(Debug)]
pub struct Session {
    pub(crate) screens: screens::ScreenSet,
    presentation_glass: Option<GlassState>,
    presentation_comms: Option<CommsSnapshot>,
    presentation_atc: Option<comms::AtcState>,
    pub(crate) presentation_commands: Option<Vec<(String, serde_json::Value)>>,
    layout_undo: Vec<screens::ScreenSet>,
    layout_redo: Vec<screens::ScreenSet>,
    #[cfg(not(target_arch = "wasm32"))]
    host: Option<host::SessionHost>,
    #[cfg(not(target_arch = "wasm32"))]
    presentation_efb: Option<EfbStatus>,
    crew: crew::Crew,
    scenario: scenario::ScenarioRuntime,
    lesson: lesson::LessonRuntime,
    flight_archive: comms::archive::FlightArchive,
    comms: comms::Comms,
    sim: Sim,
    #[cfg(not(target_arch = "wasm32"))]
    efb: Option<efb::sender::Sender>,
    navigation: navigation::NavigationState,
    map: MapState,
    glass: GlassSettings,
    touch: modern::TouchAvionics,
    selected_aircraft: flightcraft_aero::Aircraft,
    aircraft_catalog: flightcraft_aero::AircraftCatalog,
    selected_configuration: Option<flightcraft_aero::ResolvedAircraft>,
    active_configuration: Option<flightcraft_aero::ResolvedAircraft>,
    paused: bool,
    rate: f64,
    camera: Option<CameraSettings>,
    airspace_visible: bool,
    flight_generation: u64,
    regions: std::collections::BTreeMap<String, flightcraft_world::World>,
}

impl Session {
    /// A session parked on the runway.
    pub fn new() -> Result<Self, CommandError> {
        let aircraft_catalog = flightcraft_aero::AircraftCatalog::builtins().map_err(SimError::from)?;
        let configuration = aircraft_catalog
            .resolve_aircraft("corvid")
            .map_err(SimError::from)?;
        Ok(Self {
            screens: Default::default(),
            presentation_glass: None,
            presentation_comms: None,
            presentation_atc: None,
            presentation_commands: None,
            layout_undo: Vec::new(),
            layout_redo: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            host: None,
            #[cfg(not(target_arch = "wasm32"))]
            presentation_efb: None,
            crew: Default::default(),
            scenario: Default::default(),
            lesson: Default::default(),
            flight_archive: Default::default(),
            comms: comms::Comms::default(),
            selected_configuration: Some(configuration.clone()),
            active_configuration: Some(configuration),
            aircraft_catalog,
            navigation: navigation::NavigationState::default(),
            sim: Sim::new(&Start::Runway)?,
            #[cfg(not(target_arch = "wasm32"))]
            efb: None,
            selected_aircraft: flightcraft_aero::Aircraft::corvid().map_err(SimError::from)?,
            map: MapState::default(),
            glass: GlassSettings::default(),
            touch: modern::TouchAvionics::default(),
            paused: true,
            rate: 1.0,
            camera: None,
            airspace_visible: true,
            flight_generation: 0,
            regions: std::collections::BTreeMap::new(),
        })
    }

    /// Enable app commands for the session owner.
    pub fn enable_app(&mut self) {
        self.camera = Some(CameraSettings::default());
    }

    #[must_use]
    pub fn camera(&self) -> Option<CameraSettings> {
        self.camera
    }

    /// Changes on every successful reset, including a reset to the same simulated time.
    #[must_use]
    pub fn flight_generation(&self) -> u64 {
        self.flight_generation
    }

    /// Whether any hold currently owns flight controls.
    #[must_use]
    pub fn holds_active(&self) -> bool {
        let targets = self.sim.readout().autopilot;
        targets.wings_level
            || targets
                .pitch_rad
                .or(targets.altitude_m)
                .or(targets.vertical_speed_mps)
                .or(targets.heading_rad)
                .or(targets.airspeed_mps)
                .is_some()
    }

    /// Read-only access for drawing and instruments. Changes go through commands.
    #[must_use]
    pub fn sim(&self) -> &Sim {
        &self.sim
    }

    /// Commands change the simulation through this.
    pub fn sim_mut(&mut self) -> &mut Sim {
        &mut self.sim
    }

    /// Whether the app's real-time stepping loop is paused. Explicit steps still run.
    #[must_use]
    pub fn paused(&self) -> bool {
        self.paused
    }

    /// Multiplier for the app's real-time stepping loop; it never changes the fixed timestep.
    #[must_use]
    pub fn rate(&self) -> f64 {
        self.rate
    }

    /// Run a typed command.
    pub fn run<C: Command>(&mut self, command: C) -> Result<C::Output, CommandError> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(host) = self.host.clone() {
            let (result, mut snapshot) = host.run(command)?;
            snapshot.host = Some(host);
            *self = snapshot;
            return result;
        }
        if self.presentation_commands.is_some() && !screens::read_only(C::NAME) {
            let input = serde_json::to_value(&command).map_err(|e| CommandError::Failed(e.to_string()))?;
            if let Some(commands) = self.presentation_commands.as_mut() {
                if commands.len() >= 256 {
                    return Err(CommandError::Failed("screen command queue full".into()));
                }
                commands.push((C::NAME.into(), input));
            }
        }
        if self.presentation_commands.is_some() {
            return command.run(self);
        }
        self.sequence_glass();
        self.sequence_comms();
        if self.crew_owns_controls() && matches!(C::NAME, "sim.set_controls" | "assist.set" | "autopilot.set")
        {
            return Err(invalid(C::NAME, "crew owns controls; confirm a handover first"));
        }
        let result = command.run(self);
        if result.is_ok() && C::NAME == "failure.set" {
            self.begin_emergency(EventSource::Rules);
        }
        self.sequence_glass();
        self.sequence_comms();
        #[cfg(not(target_arch = "wasm32"))]
        self.update_efb();
        result
    }

    /// Run a command by name with JSON input; the output is JSON.
    pub fn execute(
        &mut self,
        name: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, CommandError> {
        if self.camera.is_some() && !self.paused && matches!(name, "sim.step" | "sim.run_until") {
            return Err(CommandError::Failed(
                "pause first with sim.pause before explicit stepping; use sim.wait for real-time flight"
                    .into(),
            ));
        }
        dispatch(self, name, input)
    }

    /// Every registered command with its schemas.
    #[must_use]
    pub fn commands() -> Vec<CommandInfo> {
        registry()
    }

    fn snapshot(&self) -> StateOutput {
        let readout = self.sim.readout();
        StateOutput {
            latitude_deg: readout.latitude_deg,
            longitude_deg: readout.longitude_deg,
            altitude_msl_m: readout.latitude_deg.map(|_| readout.altitude_msl_m),
            state: *self.sim.state(),
            readout,
            systems: self.sim.systems().clone(),
            paused: self.paused,
            rate: self.rate,
            world: self.sim.world().choice(),
            simulation_date: self.navigation.date,
            conditions: self.sim.conditions().clone(),
            hood: self.sim.hood(),
            simulated_instrument_time: self.sim.instrument_time(),
            airspace: self.sim.world().airspace().map(|_| {
                let [n, e, d] = self.sim.state().position_m;
                self.sim
                    .world()
                    .airspace_at(n, e, -d)
                    .map(|q| {
                        q.containing
                            .into_iter()
                            .map(|v| flightcraft_world::AirspaceMembership {
                                id: v.id,
                                class: v.class,
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            }),
        }
    }

    fn settings(&self) -> Playback {
        Playback {
            paused: self.paused,
            rate: self.rate,
        }
    }
}

/// Full simulation state in SI units, plus app playback settings.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StateOutput {
    pub latitude_deg: Option<f64>,
    pub longitude_deg: Option<f64>,
    pub altitude_msl_m: Option<f64>,
    pub state: State,
    pub readout: Readout,
    pub systems: flightcraft_sim::systems::SystemsState,
    pub paused: bool,
    pub rate: f64,
    pub world: WorldChoice,
    pub simulation_date: flightcraft_world::geomag::SimulationDate,
    pub conditions: flightcraft_world::Conditions,
    pub hood: bool,
    pub simulated_instrument_time: flightcraft_sim::InstrumentTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub airspace: Option<Vec<flightcraft_world::AirspaceMembership>>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Playback {
    pub paused: bool,
    /// Dimensionless real-time multiplier.
    pub rate: f64,
}

/// Start on the default runway or in the air above its threshold.
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResetStart {
    Runway {},
    AirportRunway {
        airport: String,
        end: String,
    },
    AirportAir(AirportAirStart),
    GeographicAir {
        position: flightcraft_world::GeoPoint,
        height_agl_m: f64,
        airspeed_mps: f64,
        /// Magnetic heading, radians clockwise from magnetic north.
        heading_rad: f64,
    },
    Air {
        #[schemars(range(min = 0))]
        height_m: f64,
        #[schemars(range(min = 0))]
        airspeed_mps: f64,
        #[schemars(range(min = 0, max = std::f64::consts::TAU))]
        /// Magnetic heading, radians clockwise from magnetic north.
        heading_rad: f64,
    },
}

#[derive(Deserialize, JsonSchema, serde::Serialize)]
#[serde(untagged)]
enum StartInput {
    Existing(ResetStart),
    Runway(RunwayInput),
    Air(AirInput),
}
#[derive(Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct RunwayInput {
    runway: AirportRunwayStart,
}
#[derive(Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct AirInput {
    air: AirportAirStart,
}
fn deserialize_start<'de, D: serde::Deserializer<'de>>(d: D) -> Result<ResetStart, D::Error> {
    Ok(match StartInput::deserialize(d)? {
        StartInput::Existing(s) => s,
        StartInput::Runway(s) => ResetStart::AirportRunway {
            airport: s.runway.airport,
            end: s.runway.end,
        },
        StartInput::Air(s) => ResetStart::AirportAir(s.air),
    })
}

#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Reset {
    /// World data is part of the start. Omission retains the original flat world.
    #[serde(default)]
    pub world: WorldChoice,
    #[serde(deserialize_with = "deserialize_start")]
    #[schemars(with = "StartInput")]
    pub start: ResetStart,
    /// Omission starts in calm air. Supplying weather makes its seed part of the start.
    #[serde(default, deserialize_with = "provided")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "flightcraft_sim::Weather", transform = without_default)]
    pub weather: Option<flightcraft_sim::Weather>,
    /// Defaults to paused. The existing rate is retained across resets.
    #[serde(default, deserialize_with = "provided")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "bool", extend("default" = true))]
    pub paused: Option<bool>,
}

impl Command for Reset {
    const NAME: &'static str = "sim.reset";
    const DESCRIPTION: &'static str = "Start on the runway or in the air, in SI units.";
    type Output = StateOutput;

    fn run(self, session: &mut Session) -> Result<Self::Output, CommandError> {
        let world = session.resolve_world(self.world)?;
        let start = match self.start {
            ResetStart::AirportRunway { airport, end } => {
                let start = regions::runway_start(&world, AirportRunwayStart { airport, end })?;
                // Keep every landing gear contact on the surveyed pavement, rather than behind its edge.
                // This applies only to a named runway reset, never an airborne aircraft.
                match start {
                    Start::RunwayAt {
                        mut threshold_ned_m,
                        heading_rad,
                    } => {
                        let inset = session
                            .selected_aircraft
                            .geometry
                            .gear
                            .iter()
                            .map(|g| -session.selected_aircraft.relative_point(g.point_m)[0])
                            .fold(0., f64::max)
                            + 2.;
                        threshold_ned_m[0] += inset * libm::cos(heading_rad);
                        threshold_ned_m[1] += inset * libm::sin(heading_rad);
                        threshold_ned_m[2] =
                            -world.ground(threshold_ned_m[0], threshold_ned_m[1]).height_msl_m;
                        Start::RunwayAt {
                            threshold_ned_m,
                            heading_rad,
                        }
                    }
                    other => other,
                }
            }
            ResetStart::AirportAir(s) => regions::air_start(&world, s, session.simulation_date())?,
            ResetStart::GeographicAir {
                position,
                height_agl_m,
                airspeed_mps,
                heading_rad,
            } => {
                position
                    .validate()
                    .map_err(|e| CommandError::Failed(e.to_string()))?;
                range(Self::NAME, "height_agl_m", height_agl_m, 2.0, 15000.0)?;
                range(Self::NAME, "airspeed_mps", airspeed_mps, 0.0, 300.0)?;
                range(
                    Self::NAME,
                    "heading_rad",
                    heading_rad,
                    0.0,
                    core::f64::consts::TAU,
                )?;
                let region = world
                    .region()
                    .ok_or_else(|| CommandError::Failed("geographic air start requires a region".into()))?;
                let projection = flightcraft_world::geo::LocalProjection::new(
                    region.origin.latitude_deg,
                    region.origin.longitude_deg,
                )
                .map_err(|e| CommandError::Failed(e.to_string()))?;
                let [n, e] = projection
                    .project(position.latitude_deg, position.longitude_deg)
                    .map_err(|e| CommandError::Failed(e.to_string()))?;
                flightcraft_world::inside(region, [n, e]).map_err(|e| CommandError::Failed(e.to_string()))?;
                Start::AirAt {
                    position_m: [n, e, -(world.ground(n, e).height_msl_m + height_agl_m)],
                    airspeed_mps,
                    heading_rad,
                }
            }
            ResetStart::Runway {} => Start::Runway,
            ResetStart::Air {
                height_m,
                airspeed_mps,
                heading_rad,
            } => {
                range(Self::NAME, "start.height_m", height_m, 0.0, f64::MAX)?;
                range(Self::NAME, "start.airspeed_mps", airspeed_mps, 0.0, f64::MAX)?;
                range(
                    Self::NAME,
                    "start.heading_rad",
                    heading_rad,
                    0.0,
                    std::f64::consts::TAU,
                )?;
                Start::Air {
                    height_m,
                    airspeed_mps,
                    heading_rad,
                }
            }
        };
        let start = match start {
            Start::Air {
                height_m,
                airspeed_mps,
                heading_rad,
            } => {
                let reference = flightcraft_sim::magnetic::reference_at(
                    &world,
                    [0., 0., -height_m],
                    session.simulation_date(),
                )
                .map_err(SimError::from)?;
                Start::Air {
                    height_m,
                    airspeed_mps,
                    heading_rad: reference.magnetic_to_grid(heading_rad),
                }
            }
            Start::AirAt {
                position_m,
                airspeed_mps,
                heading_rad,
            } => {
                let reference =
                    flightcraft_sim::magnetic::reference_at(&world, position_m, session.simulation_date())
                        .map_err(SimError::from)?;
                Start::AirAt {
                    position_m,
                    airspeed_mps,
                    heading_rad: reference.magnetic_to_grid(heading_rad),
                }
            }
            other => other,
        };
        let weather_position = match &start {
            Start::Air { height_m, .. } => [0., 0., -*height_m],
            Start::AirAt { position_m, .. } => *position_m,
            Start::RunwayAt { threshold_ned_m, .. } => *threshold_ned_m,
            Start::Runway => [0., 0., 0.],
        };
        let reference =
            flightcraft_sim::magnetic::reference_at(&world, weather_position, session.simulation_date())
                .map_err(SimError::from)?;
        let mut weather = self.weather.unwrap_or_default();
        weather.validate().map_err(SimError::from)?;
        weather.wind_from_deg = reference
            .magnetic_to_grid(weather.wind_from_deg.to_radians())
            .to_degrees();
        let mut replacement =
            Sim::new_with_aircraft_weather_world(&start, session.selected_aircraft.clone(), weather, world)?;
        replacement.set_simulation_date(session.simulation_date())?;
        if let Some(configuration) = &session.selected_configuration {
            replacement.configure_systems(configuration)?;
        }
        if matches!(start, Start::Runway | Start::RunwayAt { .. }) {
            replacement.prepare_departure();
        }
        if session.flight_archive.autosave_enabled {
            session.save_flight()?;
        }
        session.crew = crew::Crew::default();
        session.lesson = lesson::LessonRuntime::default();
        session.sim = replacement;
        session.active_configuration = session.selected_configuration.clone();
        session.flight_generation = session.flight_generation.wrapping_add(1);
        session.paused = self.paused.unwrap_or(true);
        Ok(session.snapshot())
    }
}

// Optional fields may be omitted, but a supplied field must have the declared type (not null).
fn provided<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

// Omission means "keep the current control", so no fixed default belongs in its schema.
fn without_default(schema: &mut schemars::Schema) {
    schema.remove("default");
}

/// Partial controls; values persist until explicitly changed. Surface inputs are dimensionless.
#[derive(Debug, Default, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetControls {
    #[serde(default, deserialize_with = "provided")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "f64", range(min = -1, max = 1), transform = without_default)]
    pub elevator: Option<f64>,
    #[serde(default, deserialize_with = "provided")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "f64", range(min = -1, max = 1), transform = without_default)]
    pub aileron: Option<f64>,
    #[serde(default, deserialize_with = "provided")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "f64", range(min = -1, max = 1), transform = without_default)]
    pub rudder: Option<f64>,
    #[serde(default, deserialize_with = "provided")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "f64", range(min = 0, max = 1), transform = without_default)]
    pub throttle: Option<f64>,
    #[serde(default, deserialize_with = "provided")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "f64", range(min = 0, max = 1), transform = without_default)]
    pub flaps: Option<f64>,
    #[serde(default, deserialize_with = "provided")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "f64", range(min = 0, max = 1), transform = without_default)]
    pub brake: Option<f64>,
}

impl Command for SetControls {
    const NAME: &'static str = "sim.set_controls";
    const DESCRIPTION: &'static str = "Set any subset of the six dimensionless pilot controls.";
    type Output = Controls;

    fn run(self, session: &mut Session) -> Result<Self::Output, CommandError> {
        let mut controls = session.sim.controls();
        for (name, supplied, target, min) in [
            ("elevator", self.elevator, &mut controls.elevator, -1.0),
            ("aileron", self.aileron, &mut controls.aileron, -1.0),
            ("rudder", self.rudder, &mut controls.rudder, -1.0),
            ("throttle", self.throttle, &mut controls.throttle, 0.0),
            ("flaps", self.flaps, &mut controls.flaps, 0.0),
            ("brake", self.brake, &mut controls.brake, 0.0),
        ] {
            if let Some(value) = supplied {
                range(Self::NAME, name, value, min, 1.0)?;
                *target = value;
            }
        }
        session.sim.set_controls(controls);
        Ok(session.sim.controls())
    }
}

#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    #[schemars(range(min = 0, max = MAX_STEPS))]
    pub steps: u32,
}

impl Command for Step {
    const NAME: &'static str = "sim.step";
    const DESCRIPTION: &'static str =
        "Advance up to 72000 fixed steps, even while paused; return the readout.";
    type Output = Readout;

    fn run(self, session: &mut Session) -> Result<Self::Output, CommandError> {
        if self.steps > MAX_STEPS {
            return Err(invalid(Self::NAME, "steps must be between 0 and 72000"));
        }
        session.step_with_comms(self.steps);
        Ok(session.sim.readout())
    }
}

#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct GetState {}

impl Command for GetState {
    const NAME: &'static str = "sim.state";
    const DESCRIPTION: &'static str = "Read the full state, instrument readout, and playback settings.";
    type Output = StateOutput;

    fn run(self, session: &mut Session) -> Result<Self::Output, CommandError> {
        Ok(session.snapshot())
    }
}

#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Pause {}

impl Command for Pause {
    const NAME: &'static str = "sim.pause";
    const DESCRIPTION: &'static str = "Pause the app's real-time stepping loop.";
    type Output = Playback;

    fn run(self, session: &mut Session) -> Result<Self::Output, CommandError> {
        session.paused = true;
        Ok(session.settings())
    }
}

#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Resume {}

impl Command for Resume {
    const NAME: &'static str = "sim.resume";
    const DESCRIPTION: &'static str = "Resume the app's real-time stepping loop.";
    type Output = Playback;

    fn run(self, session: &mut Session) -> Result<Self::Output, CommandError> {
        session.paused = false;
        Ok(session.settings())
    }
}

#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetRate {
    #[schemars(range(min = 0.25, max = 8))]
    pub rate: f64,
}

impl Command for SetRate {
    const NAME: &'static str = "sim.set_rate";
    const DESCRIPTION: &'static str = "Set the app's real-time multiplier from 0.25 to 8.";
    type Output = Playback;

    fn run(self, session: &mut Session) -> Result<Self::Output, CommandError> {
        range(Self::NAME, "rate", self.rate, 0.25, 8.0)?;
        if session.crew_owns_controls() && session.crew_state().phase != PilotPhase::Cruise && self.rate > 1.0
        {
            return Err(invalid(
                Self::NAME,
                "crew permits accelerated time only during cruise",
            ));
        }
        session.rate = self.rate;
        Ok(session.settings())
    }
}

#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ListCommands {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CommandList {
    pub commands: Vec<CommandInfo>,
}

impl Command for ListCommands {
    const NAME: &'static str = "commands.list";
    const DESCRIPTION: &'static str = "List every command and its input and output JSON schemas.";
    type Output = CommandList;

    fn run(self, _session: &mut Session) -> Result<Self::Output, CommandError> {
        Ok(CommandList {
            commands: Session::commands(),
        })
    }
}

fn invalid(command: &str, message: impl Into<String>) -> CommandError {
    CommandError::InvalidInput {
        command: command.into(),
        message: message.into(),
    }
}

fn range(command: &str, field: &str, value: f64, min: f64, max: f64) -> Result<(), CommandError> {
    if value.is_finite() && (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(invalid(
            command,
            format!("{field} must be finite and between {min} and {max}"),
        ))
    }
}

fn execute_typed<C: Command>(
    session: &mut Session,
    input: serde_json::Value,
) -> Result<serde_json::Value, CommandError> {
    if !input.is_object() {
        return Err(invalid(C::NAME, "input must be an object"));
    }
    let command: C =
        serde_path_to_error::deserialize(input).map_err(|error| invalid(C::NAME, error.to_string()))?;
    serde_json::to_value(session.run(command)?).map_err(|error| CommandError::Failed(error.to_string()))
}

fn info<C: Command>() -> CommandInfo {
    CommandInfo {
        name: C::NAME,
        description: C::DESCRIPTION,
        input_schema: schemars::schema_for!(C).to_value(),
        output_schema: schemars::schema_for!(C::Output).to_value(),
    }
}

// A single list drives dispatch, schemas, and MCP tools, so the interfaces cannot drift apart.
macro_rules! registry {
    ($($command:ty),+ $(,)?) => {
        fn registry() -> Vec<CommandInfo> {
            vec![$(info::<$command>()),+]
        }

        fn dispatch(session: &mut Session, name: &str, input: serde_json::Value) -> Result<serde_json::Value, CommandError> {
            match name {
                $(<$command>::NAME => execute_typed::<$command>(session, input),)+
                _ => Err(CommandError::Unknown(name.into())),
            }
        }
    };
}

registry!(
    screens::OpenScreen,
    screens::CloseScreen,
    screens::ListScreens,
    screens::SetScreenPage,
    screens::SetLayout,
    screens::ConfigureScreen,
    screens::ConfigureMonitor,
    screens::EditElement,
    screens::MoveElement,
    screens::AddElement,
    screens::RemoveElement,
    screens::UndoLayout,
    screens::RedoLayout,
    screens::SaveLayout,
    LessonList,
    LessonStart,
    GetLessonDebrief,
    LessonFinish,
    InstructorInstrumentFailure,
    InstructorAttitude,
    SetCrewMode,
    CrewHandover,
    CrewIntent,
    GetCrewState,
    ScenarioValidate,
    ScenarioStart,
    ScenarioAmend,
    GetScenarioState,
    ScenarioSignal,
    CommsAtc,
    EmergencyRecord,
    InstructorFailure,
    IssueIfr,
    GetAtcState,
    ConfigureAtc,
    SetLightGun,
    CommsCallsign,
    CommsTune,
    CommsAudioPanel,
    CommsTransmit,
    CommsListen,
    CommsFacilities,
    SetTraffic,
    FlightSave,
    FlightRecords,
    FlightLoad,
    FlightExport,
    FlightEvents,
    SetEquipment,
    SetSimulationDate,
    SearchQuery,
    NearestNavdata,
    PinsList,
    PinsAdd,
    PinsUpdate,
    PinsRemove,
    PinsImport,
    PinsExport,
    CurrentLocation,
    GeocodeLookup,
    SetMap,
    GetMap,
    PlanNew,
    PlanAdd,
    PlanInsert,
    PlanMove,
    PlanRemove,
    PlanSetAltitude,
    SummaryPlan,
    ProfilePlan,
    PlanSave,
    PlanLoad,
    PlanImport,
    PlanExport,
    PlanActivate,
    Reset,
    WorldHeight,
    ListWorlds,
    ListAirspace,
    AirspaceAt,
    SetAirspaceView,
    SetControls,
    Step,
    SetAutopilot,
    AutopilotOff,
    RunUntil,
    GetState,
    SetWeather,
    GetWeather,
    SetHood,
    SetSystems,
    SetAvionics,
    GetAvionics,
    SetFailure,
    ClearFailure,
    Pause,
    Resume,
    SetRate,
    ListCommands,
    ListAircraft,
    LoadAircraft,
    ComputeWeightBalance,
    SetCamera,
    AppScreenshot,
    Wait,
    AirportQuery,
    Geodetic,
    EfbBroadcast,
    EfbGetStatus,
    SetAssist,
    AssistOff,
    GetAutopilot
);

/// Query the selected world's elevation, upward NED normal, and surface.
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorldHeight {
    #[schemars(range(min = -100_000_000.0, max = 100_000_000.0))]
    pub north_m: f64,
    #[schemars(range(min = -100_000_000.0, max = 100_000_000.0))]
    pub east_m: f64,
}
impl Command for WorldHeight {
    const NAME: &'static str = "world.height";
    const DESCRIPTION: &'static str =
        "Read terrain height MSL in metres, upward NED unit normal, and surface at a local point.";
    type Output = flightcraft_world::Ground;
    fn run(self, session: &mut Session) -> Result<Self::Output, CommandError> {
        range(Self::NAME, "north_m", self.north_m, -1e8, 1e8)?;
        range(Self::NAME, "east_m", self.east_m, -1e8, 1e8)?;
        Ok(session.sim.world().ground(self.north_m, self.east_m))
    }
}

pub use flightcraft_world::MAX_LANDMARK_BYTES;
