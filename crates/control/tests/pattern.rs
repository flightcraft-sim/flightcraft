//! A future flight-quality acceptance test. All pilot interaction uses the JSON command boundary.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use flightcraft_control::Session;
use flightcraft_sim::Readout;
use serde_json::{Value, json};

fn execute(session: &mut Session, command: &str, input: Value) -> Value {
    session.execute(command, input).unwrap()
}

#[test]
#[ignore = "needs the flight model"]
fn agent_flies_a_traffic_pattern_and_lands_on_the_runway() {
    let mut session = Session::new().unwrap();
    execute(
        &mut session,
        "sim.reset",
        json!({"start": {"kind": "runway"}, "paused": true}),
    );
    // Upwind, crosswind, downwind, base, final: four right turns at 300 m.
    // Positions are north/east metres relative to the runway threshold.
    let waypoints: [(f64, f64); 5] = [
        (1800.0, 0.0),
        (1800.0, 1000.0),
        (-1200.0, 1000.0),
        (-1200.0, 0.0),
        (150.0, 0.0),
    ];
    let mut leg = 0;
    let mut reached_pattern_height = false;
    let mut took_off = false;
    let mut turns = 0;
    let mut last_heading = 0.0;
    let mut accumulated_turn_rad: f64 = 0.0;
    let mut landed = false;
    for _ in 0..4800 {
        let state = execute(&mut session, "sim.state", json!({}));
        let readout: Readout = serde_json::from_value(state["readout"].clone()).unwrap();
        assert!(!readout.crashed, "pattern must finish without a crash");
        let north = state["state"]["position_m"][0].as_f64().unwrap();
        let east = state["state"]["position_m"][1].as_f64().unwrap();
        if readout.height_agl_m > 10.0 {
            took_off = true;
        }
        if readout.height_agl_m >= 290.0 {
            reached_pattern_height = true;
        }
        let heading_change = (readout.heading_rad - last_heading + std::f64::consts::PI)
            .rem_euclid(std::f64::consts::TAU)
            - std::f64::consts::PI;
        accumulated_turn_rad += heading_change;
        last_heading = readout.heading_rad;
        let (target_north, target_east) = waypoints[leg];
        let dn = target_north - north;
        let de = target_east - east;
        if reached_pattern_height && dn.hypot(de) < 150.0 && leg < 4 {
            leg += 1;
            turns += 1;
        }
        let final_approach = leg == 4;
        // Descend along a 0.06 gradient to the runway threshold, then flare.
        let target_height = if final_approach {
            ((-north) * 0.06).clamp(0.5, 300.0)
        } else {
            300.0
        };
        let target_speed = if final_approach { 28.0 } else { 38.0 };
        let heading = de.atan2(dn).rem_euclid(std::f64::consts::TAU);
        let heading_error = (heading - readout.heading_rad + std::f64::consts::PI)
            .rem_euclid(std::f64::consts::TAU)
            - std::f64::consts::PI;
        let target_roll = (heading_error * 0.7).clamp(-0.35, 0.35);
        let target_pitch = ((target_height - readout.height_agl_m) * 0.003
            - readout.vertical_speed_mps * 0.025)
            .clamp(-0.10, 0.15);
        let elevator = if !took_off && readout.true_airspeed_mps < 27.0 {
            0.0
        } else {
            ((target_pitch - readout.pitch_rad) * 2.0).clamp(-1.0, 1.0)
        };
        let throttle = if !reached_pattern_height {
            1.0
        } else {
            (0.55 + (target_speed - readout.true_airspeed_mps) * 0.06).clamp(0.0, 1.0)
        };
        execute(
            &mut session,
            "sim.set_controls",
            json!({
                "elevator": elevator,
                "aileron": ((target_roll - readout.roll_rad) * 2.0).clamp(-1.0, 1.0),
                "rudder": (-readout.beta_rad * 0.5).clamp(-1.0, 1.0),
                "throttle": if final_approach && readout.height_agl_m < 2.0 { 0.0 } else { throttle },
                "flaps": if final_approach { 1.0 } else { 0.0 },
                "brake": if final_approach && readout.on_ground { 1.0 } else { 0.0 },
            }),
        );
        execute(&mut session, "sim.step", json!({"steps": 60}));
        if took_off && final_approach && readout.on_ground && readout.ground_speed_mps < 1.0 {
            assert!(
                (0.0..=900.0).contains(&north),
                "landing must be within runway length"
            );
            assert!(east.abs() <= 11.5, "landing must be within runway width");
            landed = true;
            break;
        }
    }
    assert!(took_off);
    assert!(reached_pattern_height, "climb must reach 300 m within 10 m");
    assert_eq!(turns, 4);
    assert!(accumulated_turn_rad > 5.5, "complete four right turns");
    assert!(
        landed,
        "complete the pattern and stop on the runway within 40 simulated minutes"
    );
}
