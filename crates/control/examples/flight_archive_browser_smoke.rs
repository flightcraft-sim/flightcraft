//! Browser smoke harness: compile for wasm, process with wasm-bindgen, and call run_archive_smoke.
#[cfg(target_arch = "wasm32")]
mod browser {
    use flightcraft_control::{FlightExport, FlightLoad, FlightRecords, FlightSave, Session};
    use serde_json::json;
    use wasm_bindgen::{JsValue, prelude::wasm_bindgen};
    async fn wait(s: &Session) -> Result<(), String> {
        for _ in 0..500 {
            let state = s.flight_archive_status();
            if let Some(error) = state.error {
                return Err(error);
            }
            if !state.busy {
                return Ok(());
            }
            let ready = js_sys::Promise::new(&mut |resolve, _| {
                if let Some(window) = web_sys::window() {
                    let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 20);
                } else {
                    let _ = resolve.call0(&JsValue::UNDEFINED);
                }
            });
            wasm_bindgen_futures::JsFuture::from(ready)
                .await
                .map_err(|e| format!("{e:?}"))?;
        }
        Err("IndexedDB operation timed out".into())
    }
    async fn smoke() -> Result<String, String> {
        let mut first = Session::new().map_err(|e| e.to_string())?;
        first
            .execute(
                "comms.callsign",
                json!({"registration":"N371FC","spoken_type":"Sling"}),
            )
            .map_err(|e| e.to_string())?;
        first
            .execute(
                "comms.transmit",
                json!({"text":"Blairstown traffic, {callsign} taxiing runway 25, Blairstown"}),
            )
            .map_err(|e| e.to_string())?;
        first
            .execute("sim.step", json!({"steps":2400}))
            .map_err(|e| e.to_string())?;
        first.run(FlightSave {}).map_err(|e| e.to_string())?;
        wait(&first).await?;
        let id = first
            .flight_archive_status()
            .last_saved
            .ok_or("no saved id after commit")?;
        drop(first);
        let mut reopened = Session::new().map_err(|e| e.to_string())?;
        let hash = reopened.sim().state_hash();
        reopened
            .run(FlightRecords { refresh: true })
            .map_err(|e| e.to_string())?;
        wait(&reopened).await?;
        if !reopened
            .flight_archive_status()
            .flights
            .iter()
            .any(|f| f.id == id)
        {
            return Err("saved flight missing after reopening session".into());
        }
        reopened
            .run(FlightLoad { id: id.clone() })
            .map_err(|e| e.to_string())?;
        wait(&reopened).await?;
        let loaded = reopened
            .flight_archive_status()
            .loaded
            .ok_or("loaded record missing")?;
        if loaded.summary.registration != "N371FC"
            || !loaded
                .record
                .events
                .iter()
                .any(|e| e.text.contains("Sling Three Seven One Foxtrot Charlie"))
        {
            return Err("saved radio identity/transmission changed".into());
        }
        let exported = reopened
            .run(FlightExport { id: Some(id.clone()) })
            .map_err(|e| e.to_string())?;
        if !exported.json.contains("taxiing runway 25") || reopened.sim().state_hash() != hash {
            return Err("export missing transcript or debrief altered simulation".into());
        }
        Ok(format!(
            "PASS: IndexedDB committed save, fresh-session listing, load, callsign/transmission round trip, JSON export, and unchanged live hash. Record {id}."
        ))
    }
    #[wasm_bindgen]
    pub async fn verify_archive_reopen(id: String) -> Result<String, JsValue> {
        async fn verify(id: String) -> Result<String, String> {
            let mut s = Session::new().map_err(|e| e.to_string())?;
            s.run(FlightRecords { refresh: true })
                .map_err(|e| e.to_string())?;
            wait(&s).await?;
            if !s.flight_archive_status().flights.iter().any(|f| f.id == id) {
                return Err("record lost across browser restart".into());
            }
            s.run(FlightLoad { id: id.clone() }).map_err(|e| e.to_string())?;
            wait(&s).await?;
            let exported = s.run(FlightExport { id: Some(id) }).map_err(|e| e.to_string())?;
            if !exported.json.contains("Sling Three Seven One Foxtrot Charlie") {
                return Err("reopened browser record differs".into());
            }
            Ok("PASS: previously saved IndexedDB flight survived a complete browser restart and exported its original transcript.".into())
        }
        verify(id).await.map_err(|e| JsValue::from_str(&e))
    }
    #[wasm_bindgen]
    pub async fn run_archive_smoke() -> Result<String, JsValue> {
        smoke().await.map_err(|e| JsValue::from_str(&e))
    }
}
