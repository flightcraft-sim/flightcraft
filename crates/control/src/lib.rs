//! Everything is a command: each has input and output schemas, and none opens a dialog.
//! The session owns a simulation; the MCP server and the UI both drive it through commands.
//!
//! Two ways in, same code path: typed ([`Session::run`] with a [`Command`]) for Rust callers such
//! as the UI, and JSON ([`Session::execute`]) for MCP and scripts. Both go through the registry.

mod holds;
mod systems;
pub use systems::*;
pub mod mcp;
pub use holds::*;

use flightcraft_sim::{Controls, Readout, Sim, SimError, Start, State};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// Ten simulated minutes per explicit step call, at 120 Hz.
pub const MAX_STEPS: u32 = 120 * 600;

/// A command: a name, a typed input, and a typed output, both with JSON schemas.
pub trait Command: DeserializeOwned + schemars::JsonSchema {
    /// Dotted name, for example `sim.step`.
    const NAME: &'static str;
    /// One sentence for the command list and MCP tool description.
    const DESCRIPTION: &'static str;
    type Output: Serialize + schemars::JsonSchema;
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
    Sim(#[from] SimError),
    #[error("{0}")]
    Failed(String),
}

/// One flight session: the simulation and the settings commands change.
#[derive(Debug)]
pub struct Session {
    sim: Sim,
    paused: bool,
    rate: f64,
}

impl Session {
    /// A session parked on the runway.
    pub fn new() -> Result<Self, CommandError> {
        Ok(Self {
            sim: Sim::new(&Start::Runway)?,
            paused: true,
            rate: 1.0,
        })
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
        command.run(self)
    }

    /// Run a command by name with JSON input; the output is JSON.
    pub fn execute(
        &mut self,
        name: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, CommandError> {
        dispatch(self, name, input)
    }

    /// Every registered command with its schemas.
    #[must_use]
    pub fn commands() -> Vec<CommandInfo> {
        registry()
    }

    fn snapshot(&self) -> StateOutput {
        StateOutput {
            state: *self.sim.state(),
            readout: self.sim.readout(),
            paused: self.paused,
            rate: self.rate,
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
    pub state: State,
    pub readout: Readout,
    pub paused: bool,
    pub rate: f64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Playback {
    pub paused: bool,
    /// Dimensionless real-time multiplier.
    pub rate: f64,
}

/// Start on the default runway or in the air above its threshold.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResetStart {
    Runway {},
    Air {
        #[schemars(range(min = 0))]
        height_m: f64,
        #[schemars(range(min = 0))]
        airspeed_mps: f64,
        #[schemars(range(min = 0, max = std::f64::consts::TAU))]
        heading_rad: f64,
    },
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Reset {
    pub start: ResetStart,
    /// Omission starts in calm air. Supplying weather makes its seed part of the start.
    #[serde(default, deserialize_with = "provided")]
    #[schemars(with = "flightcraft_sim::Weather", transform = without_default)]
    pub weather: Option<flightcraft_sim::Weather>,
    /// Defaults to paused. The existing rate is retained across resets.
    #[serde(default, deserialize_with = "provided")]
    #[schemars(with = "bool", extend("default" = true))]
    pub paused: Option<bool>,
}

impl Command for Reset {
    const NAME: &'static str = "sim.reset";
    const DESCRIPTION: &'static str = "Start on the runway or in the air, in SI units.";
    type Output = StateOutput;

    fn run(self, session: &mut Session) -> Result<Self::Output, CommandError> {
        let start = match self.start {
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
        session.sim = Sim::new_with_weather(&start, self.weather.unwrap_or_default())?;
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
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetControls {
    #[serde(default, deserialize_with = "provided")]
    #[schemars(with = "f64", range(min = -1, max = 1), transform = without_default)]
    pub elevator: Option<f64>,
    #[serde(default, deserialize_with = "provided")]
    #[schemars(with = "f64", range(min = -1, max = 1), transform = without_default)]
    pub aileron: Option<f64>,
    #[serde(default, deserialize_with = "provided")]
    #[schemars(with = "f64", range(min = -1, max = 1), transform = without_default)]
    pub rudder: Option<f64>,
    #[serde(default, deserialize_with = "provided")]
    #[schemars(with = "f64", range(min = 0, max = 1), transform = without_default)]
    pub throttle: Option<f64>,
    #[serde(default, deserialize_with = "provided")]
    #[schemars(with = "f64", range(min = 0, max = 1), transform = without_default)]
    pub flaps: Option<f64>,
    #[serde(default, deserialize_with = "provided")]
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

#[derive(Debug, Deserialize, JsonSchema)]
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
        session.sim.step(self.steps);
        Ok(session.sim.readout())
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
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

#[derive(Debug, Deserialize, JsonSchema)]
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

#[derive(Debug, Deserialize, JsonSchema)]
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

#[derive(Debug, Deserialize, JsonSchema)]
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
        session.rate = self.rate;
        Ok(session.settings())
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
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
    Reset,
    SetControls,
    Step,
    SetAutopilot,
    AutopilotOff,
    RunUntil,
    GetState,
    SetWeather,
    SetFailure,
    ClearFailure,
    Pause,
    Resume,
    SetRate,
    ListCommands
);
