//! Aircraft selection is staged until reset, or applied atomically with a runway reset.
use crate::{Command, CommandError, Session};
#[cfg(not(target_arch = "wasm32"))]
use flightcraft_aero::MAX_AIRCRAFT_BYTES;
use flightcraft_aero::{Aircraft, Identity};
use flightcraft_sim::{Sim, Start};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListAircraft {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AircraftList {
    pub aircraft: Vec<Identity>,
}

impl Command for ListAircraft {
    const NAME: &'static str = "aircraft.list";
    const DESCRIPTION: &'static str = "List built-in aircraft identities.";
    type Output = AircraftList;
    fn run(self, _session: &mut Session) -> Result<Self::Output, CommandError> {
        Ok(AircraftList {
            aircraft: vec![
                Aircraft::corvid()
                    .map_err(flightcraft_sim::SimError::from)?
                    .identity,
            ],
        })
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AircraftSource {
    Json {
        text: String,
    },
    #[cfg(not(target_arch = "wasm32"))]
    File {
        path: String,
    },
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LoadAircraft {
    pub source: AircraftSource,
    /// False stages selection for sim.reset; true resets immediately on the runway, paused.
    #[serde(default)]
    pub reset: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AircraftLoaded {
    pub aircraft: Identity,
    pub active_aircraft_id: String,
    /// True when this command also reset the simulation.
    pub reset: bool,
}

impl Command for LoadAircraft {
    const NAME: &'static str = "aircraft.load";
    const DESCRIPTION: &'static str =
        "Load aircraft JSON (or a native file), staging for sim.reset or resetting now.";
    type Output = AircraftLoaded;
    fn run(self, session: &mut Session) -> Result<Self::Output, CommandError> {
        let invalid = |message| CommandError::InvalidInput {
            command: Self::NAME.into(),
            message,
        };
        let text = match self.source {
            AircraftSource::Json { text } => text,
            #[cfg(not(target_arch = "wasm32"))]
            AircraftSource::File { path } => {
                use std::io::Read;
                let file = std::fs::File::open(path).map_err(|e| invalid(format!("source.path: {e}")))?;
                if !file
                    .metadata()
                    .map_err(|e| invalid(format!("source.path: {e}")))?
                    .is_file()
                {
                    return Err(invalid("source.path: requires a regular file".into()));
                }
                // Bound the read itself: metadata alone is vulnerable to a growing file.
                let mut bytes = Vec::new();
                file.take(MAX_AIRCRAFT_BYTES as u64 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| invalid(format!("source.path: {e}")))?;
                if bytes.len() > MAX_AIRCRAFT_BYTES {
                    return Err(invalid("file_size: aircraft exceeds 1 MiB".into()));
                }
                String::from_utf8(bytes).map_err(|e| invalid(format!("source.path: requires UTF-8: {e}")))?
            }
        };
        let aircraft = Aircraft::from_json(&text).map_err(|e| invalid(e.to_string()))?;
        let replacement = if self.reset {
            Some(Sim::new_with_aircraft_and_weather(
                &Start::Runway,
                aircraft.clone(),
                session.sim.weather(),
            )?)
        } else {
            None
        };
        if let Some(sim) = replacement {
            session.sim = sim;
            session.paused = true;
            session.flight_generation = session.flight_generation.wrapping_add(1);
        }
        let output = AircraftLoaded {
            aircraft: aircraft.identity.clone(),
            active_aircraft_id: session.sim.aircraft().identity.id.clone(),
            reset: self.reset,
        };
        session.selected_aircraft = aircraft;
        Ok(output)
    }
}
