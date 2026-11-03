//! Aircraft selection is staged until reset, or applied atomically with a runway reset.
use crate::{Command, CommandError, Session};
use flightcraft_aero::{
    Aircraft, AircraftCatalog, DefinitionSource, Identity, Load, ResolvedAircraft, WeightBalance,
};
use flightcraft_sim::{Sim, SimError, Start};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ListAircraft {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AircraftList {
    pub aircraft: Vec<AircraftEntry>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AircraftEntry {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub author: String,
    pub variant: String,
    pub panel: String,
    pub source: DefinitionSource,
}

impl Command for ListAircraft {
    const NAME: &'static str = "aircraft.list";
    const DESCRIPTION: &'static str = "List installed aircraft, their variant, panel, and source.";
    type Output = AircraftList;
    fn run(self, session: &mut Session) -> Result<Self::Output, CommandError> {
        Ok(AircraftList {
            aircraft: session
                .aircraft_catalog
                .aircraft_definitions()
                .map(|(a, source)| AircraftEntry {
                    id: a.identity.id.clone(),
                    display_name: a.identity.display_name.clone(),
                    description: a.identity.description.clone(),
                    author: a.identity.author.clone(),
                    variant: a.variant.clone(),
                    panel: a.panel.clone(),
                    source: source.clone(),
                })
                .collect(),
        })
    }
}

#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AircraftSource {
    BuiltIn {
        id: String,
    },
    Installed {
        id: String,
    },
    Json {
        text: String,
    },
    #[cfg(not(target_arch = "wasm32"))]
    File {
        path: String,
    },
}

#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
#[schemars(transform = load_selection_schema)]
pub struct LoadAircraft {
    /// Legacy source selection and explicit variant/panel/load selection are mutually exclusive.
    #[serde(default, deserialize_with = "crate::provided")]
    #[schemars(with = "AircraftSource", transform = crate::without_default)]
    pub source: Option<AircraftSource>,
    #[serde(default, deserialize_with = "crate::provided")]
    #[schemars(with = "String", transform = crate::without_default)]
    pub variant: Option<String>,
    #[serde(default, deserialize_with = "crate::provided")]
    #[schemars(with = "String", transform = crate::without_default)]
    pub panel: Option<String>,
    #[serde(default, deserialize_with = "crate::provided")]
    #[schemars(with = "Load", transform = crate::without_default)]
    pub load: Option<Load>,
    /// Permit a structurally valid load beyond certified limits, returning warnings.
    #[serde(default)]
    pub allow_out_of_limits: bool,
    /// False stages selection for sim.reset; true resets immediately on the runway, paused.
    #[serde(default)]
    pub reset: bool,
}

fn load_selection_schema(schema: &mut schemars::Schema) {
    schema.insert(
        "oneOf".into(),
        serde_json::json!([
            {"required": ["source"], "not": {"anyOf": [
                {"required": ["variant"]}, {"required": ["panel"]}, {"required": ["load"]}
            ]}},
            {"required": ["variant", "panel", "load"], "not": {"required": ["source"]}}
        ]),
    );
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AircraftLoaded {
    pub aircraft: Identity,
    pub active_aircraft_id: String,
    pub reset: bool,
    /// Absent for version 1 files, which have fixed mass and no envelope.
    pub weight_balance: Option<WeightBalance>,
    pub warnings: Vec<String>,
}

impl Command for LoadAircraft {
    const NAME: &'static str = "aircraft.load";
    const DESCRIPTION: &'static str = "Select a variant, panel, and load, or an installed aircraft or legacy source; stage or reset atomically.";
    type Output = AircraftLoaded;
    fn run(self, session: &mut Session) -> Result<Self::Output, CommandError> {
        let invalid = |message| CommandError::InvalidInput {
            command: Self::NAME.into(),
            message,
        };
        let (aircraft, configuration) = if let Some(source) = self.source {
            if self.variant.is_some() || self.panel.is_some() || self.load.is_some() {
                return Err(invalid(
                    "source and variant/panel/load are mutually exclusive".into(),
                ));
            }
            if let AircraftSource::BuiltIn { id } = &source
                && session
                    .aircraft_catalog
                    .source(flightcraft_aero::FileKind::Aircraft, id)
                    != Some(&DefinitionSource::BuiltIn)
            {
                return Err(invalid(format!("source.id: unknown built-in aircraft {id}")));
            }
            match source {
                AircraftSource::BuiltIn { id } | AircraftSource::Installed { id } => {
                    let configuration = session
                        .aircraft_catalog
                        .resolve_aircraft(&id)
                        .map_err(|e| invalid(e.to_string()))?;
                    (configuration.aircraft.clone(), Some(configuration))
                }
                AircraftSource::Json { text } => {
                    decode_source(&session.aircraft_catalog, &text).map_err(invalid)?
                }
                #[cfg(not(target_arch = "wasm32"))]
                AircraftSource::File { path } => {
                    let text = crate::packs::read_bounded(
                        std::path::Path::new(&path),
                        flightcraft_aero::MAX_AIRCRAFT_BYTES,
                    )
                    .map_err(|e| invalid(format!("source.path: {e}")))?;
                    decode_source(&session.aircraft_catalog, &text).map_err(invalid)?
                }
            }
        } else {
            let variant = self
                .variant
                .ok_or_else(|| invalid("variant is required".into()))?;
            let panel = self.panel.ok_or_else(|| invalid("panel is required".into()))?;
            let load = self.load.ok_or_else(|| invalid("load is required".into()))?;
            let configuration = session
                .aircraft_catalog
                .resolve(&variant, &panel, &load)
                .map_err(|e| invalid(e.to_string()))?;
            (configuration.aircraft.clone(), Some(configuration))
        };
        let balance = configuration.as_ref().map(|c| c.weight_balance.clone());
        let warnings = balance.as_ref().map(|b| b.violations.clone()).unwrap_or_default();
        if !warnings.is_empty() && !self.allow_out_of_limits {
            return Err(invalid(format!(
                "{}; set allow_out_of_limits to fly with warnings",
                warnings.join("; ")
            )));
        }
        let replacement = if self.reset {
            Some(Sim::new_with_aircraft_weather_world(
                &Start::Runway,
                aircraft.clone(),
                session.sim.weather(),
                session.sim.world().clone(),
            )?)
        } else {
            None
        };
        if let Some(mut sim) = replacement {
            sim.set_simulation_date(session.simulation_date())?;
            if let Some(configuration) = &configuration {
                sim.configure_systems(configuration)?;
            }
            if session.flight_archive.autosave_enabled {
                session.save_flight()?;
            }
            session.sim = sim;
            session.active_configuration = configuration.clone();
            session.paused = true;
            session.flight_generation = session.flight_generation.wrapping_add(1);
        }
        let output = AircraftLoaded {
            aircraft: aircraft.identity.clone(),
            active_aircraft_id: session.sim.aircraft().identity.id.clone(),
            reset: self.reset,
            weight_balance: balance,
            warnings,
        };
        session.selected_aircraft = aircraft;
        session.selected_configuration = configuration;
        Ok(output)
    }
}

fn decode_source(
    catalog: &AircraftCatalog,
    text: &str,
) -> Result<(Aircraft, Option<ResolvedAircraft>), String> {
    if text.len() > flightcraft_aero::MAX_AIRCRAFT_BYTES {
        return Err("file_size: aircraft exceeds 1 MiB".into());
    }
    if flightcraft_aero::definition_version(text).map_err(|e| e.to_string())? == 2 {
        let flightcraft_aero::LayerFile::Aircraft(definition) =
            flightcraft_aero::LayerFile::from_json(text).map_err(|e| e.to_string())?
        else {
            return Err("kind: requires an aircraft selection".into());
        };
        let configuration = catalog
            .resolve_definition(&definition)
            .map_err(|e| e.to_string())?;
        Ok((configuration.aircraft.clone(), Some(configuration)))
    } else {
        Aircraft::from_json(text)
            .map(|a| (a, None))
            .map_err(|e| e.to_string())
    }
}

#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeWeightBalance {
    pub variant: String,
    pub load: Load,
}

impl Command for ComputeWeightBalance {
    const NAME: &'static str = "weight_balance.compute";
    const DESCRIPTION: &'static str =
        "Compute total mass, weight, CG, and limit violations for a variant and station load.";
    type Output = WeightBalance;
    fn run(self, session: &mut Session) -> Result<Self::Output, CommandError> {
        session
            .aircraft_catalog
            .variant(&self.variant)
            .and_then(|v| v.weight_balance(&self.load))
            .map_err(|e| CommandError::InvalidInput {
                command: Self::NAME.into(),
                message: e.to_string(),
            })
    }
}

impl Session {
    /// Equipment, load, and layer metadata for the active flight. Legacy files have no metadata.
    pub fn aircraft_configuration(&self) -> Option<&ResolvedAircraft> {
        self.active_configuration.as_ref()
    }
    pub fn aircraft_catalog(&self) -> &AircraftCatalog {
        &self.aircraft_catalog
    }
    pub fn install_pack(&mut self, pack: &crate::PackContents) -> Result<(), CommandError> {
        self.aircraft_catalog
            .install_pack(&pack.manifest, &pack.files)
            .map_err(SimError::from)?;
        Ok(())
    }
}
