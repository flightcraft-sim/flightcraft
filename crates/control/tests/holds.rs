#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use flightcraft_control::{HoldTarget, MAX_STEPS, RunCondition, RunUntil, Session, SetAutopilot};
use serde_json::{Value, json};

fn airborne() -> Session {
    let mut s = Session::new().unwrap();
    s.execute(
        "sim.reset",
        json!({"start": {"kind": "air", "height_m": 350, "airspeed_mps": 40, "heading_rad": 0}}),
    )
    .unwrap();
    s
}

#[test]
fn holds_patch_disengage_reset_and_override_only_owned_controls() {
    let mut s = airborne();
    s.execute(
        "sim.set_controls",
        json!({"throttle": 0.7, "brake": 0.2, "flaps": 0.1, "aileron": 0.03, "rudder": 0.04}),
    )
    .unwrap();
    s.execute("autopilot.set", json!({"pitch_rad": 0.08})).unwrap();
    let r = s.execute("sim.step", json!({"steps": 120})).unwrap();
    assert_eq!(r["controls"]["throttle"], 0.7);
    assert_eq!(r["controls"]["aileron"], 0.03);
    assert_eq!(r["controls"]["rudder"], 0.04);
    assert_eq!(r["controls"]["brake"], 0.2);
    assert_eq!(r["controls"]["flaps"], 0.1);
    let a = s
        .execute("autopilot.set", json!({"heading_rad": 1.0, "airspeed_mps": 40}))
        .unwrap();
    assert_eq!(a["pitch_rad"], 0.08);
    let a = s
        .execute(
            "autopilot.set",
            json!({"pitch_rad": null, "heading_rad": false, "wings_level": true, "altitude_m": 400}),
        )
        .unwrap();
    assert_eq!(a["pitch_rad"], Value::Null);
    assert_eq!(a["heading_rad"], Value::Null);
    assert_eq!(a["wings_level"], true);
    assert_eq!(a["airspeed_mps"], 40.0);
    let a = s.execute("autopilot.set", json!({"wings_level": null})).unwrap();
    assert_eq!(a["wings_level"], false);
    let state = s.execute("sim.state", json!({})).unwrap();
    assert_eq!(state["readout"]["autopilot"], a);
    s.execute("autopilot.off", json!({})).unwrap();
    assert_eq!(s.sim().readout().autopilot, Default::default());
    s.execute("autopilot.set", json!({"pitch_rad": 0.1})).unwrap();
    s.execute("sim.reset", json!({"start": {"kind": "runway"}}))
        .unwrap();
    assert_eq!(s.sim().readout().autopilot, Default::default());
}

#[test]
fn hostile_holds_and_waits_are_atomic_errors_in_json_and_typed_calls() {
    let mut s = airborne();
    let before = s.execute("sim.state", json!({})).unwrap();
    for input in [
        json!({"altitude_m": -1}),
        json!({"altitude_m": 10001}),
        json!({"vertical_speed_mps": 9}),
        json!({"pitch_rad": 0.36}),
        json!({"heading_rad": 7}),
        json!({"airspeed_mps": 24}),
        json!({"bank_limit_rad": 0}),
        json!({"heading_rad": true}),
        json!({"pitch_rad": "NaN"}),
        json!({"airspeed_mode": "rudder"}),
        json!({"pitch_rad": 0.1, "altitude_m": 400}),
        json!({"wings_level": true, "heading_rad": 1}),
        json!({"altitude_m": 400, "airspeed_mps": 40, "airspeed_mode": "pitch"}),
    ] {
        assert!(s.execute("autopilot.set", input.clone()).is_err(), "{input}");
        assert_eq!(s.execute("sim.state", json!({})).unwrap(), before);
    }
    for input in [
        json!({"conditions": [], "max_steps": MAX_STEPS + 1}),
        json!({"conditions": [{"kind": "time_elapsed", "seconds": -1}]}),
        json!({"conditions": [{"kind": "heading_within", "heading_rad": 0, "tolerance_rad": 0}]}),
        json!({"conditions": [{"kind": "airspeed_above", "airspeed_mps": 151}]}),
        json!({"conditions": [{"kind": "altitude_below", "altitude_m": -1}]}),
        json!({"conditions": [{"kind": "on_ground", "surprise": true}]}),
        json!({"conditions": vec![json!({"kind": "on_ground"}); 17]}),
    ] {
        assert!(s.execute("sim.run_until", input.clone()).is_err(), "{input}");
        assert_eq!(s.execute("sim.state", json!({})).unwrap(), before);
    }
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(
            s.run(SetAutopilot {
                pitch_rad: Some(HoldTarget::Target(value)),
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            s.run(RunUntil {
                conditions: vec![RunCondition::TimeElapsed { seconds: value }],
                max_steps: 1
            })
            .is_err()
        );
        assert_eq!(s.execute("sim.state", json!({})).unwrap(), before);
    }
}

#[test]
fn wait_checks_initial_state_orders_ties_wraps_heading_and_reports_caps() {
    let mut s = airborne();
    for condition in [
        json!({"kind": "altitude_above", "altitude_m": 349}),
        json!({"kind": "altitude_below", "altitude_m": 351}),
        json!({"kind": "airspeed_above", "airspeed_mps": 39}),
        json!({"kind": "airspeed_below", "airspeed_mps": 41}),
        json!({"kind": "heading_within", "heading_rad": std::f64::consts::TAU - 0.01, "tolerance_rad": 0.02}),
    ] {
        let r = s
            .execute(
                "sim.run_until",
                json!({"conditions": [condition, {"kind": "time_elapsed", "seconds": 0}]}),
            )
            .unwrap();
        assert_eq!(r["steps"], 0);
        assert_eq!(r["fired"]["index"], 0);
    }
    for _ in 0..2 {
        let r = s
            .execute(
                "sim.run_until",
                json!({"conditions": [{"kind": "time_elapsed", "seconds": 0.1}]}),
            )
            .unwrap();
        assert_eq!(r["steps"], 12);
        assert_eq!(r["fired"]["condition"]["kind"], "time_elapsed");
    }
    let r = s
        .execute(
            "sim.run_until",
            json!({"conditions": [{"kind": "on_ground"}], "max_steps": 3}),
        )
        .unwrap();
    assert_eq!(r["steps"], 3);
    assert_eq!(r["fired"]["kind"], "step_cap");
    let r = s
        .execute("sim.run_until", json!({"conditions": [], "max_steps": 0}))
        .unwrap();
    assert_eq!(r["steps"], 0);
    assert_eq!(r["fired"]["kind"], "step_cap");
    let mut runway = Session::new().unwrap();
    let r = runway
        .execute("sim.run_until", json!({"conditions": [{"kind": "on_ground"}]}))
        .unwrap();
    assert_eq!(r["steps"], 0);
}

#[test]
fn wait_stops_on_crash_instead_of_exhausting_the_cap() {
    let mut s = Session::new().unwrap();
    s.execute(
        "sim.reset",
        json!({"start": {"kind": "air", "height_m": 20, "airspeed_mps": 1, "heading_rad": 0}}),
    )
    .unwrap();
    let r = s
        .execute(
            "sim.run_until",
            json!({"conditions": [{"kind": "time_elapsed", "seconds": 600}]}),
        )
        .unwrap();
    assert_eq!(r["fired"]["kind"], "crashed");
    assert!(r["steps"].as_u64().unwrap() < u64::from(MAX_STEPS));
    let r = s.execute("sim.run_until", json!({"conditions": []})).unwrap();
    assert_eq!(r["steps"], 0);
    assert_eq!(r["fired"]["kind"], "crashed");
}

#[test]
fn hold_schema_accepts_disengagement_and_rejects_hostile_targets() {
    let commands = Session::commands();
    let set = commands.iter().find(|c| c.name == "autopilot.set").unwrap();
    let validator = jsonschema::validator_for(&set.input_schema).unwrap();
    for input in [
        json!({}),
        json!({"altitude_m": null}),
        json!({"heading_rad": false}),
        json!({"wings_level": null}),
        json!({"airspeed_mps": 40}),
    ] {
        assert!(validator.is_valid(&input), "{input}");
    }
    for input in [
        json!({"altitude_m": -1}),
        json!({"heading_rad": true}),
        json!({"airspeed_mps": 24}),
        json!({"bank_limit_rad": null}),
    ] {
        assert!(!validator.is_valid(&input), "{input}");
    }
}
