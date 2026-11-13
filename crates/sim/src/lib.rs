//! Deterministic, headless 6-DOF flight simulation.
//!
//! See the crate README for start envelopes, trim, contact, crash, and replay semantics.
//!
//! Frames: the world frame is local north-east-down (NED) with its origin at the threshold of
//! runway 36 on the default airfield; the body frame is x forward, y right, z down. Units are SI
//! throughout (metres, seconds, radians, kilograms, newtons); conversions to knots and feet live in
//! the presentation layers only.
//!
//! Determinism: a fixed timestep ([`DT_S`]), no wall clock, no threads, no hash maps in the
//! stepping path, and transcendental maths only through `libm`. The same inputs give the same bits
//! on every platform, checked by [`Sim::state_hash`] in the golden tests.

#![deny(clippy::disallowed_methods)]

pub mod autopilot;
pub mod avionics;
pub mod canopy;
mod dynamics;
pub mod equipment;
pub mod hazards;
#[cfg(test)]
mod hazards_tests;
pub use avionics::navigation as glass_navigation;
pub use avionics::sensor as glass_air_data;
pub mod magnetic;
mod math;
pub mod modern;
#[cfg(test)]
mod modern_tests;
pub mod nav_source;
pub mod systems;
#[cfg(test)]
mod systems_tests;

pub use flightcraft_aero::{Aircraft, AircraftError, ControlSurface, Failures};
use flightcraft_world as world;
pub use flightcraft_world::{TurbulenceLevel, Weather, World, WorldChoice};
use math::*;
use serde::{Deserialize, Serialize};

/// The fixed simulation timestep: 1/120 s.
pub const DT_S: f64 = 1.0 / 120.0;

/// Pilot controls. Each value is clamped to its range when set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Controls {
    /// Elevator, -1 (nose down) to 1 (nose up).
    pub elevator: f64,
    /// Aileron, -1 (roll left) to 1 (roll right).
    pub aileron: f64,
    /// Rudder, -1 (yaw left) to 1 (yaw right).
    pub rudder: f64,
    /// Throttle, 0 to 1.
    pub throttle: f64,
    /// Flaps, 0 (up) to 1 (fully down).
    pub flaps: f64,
    /// Wheel brakes, 0 to 1.
    pub brake: f64,
}

impl Controls {
    /// The same controls with every value clamped to its range; a non-finite value becomes 0.
    #[must_use]
    pub fn clamped(self) -> Self {
        fn c(v: f64, lo: f64, hi: f64) -> f64 {
            if v.is_finite() { v.clamp(lo, hi) } else { 0.0 }
        }
        Self {
            elevator: c(self.elevator, -1.0, 1.0),
            aileron: c(self.aileron, -1.0, 1.0),
            rudder: c(self.rudder, -1.0, 1.0),
            throttle: c(self.throttle, 0.0, 1.0),
            flaps: c(self.flaps, 0.0, 1.0),
            brake: c(self.brake, 0.0, 1.0),
        }
    }
}

/// The full rigid-body state.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct State {
    /// Simulated time since the start, seconds.
    pub time_s: f64,
    /// Position of the centre of gravity in the world (NED) frame, metres.
    pub position_m: [f64; 3],
    /// Velocity in the world (NED) frame, m/s.
    pub velocity_mps: [f64; 3],
    /// Attitude, body to world, as a unit quaternion `[w, x, y, z]`.
    pub attitude: [f64; 4],
    /// Angular rate in the body frame `[p, q, r]`, rad/s.
    pub angular_rate_rps: [f64; 3],
}

impl Default for State {
    fn default() -> Self {
        Self {
            time_s: 0.0,
            position_m: [0.0; 3],
            velocity_mps: [0.0; 3],
            attitude: [1.0, 0.0, 0.0, 0.0],
            angular_rate_rps: [0.0; 3],
        }
    }
}

/// Derived values for instruments, agents, and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Readout {
    pub latitude_deg: Option<f64>,
    pub longitude_deg: Option<f64>,
    pub time_s: f64,
    /// Indicated airspeed, m/s.
    pub indicated_airspeed_mps: f64,
    /// True airspeed, m/s.
    pub true_airspeed_mps: f64,
    /// Scenario sea-level pressure (QNH), used by pressure instruments.
    pub sea_level_pressure_pa: f64,
    /// Ground speed, m/s.
    pub ground_speed_mps: f64,
    /// Altitude above mean sea level, metres.
    pub altitude_msl_m: f64,
    /// Height of the lowest wheel above the ground, metres.
    pub height_agl_m: f64,
    /// Climb rate, m/s (positive up).
    pub vertical_speed_mps: f64,
    /// Pilot magnetic heading, radians, clockwise from magnetic north.
    pub heading_rad: f64,
    /// Internal local-grid heading; use only for geometry and truth transports.
    pub heading_grid_rad: f64,
    pub heading_true_rad: f64,
    pub magnetic_declination_rad: f64,
    pub grid_north_true_rad: f64,
    pub pitch_rad: f64,
    pub roll_rad: f64,
    /// Heading rate about the local vertical, rad/s, positive right.
    /// Zero within 1e-6 rad of vertical pitch, where heading is undefined.
    pub turn_rate_rps: f64,
    /// Aerodynamic specific force along body y, g, positive right.
    /// A slip ball moves opposite this force; coordinated flight reads zero.
    pub lateral_acceleration_g: f64,
    /// Angle of attack, radians.
    pub alpha_rad: f64,
    /// Sideslip, radians.
    pub beta_rad: f64,
    /// Load factor along body z, g (1 in level flight).
    pub load_factor_g: f64,
    /// Engine speed, revolutions per minute.
    pub engine_rpm: f64,
    /// True while any wheel touches the ground.
    pub on_ground: bool,
    /// True once the aircraft has hit the ground too hard or off a gear contact.
    pub crashed: bool,
    pub controls: Controls,
    /// Engaged holds and their SI targets.
    pub autopilot: autopilot::Targets,
    /// Pilot weather readout; wind-from direction is magnetic.
    pub weather: Weather,
    /// Total moving-air velocity at the CG, NED m/s.
    pub wind_ned_mps: [f64; 3],
    pub gust_ned_mps: [f64; 3],
    pub failures: Failures,
    /// Actual surface positions after applying jams (throttle remains a demand).
    pub effective_controls: Controls,
}

/// How a simulation starts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Start {
    /// Parked on the runway threshold, engine idling, pointing down the runway.
    Runway,
    /// Ground position off the runway, engine at idle and brakes held.
    ParkedAt { position_m: [f64; 3], heading_rad: f64 },
    /// Explicit threshold and local grid heading, used for region airport starts.
    RunwayAt {
        threshold_ned_m: [f64; 3],
        heading_rad: f64,
    },
    /// Explicit local air position, MSL altitude, and grid heading.
    AirAt {
        position_m: [f64; 3],
        airspeed_mps: f64,
        heading_rad: f64,
    },
    /// In the air: height above the runway threshold (m), airspeed (m/s), and heading (rad).
    Air {
        height_m: f64,
        airspeed_mps: f64,
        heading_rad: f64,
    },
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SimError {
    #[error("invalid systems input: {0}")]
    InvalidSystems(String),
    #[error(transparent)]
    Aircraft(#[from] AircraftError),
    #[error(transparent)]
    Terrain(#[from] world::TerrainError),
    #[error("invalid start: {0}")]
    InvalidStart(String),
    #[error("unknown aircraft: {0}")]
    UnknownAircraft(String),
    #[error("invalid autopilot targets: {0}")]
    InvalidAutopilot(String),
    #[error(transparent)]
    Weather(#[from] world::WeatherError),
    #[error(transparent)]
    Failure(#[from] flightcraft_aero::FailureError),
}

/// Fixed-step flight record totals. These are simulated practice, never FAA logbook credit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct InstrumentTime {
    pub hood_s: f64,
    pub imc_s: f64,
}

/// One simulation: an aircraft, a world, and its state.
#[derive(Clone, Debug)]
pub struct Sim {
    presentation: Option<Readout>,
    aircraft: Aircraft,
    world: World,
    magnetism: magnetic::Magnetism,
    state: State,
    controls: Controls,
    steps: u64,
    crashed: bool,
    autopilot: autopilot::Autopilot,
    equipment: Option<equipment::Equipment>,
    pilot_controls: Controls,
    navigation: Option<nav_source::DirectTo>,
    nav_guidance: Option<nav_source::NavGuidance>,
    receiver_leg: Option<nav_source::ReceiverLeg>,
    procedures: avionics::procedures::ProcedureReceiver,
    radios: avionics::radio::Radios,
    wind: world::Wind,
    conditions: world::Conditions,
    hood: bool,
    instrument_time: InstrumentTime,
    failures: Failures,
    systems: systems::SystemsState,
    systems_model: Option<systems::SystemsModel>,
    systems_changed: bool,
    parked_start: bool,
    glass: avionics::glass::GlassComputers,
    hazards: hazards::GroundHazards,
    tyre: hazards::TyreWear,
}

impl Sim {
    /// Install a display-only snapshot. A presentation must never be stepped.
    pub fn install_presentation(
        &mut self,
        state: State,
        readout: Readout,
        mut systems: systems::SystemsState,
    ) {
        self.state = state;
        self.controls = readout.controls;
        self.pilot_controls = readout.controls;
        self.failures = readout.failures;
        self.crashed = readout.crashed;
        if let (Some(source), Some(target)) = (&self.systems.modern, &mut systems.modern) {
            target.panel_bindings = source.panel_bindings.clone();
            target.wigwag_installed = source.wigwag_installed;
        }
        self.systems = systems;
        self.presentation = Some(readout);
    }

    /// A new simulation of the default aircraft at `start`.
    pub fn new(start: &Start) -> Result<Self, SimError> {
        Self::new_with_weather(start, Weather::default())
    }

    /// Start with weather and its seed. An air start has the requested airspeed;
    /// its ground velocity includes the initial mean wind. Gust filters start at zero.
    pub fn new_with_weather(start: &Start, weather: Weather) -> Result<Self, SimError> {
        Self::new_with_aircraft_and_weather(start, Aircraft::corvid()?, weather)
    }

    /// Start with a validated custom aircraft and calm weather.
    pub fn new_with_aircraft(start: &Start, aircraft: Aircraft) -> Result<Self, SimError> {
        Self::new_with_aircraft_and_weather(start, aircraft, Weather::default())
    }

    pub fn new_with_aircraft_and_weather(
        start: &Start,
        aircraft: Aircraft,
        weather: Weather,
    ) -> Result<Self, SimError> {
        Self::new_with_aircraft_weather_world(start, aircraft, weather, World::default())
    }

    pub fn new_with_world(start: &Start, world: World) -> Result<Self, SimError> {
        Self::new_with_aircraft_weather_world(start, Aircraft::corvid()?, Weather::default(), world)
    }

    pub fn new_with_aircraft_weather_world(
        start: &Start,
        aircraft: Aircraft,
        weather: Weather,
        world: World,
    ) -> Result<Self, SimError> {
        aircraft.validate()?;
        let wind = world::Wind::new(weather)?;
        let mut state = State::default();
        let controls = match *start {
            Start::Runway | Start::RunwayAt { .. } | Start::ParkedAt { .. } => {
                // Unequal wheel heights encode a tail-down (or nose-down) parked attitude.
                // Equal-height gear retains the original level initialization exactly.
                let pitch = dynamics::runway_pitch(&aircraft);
                let mut runway = world.runway();
                if let Start::RunwayAt {
                    threshold_ned_m,
                    heading_rad,
                } = *start
                {
                    if threshold_ned_m.iter().any(|v| !v.is_finite()) || !heading_rad.is_finite() {
                        return Err(SimError::InvalidStart("non-finite runway start".into()));
                    }
                    runway.threshold_ned_m = threshold_ned_m;
                    runway.heading_deg = heading_rad.to_degrees();
                }
                if let Start::ParkedAt {
                    position_m,
                    heading_rad,
                } = *start
                {
                    if position_m.iter().any(|v| !v.is_finite() || v.abs() > 1e8) || !heading_rad.is_finite()
                    {
                        return Err(SimError::InvalidStart(
                            "invalid parked position or heading".into(),
                        ));
                    }
                    runway.threshold_ned_m = position_m;
                    runway.heading_deg = heading_rad.to_degrees();
                }
                let heading = runway.heading_deg.to_radians().rem_euclid(core::f64::consts::TAU);
                if pitch != 0.0 || heading != 0.0 {
                    state.attitude = attitude(pitch, heading);
                }
                state.position_m = [
                    0.0,
                    0.0,
                    -(aircraft
                        .geometry
                        .gear
                        .iter()
                        .map(|g| {
                            let point = aircraft.relative_point(g.point_m);
                            let [_, _, z] = if pitch == 0.0 {
                                point
                            } else {
                                rotate(state.attitude, point)
                            };
                            z
                        })
                        .fold(0.0, f64::max)
                        - aircraft.mass.mass_kg * world::GRAVITY_MPS2
                            / aircraft
                                .geometry
                                .gear
                                .iter()
                                .map(|g| g.stiffness_n_m)
                                .sum::<f64>()),
                ];
                state.position_m[0] = runway.threshold_ned_m[0];
                state.position_m[1] = runway.threshold_ned_m[1];
                state.position_m[2] += runway.threshold_ned_m[2];
                if matches!(start, Start::ParkedAt { .. }) {
                    Controls {
                        throttle: 0.,
                        brake: 1.,
                        ..Controls::default()
                    }
                } else {
                    Controls::default()
                }
            }
            Start::Air {
                height_m,
                airspeed_mps,
                heading_rad,
            } => {
                let (initial, controls) = air_state(&aircraft, height_m, airspeed_mps, heading_rad)?;
                state = initial;
                controls
            }
            Start::AirAt {
                position_m,
                airspeed_mps,
                heading_rad,
            } => {
                let [north, east, down] = position_m;
                if !north.is_finite()
                    || !east.is_finite()
                    || north.abs() > 250_000.0
                    || east.abs() > 250_000.0
                {
                    return Err(SimError::InvalidStart(
                        "air position outside region bounds".into(),
                    ));
                }
                let (initial, controls) = air_state(&aircraft, -down, airspeed_mps, heading_rad)?;
                state = initial;
                state.position_m = position_m;
                controls
            }
        };
        if matches!(start, Start::Air { .. } | Start::AirAt { .. })
            && wind.velocity_mps(state.position_m) != [0.0; 3]
        {
            state.velocity_mps = add(state.velocity_mps, wind.velocity_mps(state.position_m));
        }
        if !dynamics::valid(&state) {
            return Err(SimError::InvalidStart(
                "aircraft initialization outside finite simulation envelope".into(),
            ));
        }
        let magnetism = magnetic::Magnetism::new(&world, state)?;
        let mut sim = Self {
            aircraft,
            world,
            magnetism,
            presentation: None,
            conditions: world::Conditions::default(),
            hood: false,
            instrument_time: InstrumentTime::default(),
            wind,
            failures: Failures::default(),
            systems: systems::SystemsState::default(),
            systems_model: None,
            glass: avionics::glass::GlassComputers::default(),
            systems_changed: false,
            parked_start: matches!(start, Start::ParkedAt { .. }),
            hazards: hazards::GroundHazards::default(),
            tyre: hazards::TyreWear::default(),
            state,
            controls,
            steps: 0,
            crashed: false,
            autopilot: autopilot::Autopilot::default(),
            equipment: None,
            pilot_controls: controls,
            navigation: None,
            nav_guidance: None,
            receiver_leg: None,
            procedures: Default::default(),
            radios: Default::default(),
        };
        sim.controls = sim.limit_controls(sim.controls);
        sim.configure_builtin_systems()?;
        Ok(sim)
    }

    /// Restore a finite state for experiments and replay. Quaternion is normalised.
    /// Time must be nonnegative, positions within 100,000 km, body rates within 50 rad/s,
    /// and each velocity component within 500 m/s. Out-of-envelope integration freezes
    /// the last valid state and marks the simulation crashed.
    pub fn from_state(state: State, controls: Controls) -> Result<Self, SimError> {
        Self::from_state_in_world(state, controls, World::default())
    }

    pub fn from_state_in_world(mut state: State, controls: Controls, world: World) -> Result<Self, SimError> {
        if !dynamics::valid(&state) || state.time_s > 1e12 {
            return Err(SimError::InvalidStart(
                "state outside finite simulation envelope".into(),
            ));
        }
        let [w, x, y, z] = state.attitude;
        if w * w + x * x + y * y + z * z < 1e-12 {
            return Err(SimError::InvalidStart("zero attitude quaternion".into()));
        }
        state.attitude = normalise(state.attitude);
        let steps = libm::round(state.time_s / DT_S) as u64;
        state.time_s = steps as f64 * DT_S;
        let aircraft = Aircraft::corvid()?;
        let crashed = dynamics::crash(&aircraft, &state, &world);
        let magnetism = magnetic::Magnetism::new(&world, state)?;
        let mut sim = Self {
            aircraft,
            world,
            magnetism,
            presentation: None,
            conditions: world::Conditions::default(),
            hood: false,
            instrument_time: InstrumentTime::default(),
            wind: world::Wind::new(Weather::default())?,
            failures: Failures::default(),
            systems: systems::SystemsState::default(),
            systems_model: None,
            glass: avionics::glass::GlassComputers::default(),
            systems_changed: false,
            parked_start: false,
            hazards: hazards::GroundHazards::default(),
            tyre: hazards::TyreWear::default(),
            state,
            controls: controls.clamped(),
            steps,
            crashed,
            autopilot: autopilot::Autopilot::default(),
            equipment: None,
            pilot_controls: controls,
            navigation: None,
            nav_guidance: None,
            receiver_leg: None,
            procedures: Default::default(),
            radios: Default::default(),
        };
        sim.configure_builtin_systems()?;
        Ok(sim)
    }

    fn configure_builtin_systems(&mut self) -> Result<(), SimError> {
        let name = match self.aircraft.identity.id.as_str() {
            "corvid" | "corvid-standard" => "corvid",
            "larrow" | "larrow-standard" => "larrow",
            _ => {
                self.systems.ideal_sensors(self.readout());
                return Ok(());
            }
        };
        let configuration = flightcraft_aero::AircraftCatalog::builtins()?.resolve_aircraft(name)?;
        self.configure_systems(&configuration)
    }

    /// Install the resolved systems and station fuel for a newly created flight.
    /// Legacy files with placeholder systems retain their original flight behavior.
    pub fn configure_systems(
        &mut self,
        configuration: &flightcraft_aero::ResolvedAircraft,
    ) -> Result<(), SimError> {
        let mut readout = self.readout();
        readout.engine_rpm = readout.engine_rpm / self.systems_model.as_ref().map_or(1., |m| m.gearbox_ratio)
            * configuration.engine.gearbox_ratio;
        let (state, model) = systems::SystemsState::configure(configuration, readout)?;
        let equipment = configuration
            .panel
            .autopilot
            .clone()
            .map(|installation| equipment::Equipment::new(installation, &state, self.controls))
            .transpose()?;
        self.equipment = equipment;
        self.pilot_controls = self.controls;
        self.navigation = None;
        self.nav_guidance = None;
        self.receiver_leg = None;
        self.procedures = Default::default();
        self.radios = Default::default();
        self.systems = state;
        self.systems.canopy = canopy::CanopyState::new(
            configuration
                .installed_systems
                .definition
                .as_ref()
                .is_some_and(|d| d.sliding_canopy),
            self.parked_start,
        );
        self.systems.avionics = avionics::AvionicsState::configure(configuration);
        self.systems_model = model;
        self.systems_changed = false;
        self.glass = avionics::glass::GlassComputers::default();
        self.refresh_glass_computers();
        self.refresh_avionics(false);
        Ok(())
    }

    /// A runway or airborne reset represents completed departure preparation. Power-cycle and
    /// cold instrument installation still use their normal fixed-step alignment delay.
    pub fn prepare_departure(&mut self) {
        self.glass.primary.sensor.align(self.state);
        self.glass.standby.sensor.align(self.state);
        self.systems
            .avionics
            .electronic_attitude
            .computer
            .sensor
            .align(self.state);
        self.systems
            .avionics
            .electronic_hsi
            .reversion_attitude
            .computer
            .sensor
            .align(self.state);
        self.refresh_glass_computers();
        self.refresh_avionics(false);
        self.refresh_equipment_sensors();
        // A pilot sets the heading bug to the departure heading; a bug left on 000 reads as a fault.
        let degrees = libm::round(self.readout().heading_rad.to_degrees()).rem_euclid(360.);
        if self.set_heading_bug(degrees).is_err() {
            self.systems.controls.heading_bug_rad = 0.;
        }
    }

    #[must_use]
    /// Electrical state of an exterior light, including its switch, breaker, and bus.
    pub fn light_powered(&self, switch: flightcraft_aero::LoadSwitch) -> bool {
        let s = &self.systems;
        let Some(model) = &self.systems_model else {
            return s.controls.master && s.controls.lights;
        };
        let installed = |kind| model.definition.electrical.loads.iter().any(|l| l.switch == kind);
        let powered = |kind| {
            model
                .definition
                .electrical
                .loads
                .iter()
                .filter(|l| l.switch == kind)
                .any(|l| {
                    s.electrical
                        .loads
                        .iter()
                        .any(|state| state.id == l.id && state.powered)
                })
        };
        if installed(switch) {
            return powered(switch);
        }
        let selected = s
            .modern
            .as_ref()
            .and_then(|m| {
                m.light_switches
                    .iter()
                    .find(|(kind, _)| *kind == switch)
                    .map(|(_, on)| *on)
            })
            .unwrap_or(s.controls.lights);
        selected
            && if installed(flightcraft_aero::LoadSwitch::Lights) {
                powered(flightcraft_aero::LoadSwitch::Lights)
            } else {
                s.electrical.main_bus_voltage > 9.
            }
    }

    pub fn systems(&self) -> &systems::SystemsState {
        &self.systems
    }

    pub fn set_systems(&mut self, setting: systems::SystemSetting) -> Result<(), SimError> {
        if self.canopy().sliding
            && matches!(
                setting,
                systems::SystemSetting::Equipment {
                    setting: modern::ModernSetting::Door { .. }
                }
            )
        {
            return Err(SimError::InvalidSystems(
                "sliding canopy uses aircraft.canopy actions".into(),
            ));
        }
        if let systems::SystemSetting::HeadingBug { heading_rad } = &setting {
            if !heading_rad.is_finite() || !(0. ..=core::f64::consts::TAU).contains(heading_rad) {
                return Err(SimError::InvalidSystems("heading bug outside range".into()));
            }
            return self.set_avionics(avionics::AvionicsSetting::HeadingBug {
                degrees: heading_rad.to_degrees(),
            });
        }
        if let systems::SystemSetting::Throttle { value }
        | systems::SystemSetting::Flaps { value }
        | systems::SystemSetting::WheelBrake { value } = &setting
        {
            if !value.is_finite() || !(0. ..=1.).contains(value) {
                return Err(SimError::InvalidSystems("control outside 0 to 1".into()));
            }
            let mut controls = self.controls();
            match &setting {
                systems::SystemSetting::Throttle { value } => controls.throttle = *value,
                systems::SystemSetting::Flaps { value } => controls.flaps = *value,
                _ => controls.brake = *value,
            }
            self.set_controls(controls);
            return Ok(());
        }
        let previous = self.systems.clone();
        self.systems.set(setting, self.systems_model.as_ref())?;
        self.systems_changed |= self.systems != previous;
        self.refresh_glass_computers();
        Ok(())
    }

    /// Set a unit control through the same typed path used by the command registry.
    pub fn set_avionics(&mut self, setting: avionics::AvionicsSetting) -> Result<(), SimError> {
        let declination_rad = self.heading_reference().declination_rad;
        let clear_procedure = matches!(&setting, avionics::AvionicsSetting::DirectTo { .. });
        match setting {
            avionics::AvionicsSetting::LoadProcedure {
                airport,
                procedure,
                transition,
                vectors_to_final,
            } => {
                self.procedures.load(
                    &self.world,
                    self.state,
                    &airport,
                    &procedure,
                    transition.as_deref(),
                    vectors_to_final,
                )?;
                self.navigation = None;
                self.nav_guidance = None;
            }
            avionics::AvionicsSetting::ApproachService { lnav_only } => self.procedures.lnav_only = lnav_only,
            avionics::AvionicsSetting::ActivateApproach {} => self.procedures.activate(false)?,
            avionics::AvionicsSetting::VectorsToFinal {} => self.procedures.activate(true)?,
            avionics::AvionicsSetting::MissedApproach {} => self.procedures.missed()?,
            avionics::AvionicsSetting::HoldOptions { suspend, exit } => {
                self.procedures.hold(suspend, exit)?
            }
            avionics::AvionicsSetting::ClearProcedure {} => self.procedures.clear(),
            avionics::AvionicsSetting::SequenceLeg {} => self.procedures.sequence()?,
            avionics::AvionicsSetting::NavRadio { setting } => self.radios.set(setting)?,
            avionics::AvionicsSetting::CdiSource { source } => {
                self.radios.source = source;
                self.navigation = None;
                self.nav_guidance = None;
            }
            avionics::AvionicsSetting::Autopilot { setting } => {
                return self.set_autopilot_panel(setting);
            }
            avionics::AvionicsSetting::DirectToLocal { north_m, east_m } => {
                return self.set_direct_to([north_m, east_m]);
            }
            avionics::AvionicsSetting::SelectedAltitude { feet } if self.equipment.is_some() => {
                if !feet.is_finite() || !(-1000.0..=30000.0).contains(&feet) {
                    return Err(SimError::InvalidSystems("selected altitude outside range".into()));
                }
                return self.set_autopilot_panel(equipment::PanelSetting::Altitude {
                    altitude_m: feet * 0.3048,
                });
            }
            avionics::AvionicsSetting::HeadingBug { degrees } => self.set_heading_bug(degrees)?,
            avionics::AvionicsSetting::HeadingBugStep { detents, interval_ms } => {
                if !(-360..=360).contains(&detents) {
                    return Err(SimError::InvalidSystems("heading detents outside range".into()));
                }
                let degrees = self.systems.controls.heading_bug_rad.to_degrees()
                    + f64::from(detents) * avionics::heading_detent_scale(interval_ms);
                self.set_heading_bug(degrees.rem_euclid(360.))?;
            }
            avionics::AvionicsSetting::HeadingBugSync {} => {
                let degrees = libm::round(self.display_heading_degrees()).rem_euclid(360.);
                self.set_heading_bug(degrees)?;
            }
            avionics::AvionicsSetting::Glass { .. } => {
                return Err(SimError::InvalidSystems(
                    "glass settings require the session command handler".into(),
                ));
            }
            setting => self
                .systems
                .avionics
                .set(setting, &self.world, self.state, declination_rad)?,
        }
        if clear_procedure {
            self.procedures.clear();
        }
        self.refresh_avionics(false);
        self.systems_changed = true;
        Ok(())
    }
    /// The single selected heading also drives an engaged heading hold.
    pub fn set_heading_bug(&mut self, degrees: f64) -> Result<(), SimError> {
        if !degrees.is_finite() || !(0. ..=360.).contains(&degrees) {
            return Err(SimError::InvalidSystems(
                "heading bug outside finite 0–360 degrees".into(),
            ));
        }
        let radians = degrees.rem_euclid(360.).to_radians();
        self.systems.controls.heading_bug_rad = radians;
        self.systems.avionics.electronic_hsi.heading_bug_deg = degrees.rem_euclid(360.);
        let mut targets = self.autopilot.targets();
        if targets.heading_rad.is_some() {
            targets.heading_rad = Some(radians);
            self.autopilot.set(targets);
        }
        self.systems_changed = true;
        Ok(())
    }
    fn display_heading_degrees(&self) -> f64 {
        let h = &self.systems.avionics.electronic_hsi.reversion_attitude;
        if self.systems.avionics.installed && h.attitude_valid {
            h.heading_deg
        } else if self.glass.primary.attitude_valid {
            self.glass.primary.heading_rad.to_degrees()
        } else {
            self.systems.sensed.heading_rad.to_degrees()
        }
    }

    fn refresh_avionics(&mut self, advance: bool) {
        let mut avionics = std::mem::take(&mut self.systems.avionics);
        avionics.update(self.state, self.readout(), &self.systems, &self.world, advance);
        if avionics.installed && !self.procedures.loaded.is_empty() {
            let h = &mut avionics.electronic_hsi;
            h.gps_fix = self.procedures.valid;
            h.full_scale_nm = self.procedures.cdi_full_scale_nm;
            h.course_deg = self.procedures.desired_track_rad.to_degrees();
            h.lateral_deviation_nm = self.procedures.valid.then_some(self.procedures.cross_track_nm);
            h.cdi = self.procedures.valid.then_some(
                (self.procedures.cross_track_nm / self.procedures.cdi_full_scale_nm.max(0.001))
                    .clamp(-1., 1.),
            );
            h.distance_nm = self.procedures.valid.then_some(self.procedures.distance_nm);
            h.destination = self
                .procedures
                .legs
                .get(self.procedures.active_leg)
                .and_then(|l| l.leg.fix.as_deref())
                .and_then(|id| self.world.procedures()?.point(id))
                .map(|p| p.ident.clone());
        }
        if avionics.installed
            && self.radios.source == avionics::radio::CdiSource::Nav1
            && let Some(signal) = self.radio_state().first()
        {
            let h = &mut avionics.electronic_hsi;
            h.gps_fix = signal.signal_valid;
            h.destination = signal.ident.clone();
            h.distance_nm = signal.dme_nm;
            h.course_deg = self.radios.receivers.first().map_or(0., |r| r.course_deg);
            h.lateral_deviation_nm = signal.guidance.map(|g| g.cross_track_m / 1852.);
            h.cdi = signal.lateral_dots.map(|dots| -dots / 2.5);
        }
        self.systems.avionics = avionics;
    }

    fn propulsion_failures(&self) -> Failures {
        let mut failures = self.failures();
        if failures.airframe_icing {
            let [_, _, down] = self.state.position_m;
            let altitude_ft = -down / 0.3048;
            let visible = self.conditions.precipitation != world::Precipitation::None
                || self
                    .conditions
                    .layers
                    .iter()
                    .any(|l| altitude_ft >= l.base_ft_msl && altitude_ft <= l.top_ft_msl);
            failures.airframe_icing = visible && self.conditions.temperature_c + down * 0.0065 < 0.0;
        }
        if self.systems.enabled && self.systems.engine_power_fraction != 1.0 {
            failures.engine_power_fraction *= self.systems.engine_power_fraction;
        }
        failures
    }

    #[must_use]
    pub fn heading_reference(&self) -> magnetic::HeadingReference {
        self.magnetism.reference
    }
    pub fn simulation_date(&self) -> world::geomag::SimulationDate {
        self.magnetism.date
    }
    /// Update WMM without advancing sensors or simulation time. The free gyro is not reslaved.
    pub fn set_simulation_date(&mut self, date: world::geomag::SimulationDate) -> Result<(), SimError> {
        date.decimal_year()?;
        self.magnetism.date = date;
        self.magnetism.refresh(&self.world, self.state);
        self.refresh_glass_computers();
        self.refresh_avionics(false);
        Ok(())
    }
    pub fn aircraft(&self) -> &Aircraft {
        &self.aircraft
    }

    #[must_use]
    pub fn world(&self) -> &World {
        &self.world
    }

    /// Read-only visual conditions; independent of forces and seeded wind.
    pub fn conditions(&self) -> &world::Conditions {
        &self.conditions
    }
    /// Validate before atomically replacing visual conditions.
    pub fn set_conditions(&mut self, conditions: world::Conditions) -> Result<(), SimError> {
        conditions.validate()?;
        if conditions.temperature_c != self.conditions.temperature_c
            || conditions.dewpoint_c != self.conditions.dewpoint_c
        {
            let exponent = 17.625 * conditions.dewpoint_c / (243.04 + conditions.dewpoint_c)
                - 17.625 * conditions.temperature_c / (243.04 + conditions.temperature_c);
            self.systems.set(
                systems::SystemSetting::Environment {
                    temperature_c: conditions.temperature_c,
                    relative_humidity: libm::exp(exponent).clamp(0.0, 1.0),
                },
                self.systems_model.as_ref(),
            )?;
        }
        self.conditions = conditions;
        Ok(())
    }
    /// Instructor-only attitude setup. Preserve position, velocity, systems, and fixed-step time.
    pub fn set_training_attitude(&mut self, pitch_rad: f64, bank_rad: f64) -> Result<(), SimError> {
        if !pitch_rad.is_finite()
            || !bank_rad.is_finite()
            || pitch_rad.abs() > 25_f64.to_radians()
            || bank_rad.abs() > 60_f64.to_radians()
        {
            return Err(SimError::InvalidStart("training attitude outside limits".into()));
        }
        let [_, _, heading] = math::euler(self.state.attitude);
        let (s, c) = libm::sincos(bank_rad * 0.5);
        self.state.attitude = math::mul(math::attitude(pitch_rad, heading), [c, s, 0., 0.]);
        self.state.angular_rate_rps = [0.; 3];
        self.glass.primary.sensor.reposition(self.state);
        self.glass.standby.sensor.reposition(self.state);
        self.systems
            .avionics
            .electronic_attitude
            .computer
            .sensor
            .reposition(self.state);
        self.systems
            .avionics
            .electronic_hsi
            .reversion_attitude
            .computer
            .sensor
            .reposition(self.state);
        self.refresh_glass_computers();
        self.refresh_avionics(false);
        self.set_autopilot(Default::default())?;
        Ok(())
    }
    pub fn hood(&self) -> bool {
        self.hood
    }
    pub fn set_hood(&mut self, enabled: bool) {
        self.hood = enabled;
    }
    pub fn instrument_time(&self) -> InstrumentTime {
        self.instrument_time
    }

    /// Replace wind atomically and restart gusts without changing ground velocity.
    pub fn set_weather(&mut self, weather: Weather) -> Result<(), SimError> {
        self.wind = world::Wind::new(weather)?;
        Ok(())
    }

    #[must_use]
    pub fn weather(&self) -> Weather {
        self.wind.weather()
    }

    pub fn set_failures(&mut self, failures: Failures) -> Result<(), SimError> {
        failures.validate()?;
        self.failures = failures;
        self.systems.failures = self.failures();
        if failures != Failures::default() {
            self.systems_changed = true;
        }
        Ok(())
    }

    /// Ground hazards left from a pre-flight: covers, chocks, ropes, and missing hardware.
    #[must_use]
    pub fn hazards(&self) -> hazards::GroundHazards {
        self.hazards
    }

    /// Wear of a damaged tyre: its remaining integrity, and whether it has burst.
    #[must_use]
    pub fn tyre_wear(&self) -> hazards::TyreWear {
        self.tyre
    }

    /// Test hook: burst or restore the damaged tyre without waiting for it to wear out.
    #[doc(hidden)]
    pub fn force_tyre_flat(&mut self, flat: bool) {
        self.tyre.flat = flat;
    }

    pub fn set_hazards(&mut self, hazards: hazards::GroundHazards) {
        let previous = self.hazards;
        self.hazards = hazards;
        if hazards != hazards::GroundHazards::default() {
            self.systems_changed = true;
        }
        if hazards.low_oil != previous.low_oil {
            self.systems
                .set_low_oil(self.systems_model.as_ref(), hazards.low_oil);
        }
        if hazards.damaged_tyre != previous.damaged_tyre {
            // A new tyre finding starts fresh; a repaired tyre is a new one.
            self.tyre = hazards::TyreWear::default();
        }
        self.systems.failures = self.failures();
    }

    pub fn injected_failures(&self) -> Failures {
        self.failures
    }

    #[must_use]
    pub fn failures(&self) -> Failures {
        let mut failures = self.failures;
        failures.door_open |= self.systems.canopy.sliding
            && !self.systems.canopy.latched
            && dynamics::height(&self.aircraft, &self.state, &self.world) > 0.;
        if self.hazards != hazards::GroundHazards::default() {
            let airborne = dynamics::height(&self.aircraft, &self.state, &self.world) > 0.;
            self.hazards.apply(&mut failures, airborne);
        }
        failures.tyre_flat |= self.tyre.flat;
        failures.cowl_separated |= self.systems.cowl.separated;
        failures
    }

    #[must_use]
    fn trimmed_controls(&self) -> Controls {
        let mut controls = self.controls;
        if self.systems.controls.park_brake {
            controls.brake = 1.0;
        }
        if self.systems.controls.trim != 0.0 {
            controls.elevator = (controls.elevator + self.systems.controls.trim).clamp(-1.0, 1.0);
        }
        controls
    }
    pub fn effective_controls(&self) -> Controls {
        let c = self.aircraft.limit_controls(
            self.failures
                .apply(dynamics::aero_controls(self.trimmed_controls())),
        );
        Controls {
            elevator: c.elevator,
            aileron: c.aileron,
            rudder: c.rudder,
            flaps: c.flaps,
            ..self.controls
        }
    }

    /// Set the controls (clamped). They hold until set again.
    pub fn set_controls(&mut self, controls: Controls) {
        self.controls = self.limit_controls(controls);
        self.pilot_controls = self.controls;
    }

    /// Hold an air start's trimmed elevator with the pitch trim, the way a pilot flies hands off:
    /// the stick is then free, and centering it (a released key) leaves the aircraft in trim.
    pub fn move_elevator_to_trim(&mut self) {
        let trim = self.controls.elevator.clamp(-0.3, 0.3);
        self.systems.controls.trim = trim;
        self.controls.elevator -= trim;
        self.pilot_controls.elevator -= trim;
    }

    fn limit_controls(&self, controls: Controls) -> Controls {
        let controls = controls.clamped();
        let c = self.aircraft.limit_controls(dynamics::aero_controls(controls));
        let [lo, hi] = self.aircraft.systems.brake;
        Controls {
            elevator: c.elevator,
            aileron: c.aileron,
            rudder: c.rudder,
            throttle: c.throttle,
            flaps: c.flaps,
            brake: controls.brake.clamp(lo, hi),
        }
    }

    #[must_use]
    pub fn controls(&self) -> Controls {
        if self.equipment.is_some() {
            self.pilot_controls
        } else {
            self.controls
        }
    }

    /// Replace the engaged holds atomically. Incompatible or hostile targets are errors.
    pub fn set_autopilot(&mut self, targets: autopilot::Targets) -> Result<(), SimError> {
        targets.validate()?;
        if targets.engaged() && self.equipment.as_ref().is_some_and(|e| e.state.engaged) {
            return Err(SimError::InvalidAutopilot(
                "disconnect installed autopilot before engaging assists".into(),
            ));
        }
        if let Some(heading) = targets.heading_rad {
            self.systems_changed |=
                self.systems.controls.heading_bug_rad != heading.rem_euclid(core::f64::consts::TAU);
            self.systems.controls.heading_bug_rad = heading.rem_euclid(core::f64::consts::TAU);
            self.systems.avionics.electronic_hsi.heading_bug_deg =
                self.systems.controls.heading_bug_rad.to_degrees();
        }
        self.autopilot.set(targets);
        Ok(())
    }

    /// Installed equipment state. The default panels have no autopilot installed.
    #[must_use]
    pub fn installed_autopilot(&self) -> Option<&equipment::EquipmentState> {
        self.equipment.as_ref().map(|e| &e.state)
    }

    pub fn set_autopilot_panel(&mut self, setting: equipment::PanelSetting) -> Result<(), SimError> {
        self.refresh_equipment_sensors();
        if self.autopilot.targets().engaged()
            && matches!(
                setting,
                equipment::PanelSetting::Engage { on: true } | equipment::PanelSetting::Level
            )
        {
            return Err(SimError::InvalidAutopilot(
                "disengage assists before coupling installed autopilot".into(),
            ));
        }
        self.equipment
            .as_mut()
            .ok_or_else(|| SimError::InvalidAutopilot("no autopilot installed".into()))?
            .set(setting, &self.systems, self.pilot_controls)?;
        if let Some(ap) = &self.equipment {
            let feet = ap.state.altitude_m / 0.3048;
            self.systems.avionics.electronic_attitude.selected_altitude_ft = feet;
            self.systems
                .avionics
                .electronic_hsi
                .reversion_attitude
                .selected_altitude_ft = feet;
        }
        Ok(())
    }

    /// Attach an electronic suite's sensed indications at its fixed-step boundary.
    /// The host refreshes the snapshot every step; None restores the aircraft systems' sensors.
    pub fn set_autopilot_sensors(
        &mut self,
        sensors: Option<systems::SensedInstruments>,
    ) -> Result<(), SimError> {
        self.equipment
            .as_mut()
            .ok_or_else(|| SimError::InvalidAutopilot("no autopilot installed".into()))?
            .set_sensors(sensors)
    }

    /// Receiver adapters supply guidance at fixed-step boundaries. None flags signal loss.
    pub fn set_nav_guidance(&mut self, guidance: Option<nav_source::NavGuidance>) -> Result<(), SimError> {
        if let Some(n) = guidance {
            n.validate()?;
        }
        self.navigation = None;
        self.nav_guidance = guidance;
        Ok(())
    }

    /// Offline direct-to fallback for panels without an installed navigation receiver.
    pub fn set_direct_to(&mut self, destination: [f64; 2]) -> Result<(), SimError> {
        let [n, e, _] = self.state.position_m;
        let source = nav_source::DirectTo::new([n, e], destination)?;
        self.navigation = Some(source);
        self.procedures.clear();
        self.nav_guidance = None;
        Ok(())
    }

    pub fn procedure_receiver(&self) -> &avionics::procedures::ProcedureReceiver {
        &self.procedures
    }
    pub fn radios(&self) -> &avionics::radio::Radios {
        &self.radios
    }
    pub fn radio_state(&self) -> [avionics::radio::RadioState; 2] {
        self.radios.sample_on_date(
            &self.world,
            self.state,
            self.navigation_powered(),
            self.simulation_date(),
        )
    }
    fn navigation_powered(&self) -> bool {
        if self
            .systems_model
            .as_ref()
            .is_some_and(|m| m.navigator_load.is_some())
        {
            self.navigator_powered()
        } else {
            self.systems.controls.avionics && self.systems.electrical.main_bus_voltage > 9.0
        }
    }
    fn navigator_powered(&self) -> bool {
        self.systems_model
            .as_ref()
            .and_then(|m| m.navigator_load.as_ref())
            .is_some_and(|id| {
                self.systems
                    .electrical
                    .loads
                    .iter()
                    .any(|l| &l.id == id && l.powered)
            })
    }
    fn procedure_powered(&self) -> bool {
        self.navigator_powered()
            || self.systems.electrical.loads.iter().any(|l| {
                (l.id == "glass_mfd" && !self.glass.mfd_failed
                    || l.id == "electronic_hsi"
                        && self
                            .systems
                            .avionics
                            .electronic_hsi
                            .reversion_attitude
                            .power
                            .switch_on)
                    && l.powered
            })
    }
    /// The flight-plan receiver supplies its active leg; geodesy remains in avionics.
    pub fn set_receiver_leg(&mut self, leg: Option<nav_source::ReceiverLeg>) {
        self.receiver_leg = leg;
    }
    /// Give the active leg a vertical navigation path (or take it away). Nothing happens without
    /// a leg.
    pub fn set_receiver_vnav(&mut self, vnav: Option<avionics::vnav::LegPath>) {
        if let Some(leg) = &mut self.receiver_leg {
            leg.vnav = vnav;
        }
    }
    /// The active leg's vertical path, if it has one.
    pub fn receiver_vnav(&self) -> Option<avionics::vnav::LegPath> {
        self.receiver_leg.and_then(|leg| leg.vnav)
    }

    fn refresh_equipment_sensors(&mut self) {
        let Some(ap) = &self.equipment else {
            return;
        };
        if ap.state.installation.family != equipment::AutopilotFamily::DigitalTwoAxis {
            return;
        }
        let load = ap.state.installation.roll_sensor_load.as_str();
        let mut sensed = self.systems.sensed;
        let valid = if load == "glass_ahrs" {
            let a = &self.glass.primary;
            sensed.roll_rad = a.roll_rad;
            sensed.pitch_rad = a.pitch_rad;
            sensed.heading_rad = a.heading_rad;
            sensed.indicated_airspeed_mps = a.indicated_airspeed_mps;
            sensed.altitude_msl_m = a.altitude_msl_m;
            sensed.vertical_speed_mps = a.vertical_speed_mps;
            sensed.attitude_valid = a.attitude_valid;
            Some(a.air_data_valid)
        } else if load == "electronic_attitude" || load == "electronic_hsi" {
            let a = if load == "electronic_attitude" {
                &self.systems.avionics.electronic_attitude
            } else {
                &self.systems.avionics.electronic_hsi.reversion_attitude
            };
            sensed.roll_rad = a.roll_deg.to_radians();
            sensed.pitch_rad = a.pitch_deg.to_radians();
            sensed.heading_rad = a.heading_deg.to_radians();
            sensed.indicated_airspeed_mps = a.airspeed_kt / 1.9438444924406;
            sensed.altitude_msl_m = a.altitude_ft * 0.3048;
            sensed.vertical_speed_mps = a.vertical_speed_fpm * 0.00508;
            sensed.attitude_valid = a.attitude_valid;
            Some(a.air_data_valid)
        } else {
            None
        };
        if let (Some(valid), Some(ap)) = (valid, &mut self.equipment) {
            let _ = ap.set_sensors(Some(sensed));
            ap.set_receiver_air_data_valid(valid);
        }
    }

    /// Disengage all holds. The last controls remain available to the pilot.
    pub fn autopilot_off(&mut self) {
        self.autopilot = autopilot::Autopilot::default();
    }

    pub fn refresh_glass_computers(&mut self) {
        let truth = self.readout();
        self.glass.step(&self.systems, self.state, truth, false);
    }
    pub fn glass_computers(&self) -> &avionics::glass::GlassComputers {
        &self.glass
    }
    pub fn glass_computers_mut(&mut self) -> &mut avionics::glass::GlassComputers {
        &mut self.glass
    }

    /// Drive guidance from any receiver at each fixed-step boundary, including test doubles.
    pub fn step_with_nav_source(&mut self, n: u32, source: &impl nav_source::NavSource) {
        for _ in 0..n {
            if self.crashed {
                break;
            }
            let [north, east, _] = self.state.position_m;
            self.navigation = None;
            self.nav_guidance = source.guidance(north, east).filter(|n| n.validate().is_ok());
            self.step(1);
        }
    }

    /// Advance up to `n` fixed steps of [`DT_S`], stopping at a crash or numerical envelope exit.
    /// The last finite state is retained. Crashed simulations remain frozen.
    pub fn step(&mut self, n: u32) {
        if self.presentation.is_some() {
            return;
        }
        for _ in 0..n {
            if self.crashed {
                break;
            }
            if self.autopilot.targets().engaged() {
                let readout = self.readout();
                self.controls = self
                    .autopilot
                    .update(&self.aircraft, readout, self.state.angular_rate_rps);
            }
            let procedure_powered = self.procedure_powered();
            let [wn, we, _] = self.readout().wind_ned_mps;
            let reference = self.heading_reference();
            self.procedures
                .step_in_wind(&self.world, self.state, procedure_powered, [wn, we], reference);
            self.refresh_equipment_sensors();
            if self.equipment.is_some() && !self.autopilot.targets().engaged() {
                use nav_source::NavSource;
                let [north, east, _] = self.state.position_m;
                let [vn, ve, _] = self.state.velocity_mps;
                let radio_selected = self.radios.source == avionics::radio::CdiSource::Nav1;
                let procedure_selected = !self.procedures.loaded.is_empty();
                let nav = if radio_selected {
                    self.radio_state().first().and_then(|r| r.guidance)
                } else if procedure_selected {
                    self.procedures.guidance
                } else {
                    self.navigation
                        .as_ref()
                        .and_then(|n| n.guidance(north, east))
                        .or(self.nav_guidance)
                        .or_else(|| {
                            let powered = self
                                .systems
                                .electrical
                                .loads
                                .iter()
                                .any(|l| l.id == "glass_mfd" && l.powered);
                            if self.navigator_powered() || (powered && !self.glass.mfd_failed) {
                                self.receiver_leg.as_ref().and_then(|leg| {
                                    if leg.vnav.is_some() {
                                        // The vertical path is judged against the indicated altitude.
                                        let altitude = if self.glass.primary.air_data_valid {
                                            self.glass.primary.altitude_msl_m
                                        } else {
                                            self.readout().altitude_msl_m
                                        };
                                        leg.guidance_at(north, east, altitude)
                                    } else {
                                        leg.guidance(north, east)
                                    }
                                })
                            } else {
                                None
                            }
                        })
                        .or_else(|| {
                            self.systems
                                .avionics
                                .electronic_hsi
                                .nav_guidance(&self.world, self.state)
                        })
                };
                let reference = self.heading_reference();
                let nav = nav.map(|mut n| {
                    n.desired_track_rad = reference.grid_to_magnetic(n.desired_track_rad);
                    n
                });
                let r = self.readout();
                if let Some(equipment) = &mut self.equipment {
                    self.controls = equipment.update(
                        &self.systems,
                        self.pilot_controls,
                        reference.grid_to_magnetic(libm::atan2(ve, vn)),
                        r.ground_speed_mps,
                        r.load_factor_g,
                        nav,
                    );
                }
            }
            self.controls = self.limit_controls(self.controls);
            let relative = dynamics::relative_velocity(&self.state, &self.wind);
            // Advance exactly once; substages and observational reads never consume noise.
            if self.wind.advance(DT_S, relative).is_err() {
                self.crashed = true;
                break;
            }
            let recovery_motion = self.recovery_motion();
            let Some(mut next) = recovery_motion.or_else(|| {
                dynamics::rk4(
                    &self.aircraft,
                    self.state,
                    self.trimmed_controls(),
                    &self.wind,
                    self.propulsion_failures(),
                    &self.world,
                )
            }) else {
                self.crashed = true;
                break;
            };
            if !dynamics::valid(&next) {
                self.crashed = true;
                break;
            }
            if self.hood {
                self.instrument_time.hood_s += DT_S;
            }
            if self.conditions.imc(self.state.position_m) {
                self.instrument_time.imc_s += DT_S;
            }
            self.steps = self.steps.saturating_add(1);
            next.time_s = self.steps as f64 * DT_S;
            self.crashed = dynamics::crash(&self.aircraft, &next, &self.world)
                || dynamics::hard_touchdown(&self.aircraft, &self.state, &next, &self.world);
            if recovery_motion.is_some()
                && self.crashed
                && let Some(p) = self.systems.modern.as_mut().and_then(|s| s.recovery.as_mut())
            {
                p.phase = modern::RecoveryPhase::Impact;
                let [_, _, down] = next.velocity_mps;
                p.impact_vertical_speed_mps = Some(down);
            }
            if self.hazards.restrained()
                && dynamics::height(&self.aircraft, &next, &self.world) <= hazards::RESTRAINT_HEIGHT_M
            {
                // Chocks and tie-down ropes hold the aircraft where it stands.
                next.velocity_mps[0] = 0.;
                next.velocity_mps[1] = 0.;
                next.angular_rate_rps[2] = 0.;
                if self.hazards.tie_downs {
                    next.velocity_mps[2] = next.velocity_mps[2].max(0.);
                }
            }
            if self.hazards.damaged_tyre.is_some() && !self.tyre.flat {
                let before = dynamics::height(&self.aircraft, &self.state, &self.world);
                let after = dynamics::height(&self.aircraft, &next, &self.world);
                if after <= hazards::TYRE_LOADED_HEIGHT_M {
                    let touchdown =
                        (before > hazards::TYRE_LOADED_HEIGHT_M).then_some(self.state.velocity_mps[2]);
                    let [vn, ve, _] = next.velocity_mps;
                    self.tyre.step(DT_S, libm::hypot(vn, ve), touchdown);
                }
            }
            self.state = next;
            self.magnetism.refresh(&self.world, self.state);
            if let Some(model) = &self.systems_model {
                let readout = self.readout();
                self.systems.step(model, readout, self.failures());
                self.glass.step(&self.systems, self.state, readout, true);
                let mut load = model.load.clone();
                // Tanks can share a station. Sum them in declaration order, once per step.
                for station in model
                    .variant
                    .stations
                    .iter()
                    .filter(|s| s.kind == flightcraft_aero::StationKind::Fuel)
                {
                    let mass_kg = self
                        .systems
                        .tanks
                        .iter()
                        .zip(&model.definition.tanks)
                        .filter(|(_, tank)| tank.station == station.id)
                        .map(|(tank, _)| tank.quantity_kg)
                        .sum();
                    if let Some(entry) = load.stations.iter_mut().find(|e| e.station == station.id) {
                        entry.mass_kg = mass_kg;
                    } else {
                        load.stations.push(flightcraft_aero::StationLoad {
                            station: station.id.clone(),
                            mass_kg,
                        });
                    }
                }
                match model.variant.loaded_mass(&load) {
                    Ok(mass) => self.aircraft.mass = mass,
                    Err(_) => {
                        self.crashed = true;
                        break;
                    }
                }
            } else {
                self.systems.ideal_sensors(self.readout());
            }
            self.refresh_avionics(true);
        }
    }

    fn recovery_motion(&self) -> Option<State> {
        let model = self.systems_model.as_ref()?;
        let p = self.systems.modern.as_ref()?.recovery.as_ref()?;
        let d = model.definition.modern.as_ref()?.parachute.as_ref()?;
        if p.inflation <= 0. {
            return None;
        }
        let mut next = self.state;
        let f = p.inflation;
        let blend = if f >= 1. {
            1.
        } else {
            (DT_S / d.inflation_s).min(1.)
        };
        for (v, target) in next.velocity_mps.iter_mut().zip([0., 0., d.descent_rate_mps]) {
            *v += (target - *v) * blend;
        }
        for (pos, v) in next.position_m.iter_mut().zip(next.velocity_mps) {
            *pos += v * DT_S;
        }
        let [_, pitch, heading] = math::euler(self.state.attitude);
        next.attitude = math::attitude(pitch + (d.hanging_pitch_rad - pitch) * blend, heading);
        next.angular_rate_rps = [0.; 3];
        Some(next)
    }

    #[must_use]
    pub fn state(&self) -> &State {
        &self.state
    }

    #[must_use]
    pub fn steps(&self) -> u64 {
        self.steps
    }

    #[must_use]
    pub fn readout(&self) -> Readout {
        if let Some(readout) = self.presentation {
            return readout;
        }
        let s = &self.state;
        let [_, _, z] = s.position_m;
        let [vn, ve, vd] = s.velocity_mps;
        let [roll, pitch, heading] = euler(s.attitude);
        let [u, v, w] = dynamics::airflow(s, &self.wind).velocity_body_mps;
        let speed = norm(dynamics::relative_velocity(s, &self.wind));
        let loads = dynamics::loads(
            &self.aircraft,
            s,
            self.trimmed_controls(),
            &self.wind,
            self.propulsion_failures(),
        );
        let [_, fy, fz] = loads.force_n;
        let [_, q, r] = s.angular_rate_rps;
        let cos_pitch = libm::cos(pitch);
        let h = dynamics::height(&self.aircraft, s, &self.world);
        let geodetic = self.world.geodetic(s.position_m[0], s.position_m[1]);
        let mut weather = self.weather();
        weather.wind_from_deg = self
            .heading_reference()
            .grid_to_magnetic(weather.wind_from_deg.to_radians())
            .to_degrees();
        Readout {
            latitude_deg: geodetic.map(|[lat, _]| lat),
            longitude_deg: geodetic.map(|[_, lon]| lon),
            time_s: s.time_s,
            indicated_airspeed_mps: speed
                * libm::sqrt(world::atmosphere(-z).density_kg_m3 / world::SEA_LEVEL_DENSITY),
            true_airspeed_mps: speed,
            sea_level_pressure_pa: self.conditions.sea_level_pressure_pa,
            ground_speed_mps: libm::sqrt(vn * vn + ve * ve),
            altitude_msl_m: -z,
            height_agl_m: h.max(0.0),
            vertical_speed_mps: -vd,
            heading_rad: self.heading_reference().grid_to_magnetic(heading),
            heading_grid_rad: heading,
            heading_true_rad: (heading + self.heading_reference().grid_north_true_rad)
                .rem_euclid(core::f64::consts::TAU),
            magnetic_declination_rad: self.heading_reference().declination_rad,
            grid_north_true_rad: self.heading_reference().grid_north_true_rad,
            pitch_rad: pitch,
            roll_rad: roll,
            turn_rate_rps: if cos_pitch.abs() > 1e-6 {
                (q * libm::sin(roll) + r * libm::cos(roll)) / cos_pitch
            } else {
                0.0
            },
            lateral_acceleration_g: fy / (self.aircraft.mass.mass_kg * world::GRAVITY_MPS2),
            alpha_rad: libm::atan2(w, u),
            beta_rad: if speed > 0.01 {
                libm::asin((v / speed).clamp(-1.0, 1.0))
            } else {
                0.0
            },
            load_factor_g: -fz / (self.aircraft.mass.mass_kg * world::GRAVITY_MPS2),
            engine_rpm: loads.engine_rpm * self.systems_model.as_ref().map_or(1., |m| m.gearbox_ratio),
            on_ground: h <= 0.0,
            crashed: self.crashed,
            controls: self.controls,
            autopilot: self.autopilot.targets(),
            weather,
            wind_ned_mps: self.wind.velocity_mps(s.position_m),
            gust_ned_mps: self.wind.gust_mps(),
            failures: self.failures(),
            effective_controls: self.effective_controls(),
        }
    }

    /// FNV-1a over rigid-body state, crash flag, and step count, plus nondefault
    /// weather/process memory, failures, and operated systems. Equal on every platform
    /// for the same inputs, including mass changes from fuel burn.
    #[must_use]
    pub fn state_hash(&self) -> u64 {
        let s = &self.state;
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |v: u64| {
            for b in v.to_le_bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        eat(self.steps);
        eat(u64::from(self.crashed));
        for v in [s.time_s]
            .iter()
            .chain(&s.position_m)
            .chain(&s.velocity_mps)
            .chain(&s.attitude)
            .chain(&s.angular_rate_rps)
        {
            eat(v.to_bits());
        }
        if self.conditions != world::Conditions::default()
            || self.hood
            || self.instrument_time != InstrumentTime::default()
        {
            eat(0x636c_6f75_6473_0001);
            for word in self.conditions.hash_words() {
                eat(word);
            }
            eat(u64::from(self.hood));
            eat(self.instrument_time.hood_s.to_bits());
            eat(self.instrument_time.imc_s.to_bits());
        }
        // Preserve the original hash stream for the default calm, healthy scenario.
        let weather = self.weather();
        if weather != Weather::default() || self.failures != Failures::default() {
            eat(0x7765_6174_6865_7201);
            eat(weather.wind_from_deg.to_bits());
            eat(weather.wind_speed_mps.to_bits());
            eat(u64::from(weather.shear));
            eat(match weather.turbulence {
                TurbulenceLevel::Off => 0,
                TurbulenceLevel::Light => 1,
                TurbulenceLevel::Moderate => 2,
                TurbulenceLevel::Severe => 3,
            });
            eat(weather.seed);
            for word in self.wind.hash_words() {
                eat(word);
            }
            for word in self.failures.emergency_hash_words() {
                eat(word);
            }
            eat(self.failures.engine_power_fraction.to_bits());
            for jam in [
                self.failures.elevator,
                self.failures.aileron,
                self.failures.rudder,
                self.failures.flaps,
            ] {
                eat(u64::from(jam.is_some()));
                if let Some(position) = jam {
                    eat(position.to_bits());
                }
            }
        }
        if self.hazards != hazards::GroundHazards::default() {
            eat(0x6861_7a61_7264_0001);
            for word in self.hazards.hash_words() {
                eat(word);
            }
            if self.hazards.damaged_tyre.is_some() {
                eat(self.tyre.integrity.to_bits());
                eat(u64::from(self.tyre.flat));
            }
        }
        if self.systems_changed
            || self.systems.avionics.installed
            || self.systems.modern.is_some()
            || self.systems.canopy.sliding
        {
            eat(0x7379_7374_656d_7301);
            for word in self
                .systems
                .hash_words()
                .into_iter()
                .chain(if self.systems.avionics.installed {
                    self.systems.avionics.hash_words()
                } else {
                    Vec::new()
                })
            {
                eat(word);
            }
        }
        if self.systems.electrical.loads.iter().any(|l| l.id == "glass_ahrs") {
            eat(0x676c_6173_7300_0001);
            for word in self.glass.hash_words() {
                eat(word);
            }
        }
        if let Some(equipment) = &self.equipment {
            eat(0x6175_746f_7069_6c01);
            if let Ok(bytes) = serde_json::to_vec(&(
                equipment,
                self.pilot_controls,
                self.navigation,
                self.nav_guidance,
                self.receiver_leg,
                &self.procedures,
                &self.radios,
            )) {
                eat(bytes.len() as u64);
                for byte in bytes {
                    eat(u64::from(byte));
                }
            }
        }
        if self.world.region().is_some() || self.simulation_date() != world::geomag::SimulationDate::default()
        {
            eat(0x6d61_676e_6574_6963);
            let date = self.simulation_date();
            eat(u64::from(date.year));
            eat(u64::from(date.month));
            eat(u64::from(date.day));
        }
        if !self.world.tiles().is_empty() {
            eat(0x7465_7272_6169_6e01);
            eat(self.world.fingerprint());
        }
        h
    }
}

/// Shared deterministic trim for absolute and airport-relative air starts.
fn air_state(
    aircraft: &Aircraft,
    height_m: f64,
    airspeed_mps: f64,
    heading_rad: f64,
) -> Result<(State, Controls), SimError> {
    if !height_m.is_finite()
        || !(2.0..=20000.0).contains(&height_m)
        || !airspeed_mps.is_finite()
        || !(1.0..=150.0).contains(&airspeed_mps)
        || !heading_rad.is_finite()
    {
        return Err(SimError::InvalidStart(
            "finite height in 2..=20000 m and speed in 1..=150 m/s required".into(),
        ));
    }
    let heading = heading_rad.rem_euclid(core::f64::consts::TAU);
    let (alpha, controls) = dynamics::trim(aircraft, height_m, airspeed_mps);
    Ok((
        State {
            position_m: [0.0, 0.0, -height_m],
            velocity_mps: [
                airspeed_mps * libm::cos(heading),
                airspeed_mps * libm::sin(heading),
                0.0,
            ],
            attitude: attitude(alpha, heading),
            ..State::default()
        },
        controls,
    ))
}
