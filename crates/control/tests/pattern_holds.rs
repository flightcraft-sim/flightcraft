//! An entire traffic pattern using JSON observations, hold targets, and bounded waits.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use flightcraft_control::Session;
use serde_json::{Value, json};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

struct Pilot {
    session: Session,
    calls: usize,
}

impl Pilot {
    fn call(&mut self, name: &str, input: Value) -> Value {
        self.calls += 1;
        assert!(self.calls < 60, "exceeded command budget");
        let output = self.session.execute(name, input).unwrap();
        if let Some(r) = output.get("readout") {
            assert_eq!(r["crashed"], false, "{output}");
        }
        output
    }

    fn hold(&mut self, targets: Value) {
        self.call("autopilot.set", targets);
    }

    fn until(&mut self, condition: Value) -> Value {
        let out = self.call(
            "sim.run_until",
            json!({"conditions": [condition], "max_steps": 72000}),
        );
        assert_eq!(out["fired"]["kind"], "condition", "{out}");
        out
    }

    fn wait(&mut self, seconds: f64) -> Value {
        self.until(json!({"kind": "time_elapsed", "seconds": seconds.max(0.0)}))
    }

    fn turn(&mut self, heading: f64) -> Value {
        self.hold(json!({"heading_rad": heading}));
        let out =
            self.until(json!({"kind": "heading_within", "heading_rad": heading, "tolerance_rad": 0.0002}));
        println!(
            "turn heading={heading:.4}; north={:.2} m; east={:.2} m",
            north(&out),
            east(&out)
        );
        assert!((number(&out, "altitude_msl_m") - 315.0).abs() < 15.0);
        out
    }
}

fn number(output: &Value, field: &str) -> f64 {
    output["readout"][field].as_f64().unwrap()
}
fn north(output: &Value) -> f64 {
    output["state"]["position_m"][0].as_f64().unwrap()
}
fn east(output: &Value) -> f64 {
    output["state"]["position_m"][1].as_f64().unwrap()
}
fn runway(output: &Value) -> bool {
    (0.0..900.0).contains(&north(output)) && east(output).abs() <= 11.5
}

#[test]
fn agent_flies_pattern_with_holds_in_fewer_than_sixty_calls() {
    let mut p = Pilot {
        session: Session::new().unwrap(),
        calls: 0,
    };
    p.call("sim.reset", json!({"start": {"kind": "runway"}}));
    p.call("sim.set_controls", json!({"throttle": 1.0}));
    p.hold(json!({"pitch_rad": 0.0, "heading_rad": 0.0}));
    p.until(json!({"kind": "airspeed_above", "airspeed_mps": 28.0}));
    p.hold(json!({"pitch_rad": 0.20}));
    p.until(json!({"kind": "altitude_above", "altitude_m": 25.0}));
    p.hold(json!({"pitch_rad": null, "airspeed_mps": 36.0, "airspeed_mode": "pitch"}));
    let climbed = p.until(json!({"kind": "altitude_above", "altitude_m": 300.0}));
    assert!(number(&climbed, "altitude_msl_m") >= 300.0);
    p.hold(json!({"altitude_m": 315.0, "airspeed_mode": "throttle", "airspeed_mps": 40.0}));
    // Wait to the upwind turn using only returned position and speed observations.
    let entry = p.wait((3000.0 - north(&climbed)) / 40.0);
    let crosswind = p.turn(FRAC_PI_2);
    assert!(number(&crosswind, "height_agl_m") >= 300.0);
    let turn_north = north(&crosswind) - north(&entry);
    let crosswind_end = p.wait((1700.0 - east(&crosswind)) / 40.0);
    assert!(east(&crosswind_end) > 1600.0);
    let downwind = p.turn(PI);
    let downwind_end = p.wait((north(&downwind) + 4000.0) / 40.0);
    assert!(north(&downwind_end) < -3900.0);
    let base = p.turn(3.0 * FRAC_PI_2);
    // Equal-speed quarter turns are rotationally symmetric. Measure the first turn's
    // northward displacement to choose the final turn's east coordinate.
    p.wait((east(&base) - turn_north) / 40.0);
    let final_entry = p.turn(0.0);
    // Aim at the threshold using the observed position to remove residual cross-track error.
    let approach_heading = (-east(&final_entry)).atan2(-north(&final_entry)).rem_euclid(TAU);
    p.hold(json!({"altitude_m": false, "vertical_speed_mps": -3.0, "airspeed_mps": 30.0, "heading_rad": approach_heading}));
    p.until(json!({"kind": "altitude_below", "altitude_m": 9.0}));
    p.hold(json!({"vertical_speed_mps": -0.6}));
    p.until(json!({"kind": "altitude_below", "altitude_m": 4.0}));
    p.hold(json!({"airspeed_mps": null}));
    p.call("sim.set_controls", json!({"throttle": 0.0}));
    let touchdown = p.until(json!({"kind": "on_ground"}));
    assert!(runway(&touchdown), "touchdown outside runway: {touchdown}");
    p.hold(json!({"vertical_speed_mps": null, "pitch_rad": 0.0}));
    p.call("sim.set_controls", json!({"throttle": 0.0, "brake": 1.0}));
    let stopped = p.until(json!({"kind": "airspeed_below", "airspeed_mps": 0.2}));
    assert!(runway(&stopped), "stopped outside runway: {stopped}");
    assert_eq!(stopped["readout"]["on_ground"], true);
    p.call("autopilot.off", json!({}));
    println!(
        "hold pattern: {} command calls; touchdown N={:.2} m, E={:.2} m; stopped N={:.2} m, E={:.2} m",
        p.calls,
        north(&touchdown),
        east(&touchdown),
        north(&stopped),
        east(&stopped)
    );
    assert!(p.calls < 60);
}
