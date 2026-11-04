//! The view choices (ground layout mode, exterior inset mode and placement) in the eframe
//! preferences store beside the display choices: desktop and web alike.
use flightcraft_control::Session;

const KEY: &str = "flightcraft.view.v1";

pub(crate) fn restore(session: &mut Session, storage: Option<&dyn eframe::Storage>) -> Result<(), String> {
    let Some(json) = storage.and_then(|s| s.get_string(KEY)) else {
        return Ok(());
    };
    if json.len() > 16 * 1024 {
        return Err("Saved view choices exceed the preferences size limit.".into());
    }
    let values = serde_json::from_str(&json).map_err(|e| format!("Saved view choices: {e}"))?;
    session
        .restore_view_preferences(values)
        .map_err(|e| e.to_string())
}

pub(crate) fn save(session: &Session, storage: &mut dyn eframe::Storage) {
    if let Ok(json) = serde_json::to_string(session.view_preferences()) {
        storage.set_string(KEY, json);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

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
    fn inset_setting_round_trips_through_the_preferences_store() {
        let mut store = Storage::default();
        let mut session = Session::new().unwrap();
        restore(&mut session, Some(&store)).unwrap();
        assert_eq!(
            session.view_preferences().exterior_inset,
            flightcraft_control::InsetMode::Ground
        );
        session
            .execute(
                "view.exterior_inset",
                serde_json::json!({"mode":"always","hold_s":7.0,"placement":{"x":0.3,"y":0.2,"w":0.25,"h":0.3}}),
            )
            .unwrap();
        session
            .execute("view.layout", serde_json::json!({"mode":"normal"}))
            .unwrap();
        save(&session, &mut store);
        let mut reopened = Session::new().unwrap();
        restore(&mut reopened, Some(&store)).unwrap();
        assert_eq!(reopened.view_preferences(), session.view_preferences());
        assert_eq!(
            reopened.view_preferences().exterior_inset,
            flightcraft_control::InsetMode::Always
        );
        let mut bad = Storage::default();
        eframe::Storage::set_string(&mut bad, KEY, r#"{"exterior_inset":"sometimes"}"#.into());
        assert!(restore(&mut Session::new().unwrap(), Some(&bad)).is_err());
        eframe::Storage::set_string(&mut bad, KEY, r#"{"framing_hold_s":0.0}"#.into());
        assert!(restore(&mut Session::new().unwrap(), Some(&bad)).is_err());
        eframe::Storage::set_string(&mut bad, KEY, "not json".into());
        assert!(restore(&mut Session::new().unwrap(), Some(&bad)).is_err());
    }
}
