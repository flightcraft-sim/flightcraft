#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_control::{Session, read_pack};
use serde_json::{Value, json};
fn session() -> Session {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/packs/equipment-demo");
    let mut pack = read_pack(&path).unwrap();
    let mut systems: Value = serde_json::from_str(&pack.files["systems.json"]).unwrap();
    systems["definition"]["modern"]["backup_buses"] = json!([
        {"circuit":"ems","capacity_ah":1.,"voltage":12.,"loads":["pump_1"]},
        {"circuit":"efis","capacity_ah":0.01,"voltage":12.,"loads":["touch_left"]}
    ]);
    systems["definition"]["modern"]["wigwag"] =
        json!({"left_load":"landing","right_load":"landing_right","half_period_s":0.5});
    let loads = systems["definition"]["electrical"]["loads"]
        .as_array_mut()
        .unwrap();
    for load in loads.iter_mut() {
        if load["id"] == "touch_left" || load["id"] == "pump_1" {
            load["backup_capacity_ah"] = json!(0.);
        }
    }
    let mut right = loads.iter().find(|v| v["id"] == "landing").unwrap().clone();
    right["id"] = json!("landing_right");
    loads.push(right);
    pack.files.insert("systems.json".into(), systems.to_string());
    let mut panel: Value = serde_json::from_str(&pack.files["panel.json"]).unwrap();
    panel["equipment"]["elements"].as_array_mut().unwrap().push(json!({"id":"guarded_lane","position":[1.,1.],"size":[20.,30.],"element":{"kind":"toggle","label":"A","guarded":true,"lamp":true},"appearance":{"binding":"lane_a"}}));
    panel["equipment"]["elements"][0]["element"]["layout"] = json!("integrated_pfd");
    pack.files.insert("panel.json".into(), panel.to_string());
    let mut s = Session::new().unwrap();
    s.install_pack(&pack).unwrap();
    s.execute(
        "aircraft.load",
        json!({"source":{"kind":"installed","id":"equipment-demonstrator"},"reset":true}),
    )
    .unwrap();
    s
}
fn modern(s: &mut Session, setting: Value) {
    s.execute(
        "equipment.set",
        json!({"setting":{"kind":"systems","setting":setting}}),
    )
    .unwrap();
}
fn step(s: &mut Session, n: u32) {
    s.execute("sim.step", json!({"steps":n})).unwrap();
}
fn load<'a>(s: &'a Session, id: &str) -> &'a flightcraft_sim::systems::LoadState {
    s.sim()
        .systems()
        .electrical
        .loads
        .iter()
        .find(|l| l.id == id)
        .unwrap()
}
#[test]
fn backup_switches_route_only_named_loads_and_deplete_without_main_power() {
    let mut s = session();
    s.execute(
        "systems.set",
        json!({"setting":{"kind":"switch","switch":"master","on":false}}),
    )
    .unwrap();
    step(&mut s, 1);
    assert!(!load(&s, "touch_left").powered);
    modern(&mut s, json!({"kind":"backup_power","circuit":"efis","on":true}));
    step(&mut s, 1);
    assert!(load(&s, "touch_left").powered && load(&s, "touch_left").on_backup);
    assert!(!load(&s, "touch_right").powered);
    let state = s.sim().systems().modern.as_ref().unwrap();
    assert_eq!(state.backup_buses[1].voltage, 12.);
    assert!(state.backup_buses[1].remaining_ah < 0.01);
    s.execute(
        "systems.set",
        json!({"setting":{"kind":"breaker","load":"touch_left","closed":false}}),
    )
    .unwrap();
    step(&mut s, 1);
    assert!(!load(&s, "touch_left").powered);
    s.execute(
        "systems.set",
        json!({"setting":{"kind":"breaker","load":"touch_left","closed":true}}),
    )
    .unwrap();
    modern(&mut s, json!({"kind":"backup_power","circuit":"ems","on":true}));
    step(&mut s, 1);
    assert!(load(&s, "pump_1").powered);
    modern(&mut s, json!({"kind":"backup_power","circuit":"efis","on":false}));
    step(&mut s, 1);
    assert!(!load(&s, "touch_left").powered);
    assert!(load(&s, "pump_1").powered);
    modern(&mut s, json!({"kind":"backup_power","circuit":"efis","on":true}));
    step(&mut s, 2400);
    assert!(!load(&s, "touch_left").powered);
    assert_eq!(
        s.sim().systems().modern.as_ref().unwrap().backup_buses[1].remaining_ah,
        0.
    );
}
#[test]
fn guarded_lane_binding_operates_engine_and_reports_actual_fault() {
    let mut s = session();
    assert!(s.execute("equipment.set",json!({"setting":{"kind":"systems","setting":{"kind":"panel_control","id":"guarded_lane","value":0.}}})).is_err());
    assert!(s.sim().systems().modern.as_ref().unwrap().lane_a.on);
    modern(
        &mut s,
        json!({"kind":"panel_guard","id":"guarded_lane","open":true}),
    );
    modern(
        &mut s,
        json!({"kind":"panel_control","id":"guarded_lane","value":0.}),
    );
    step(&mut s, 1);
    let lane = &s.sim().systems().modern.as_ref().unwrap().lane_a;
    assert!(!lane.on && !lane.running && lane.light);
    modern(
        &mut s,
        json!({"kind":"panel_control","id":"guarded_lane","value":1.}),
    );
    step(&mut s, 1);
    assert!(s.sim().systems().modern.as_ref().unwrap().lane_a.running);
    modern(
        &mut s,
        json!({"kind":"failure","component":"lane_a","failed":true}),
    );
    step(&mut s, 1);
    assert!(s.sim().systems().modern.as_ref().unwrap().lane_a.light);
}
#[test]
fn wigwag_alternates_on_fixed_steps_and_batches_identically() {
    let mut a = session();
    let mut b = session();
    for s in [&mut a, &mut b] {
        modern(s, json!({"kind":"wigwag","on":true}));
    }
    step(&mut a, 30);
    assert!(load(&a, "landing").powered);
    assert!(!load(&a, "landing_right").powered);
    step(&mut a, 60);
    assert!(!load(&a, "landing").powered);
    assert!(load(&a, "landing_right").powered);
    step(&mut b, 90);
    assert_eq!(a.sim().systems(), b.sim().systems());
    let snapshot = a.execute("sim.state", json!({})).unwrap();
    for _ in 0..3 {
        assert_eq!(a.execute("sim.state", json!({})).unwrap(), snapshot);
    }
    modern(&mut a, json!({"kind":"wigwag","on":false}));
    step(&mut a, 1);
    assert!(load(&a, "landing").powered && load(&a, "landing_right").powered);
    assert!(a.touch_state().displays[0].synthetic_vision);
}

#[test]
fn fidelity_example_and_new_commands_match_generated_schemas() {
    let schema: Value = serde_json::from_str(include_str!("../../../schema/aircraft.json")).unwrap();
    let pack = read_pack(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/packs/panel-fidelity-demo"),
    )
    .unwrap();
    for (name, file) in &pack.files {
        let value: Value = serde_json::from_str(file).unwrap();
        assert!(jsonschema::is_valid(&schema, &value), "{name}");
    }
    let mut s = session();
    for setting in [
        json!({"kind":"backup_power","circuit":"efis","on":true}),
        json!({"kind":"wigwag","on":true}),
    ] {
        let input = json!({"setting":{"kind":"systems","setting":setting}});
        let command = Session::commands()
            .into_iter()
            .find(|c| c.name == "equipment.set")
            .unwrap();
        assert!(jsonschema::is_valid(&command.input_schema, &input));
        let output = s.execute("equipment.set", input).unwrap();
        assert!(jsonschema::is_valid(&command.output_schema, &output));
    }
}
