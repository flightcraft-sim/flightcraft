#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_control::{Failure, Session, SetFailure, SetWeather};
use flightcraft_sim::{ControlSurface, Weather};
use serde_json::{Value, json};

fn weather() -> Value {
    json!({"wind_from_deg": 270.0, "wind_speed_mps": 4.0, "shear": true, "turbulence": "moderate", "seed": 839})
}

#[test]
fn set_clear_and_readout_report_weather_and_independent_failures() {
    let mut s = Session::new().unwrap();
    let w = s.execute("weather.set", weather()).unwrap();
    assert_eq!(w["weather"], weather());
    let e = s
        .execute("failure.set", json!({"failure": {"kind": "engine"}}))
        .unwrap();
    assert_eq!(e["failures"]["engine_power_fraction"], 0.0);
    for surface in ["elevator", "aileron", "rudder", "flaps"] {
        let f = s
            .execute(
                "failure.set",
                json!({"failure": {"kind": "jam", "surface": surface, "position": 0.2}}),
            )
            .unwrap();
        assert_eq!(f["failures"][surface], 0.2);
        assert_eq!(f["weather"], weather());
    }
    let f = s
        .execute(
            "failure.set",
            json!({"failure": {"kind": "partial_power", "fraction": 0.4}}),
        )
        .unwrap();
    assert_eq!(f["failures"]["engine_power_fraction"], 0.4);
    let f = s.execute("failure.clear", json!({"target": "elevator"})).unwrap();
    assert_eq!(f["failures"]["elevator"], Value::Null);
    assert_eq!(f["failures"]["rudder"], 0.2);
    assert_eq!(f["failures"]["engine_power_fraction"], 0.4);
    let state = s.execute("sim.state", json!({})).unwrap();
    assert_eq!(state["readout"]["weather"], weather());
    assert_eq!(state["readout"]["failures"], f["failures"]);
    let f = s.execute("failure.clear", json!({"target": "engine"})).unwrap();
    assert_eq!(f["failures"]["engine_power_fraction"], 1.0);
    assert_eq!(f["failures"]["rudder"], 0.2);
    s.execute("failure.clear", json!({})).unwrap();
    assert_eq!(s.sim().failures(), Default::default());
    s.execute("sim.reset", json!({"start": {"kind": "runway"}}))
        .unwrap();
    assert_eq!(s.sim().weather(), Weather::default());
}

#[test]
fn reset_seed_repeats_the_run_and_failed_reset_preserves_it() {
    let mut s = Session::new().unwrap();
    let reset = json!({"start": {"kind": "air", "height_m": 1000, "airspeed_mps": 40, "heading_rad": 0}, "weather": weather()});
    s.execute("sim.reset", reset.clone()).unwrap();
    s.execute("sim.step", json!({"steps": 1200})).unwrap();
    let hash = s.sim().state_hash();
    s.execute("failure.set", json!({"failure": {"kind": "engine"}}))
        .unwrap();
    s.execute("sim.reset", reset.clone()).unwrap();
    assert_eq!(s.sim().failures(), Default::default());
    s.execute("sim.step", json!({"steps": 1200})).unwrap();
    assert_eq!(s.sim().state_hash(), hash);
    let mut different = reset.clone();
    different["weather"]["seed"] = json!(840);
    s.execute("sim.reset", different).unwrap();
    s.execute("sim.step", json!({"steps": 1200})).unwrap();
    assert_ne!(s.sim().state_hash(), hash);
    let before = s.execute("sim.state", json!({})).unwrap();
    for invalid in [Value::Null, json!({}), json!({"wind_speed_mps": -1})] {
        let mut reset = reset.clone();
        reset["weather"] = invalid;
        assert!(s.execute("sim.reset", reset).is_err());
        assert_eq!(s.execute("sim.state", json!({})).unwrap(), before);
    }
}

#[test]
fn hostile_systems_inputs_are_atomic_errors() {
    let mut s = Session::new().unwrap();
    s.execute("weather.set", weather()).unwrap();
    let before = s.execute("sim.state", json!({})).unwrap();
    for (command, input) in [
        (
            "failure.set",
            json!({"failure": {"kind": "partial_power", "fraction": -0.1}}),
        ),
        (
            "failure.set",
            json!({"failure": {"kind": "partial_power", "fraction": 1.1}}),
        ),
        ("failure.set", json!({"failure": {"kind": "engine", "extra": 1}})),
        (
            "failure.set",
            json!({"failure": {"kind": "jam", "surface": "elevator", "position": 2}}),
        ),
        (
            "failure.set",
            json!({"failure": {"kind": "jam", "surface": "flaps", "position": -0.1}}),
        ),
        (
            "failure.set",
            json!({"failure": {"kind": "jam", "surface": "spoiler", "position": 0}}),
        ),
        ("failure.set", json!({"failure": null})),
        ("failure.clear", json!({"target": "spoiler"})),
        ("failure.clear", json!({"target": null})),
    ] {
        assert!(s.execute(command, input.clone()).is_err(), "{command}: {input}");
        assert_eq!(s.execute("sim.state", json!({})).unwrap(), before);
    }
    for (field, value) in [
        ("wind_from_deg", json!(-1)),
        ("wind_from_deg", json!(361)),
        ("wind_speed_mps", json!(-1)),
        ("wind_speed_mps", json!(76)),
        ("seed", json!(-1)),
        ("seed", json!(1.5)),
        ("seed", json!(null)),
        ("turbulence", json!("extreme")),
        ("shear", json!(1)),
        ("unknown", json!(true)),
    ] {
        let mut input = weather();
        input[field] = value;
        assert!(s.execute("weather.set", input.clone()).is_err(), "{input}");
        assert_eq!(s.execute("sim.state", json!({})).unwrap(), before);
    }
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(
            s.run(SetWeather {
                weather: Weather {
                    wind_speed_mps: value,
                    ..Weather::default()
                }
            })
            .is_err()
        );
        assert!(
            s.run(SetFailure {
                failure: Failure::PartialPower { fraction: value }
            })
            .is_err()
        );
        assert!(
            s.run(SetFailure {
                failure: Failure::Jam {
                    surface: ControlSurface::Elevator,
                    position: value
                }
            })
            .is_err()
        );
        assert_eq!(s.execute("sim.state", json!({})).unwrap(), before);
    }
}
