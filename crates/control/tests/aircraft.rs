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
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../aircraft/corvid/aircraft.json");
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
    assert_eq!(list["aircraft"].as_array().unwrap().len(), 4);
    assert!(
        list["aircraft"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["id"] == "larrow")
    );
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

#[test]
fn explicit_layers_report_balance_and_out_of_limits_requires_opt_in() {
    let mut session = Session::new().unwrap();
    let load = json!({"stations":[{"station":"pilot","mass_kg":80},{"station":"passenger","mass_kg":80},{"station":"fuel","mass_kg":40}]});
    let balance = session
        .execute(
            "weight_balance.compute",
            json!({"variant":"corvid-standard","load":load}),
        )
        .unwrap();
    assert_eq!(balance["mass_kg"], 1000.0);
    assert_eq!(balance["cg_m"], json!([0.0, 0.0, 0.0]));
    assert_eq!(balance["in_limits"], true);
    let params = json!({"variant":"corvid-standard","panel":"larrow-six","load":load,"reset":true});
    session.execute("aircraft.load", params).unwrap();
    assert_eq!(
        session.aircraft_configuration().unwrap().panel.identity.id,
        "larrow-six"
    );
    let overload = json!({"stations":[{"station":"pilot","mass_kg":500}]});
    let params = json!({"variant":"corvid-standard","panel":"corvid-six","load":overload,"reset":true});
    let before = session.sim().state_hash();
    assert!(
        session
            .execute("aircraft.load", params.clone())
            .unwrap_err()
            .to_string()
            .contains("allow_out_of_limits")
    );
    assert_eq!(session.sim().state_hash(), before);
    let mut params = params;
    params["allow_out_of_limits"] = json!(true);
    let result = session.execute("aircraft.load", params).unwrap();
    assert_eq!(result["weight_balance"]["in_limits"], false);
    assert!(!result["warnings"].as_array().unwrap().is_empty());
    assert_eq!(session.sim().aircraft().mass.mass_kg, 1300.0);
    session
        .execute("sim.reset", json!({"start":{"kind":"runway"}}))
        .unwrap();
    assert_eq!(session.sim().aircraft().mass.mass_kg, 1300.0);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn example_pack_is_loaded_from_a_directory_and_listed_with_its_source() {
    let mut session = Session::new().unwrap();
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/packs"));
    assert!(session.load_packs_directory(root).is_empty());
    let listed = session.execute("aircraft.list", json!({})).unwrap();
    let entry = listed["aircraft"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == "corvid-tourer")
        .unwrap();
    assert_eq!(entry["source"], json!({"kind":"pack","id":"demo-pack"}));
    session
        .execute(
            "aircraft.load",
            json!({"source":{"kind":"installed","id":"corvid-tourer"},"reset":true}),
        )
        .unwrap();
    assert_eq!(session.sim().aircraft().propulsion.max_power_w, 135000.0);
    assert_eq!(
        session.aircraft_configuration().unwrap().panel.instruments.len(),
        3
    );
    let listed = listed["aircraft"].as_array().unwrap();
    let builtin = listed.iter().find(|a| a["id"] == "corvid").unwrap();
    assert_eq!(builtin["source"], json!({"kind":"built_in"}));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn bad_native_packs_are_skipped_without_hiding_good_packs() {
    let root = std::env::temp_dir().join(format!("flightcraft-pack-hostile-{}", std::process::id()));
    std::fs::create_dir_all(root.join("bad")).unwrap();
    std::fs::write(root.join("bad/pack.json"), "{}").unwrap();
    let demo = flightcraft_control::read_pack(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/packs/demo-pack"
    )))
    .unwrap();
    std::fs::create_dir_all(root.join("good")).unwrap();
    std::fs::write(
        root.join("good/pack.json"),
        serde_json::to_string(&demo.manifest).unwrap(),
    )
    .unwrap();
    for (path, text) in &demo.files {
        std::fs::write(root.join("good").join(path), text).unwrap();
    }
    let mut session = Session::new().unwrap();
    let errors = session.load_packs_directory(&root);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("bad"));
    assert!(
        session
            .aircraft_catalog()
            .resolve_aircraft("corvid-tourer")
            .is_ok()
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(root.join("good/engine.json"), root.join("good/alias.json")).unwrap();
        let mut manifest = demo.manifest.clone();
        manifest.files[1].path = "alias.json".into();
        std::fs::write(
            root.join("good/pack.json"),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();
        assert!(
            flightcraft_control::read_pack(&root.join("good"))
                .unwrap_err()
                .contains("symlink")
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn version_two_document_and_manifest_schemas_match_the_examples() {
    let schema = schemars::schema_for!(flightcraft_aero::LayerFile).to_value();
    jsonschema::meta::validate(&schema).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    for text in [
        include_str!("../../../aircraft/corvid/family.json"),
        include_str!("../../../aircraft/corvid/variant.json"),
        include_str!("../../../aircraft/corvid/engine.json"),
        include_str!("../../../aircraft/corvid/systems.json"),
        include_str!("../../../aircraft/corvid/panel.json"),
        include_str!("../../../aircraft/corvid/aircraft.json"),
    ] {
        let mut document: Value = serde_json::from_str(text).unwrap();
        assert!(validator.is_valid(&document), "{document}");
        document["unexpected"] = json!(true);
        assert!(!validator.is_valid(&document));
    }
    let schema = schemars::schema_for!(flightcraft_aero::PackManifest).to_value();
    jsonschema::meta::validate(&schema).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let mut manifest: Value =
        serde_json::from_str(include_str!("../../../examples/packs/demo-pack/pack.json")).unwrap();
    assert!(validator.is_valid(&manifest));
    manifest.as_object_mut().unwrap().remove("affiliation");
    assert!(!validator.is_valid(&manifest));
}

#[test]
fn aircraft_load_schema_requires_one_complete_selection_and_matches_command_validation() {
    let schema = schemars::schema_for!(flightcraft_control::LoadAircraft).to_value();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let mut session = Session::new().unwrap();
    for input in [
        json!({}),
        json!({"variant":"corvid-standard"}),
        json!({"variant":"corvid-standard","panel":"corvid-six"}),
        json!({"source":{"kind":"built_in","id":"corvid"},"load":{"stations":[]}}),
        json!({"source":null}),
        json!({"variant":"corvid-standard","panel":null,"load":{"stations":[]}}),
    ] {
        assert!(!validator.is_valid(&input), "{input}");
        assert!(session.execute("aircraft.load", input).is_err());
    }
}
