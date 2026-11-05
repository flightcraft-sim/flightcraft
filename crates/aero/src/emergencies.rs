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
    /// Best glide speed. Absent for emergencies where gliding does not apply (ground and take-off-run
    /// events, fires on the ground).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best_glide_mps: Option<f64>,
    /// Glide ratio. Absent where gliding does not apply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glide_ratio: Option<f64>,
    /// Allowed deviation from the best glide speed when grading speed. Absent where gliding does not apply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_tolerance_mps: Option<f64>,
    pub items: Vec<EmergencyItem>,
    /// POH warnings and notes for the whole procedure, shown beneath the items.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}
impl EmergencyProcedure {
    /// Both numbers that define a glide footprint, when the procedure gives them.
    pub fn glide(&self) -> Option<(f64, f64)> {
        Some((self.glide_ratio?, self.best_glide_mps?))
    }
    pub fn validate(&self) -> Result<(), crate::AircraftError> {
        let valid = |s: &str, max| !s.trim().is_empty() && s.len() <= max && !s.chars().any(char::is_control);
        let mut ids = std::collections::BTreeSet::new();
        if !valid(&self.id, 64)
            || !valid(&self.title, 256)
            || self
                .best_glide_mps
                .is_some_and(|v| !v.is_finite() || !(10.0..=150.0).contains(&v))
            || self
                .glide_ratio
                .is_some_and(|v| !v.is_finite() || !(2.0..=40.0).contains(&v))
            || self
                .speed_tolerance_mps
                .is_some_and(|v| !v.is_finite() || !(0.1..=20.0).contains(&v))
            || self.note.as_deref().is_some_and(|n| !valid(n, 2048))
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

#[cfg(test)]
mod tests {
    use super::*;
    fn json(extra: &str) -> String {
        format!(
            r#"{{"id":"x","title":"X","items":[{{"id":"a","instruction":"Do it","memory":true}}]{extra}}}"#
        )
    }
    #[test]
    fn glide_numbers_and_note_are_optional() {
        let p: EmergencyProcedure = serde_json::from_str(&json("")).unwrap();
        p.validate().unwrap();
        assert!(p.glide().is_none() && p.note.is_none());
        let text = serde_json::to_string(&p).unwrap();
        assert!(!text.contains("glide") && !text.contains("note"));
        let p: EmergencyProcedure = serde_json::from_str(&json(
            r#","best_glide_mps":37.0,"glide_ratio":10.0,"speed_tolerance_mps":5.0,"note":"WARNING""#,
        ))
        .unwrap();
        p.validate().unwrap();
        assert_eq!(p.glide(), Some((10.0, 37.0)));
    }
    #[test]
    fn ranges_still_apply_when_a_value_is_present() {
        for bad in [
            r#","best_glide_mps":5.0"#,
            r#","best_glide_mps":200.0"#,
            r#","glide_ratio":1.0"#,
            r#","speed_tolerance_mps":0.0"#,
            r#","note":"""#,
        ] {
            let p: EmergencyProcedure = serde_json::from_str(&json(bad)).unwrap();
            assert!(p.validate().is_err(), "{bad}");
        }
        // Glide needs both the ratio and the speed.
        let p: EmergencyProcedure = serde_json::from_str(&json(r#","best_glide_mps":37.0"#)).unwrap();
        assert!(p.glide().is_none());
    }
}
