#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_control::Session;
use serde_json::json;
fn install(s: &mut Session) {
    let mut config = flightcraft_aero::AircraftCatalog::builtins()
        .unwrap()
        .resolve_aircraft("corvid")
        .unwrap();
    config
        .installed_systems
        .definition
        .as_mut()
        .unwrap()
        .sliding_canopy = true;
    s.sim_mut().configure_systems(&config).unwrap();
}
#[test]
fn canopy_commands_checklists_snapshots_and_airborne_procedure() {
    let mut s = Session::new().unwrap();
    s.execute(
        "sim.reset",
        json!({"start":{"kind":"parked","north_m":50.,"east_m":50.,"heading_rad":0.}}),
    )
    .unwrap();
    install(&mut s);
    assert_eq!(
        s.execute("sim.state", json!({})).unwrap()["systems"]["canopy"]["open_fraction"],
        1.
    );
    for phase in ["before_takeoff", "securing"] {
        assert_eq!(
            s.execute("aircraft.checklist", json!({"phase":phase})).unwrap()["passed"],
            false
        );
    }
    for action in ["close", "latch"] {
        s.execute("aircraft.canopy", json!({"action":action})).unwrap();
    }
    assert_eq!(
        s.execute("aircraft.checklist", json!({"phase":"before_takeoff"}))
            .unwrap()["passed"],
        true
    );
    assert!(s.screen_snapshot().systems.canopy.secure());
    let command_schema = schemars::schema_for!(flightcraft_control::SetCanopy).to_value();
    let command_validator = jsonschema::validator_for(&command_schema).unwrap();
    let before = s.sim().canopy();
    for value in [
        json!({"action":"tilt"}),
        json!({"action":"open","fraction":1.1}),
        json!({"action":"open","fraction":null}),
        json!({"action":"close","script":"bad"}),
        json!({"action":"close","fraction":0.}),
    ] {
        assert!(!command_validator.is_valid(&value));
        assert!(s.execute("aircraft.canopy", value).is_err());
        assert_eq!(s.sim().canopy(), before);
    }
    s.execute(
        "sim.reset",
        json!({"start":{"kind":"air","height_m":1000.,"airspeed_mps":40.,"heading_rad":0.}}),
    )
    .unwrap();
    install(&mut s);
    s.execute("aircraft.canopy", json!({"action":"unlatch"})).unwrap();
    let d = s.scenario_state().emergency.as_ref().unwrap();
    assert_eq!(d.procedure.id, "door_open");
    assert_eq!(d.procedure.title, "Canopy unlatched");
    assert!(
        s.execute("aircraft.canopy", json!({"action":"open"}))
            .unwrap_err()
            .to_string()
            .contains("ground")
    );
    s.execute("failure.clear", json!({})).unwrap();
    assert!(s.sim().failures().door_open);
    s.execute("aircraft.canopy", json!({"action":"latch"})).unwrap();
    assert!(!s.sim().failures().door_open);
}
#[test]
fn slide_format_version_and_hostile_travel() {
    let mut manifest: serde_json::Value =
        serde_json::from_str(include_str!("../../../examples/packs/demo-pack/pack.json")).unwrap();
    let mut exterior: serde_json::Value =
        serde_json::from_str(include_str!("../../../aircraft/corvid/exterior.json")).unwrap();
    exterior["moving_parts"] =
        json!([{"node":"canopy_slide","motion":"slide","axis":[-1.,0.,0.],"travel_m":1.1}]);
    manifest["exterior"] = json!([exterior.clone()]);
    manifest["format_version"] = json!(2);
    assert!(flightcraft_aero::PackManifest::from_json(&manifest.to_string()).is_err());
    let pack_schema = schemars::schema_for!(flightcraft_aero::PackManifest).to_value();
    let pack_validator = jsonschema::validator_for(&pack_schema).unwrap();
    assert!(!pack_validator.is_valid(&manifest));
    manifest["format_version"] = json!(3);
    assert!(pack_validator.is_valid(&manifest));
    assert!(flightcraft_aero::PackManifest::from_json(&manifest.to_string()).is_ok());
    let schema = schemars::schema_for!(flightcraft_aero::exterior::Exterior).to_value();
    let validator = jsonschema::validator_for(&schema).unwrap();
    assert!(validator.is_valid(&exterior));
    for value in [json!(-1.), json!(0.), json!(3.1), json!(null), json!("aft")] {
        let mut bad = exterior.clone();
        bad["moving_parts"][0]["travel_m"] = value;
        let valid = serde_json::from_value::<flightcraft_aero::exterior::Exterior>(bad.clone())
            .is_ok_and(|d| d.validate().is_ok());
        assert!(!valid);
        assert!(!validator.is_valid(&bad));
    }
    let mut missing = exterior.clone();
    missing["moving_parts"][0]
        .as_object_mut()
        .unwrap()
        .remove("travel_m");
    assert!(!validator.is_valid(&missing));
    let mut angular = exterior;
    angular["moving_parts"][0]["motion"] = json!("rudder");
    assert!(!validator.is_valid(&angular));
}

#[test]
fn virtual_pilot_secures_canopy_before_ground_power_and_speaks_on_intercom() {
    let mut s = Session::new().unwrap();
    s.execute(
        "sim.reset",
        json!({"start":{"kind":"parked","north_m":0.,"east_m":0.,"heading_rad":0.}}),
    )
    .unwrap();
    install(&mut s);
    assert!(!s.sim().canopy().secure());
    s.execute("crew.mode", json!({"mode":"watching","director":false}))
        .unwrap();
    s.execute(
        "crew.intent",
        json!({"intent":{"kind":"fly_to_airport","airport":"default","runway":"36","altitude_m":315.}}),
    )
    .unwrap();
    s.execute("crew.handover", json!({"to":"crew","actor":"crew"}))
        .unwrap();
    s.execute(
        "crew.handover",
        json!({"to":"crew","actor":"pilot","confirm":true}),
    )
    .unwrap();
    s.execute("sim.step", json!({"steps":1})).unwrap();
    assert!(s.sim().canopy().secure());
    assert!(
        s.flight_record()
            .events
            .iter()
            .any(|e| e.channel == flightcraft_control::Channel::Intercom
                && e.text == "Before taxi: canopy closed and latched.")
    );
}
