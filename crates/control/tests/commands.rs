#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use flightcraft_control::{CommandError, MAX_STEPS, Reset, ResetStart, Session, SetControls, SetRate, Step};
use serde_json::{Value, json};

#[test]
fn every_schema_is_a_valid_json_schema_object_and_matches_a_result() {
    let mut session = Session::new().unwrap();
    let examples = [
        ("sim.reset", json!({"start": {"kind": "runway"}})),
        ("world.height", json!({"north_m":450,"east_m":0})),
        ("world.list", json!({})),
        ("airspace.list", json!({})),
        ("airspace.at", json!({})),
        ("view.set_airspace", json!({"visible":false})),
        ("sim.set_controls", json!({"throttle": 0.7})),
        ("sim.step", json!({"steps": 120})),
        ("autopilot.set", json!({"heading_rad": 0.1, "pitch_rad": 0.0})),
        ("autopilot.off", json!({})),
        (
            "sim.run_until",
            json!({"conditions": [{"kind": "time_elapsed", "seconds": 0.1}]}),
        ),
        ("sim.state", json!({})),
        (
            "weather.set",
            json!({"wind_from_deg": 270, "wind_speed_mps": 3, "shear": false, "turbulence": "light", "seed": 42}),
        ),
        (
            "systems.set",
            json!({"setting":{"kind":"switch","switch":"lights","on":true}}),
        ),
        (
            "avionics.set",
            json!({"setting":{"kind":"glass","setting":{"kind":"map_range","range_nm":20.0}}}),
        ),
        ("avionics.state", json!({})),
        ("failure.set", json!({"failure": {"kind": "engine"}})),
        ("failure.clear", json!({})),
        ("sim.pause", json!({})),
        ("sim.resume", json!({})),
        ("sim.set_rate", json!({"rate": 2})),
        ("commands.list", json!({})),
        ("aircraft.list", json!({})),
        (
            "aircraft.load",
            json!({"source":{"kind":"json","text":flightcraft_aero::CORVID_JSON}}),
        ),
        (
            "weight_balance.compute",
            json!({"variant":"corvid-standard","load":{"stations":[]}}),
        ),
    ];
    let registry = Session::commands();
    // App and geographic commands are exercised with their own session setup.
    assert_eq!(registry.len(), examples.len() + 5);
    for command in &registry {
        for schema in [&command.input_schema, &command.output_schema] {
            jsonschema::meta::validate(schema).unwrap();
        }
    }
    for (command, (name, input)) in registry.iter().zip(examples) {
        assert_eq!(command.name, name);
        for schema in [&command.input_schema, &command.output_schema] {
            assert!(schema.is_object());
            assert_eq!(schema["type"], "object");
            jsonschema::meta::validate(schema).unwrap();
        }
        let input_validator = jsonschema::validator_for(&command.input_schema).unwrap();
        assert!(input_validator.is_valid(&input), "{name}: {input}");
        for property in command.input_schema["properties"]
            .as_object()
            .into_iter()
            .flat_map(|object| object.values())
        {
            if let Some(default) = property.get("default") {
                // A property may reference a shared type in the command's root schema.
                let mut schema = property.clone();
                if let Some(definitions) = command.input_schema.get("$defs") {
                    schema["$defs"] = definitions.clone();
                }
                assert!(jsonschema::validator_for(&schema).unwrap().is_valid(default));
            }
        }
        let output = session.execute(name, input).unwrap();
        let output_validator = jsonschema::validator_for(&command.output_schema).unwrap();
        assert!(output_validator.is_valid(&output), "{name}: {output}");
    }
}

#[test]
fn controls_are_partial_and_validation_is_atomic() {
    let mut session = Session::new().unwrap();
    session
        .execute("sim.set_controls", json!({"throttle": 0.7, "elevator": 0.2}))
        .unwrap();
    session.execute("sim.set_controls", json!({"brake": 1})).unwrap();
    let controls = session.sim().controls();
    assert_eq!(controls.throttle, 0.7);
    assert_eq!(controls.elevator, 0.2);
    assert_eq!(controls.brake, 1.0);
    assert!(
        session
            .execute("sim.set_controls", json!({"elevator": 0.5, "brake": 2}))
            .is_err()
    );
    assert_eq!(session.sim().controls(), controls);
    assert_eq!(
        session.execute("sim.set_controls", json!({})).unwrap()["throttle"],
        0.7
    );
}

#[test]
fn playback_settings_and_explicit_steps() {
    let mut session = Session::new().unwrap();
    assert!(session.paused());
    assert_eq!(session.rate(), 1.0);
    session.execute("sim.resume", json!({})).unwrap();
    assert!(!session.paused());
    session.execute("sim.set_rate", json!({"rate": 8})).unwrap();
    session.execute("sim.pause", json!({})).unwrap();
    session.execute("sim.step", json!({"steps": 120})).unwrap();
    let state = session.execute("sim.state", json!({})).unwrap();
    assert_eq!(state["state"]["time_s"], 1.0);
    assert_eq!(state["readout"]["time_s"], 1.0);
    assert_eq!(state["rate"], 8.0);
    assert_eq!(state["paused"], true);
    let reset = session.execute("sim.reset", json!({"start": {"kind": "air", "height_m": 300, "airspeed_mps": 40, "heading_rad": 1.2}, "paused": false})).unwrap();
    assert_eq!(reset["state"]["time_s"], 0.0);
    assert!(!session.paused());
    assert_eq!(session.rate(), 8.0);
    session
        .execute("sim.reset", json!({"start": {"kind": "runway"}}))
        .unwrap();
    assert!(session.paused());
    session.run(Step { steps: MAX_STEPS }).unwrap();
    assert_eq!(session.sim().state().time_s, 600.0);
    session.run(Step { steps: 0 }).unwrap();
    assert_eq!(session.sim().state().time_s, 600.0);
}

#[test]
fn hostile_inputs_are_errors_without_mutation() {
    let mut session = Session::new().unwrap();
    for command in Session::commands() {
        for input in [
            Value::Null,
            json!(false),
            json!(42),
            json!("NaN"),
            json!([]),
            json!({"surprise": 1}),
        ] {
            assert!(
                session.execute(command.name, input.clone()).is_err(),
                "{}: {input}",
                command.name
            );
        }
    }
    let cases = [
        ("missing.command", json!({})),
        ("sim.step", json!({"steps": MAX_STEPS + 1})),
        ("sim.step", json!({"steps": u64::MAX})),
        ("sim.step", json!({"steps": -1})),
        ("sim.step", json!({"steps": 1.5})),
        ("sim.step", json!({"steps": "NaN"})),
        ("sim.set_controls", json!({"throttle": "NaN"})),
        ("sim.set_controls", json!({"throttle": null})),
        ("sim.set_controls", json!({"elevator": -1.01})),
        ("sim.set_controls", json!({"aileron": 1.01})),
        ("sim.set_controls", json!({"rudder": -2})),
        ("sim.set_controls", json!({"throttle": 2})),
        ("sim.set_controls", json!({"flaps": -0.1})),
        ("sim.set_controls", json!({"brake": 2})),
        ("sim.set_rate", json!({"rate": 0.249})),
        ("sim.set_rate", json!({"rate": 8.001})),
        ("sim.set_rate", json!({"rate": "NaN"})),
        (
            "sim.reset",
            json!({"start": {"kind": "air", "height_m": -1, "airspeed_mps": 30, "heading_rad": 0}}),
        ),
        (
            "sim.reset",
            json!({"start": {"kind": "air", "height_m": 300, "airspeed_mps": -1, "heading_rad": 0}}),
        ),
        (
            "sim.reset",
            json!({"start": {"kind": "air", "height_m": 300, "airspeed_mps": 30, "heading_rad": 7}}),
        ),
        (
            "sim.reset",
            json!({"start": {"kind": "air", "height_m": "NaN", "airspeed_mps": 30, "heading_rad": 0}}),
        ),
        ("sim.reset", json!({"start": {"kind": "runway", "height_m": 300}})),
        ("sim.reset", json!({"start": {"kind": "space"}})),
        ("sim.reset", json!({"start": {"kind": "air"}})),
        ("sim.reset", json!({"start": {"kind": "runway"}, "paused": null})),
    ];
    let before = session.execute("sim.state", json!({})).unwrap();
    for (name, input) in cases {
        assert!(session.execute(name, input.clone()).is_err(), "{name}: {input}");
        assert_eq!(session.execute("sim.state", json!({})).unwrap(), before);
    }
    let error = session
        .execute("sim.set_controls", json!({"throttle": "NaN"}))
        .unwrap_err();
    assert!(matches!(error, CommandError::InvalidInput { .. }));
    assert!(error.to_string().contains("throttle"));
    let error = session
        .execute(
            "sim.reset",
            json!({"start": {"kind": "air", "height_m": "NaN", "airspeed_mps": 30, "heading_rad": 0}}),
        )
        .unwrap_err();
    assert!(error.to_string().contains("start"));
}

#[test]
fn typed_commands_reject_nonfinite_and_out_of_range_values_too() {
    let mut session = Session::new().unwrap();
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0, 9.0] {
        assert!(session.run(SetRate { rate: value }).is_err());
        assert!(
            session
                .run(SetControls {
                    throttle: Some(value),
                    ..SetControls::default()
                })
                .is_err()
        );
    }
    assert!(
        session
            .run(Reset {
                world: Default::default(),
                start: ResetStart::Air {
                    height_m: f64::NAN,
                    airspeed_mps: 30.0,
                    heading_rad: 0.0
                },
                weather: None,
                paused: None
            })
            .is_err()
    );
    assert!(session.run(Step { steps: u32::MAX }).is_err());
    for value in [0.25, 8.0] {
        session.run(SetRate { rate: value }).unwrap();
    }
}
