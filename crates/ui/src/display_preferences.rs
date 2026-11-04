//! Per-variant cockpit display choices (pages, data columns, pilot seat) in the eframe preferences
//! store, beside the callsigns: desktop and web alike.
use flightcraft_control::Session;

const KEY: &str = "flightcraft.displays.v1";

pub(crate) fn restore(session: &mut Session, storage: Option<&dyn eframe::Storage>) -> Result<(), String> {
    let Some(json) = storage.and_then(|s| s.get_string(KEY)) else {
        return Ok(());
    };
    if json.len() > 256 * 1024 {
        return Err("Saved display choices exceed the preferences size limit.".into());
    }
    let values = serde_json::from_str(&json).map_err(|e| format!("Saved display choices: {e}"))?;
    session
        .restore_display_preferences(values)
        .map_err(|e| e.to_string())
}

pub(crate) fn save(session: &Session, storage: &mut dyn eframe::Storage) {
    if let Ok(json) = serde_json::to_string(session.display_preferences()) {
        storage.set_string(KEY, json);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use eframe::Storage as _;

    #[derive(Default)]
    struct Storage(std::collections::BTreeMap<String, String>);
    impl eframe::Storage for Storage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.into(), value);
        }
        fn flush(&mut self) {}
    }

    #[test]
    fn eframe_round_trip_restores_choices_and_rejects_bad_files() {
        let mut store = Storage::default();
        let mut session = Session::new().unwrap();
        restore(&mut session, Some(&store)).unwrap();
        session
            .execute(
                "avionics.set",
                serde_json::json!({"setting":{"kind":"glass","setting":{"kind":"pilot_seat","seat":"right"}}}),
            )
            .unwrap();
        save(&session, &mut store);
        let mut reopened = Session::new().unwrap();
        restore(&mut reopened, Some(&store)).unwrap();
        assert_eq!(reopened.display_preferences(), session.display_preferences());
        assert!(!reopened.display_preferences().is_empty());
        let mut bad = Storage::default();
        bad.set_string(KEY, r#"{"Bad Key":{}}"#.into());
        assert!(restore(&mut Session::new().unwrap(), Some(&bad)).is_err());
        bad.set_string(KEY, "not json".into());
        assert!(restore(&mut Session::new().unwrap(), Some(&bad)).is_err());
    }
}
