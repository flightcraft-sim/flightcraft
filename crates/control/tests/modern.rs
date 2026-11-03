#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_control::*;
use serde_json::json;
fn session() -> Session {
    session_with_audio(false)
}
fn session_with_audio(audio: bool) -> Session {
    let mut s = Session::new().unwrap();
    let manifest =
        PackManifest::from_json(include_str!("../../../examples/packs/equipment-demo/pack.json")).unwrap();
    let files = [
        (
            "engine.json",
            include_str!("../../../examples/packs/equipment-demo/engine.json"),
        ),
        (
            "systems.json",
            include_str!("../../../examples/packs/equipment-demo/systems.json"),
        ),
        (
            "variant.json",
            include_str!("../../../examples/packs/equipment-demo/variant.json"),
        ),
        (
            "panel.json",
            include_str!("../../../examples/packs/equipment-demo/panel.json"),
        ),
        (
            "aircraft.json",
            include_str!("../../../examples/packs/equipment-demo/aircraft.json"),
        ),
    ]
    .into_iter()
    .map(|(k, v)| {
        if audio && k == "panel.json" {
            let mut panel: serde_json::Value = serde_json::from_str(v).unwrap();
            for element in panel["equipment"]["elements"].as_array_mut().unwrap() {
                if element["element"]["kind"] == "navigator" {
                    element["element"]["audio_panel"] = json!(true);
                }
            }
            (k.into(), serde_json::to_string(&panel).unwrap())
        } else {
            (k.into(), v.into())
        }
    })
    .collect();
    s.install_pack(&PackContents {
        manifest,
        files,
        assets: Default::default(),
    })
    .unwrap();
    s.execute(
        "aircraft.load",
        json!({"source":{"kind":"installed","id":"equipment-demonstrator"},"reset":true}),
    )
    .unwrap();
    s
}
fn set(s: &mut Session, setting: serde_json::Value) {
    s.execute("equipment.set", json!({"setting":setting})).unwrap();
}
#[test]
fn navigator_radios_shared_plan_and_invalid_commands() {
    let mut s = session();
    set(
        &mut s,
        json!({"kind":"navigator","action":{"kind":"tune","band":"com","frequency_khz":123450}}),
    );
    let before = s.touch_state().navigator.unwrap();
    set(
        &mut s,
        json!({"kind":"navigator","action":{"kind":"flip","band":"com"}}),
    );
    let after = s.touch_state().navigator.unwrap();
    assert_eq!(after.com_active_khz, 123450);
    assert_eq!(after.com_standby_khz, before.com_active_khz);
    set(
        &mut s,
        json!({"kind":"navigator","action":{"kind":"tune","band":"nav","frequency_khz":115500}}),
    );
    set(
        &mut s,
        json!({"kind":"navigator","action":{"kind":"flip","band":"nav"}}),
    );
    assert_eq!(s.touch_state().navigator.unwrap().nav_active_khz, 115500);
    set(
        &mut s,
        json!({"kind":"navigator","action":{"kind":"shared","setting":{"kind":"flight_plan_waypoints","waypoints":[{"ident":"TEST","kind":"coordinate","position":{"latitude_deg":40.,"longitude_deg":-74.}}]}}}),
    );
    assert_eq!(s.glass_state().settings.flight_plan, vec!["TEST"]);
    let snapshot = s.execute("avionics.state", json!({})).unwrap();
    assert_eq!(
        snapshot["glass_suite"]["settings"]["flight_plan"],
        json!(["TEST"])
    );
    for _ in 0..3 {
        assert_eq!(s.execute("avionics.state", json!({})).unwrap(), snapshot);
    }
    assert!(s.execute("equipment.set",json!({"setting":{"kind":"navigator","action":{"kind":"tune","band":"com","frequency_khz":123451}}})).is_err());
    assert_eq!(s.execute("avionics.state", json!({})).unwrap(), snapshot);
}
#[test]
fn independent_display_layout_reversion_and_reset() {
    let mut s = session();
    let initial = s.glass_state();
    assert_eq!(initial.touch.displays.len(), 2);
    assert!(!initial.touch.displays.iter().any(|d| d.reversion));
    set(
        &mut s,
        json!({"kind":"display","id":"left_display","action":{"kind":"layout","layout":"full_pfd"}}),
    );
    set(
        &mut s,
        json!({"kind":"display","id":"right_display","action":{"kind":"failure","failed":true}}),
    );
    let state = s.glass_state();
    assert!(state.touch.displays[0].reversion);
    assert!(state.touch.displays[1].failed);
    assert!(!state.touch.displays[1].reversion);
    assert_eq!(state.settings.heading_bug_rad, initial.settings.heading_bug_rad);
    assert_eq!(
        state.air_data.indicated_airspeed_mps,
        initial.air_data.indicated_airspeed_mps
    );
    set(
        &mut s,
        json!({"kind":"display","id":"right_display","action":{"kind":"failure","failed":false}}),
    );
    assert!(!s.touch_state().displays[0].reversion);
    s.execute(
        "systems.set",
        json!({"setting":{"kind":"breaker","load":"touch_right","closed":false}}),
    )
    .unwrap();
    s.execute("sim.step", json!({"steps":1})).unwrap();
    assert!(s.touch_state().displays[0].reversion);
    assert!(!s.touch_state().displays[1].powered);
    s.execute("sim.reset", json!({"start":{"kind":"runway"}}))
        .unwrap();
    assert!(!s.touch_state().displays.iter().any(|d| d.failed || d.reversion));
}
#[test]
fn knob_wraps_and_equipment_schema_is_registered() {
    let mut s = session();
    set(
        &mut s,
        json!({"kind":"navigator","action":{"kind":"tune","band":"com","frequency_khz":136975}}),
    );
    set(
        &mut s,
        json!({"kind":"navigator","action":{"kind":"knob","outer":false,"detents":1}}),
    );
    assert_eq!(s.touch_state().navigator.unwrap().com_standby_khz, 118000);
    let cmd = Session::commands()
        .into_iter()
        .find(|c| c.name == "equipment.set")
        .unwrap();
    let input = json!({"setting":{"kind":"systems","setting":{"kind":"parachute_pin","inserted":false}}});
    assert!(jsonschema::is_valid(&cmd.input_schema, &input));
    let output = s.execute("equipment.set", input).unwrap();
    assert!(jsonschema::is_valid(&cmd.output_schema, &output));
}

#[test]
fn equipment_documents_match_the_generated_schema() {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../../../schema/aircraft.json")).unwrap();
    for source in [
        include_str!("../../../examples/packs/equipment-demo/engine.json"),
        include_str!("../../../examples/packs/equipment-demo/systems.json"),
        include_str!("../../../examples/packs/equipment-demo/variant.json"),
        include_str!("../../../examples/packs/equipment-demo/panel.json"),
        include_str!("../../../examples/packs/equipment-demo/aircraft.json"),
    ] {
        let value: serde_json::Value = serde_json::from_str(source).unwrap();
        assert!(jsonschema::is_valid(&schema, &value));
    }
}

#[test]
fn navigator_drives_shared_radios_procedures_pins_and_magnetic_cdi() {
    let mut s = session();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/terrain/new-york");
    s.install_region(read_region(&path).unwrap()).unwrap();
    s.execute("sim.reset",json!({"world":"new-york","start":{"kind":"air","height_m":1200.,"airspeed_mps":45.,"heading_rad":1.}})).unwrap();
    set(
        &mut s,
        json!({"kind":"navigator","action":{"kind":"tune","band":"nav","frequency_khz":110300}}),
    );
    set(
        &mut s,
        json!({"kind":"navigator","action":{"kind":"flip","band":"nav"}}),
    );
    assert_eq!(s.sim().radios().receivers[0].active_hz, 110300000);
    s.execute("avionics.set",json!({"setting":{"kind":"nav_radio","setting":{"kind":"standby","radio":1,"frequency_hz":114000000}}})).unwrap();
    assert_eq!(s.touch_state().navigator.unwrap().nav_standby_khz, 114000);
    set(
        &mut s,
        json!({"kind":"navigator","action":{"kind":"avionics","setting":{"kind":"cdi_source","source":"nav1"}}}),
    );
    assert_eq!(s.glass_state().gps.source, "NAV1");
    set(
        &mut s,
        json!({"kind":"navigator","action":{"kind":"avionics","setting":{"kind":"cdi_source","source":"gps"}}}),
    );
    s.execute("pins.add",json!({"pin":{"id":"equipment-pin","name":"DEMO-PIN","position":{"latitude_deg":40.8,"longitude_deg":-74.7},"note":"Fictional navigation test"}})).unwrap();
    let navaid = s.sim().world().navdata().unwrap().navaids[0].ident.clone();
    let fix = s.sim().world().navdata().unwrap().fixes[0].ident.clone();
    set(
        &mut s,
        json!({"kind":"navigator","action":{"kind":"flight_plan","queries":[navaid,fix,"DEMO-PIN"]}}),
    );
    assert_eq!(s.flight_plan().waypoints.len(), 3);
    assert_eq!(s.glass_state().settings.flight_plan.len(), 3);
    assert_eq!(s.glass_state().settings.waypoints[2].kind, WaypointKind::Pin);
    set(
        &mut s,
        json!({"kind":"navigator","action":{"kind":"avionics","setting":{"kind":"load_procedure","airport":"1N7","procedure":"R07","transition":"KIMKE"}}}),
    );
    set(
        &mut s,
        json!({"kind":"navigator","action":{"kind":"avionics","setting":{"kind":"activate_approach"}}}),
    );
    s.execute("sim.step", json!({"steps":1})).unwrap();
    assert!(!s.sim().procedure_receiver().loaded.is_empty());
    let before = s.execute("avionics.state", json!({})).unwrap();
    assert_eq!(s.execute("avionics.state", json!({})).unwrap(), before);
    let reference = s.sim().heading_reference();
    let data = &s.glass_state().air_data;
    assert!((reference.declination_rad).abs() > 0.1);
    assert!(data.heading_rad.is_finite());
    // The navigator's breaker removes the selected receiver's signal, independently of displays.
    s.execute(
        "systems.set",
        json!({"setting":{"kind":"breaker","load":"navigator","closed":false}}),
    )
    .unwrap();
    s.execute("sim.step", json!({"steps":1})).unwrap();
    assert!(!s.touch_state().navigator.unwrap().powered);
    assert!(!s.sim().radio_state()[0].signal_valid);
}

#[test]
fn synthetic_vision_is_independent_per_display_and_resets_with_the_flight() {
    let mut s = session();
    assert!(s.touch_state().displays.iter().all(|d| !d.synthetic_vision));
    let input = json!({"setting":{"kind":"display","id":"left_display","action":{"kind":"synthetic_vision","enabled":true}}});
    let command = Session::commands()
        .into_iter()
        .find(|c| c.name == "equipment.set")
        .unwrap();
    assert!(jsonschema::is_valid(&command.input_schema, &input));
    let output = s.execute("equipment.set", input).unwrap();
    assert!(jsonschema::is_valid(&command.output_schema, &output));
    assert!(s.touch_state().displays[0].synthetic_vision);
    assert!(!s.touch_state().displays[1].synthetic_vision);
    s.execute(
        "avionics.set",
        json!({"setting":{"kind":"glass","setting":{"kind":"synthetic_vision","enabled":true}}}),
    )
    .unwrap();
    assert!(s.glass_state().settings.synthetic_vision);
    assert!(!s.touch_state().displays[1].synthetic_vision);
    assert!(s.execute("equipment.set",json!({"setting":{"kind":"display","id":"missing","action":{"kind":"synthetic_vision","enabled":true}}})).is_err());
    set(
        &mut s,
        json!({"kind":"display","id":"right_display","action":{"kind":"failure","failed":true}}),
    );
    assert!(s.touch_state().displays[0].synthetic_vision);
    assert!(s.touch_state().displays[0].reversion);
    s.execute("sim.reset", json!({"start":{"kind":"runway"}}))
        .unwrap();
    assert!(!s.glass_state().settings.synthetic_vision);
    assert!(s.touch_state().displays.iter().all(|d| !d.synthetic_vision));
}

#[test]
fn navigator_audio_page_is_opt_in_and_commands_share_comms_state() {
    let page = json!({"setting":{"kind":"navigator","action":{"kind":"page","page":"audio"}}});
    let mut legacy = session();
    assert!(!legacy.touch_state().navigator.unwrap().audio_panel);
    assert!(legacy.execute("equipment.set", page.clone()).is_err());
    let mut s = session_with_audio(true);
    assert!(s.touch_state().navigator.unwrap().audio_panel);
    let output = s.execute("equipment.set", page).unwrap();
    assert_eq!(output["navigator"]["page"], "audio");
    let schema = Session::commands()
        .into_iter()
        .find(|c| c.name == "equipment.set")
        .unwrap();
    assert!(jsonschema::is_valid(&schema.output_schema, &output));
    let before = *s.sim().state();
    for tx in [2, 1] {
        set(
            &mut s,
            json!({"kind":"audio_panel","setting":{
                "transmit":tx,"receive_com1":false,"receive_com2":true,"nav_ident":true,
                "intercom":false,"volume":0.4,"squelch":0.6,"speech":false,"captions":false
            }}),
        );
        let shared = s.execute("comms.audio_panel", json!({})).unwrap();
        assert_eq!(shared["panel"]["transmit"], tx);
        assert_eq!(shared["panel"]["receive_com1"], false);
        assert_eq!(shared["panel"]["receive_com2"], true);
        assert_eq!(shared["panel"]["nav_ident"], true);
        assert_eq!(shared["panel"]["intercom"], false);
        assert_eq!(shared["panel"]["volume"], 0.4);
        assert_eq!(shared["panel"]["squelch"], 0.6);
        assert_eq!(shared["panel"]["speech"], false);
        assert_eq!(shared["panel"]["captions"], false);
        assert_eq!(
            serde_json::to_value(s.glass_state().comms.panel).unwrap(),
            shared["panel"]
        );
    }
    assert_eq!(s.sim().state(), &before);
}
