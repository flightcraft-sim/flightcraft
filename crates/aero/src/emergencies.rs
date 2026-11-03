//! Original aircraft-specific emergency procedures, carried in systems pack data.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EmergencyItem {
    pub id: String,
    pub instruction: String,
    pub memory: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EmergencyProcedure {
    pub id: String,
    pub title: String,
    pub best_glide_mps: f64,
    pub glide_ratio: f64,
    pub speed_tolerance_mps: f64,
    pub items: Vec<EmergencyItem>,
}
impl EmergencyProcedure {
    pub fn validate(&self) -> Result<(), crate::AircraftError> {
        let valid = |s: &str, max| !s.trim().is_empty() && s.len() <= max && !s.chars().any(char::is_control);
        let mut ids = std::collections::BTreeSet::new();
        if !valid(&self.id, 64)
            || !valid(&self.title, 256)
            || !self.best_glide_mps.is_finite()
            || !(10.0..=150.0).contains(&self.best_glide_mps)
            || !self.glide_ratio.is_finite()
            || !(2.0..=40.0).contains(&self.glide_ratio)
            || !self.speed_tolerance_mps.is_finite()
            || !(0.1..=20.0).contains(&self.speed_tolerance_mps)
            || self.items.is_empty()
            || self.items.len() > 64
            || self
                .items
                .iter()
                .any(|i| !valid(&i.id, 64) || !ids.insert(&i.id) || !valid(&i.instruction, 1024))
        {
            return Err(crate::aircraft::error(
                "emergencies",
                "invalid emergency procedure",
            ));
        }
        Ok(())
    }
}
