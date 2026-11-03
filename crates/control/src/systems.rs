//! Weather and failure commands share the same validated state in every frontend.
use crate::{Command, CommandError, Session};
use flightcraft_sim::{ControlSurface, Failures, Weather};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, JsonSchema)]
pub struct Systems {
    pub weather: Weather,
    pub conditions: flightcraft_world::Conditions,
    pub hood: bool,
    pub simulated_instrument_time: flightcraft_sim::InstrumentTime,
    pub failures: Failures,
    pub radio_failed: bool,
    pub systems: flightcraft_sim::systems::SystemsState,
}

fn current(session: &Session) -> Systems {
    Systems {
        weather: session.pilot_weather(),
        conditions: session.sim().conditions().clone(),
        hood: session.sim().hood(),
        simulated_instrument_time: session.sim().instrument_time(),
        failures: session.sim().failures(),
        radio_failed: session.atc_state().radio_failed,
        systems: session.sim().systems().clone(),
    }
}

impl Session {
    /// Pilot meteorological wind direction is magnetic; the simulator's wind field is grid based.
    pub fn pilot_weather(&self) -> Weather {
        let mut weather = self.sim().weather();
        weather.wind_from_deg = self
            .sim()
            .heading_reference()
            .grid_to_magnetic(weather.wind_from_deg.to_radians())
            .to_degrees();
        weather
    }
    pub fn set_pilot_weather(&mut self, mut weather: Weather) -> Result<(), CommandError> {
        weather.validate().map_err(flightcraft_sim::SimError::from)?;
        weather.wind_from_deg = self
            .sim()
            .heading_reference()
            .magnetic_to_grid(weather.wind_from_deg.to_radians())
            .to_degrees();
        self.sim_mut().set_weather(weather)?;
        Ok(())
    }
}
/// Visual weather patches retain wind; a supplied wind group must be complete.
#[derive(Debug, Default, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetWeather {
    #[serde(default, deserialize_with = "crate::provided")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "f64", transform = crate::without_default, range(min = -60, max = 60))]
    pub temperature_c: Option<f64>,
    #[serde(default, deserialize_with = "crate::provided")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "f64", transform = crate::without_default, range(min = -100, max = 60))]
    pub dewpoint_c: Option<f64>,
    #[serde(default, deserialize_with = "crate::provided")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "f64", transform = crate::without_default, range(min = 80000, max = 110000))]
    pub sea_level_pressure_pa: Option<f64>,
    #[serde(default, deserialize_with = "crate::provided")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "f64", transform = crate::without_default)]
    #[schemars(range(min = 0, max = 360))]
    pub wind_from_deg: Option<f64>,
    #[serde(default, deserialize_with = "crate::provided")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "f64", transform = crate::without_default)]
    #[schemars(range(min = 0, max = 75))]
    pub wind_speed_mps: Option<f64>,
    #[serde(default, deserialize_with = "crate::provided")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "bool", transform = crate::without_default)]
    pub shear: Option<bool>,
    #[serde(default, deserialize_with = "crate::provided")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "flightcraft_world::TurbulenceLevel", transform = crate::without_default)]
    pub turbulence: Option<flightcraft_world::TurbulenceLevel>,
    #[serde(default, deserialize_with = "crate::provided")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "u64", transform = crate::without_default)]
    pub seed: Option<u64>,
    #[serde(default, deserialize_with = "crate::provided")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "flightcraft_world::WeatherPreset", transform = crate::without_default)]
    pub preset: Option<flightcraft_world::WeatherPreset>,
    #[schemars(length(max = 3))]
    #[serde(default, deserialize_with = "crate::provided")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Vec<flightcraft_world::CloudLayer>", transform = crate::without_default)]
    pub layers: Option<Vec<flightcraft_world::CloudLayer>>,
    #[schemars(range(min = 0.125, max = 100))]
    #[serde(default, deserialize_with = "crate::provided")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "f64", transform = crate::without_default)]
    pub visibility_sm: Option<f64>,
    #[serde(default, deserialize_with = "crate::provided")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "flightcraft_world::Obscuration", transform = crate::without_default)]
    pub obscuration: Option<flightcraft_world::Obscuration>,
    #[serde(default, deserialize_with = "crate::provided")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "flightcraft_world::Precipitation", transform = crate::without_default)]
    pub precipitation: Option<flightcraft_world::Precipitation>,
    #[serde(default, deserialize_with = "crate::provided")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "flightcraft_world::TimeOfDay", transform = crate::without_default)]
    pub time_of_day: Option<flightcraft_world::TimeOfDay>,
}
impl Command for SetWeather {
    const NAME: &'static str = "weather.set";
    const DESCRIPTION: &'static str = "Set cloud layers (MSL ft), visibility (SM), obscuration, precipitation, time of day, or preset; optional complete magnetic wind group.";
    type Output = Systems;
    fn run(self, session: &mut Session) -> Result<Systems, CommandError> {
        let mut conditions = self
            .preset
            .map(flightcraft_world::Conditions::preset)
            .unwrap_or_else(|| session.sim().conditions().clone());
        if let Some(v) = self.temperature_c {
            conditions.temperature_c = v;
        }
        if let Some(v) = self.dewpoint_c {
            conditions.dewpoint_c = v;
        }
        if let Some(v) = self.sea_level_pressure_pa {
            conditions.sea_level_pressure_pa = v;
        }
        if let Some(v) = self.layers {
            conditions.layers = v;
        }
        if let Some(v) = self.visibility_sm {
            conditions.visibility_sm = v;
        }
        if let Some(v) = self.obscuration {
            conditions.obscuration = v;
        }
        if let Some(v) = self.precipitation {
            conditions.precipitation = v;
        }
        if let Some(v) = self.time_of_day {
            conditions.time_of_day = v;
        }
        conditions.validate().map_err(flightcraft_sim::SimError::from)?;
        let has_wind = self.wind_from_deg.is_some()
            || self.wind_speed_mps.is_some()
            || self.shear.is_some()
            || self.turbulence.is_some()
            || self.seed.is_some();
        if has_wind {
            let (Some(wind_from_deg), Some(wind_speed_mps), Some(shear), Some(turbulence), Some(seed)) = (
                self.wind_from_deg,
                self.wind_speed_mps,
                self.shear,
                self.turbulence,
                self.seed,
            ) else {
                return Err(crate::invalid(Self::NAME, "supply all five wind fields together"));
            };
            session.set_pilot_weather(Weather {
                wind_from_deg,
                wind_speed_mps,
                shear,
                turbulence,
                seed,
            })?;
        }
        session.sim_mut().set_conditions(conditions)?;
        Ok(current(session))
    }
}

#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct GetWeather {}
impl Command for GetWeather {
    const NAME: &'static str = "weather.state";
    const DESCRIPTION: &'static str =
        "Read visual weather, wind, turbulence, hood, and simulated instrument practice totals.";
    type Output = Systems;
    fn run(self, session: &mut Session) -> Result<Systems, CommandError> {
        Ok(current(session))
    }
}
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetHood {
    pub enabled: bool,
}
impl Command for SetHood {
    const NAME: &'static str = "view.hood";
    const DESCRIPTION: &'static str =
        "Enable or remove the cockpit view-limiting hood; available to offline scenarios.";
    type Output = Systems;
    fn run(self, session: &mut Session) -> Result<Systems, CommandError> {
        if self.enabled
            && let Some(camera) = session.camera.as_mut()
        {
            camera.mode = crate::CameraMode::Cockpit;
        }
        session.sim_mut().set_hood(self.enabled);
        Ok(current(session))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Failure {
    EngineFire {},
    ElectricalFire {},
    CabinSmoke {},
    DoorOpen {},
    CarbIcing {},
    AirframeIcing {},
    FlapAsymmetry {},
    TrimRunaway {},
    TrimStuck {},
    #[serde(alias = "radio")]
    RadioFailure {},

    Engine {},
    VacuumPump {},
    Alternator {},
    PitotBlocked {},
    StaticBlocked {},
    PartialPower {
        #[schemars(range(min = 0, max = 1))]
        fraction: f64,
    },
    Jam {
        surface: ControlSurface,
        /// Fixed normalised deflection; flaps require 0..=1.
        #[schemars(range(min = -1, max = 1))]
        position: f64,
    },
}

#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetFailure {
    pub failure: Failure,
}

impl Command for SetFailure {
    const NAME: &'static str = "failure.set";
    const DESCRIPTION: &'static str = "Inject an aircraft failure, including fires, icing, trim, COM radios, power loss, or a jammed surface.";
    type Output = Systems;
    fn run(self, session: &mut Session) -> Result<Systems, CommandError> {
        let mut failures = session.sim().failures();
        match self.failure {
            Failure::EngineFire {} => failures.engine_fire = true,
            Failure::ElectricalFire {} => failures.electrical_fire = true,
            Failure::CabinSmoke {} => failures.cabin_smoke = true,
            Failure::DoorOpen {} => failures.door_open = true,
            Failure::CarbIcing {} => failures.carb_icing = true,
            Failure::AirframeIcing {} => failures.airframe_icing = true,
            Failure::FlapAsymmetry {} => failures.flap_asymmetry = true,
            Failure::TrimRunaway {} => failures.trim_runaway = true,
            Failure::TrimStuck {} => failures.trim_stuck = true,
            Failure::RadioFailure {} => failures.radio_failure = true,

            Failure::VacuumPump {} => failures.vacuum_pump = true,
            Failure::Alternator {} => failures.alternator = true,
            Failure::PitotBlocked {} => failures.pitot_blocked = true,
            Failure::StaticBlocked {} => failures.static_blocked = true,
            Failure::Engine {} => failures.engine_power_fraction = 0.0,
            Failure::PartialPower { fraction } => failures.engine_power_fraction = fraction,
            Failure::Jam { surface, position } => failures
                .jam(surface, position)
                .map_err(flightcraft_sim::SimError::from)?,
        }
        session.sim_mut().set_failures(failures)?;
        session.sync_radio_failure();
        Ok(current(session))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FailureTarget {
    EngineFire,
    ElectricalFire,
    CabinSmoke,
    DoorOpen,
    CarbIcing,
    AirframeIcing,
    FlapAsymmetry,
    TrimRunaway,
    TrimStuck,
    #[serde(alias = "radio")]
    RadioFailure,

    #[default]
    All,
    VacuumPump,
    Alternator,
    PitotBlocked,
    StaticBlocked,
    Engine,
    Elevator,
    Aileron,
    Rudder,
    Flaps,
}

#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClearFailure {
    #[serde(default)]
    pub target: FailureTarget,
}

impl Command for ClearFailure {
    const NAME: &'static str = "failure.clear";
    const DESCRIPTION: &'static str =
        "Restore a system or engine, release a surface jam, or clear all failures (the default).";
    type Output = Systems;
    fn run(self, session: &mut Session) -> Result<Systems, CommandError> {
        let mut f = session.sim().failures();
        match self.target {
            FailureTarget::EngineFire => f.engine_fire = false,
            FailureTarget::ElectricalFire => f.electrical_fire = false,
            FailureTarget::CabinSmoke => f.cabin_smoke = false,
            FailureTarget::DoorOpen => f.door_open = false,
            FailureTarget::CarbIcing => f.carb_icing = false,
            FailureTarget::AirframeIcing => f.airframe_icing = false,
            FailureTarget::FlapAsymmetry => f.flap_asymmetry = false,
            FailureTarget::TrimRunaway => f.trim_runaway = false,
            FailureTarget::TrimStuck => f.trim_stuck = false,
            FailureTarget::RadioFailure => f.radio_failure = false,

            FailureTarget::All => f = Failures::default(),
            FailureTarget::VacuumPump => f.vacuum_pump = false,
            FailureTarget::Alternator => f.alternator = false,
            FailureTarget::PitotBlocked => f.pitot_blocked = false,
            FailureTarget::StaticBlocked => f.static_blocked = false,
            FailureTarget::Engine => f.engine_power_fraction = 1.0,
            FailureTarget::Elevator => f.elevator = None,
            FailureTarget::Aileron => f.aileron = None,
            FailureTarget::Rudder => f.rudder = None,
            FailureTarget::Flaps => f.flaps = None,
        }
        session.sim_mut().set_failures(f)?;
        session.sync_radio_failure();
        Ok(current(session))
    }
}

/// A switch, selector, knob, breaker, or ground-servicing action.
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetSystems {
    pub setting: flightcraft_sim::systems::SystemSetting,
}
impl Command for SetSystems {
    const NAME: &'static str = "systems.set";
    const DESCRIPTION: &'static str = "Set an aircraft switch, selector, mixture, primer, altimeter, breaker, load fault, tank quantity, or local icing environment.";
    type Output = Systems;
    fn run(self, session: &mut Session) -> Result<Systems, CommandError> {
        session.sim_mut().set_systems(self.setting)?;
        Ok(current(session))
    }
}
