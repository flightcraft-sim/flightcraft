#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_aero::{CORVID_JSON, MAX_AIRCRAFT_BYTES};
use flightcraft_control::Session;
use serde_json::{Value, json};

fn custom() -> String {
    let mut value: Value = serde_json::from_str(CORVID_JSON).unwrap();
    value["identity"]["id"] = json!("test-trainer");
    value["mass"]["mass_kg"] = json!(1100.0);
    value.to_string()
}

#[test]
fn load_stages_until_reset_and_persists_across_resets() {
    let mut s = Session::new().unwrap();
    let list = s.execute("aircraft.list", json!({})).unwrap();
    assert_eq!(list["aircraft"][0]["id"], "corvid");
    let before = *s.sim().state();
    let generation = s.flight_generation();
    let loaded = s
        .execute("aircraft.load", json!({"source":{"kind":"json","text":custom()}}))
        .unwrap();
    assert_eq!(loaded["aircraft"]["id"], "test-trainer");
    assert_eq!(loaded["active_aircraft_id"], "corvid");
    assert_eq!(s.sim().state(), &before);
    assert_eq!(s.flight_generation(), generation);
    for _ in 0..2 {
        s.execute("sim.reset", json!({"start":{"kind":"runway"}}))
            .unwrap();
        assert_eq!(s.sim().aircraft().identity.id, "test-trainer");
        assert_eq!(s.sim().aircraft().mass.mass_kg, 1100.0);
    }
    s.execute("sim.step", json!({"steps":120})).unwrap();
    s.execute("sim.resume", json!({})).unwrap();
    let generation = s.flight_generation();
    s.execute(
        "aircraft.load",
        json!({"source":{"kind":"json","text":CORVID_JSON},"reset":true}),
    )
    .unwrap();
    assert_eq!(s.sim().aircraft().identity.id, "corvid");
    assert_eq!(s.sim().state().time_s, 0.0);
    assert!(s.paused());
    assert_eq!(s.flight_generation(), generation + 1);
}

#[test]
fn failed_loads_preserve_both_active_and_staged_aircraft() {
    let mut s = Session::new().unwrap();
    s.execute("aircraft.load", json!({"source":{"kind":"json","text":custom()}}))
        .unwrap();
    s.execute("sim.step", json!({"steps":120})).unwrap();
    let before = s.execute("sim.state", json!({})).unwrap();
    for text in [
        "{}".into(),
        CORVID_JSON.replace("1000.0", "-1.0"),
        " ".repeat(MAX_AIRCRAFT_BYTES + 1),
    ] {
        assert!(
            s.execute(
                "aircraft.load",
                json!({"source":{"kind":"json","text":text},"reset":true})
            )
            .is_err()
        );
        assert_eq!(s.execute("sim.state", json!({})).unwrap(), before);
    }
    assert!(
        s.execute(
            "sim.reset",
            json!({"start":{"kind":"air","height_m":1,"airspeed_mps":30,"heading_rad":0}})
        )
        .is_err()
    );
    assert_eq!(s.execute("sim.state", json!({})).unwrap(), before);
    s.execute("sim.reset", json!({"start":{"kind":"runway"}}))
        .unwrap();
    assert_eq!(s.sim().aircraft().identity.id, "test-trainer");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn native_files_are_bounded_and_do_not_open_dialogs() {
    let mut s = Session::new().unwrap();
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../aircraft/corvid.aircraft.json");
    s.execute(
        "aircraft.load",
        json!({"source":{"kind":"file","path":path},"reset":true}),
    )
    .unwrap();
    assert_eq!(s.sim().aircraft().identity.id, "corvid");
    assert!(
        s.execute("aircraft.load", json!({"source":{"kind":"file","path":""}}))
            .unwrap_err()
            .to_string()
            .contains("source.path")
    );
    let path = std::env::temp_dir().join(format!("flightcraft-aircraft-cap-{}.json", std::process::id()));
    std::fs::write(&path, vec![b' '; MAX_AIRCRAFT_BYTES + 1]).unwrap();
    let result = s.execute("aircraft.load", json!({"source":{"kind":"file","path":path}}));
    std::fs::remove_file(path).unwrap();
    assert!(result.unwrap_err().to_string().contains("file_size"));
}

#[test]
fn aircraft_schema_is_valid_and_matches_the_builtin_file() {
    let schema = serde_json::to_value(schemars::schema_for!(flightcraft_aero::Aircraft)).unwrap();
    jsonschema::meta::validate(&schema).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let mut value: Value = serde_json::from_str(CORVID_JSON).unwrap();
    assert!(validator.is_valid(&value));
    value["mass"]["unexpected"] = json!(true);
    assert!(!validator.is_valid(&value));
}

#[test]
fn builtin_larrow_lists_stages_resets_and_rejects_unknown_ids_atomically() {
    let mut s = Session::new().unwrap();
    let list = s.execute("aircraft.list", json!({})).unwrap();
    assert_eq!(list["aircraft"].as_array().unwrap().len(), 2);
    assert_eq!(list["aircraft"][1]["id"], "larrow");
    s.execute(
        "aircraft.load",
        json!({"source":{"kind":"built_in","id":"larrow"}}),
    )
    .unwrap();
    assert_eq!(s.sim().aircraft().identity.id, "corvid");
    let before = *s.sim().state();
    assert!(
        s.execute(
            "aircraft.load",
            json!({"source":{"kind":"built_in","id":"unknown"},"reset":true})
        )
        .is_err()
    );
    assert_eq!(*s.sim().state(), before);
    s.execute("sim.reset", json!({"start":{"kind":"runway"}}))
        .unwrap();
    assert_eq!(s.sim().aircraft().identity.id, "larrow");
    assert!(s.sim().readout().pitch_rad > 0.15);
    s.execute(
        "aircraft.load",
        json!({"source":{"kind":"built_in","id":"corvid"},"reset":true}),
    )
    .unwrap();
    assert_eq!(s.sim().aircraft().identity.id, "corvid");
    assert_eq!(s.sim().readout().pitch_rad, 0.0);
    assert_eq!(s.sim().aircraft(), &flightcraft_aero::Aircraft::corvid().unwrap());
}
