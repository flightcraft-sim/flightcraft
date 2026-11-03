//! The same eframe preferences store on desktop and web, including global-v1 migration.
use flightcraft_control::Session;

const KEY: &str = "flightcraft.callsigns.v2";

pub(crate) fn restore(session: &mut Session, storage: Option<&dyn eframe::Storage>) -> Result<(), String> {
    let Some(storage) = storage else { return Ok(()) };
    if let Some(json) = storage.get_string(KEY) {
        if json.len() > 256 * 1024 {
            return Err("Saved callsigns exceed the preferences size limit.".into());
        }
        let values = serde_json::from_str(&json).map_err(|e| format!("Saved callsigns: {e}"))?;
        session
            .restore_callsign_preferences(values)
            .map_err(|e| e.to_string())?;
    } else if let Some(json) = storage.get_string("flightcraft.callsign.v1") {
        if json.len() > 256 * 1024 {
            return Err("Saved callsign exceeds the preferences size limit.".into());
        }
        let mut value: serde_json::Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        // Old versions accepted foreign numbers without an explicit toggle.
        if let Some(registration) = value.get("registration").and_then(|v| v.as_str()) {
            let not_us = registration.contains('-');
            if let Some(object) = value.as_object_mut() {
                object.insert("not_us".into(), not_us.into());
                object.insert("persist".into(), true.into());
            }
            session
                .execute("comms.callsign", value)
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

pub(crate) fn save(session: &Session, storage: &mut dyn eframe::Storage) {
    if let Ok(json) = serde_json::to_string(session.callsign_preferences()) {
        storage.set_string(KEY, json);
        // Prevent the previous global setting from reappearing after "Use pack".
        storage.set_string("flightcraft.callsign.v1", "{}".into());
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use eframe::Storage as _;
    use serde_json::json;

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
    fn eframe_round_trip_migrates_and_clears_without_resurrecting_v1() {
        let mut store = Storage::default();
        store.set_string(
            "flightcraft.callsign.v1",
            json!({"registration":"n371fc","spoken_type":"Sling"}).to_string(),
        );
        let mut session = Session::new().unwrap();
        restore(&mut session, Some(&store)).unwrap();
        assert_eq!(session.pilot_callsign().registration, "N371FC");
        save(&session, &mut store);
        let mut reopened = Session::new().unwrap();
        restore(&mut reopened, Some(&store)).unwrap();
        assert_eq!(reopened.pilot_callsign().spoken_type, "Sling");
        reopened
            .execute("comms.callsign", json!({"reset_to_pack":true}))
            .unwrap();
        save(&reopened, &mut store);
        let mut cleared = Session::new().unwrap();
        restore(&mut cleared, Some(&store)).unwrap();
        assert!(!cleared.pilot_callsign().overridden);
    }
}
