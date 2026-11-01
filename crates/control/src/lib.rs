//! Everything is a command: each has input and output schemas, and none opens a dialog.
//! The session owns a simulation; the MCP server and the UI both drive it through commands.
//!
//! Two ways in, same code path: typed ([`Session::run`] with a [`Command`]) for Rust callers such
//! as the UI, and JSON ([`Session::execute`]) for MCP and scripts. Both go through the registry.

use flightcraft_sim::{Sim, SimError, Start};
use serde::{Serialize, de::DeserializeOwned};

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
#[derive(Clone, Debug, Serialize)]
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
}

impl Session {
    /// A session parked on the runway.
    pub fn new() -> Result<Self, CommandError> {
        Ok(Self {
            sim: Sim::new(&Start::Runway)?,
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

    /// Run a typed command.
    pub fn run<C: Command>(&mut self, command: C) -> Result<C::Output, CommandError> {
        command.run(self)
    }

    /// Run a command by name with JSON input; the output is JSON.
    pub fn execute(
        &mut self,
        name: &str,
        _input: serde_json::Value,
    ) -> Result<serde_json::Value, CommandError> {
        Err(CommandError::Unknown(name.to_string()))
    }

    /// Every registered command with its schemas.
    #[must_use]
    pub fn commands() -> Vec<CommandInfo> {
        Vec::new()
    }
}
