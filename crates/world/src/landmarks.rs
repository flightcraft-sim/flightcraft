//! Immutable USGS GNIS names and positions. These observations do not affect flight physics.
use crate::{GeoPoint, TerrainError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
pub const MAX_LANDMARK_BYTES: usize = 8 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Landmark {
    pub feature_id: u32,
    pub name: String,
    pub feature_class: String,
    pub state: String,
    pub position: GeoPoint,
    /// Lower ranks are more prominent: gap, mountain/ridge, river, major lake, town.
    /// Absent means the feature is retained for reference but is not a scenic callout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0, max = 4))]
    pub prominence: Option<u8>,
    /// Original geographic context, linked to the named GNIS features in this sidecar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LandmarkSource {
    pub file: String,
    pub sha256: String,
    pub bytes: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Landmarks {
    pub version: u8,
    pub attribution: String,
    pub sources: Vec<LandmarkSource>,
    pub features: Vec<Landmark>,
}
impl Landmarks {
    pub fn from_json(text: &str) -> Result<Self, TerrainError> {
        if text.len() > MAX_LANDMARK_BYTES {
            return Err(TerrainError("landmarks file too large"));
        }
        let data: Self = serde_json::from_str(text).map_err(|_| TerrainError("landmarks JSON"))?;
        data.validate()?;
        Ok(data)
    }
    pub fn validate(&self) -> Result<(), TerrainError> {
        if self.version != 1
            || self.features.len() > 30000
            || self.sources.len() > 60
            || self.attribution.len() > 1024
        {
            return Err(TerrainError("landmarks bounds"));
        }
        let mut ids = BTreeSet::new();
        for feature in &self.features {
            feature.position.validate()?;
            if feature.feature_id == 0
                || !ids.insert(feature.feature_id)
                || feature.name.is_empty()
                || feature.name.len() > 160
                || feature.name.chars().any(char::is_control)
                || feature.prominence.is_some_and(|rank| rank > 4)
                || feature
                    .description
                    .as_ref()
                    .is_some_and(|d| d.len() > 400 || d.chars().any(char::is_control))
                || feature.state.is_empty()
                || feature.state.len() > 40
                || feature.state.chars().any(char::is_control)
                || ![
                    "Summit",
                    "Gap",
                    "Ridge",
                    "Lake",
                    "Reservoir",
                    "Stream",
                    "Populated Place",
                ]
                .contains(&feature.feature_class.as_str())
            {
                return Err(TerrainError("invalid GNIS landmark"));
            }
        }
        for source in &self.sources {
            if source.file.is_empty()
                || source.file.len() > 100
                || source.file.contains(['/', '\\'])
                || source.sha256.len() != 64
                || !source.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(TerrainError("invalid GNIS source receipt"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_metadata_rejects_duplicate_ids_bad_coordinates_and_provenance_paths() {
        let mut d = Landmarks {
            version: 1,
            attribution: "USGS".into(),
            sources: vec![LandmarkSource {
                file: "DomesticNames_NJ.txt".into(),
                sha256: "a".repeat(64),
                bytes: 100,
            }],
            features: vec![Landmark {
                feature_id: 1,
                prominence: None,
                description: None,
                name: "Test Summit".into(),
                feature_class: "Summit".into(),
                state: "New Jersey".into(),
                position: GeoPoint {
                    latitude_deg: 40.,
                    longitude_deg: -75.,
                },
            }],
        };
        assert!(d.validate().is_ok());
        d.features.push(d.features.first().unwrap().clone());
        assert!(d.validate().is_err());
        d.features.pop();
        d.features.first_mut().unwrap().position.latitude_deg = 91.;
        assert!(d.validate().is_err());
        d.features.first_mut().unwrap().position.latitude_deg = 40.;
        d.sources.first_mut().unwrap().file = "../source.txt".into();
        assert!(d.validate().is_err());
    }
}
