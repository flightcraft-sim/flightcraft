//! Weather and failure commands share the same validated state in every frontend.
use crate::{Command, CommandError, Session};
use flightcraft_sim::{ControlSurface, Failures, Weather};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, JsonSchema)]
pub struct Systems {
    pub weather: Weather,
    pub failures: Failures,
}

fn current(session: &Session) -> Systems {
    Systems {
        weather: session.sim().weather(),
        failures: session.sim().failures(),
    }
}

/// Full weather replacement; restarts the gust filter at this seed without changing velocity.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetWeather {
    #[serde(flatten)]
    pub weather: Weather,
}

impl Command for SetWeather {
    const NAME: &'static str = "weather.set";
    const DESCRIPTION: &'static str =
        "Replace wind (from degrees, m/s at 10 m), shear, turbulence, and seed; restart gusts.";
    type Output = Systems;
    fn run(self, session: &mut Session) -> Result<Systems, CommandError> {
        session.sim_mut().set_weather(self.weather)?;
        Ok(current(session))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Failure {
    Engine {},
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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetFailure {
    pub failure: Failure,
}

impl Command for SetFailure {
    const NAME: &'static str = "failure.set";
    const DESCRIPTION: &'static str =
        "Fail the engine, set a remaining power fraction, or jam a surface at a fixed deflection.";
    type Output = Systems;
    fn run(self, session: &mut Session) -> Result<Systems, CommandError> {
        let mut failures = session.sim().failures();
        match self.failure {
            Failure::Engine {} => failures.engine_power_fraction = 0.0,
            Failure::PartialPower { fraction } => failures.engine_power_fraction = fraction,
            Failure::Jam { surface, position } => failures
                .jam(surface, position)
                .map_err(flightcraft_sim::SimError::from)?,
        }
        session.sim_mut().set_failures(failures)?;
        Ok(current(session))
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FailureTarget {
    #[default]
    All,
    Engine,
    Elevator,
    Aileron,
    Rudder,
    Flaps,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClearFailure {
    #[serde(default)]
    pub target: FailureTarget,
}

impl Command for ClearFailure {
    const NAME: &'static str = "failure.clear";
    const DESCRIPTION: &'static str =
        "Restore the engine, release a surface jam, or clear all failures (the default).";
    type Output = Systems;
    fn run(self, session: &mut Session) -> Result<Systems, CommandError> {
        let mut f = session.sim().failures();
        match self.target {
            FailureTarget::All => f = Failures::default(),
            FailureTarget::Engine => f.engine_power_fraction = 1.0,
            FailureTarget::Elevator => f.elevator = None,
            FailureTarget::Aileron => f.aileron = None,
            FailureTarget::Rudder => f.rudder = None,
            FailureTarget::Flaps => f.flaps = None,
        }
        session.sim_mut().set_failures(f)?;
        Ok(current(session))
    }
}
