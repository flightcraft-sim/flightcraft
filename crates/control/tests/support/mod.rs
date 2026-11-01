//! A feedback pilot whose entire view of the aircraft is JSON command output.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use serde_json::{Value, json};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

pub struct FlightReport {
    pub tool_calls: usize,
    pub touchdown_m: Option<[f64; 3]>,
}

fn wrap(angle: f64) -> f64 {
    (angle + PI).rem_euclid(TAU) - PI
}

fn number(object: &Value, field: &str) -> f64 {
    object[field].as_f64().unwrap()
}

fn on_runway(position: [f64; 3]) -> bool {
    // Corvid Field's published runway rectangle, in north/east metres.
    (0.0..=900.0).contains(&position[0]) && position[1].abs() <= 11.5
}

/// Fly the full pattern, or just its takeoff/climb prefix for the protocol test.
/// The callback is the only connection to a session; no simulation types or model helpers enter
/// this pilot. Gains and integral trims act on the instruments a remote client can observe.
pub fn fly(mut execute: impl FnMut(&str, Value) -> Value, full_pattern: bool) -> FlightReport {
    let mut tool_calls = 0;
    let mut call = |name: &str, input: Value| {
        tool_calls += 1;
        execute(name, input)
    };
    call("sim.reset", json!({"start": {"kind": "runway"}, "paused": true}));
    let mut took_off = false;
    let mut touchdown_m = None;
    let mut max_height: f64 = 0.0;
    let mut phase = 0;
    let mut pitch_trim = 0.0;
    let mut throttle_trim = 0.0;
    let mut previous_heading = 0.0;
    let mut heading_travel = 0.0;
    let mut turn_headings = Vec::new();
    let mut previous_on_ground = true;
    let mut stopped = false;
    for tick in 0..7200 {
        let snapshot = call("sim.state", json!({}));
        let r = &snapshot["readout"];
        let position: [f64; 3] = serde_json::from_value(snapshot["state"]["position_m"].clone()).unwrap();
        let rates: [f64; 3] = serde_json::from_value(snapshot["state"]["angular_rate_rps"].clone()).unwrap();
        let [north, east, _] = position;
        let height = number(r, "height_agl_m");
        let speed = number(r, "true_airspeed_mps");
        let heading = number(r, "heading_rad");
        let pitch = number(r, "pitch_rad");
        let roll = number(r, "roll_rad");
        let vertical_speed = number(r, "vertical_speed_mps");
        let on_ground = r["on_ground"].as_bool().unwrap();
        assert_eq!(r["crashed"], false, "crash in phase {phase}: {snapshot}");
        max_height = max_height.max(height);
        took_off |= !on_ground && height > 2.0;
        heading_travel += wrap(heading - previous_heading);
        previous_heading = heading;
        // Confirm each turn from the observed heading, independently of position-based leg
        // changes. The unwrapped angle also detects a missing turn or a turn the wrong way.
        let next_turn = (turn_headings.len() + 1) as f64 * FRAC_PI_2;
        if turn_headings.len() < 4 && phase > turn_headings.len() && heading_travel >= next_turn - 0.04 {
            turn_headings.push(heading_travel);
        }
        if !full_pattern && took_off && height >= 150.0 {
            println!("MCP climb: height={height:.2} m; tool calls={tool_calls}");
            return FlightReport {
                tool_calls,
                touchdown_m,
            };
        }
        if phase == 0 && north > 2800.0 && max_height >= 300.0 {
            phase = 1;
        }
        if phase == 1 && east > 1200.0 {
            phase = 2;
        }
        if phase == 2 && north < -2400.0 {
            phase = 3;
        }
        if phase == 3 && east < 250.0 {
            phase = 4;
        }
        if took_off && on_ground {
            assert_eq!(phase, 4, "landed before final approach: {snapshot}");
        }
        if phase == 4 && took_off && on_ground && !previous_on_ground && touchdown_m.is_none() {
            assert!(on_runway(position), "touchdown outside runway: {position:?}");
            touchdown_m = Some(position);
            println!(
                "touchdown t={:.1} at {position:?}; sink={vertical_speed:.2} m/s",
                number(r, "time_s")
            );
        }
        previous_on_ground = on_ground;
        if touchdown_m.is_some() && on_ground && number(r, "ground_speed_mps") < 0.2 {
            assert!(on_runway(position), "stopped outside runway: {position:?}");
            println!(
                "stopped t={:.1} at {position:?}; ground speed={:.3} m/s",
                number(r, "time_s"),
                number(r, "ground_speed_mps")
            );
            stopped = true;
            break;
        }
        let (desired_heading, desired_height, desired_speed) = match phase {
            0 => ((-east / 500.0).clamp(-0.35, 0.35), 315.0, 36.0),
            1 => (
                FRAC_PI_2 + ((north - 3100.0) / 600.0).clamp(-0.5, 0.5),
                315.0,
                40.0,
            ),
            2 => (PI + ((east - 1500.0) / 600.0).clamp(-0.5, 0.5), 315.0, 40.0),
            3 => (
                3.0 * FRAC_PI_2 - ((north + 2700.0) / 600.0).clamp(-0.5, 0.5),
                190.0,
                35.0,
            ),
            _ => (
                (-east * 0.004).clamp(-0.4, 0.4),
                ((100.0 - north) * 0.055 + 1.0).max(1.0),
                30.0,
            ),
        };
        let mut desired_climb = ((desired_height - height) * 0.10).clamp(-3.0, 3.5);
        if phase == 4 && height < 8.0 {
            desired_climb = -(0.35 + height * 0.12).min(1.2);
        }
        let bank = (wrap(desired_heading - heading) * 1.1).clamp(-0.40, 0.40);
        // Measured angle of attack plus desired flight-path angle supplies the pitch target.
        // Integral feedback learns the elevator and throttle trims without any aero tables.
        let mut target_pitch = (number(r, "alpha_rad")
            + desired_climb / speed.max(25.0)
            + (desired_climb - vertical_speed) * 0.015)
            .clamp(-0.12, 0.26);
        if !took_off {
            target_pitch = if speed < 28.0 { 0.0 } else { 0.20 };
        }
        let pitch_error = target_pitch - pitch;
        if took_off && touchdown_m.is_none() {
            pitch_trim = (pitch_trim + pitch_error * 0.04).clamp(-0.25, 0.35);
            throttle_trim = (throttle_trim + (desired_speed - speed) * 0.002).clamp(-0.6, 0.4);
        }
        let mut elevator = pitch_trim + 2.2 * pitch_error - 0.8 * rates[1];
        let mut throttle = (0.6 + throttle_trim + 0.08 * (desired_speed - speed)).clamp(0.0, 1.0);
        if !took_off {
            throttle = 1.0;
        }
        if phase == 4 && height < 4.0 {
            throttle = 0.0;
        }
        if touchdown_m.is_some() {
            elevator = -1.8 * pitch - 0.9 * rates[1];
            throttle = 0.0;
        }
        let rudder = if touchdown_m.is_some() {
            wrap(-heading) * 0.9 - east * 0.02
        } else {
            0.65 * (9.80665 * roll.tan() / speed.max(15.0) - rates[2]) - number(r, "beta_rad")
        };
        call(
            "sim.set_controls",
            json!({
                "elevator": elevator.clamp(-1.0, 1.0),
                "aileron": (1.6 * (bank - roll) - 0.4 * rates[0]).clamp(-1.0, 1.0),
                "rudder": rudder.clamp(-1.0, 1.0),
                "throttle": throttle,
                "flaps": 0.0,
                "brake": if touchdown_m.is_some() { 1.0 } else { 0.0 },
            }),
        );
        let stepped = call("sim.step", json!({"steps": 12}));
        assert_eq!(
            stepped["crashed"], false,
            "crash after step in phase {phase}: {stepped}"
        );
        if tick % 400 == 0 {
            println!(
                "t={:.1} phase={phase} pos=({north:.0},{east:.0}) h={height:.1} V={speed:.1} pitch={pitch:.3} head={heading:.3} trim={pitch_trim:.3}",
                number(r, "time_s")
            );
        }
    }
    assert!(took_off, "never took off");
    assert!(max_height >= 300.0, "climb reached only {max_height} m");
    assert_eq!(
        turn_headings.len(),
        4,
        "observed quarter-turn headings: {turn_headings:?}"
    );
    for (index, traveled) in turn_headings.iter().enumerate() {
        assert!((traveled - (index + 1) as f64 * FRAC_PI_2).abs() < 0.08);
    }
    assert!(
        (heading_travel - TAU).abs() < 0.1,
        "total heading change: {heading_travel}"
    );
    assert!(
        stopped,
        "did not land and stop in time; touchdown={touchdown_m:?}"
    );
    println!("pattern: max AGL={max_height:.2} m; turns={turn_headings:?}; tool calls={tool_calls}");
    FlightReport {
        tool_calls,
        touchdown_m,
    }
}
