//! Typed airspace commands share the world's immutable queries with the status display.
use crate::{Command, CommandError, Session, invalid};
use flightcraft_world::{AirspaceAltitude, AirspaceBoundary, AirspaceClass, AirspaceKind, AirspaceVolume};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct AirspaceInfo {
    pub id: String,
    pub name: String,
    pub class: Option<AirspaceClass>,
    pub kind: AirspaceKind,
    pub sector: Option<String>,
    pub floor: AirspaceAltitude,
    pub ceiling: AirspaceAltitude,
}
impl From<&AirspaceVolume> for AirspaceInfo {
    fn from(v: &AirspaceVolume) -> Self {
        Self {
            id: v.id.clone(),
            name: v.name.clone(),
            class: v.class,
            kind: v.kind,
            sector: v.sector.clone(),
            floor: v.floor,
            ceiling: v.ceiling,
        }
    }
}
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ListAirspace {}
#[derive(Debug, Serialize, JsonSchema)]
pub struct AirspaceList {
    pub attribution: Option<String>,
    pub volumes: Vec<AirspaceInfo>,
}
impl Command for ListAirspace {
    const NAME: &'static str = "airspace.list";
    const DESCRIPTION: &'static str =
        "List the active region's airspace volumes, vertical limits, and attribution.";
    type Output = AirspaceList;
    fn run(self, session: &mut Session) -> Result<Self::Output, CommandError> {
        let airspace = session.sim().world().airspace();
        Ok(AirspaceList {
            attribution: airspace.map(|a| a.attribution.clone()),
            volumes: airspace
                .map(|a| a.volumes.iter().map(|v| AirspaceInfo::from(&v.info)).collect())
                .unwrap_or_default(),
        })
    }
}
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
#[schemars(transform = coordinate_group)]
pub struct AirspaceAt {
    #[serde(default, deserialize_with = "crate::provided")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "f64", range(min = -10000000, max = 10000000), transform = crate::without_default)]
    pub north_m: Option<f64>,
    #[serde(default, deserialize_with = "crate::provided")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "f64", range(min = -10000000, max = 10000000), transform = crate::without_default)]
    pub east_m: Option<f64>,
    #[serde(default, deserialize_with = "crate::provided")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "f64", range(min = -10000000, max = 10000000), transform = crate::without_default)]
    pub altitude_msl_m: Option<f64>,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct AirspaceAtOutput {
    pub containing: Vec<AirspaceInfo>,
    pub nearest: Option<AirspaceBoundary>,
}
impl Command for AirspaceAt {
    const NAME: &'static str = "airspace.at";
    const DESCRIPTION: &'static str =
        "Query all containing airspace and the nearest B/C shelf; omit all coordinates for the aircraft.";
    type Output = AirspaceAtOutput;
    fn run(self, session: &mut Session) -> Result<Self::Output, CommandError> {
        let [n, e, down] = session.sim().state().position_m;
        let (n, e, a) = match (self.north_m, self.east_m, self.altitude_msl_m) {
            (None, None, None) => (n, e, -down),
            (Some(n), Some(e), Some(a)) => (n, e, a),
            _ => {
                return Err(invalid(
                    Self::NAME,
                    "supply all three coordinates, or omit all three",
                ));
            }
        };
        let query = session
            .sim()
            .world()
            .airspace_at(n, e, a)
            .map_err(|e| invalid(Self::NAME, e.to_string()))?;
        Ok(AirspaceAtOutput {
            containing: query.containing.iter().map(AirspaceInfo::from).collect(),
            nearest: query.nearest,
        })
    }
}
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetAirspaceView {
    pub visible: bool,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct AirspaceView {
    pub visible: bool,
}
impl Command for SetAirspaceView {
    const NAME: &'static str = "view.set_airspace";
    const DESCRIPTION: &'static str =
        "Show or hide airspace curtains and outlines (default on in regions with airspace).";
    type Output = AirspaceView;
    fn run(self, session: &mut Session) -> Result<Self::Output, CommandError> {
        session.airspace_visible = self.visible;
        Ok(AirspaceView {
            visible: self.visible,
        })
    }
}
impl Session {
    pub fn airspace_visible(&self) -> bool {
        self.airspace_visible
    }
}

// A position is an atomic coordinate triple in both the command and its published schema.
fn coordinate_group(schema: &mut schemars::Schema) {
    schema.insert("oneOf".into(), serde_json::json!([
        {"required":["north_m","east_m","altitude_msl_m"]},
        {"not":{"anyOf":[{"required":["north_m"]},{"required":["east_m"]},{"required":["altitude_msl_m"]}]}}
    ]));
}
