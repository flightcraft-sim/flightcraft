#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_control::Session;
use serde_json::json;
#[test]
fn commands_validate_atomically_and_wind_is_preserved() {
    let mut s = Session::new().unwrap();
    s.execute(
        "weather.set",
        json!({"wind_from_deg":270.,"wind_speed_mps":5.,"shear":true,"turbulence":"light","seed":42}),
    )
    .unwrap();
    let before = s.pilot_weather();
    s.execute("weather.set", json!({"preset":"ifr"})).unwrap();
    assert_eq!(s.pilot_weather(), before);
    let state = s.execute("weather.state", json!({})).unwrap();
    assert_eq!(state["conditions"]["visibility_sm"], 2.);
    for input in [
        json!({"visibility_sm":0}),
        json!({"layers":[{"base_ft_msl":700,"top_ft_msl":600,"coverage":"OVC"}]}),
        json!({"preset":"lifr","wind_speed_mps":3.}),
        json!({"mist":true}),
    ] {
        assert!(s.execute("weather.set", input).is_err());
        assert_eq!(s.execute("weather.state", json!({})).unwrap(), state);
    }
    // Headless scenarios use this hook too; it requires no app or dialog.
    s.execute("view.hood", json!({"enabled":true})).unwrap();
    assert!(s.sim().hood());
    for name in ["weather.set", "weather.state", "view.hood"] {
        let info = Session::commands().into_iter().find(|c| c.name == name).unwrap();
        jsonschema::meta::validate(&info.input_schema).unwrap();
        let result = s
            .execute(
                if name == "view.hood" {
                    "view.hood"
                } else {
                    "weather.state"
                },
                if name == "view.hood" {
                    json!({"enabled":true})
                } else {
                    json!({})
                },
            )
            .unwrap();
        assert!(
            jsonschema::validator_for(&info.output_schema)
                .unwrap()
                .is_valid(&result)
        );
    }
}
#[test]
fn deterministic_separate_practice_totals_and_default_trajectory() {
    fn start() -> Session {
        let mut s = Session::new().unwrap();
        s.execute(
            "sim.reset",
            json!({"start":{"kind":"air","height_m":213.36,"airspeed_mps":40.,"heading_rad":0.}}),
        )
        .unwrap();
        s
    }
    let mut clear = start();
    let mut cloudy = start();
    let mut repeat = start();
    for s in [&mut cloudy, &mut repeat] {
        s.execute("weather.set", json!({"preset":"ifr"})).unwrap();
        s.execute("view.hood", json!({"enabled":true})).unwrap();
        s.execute("sim.step", json!({"steps":120})).unwrap();
    }
    clear.execute("sim.step", json!({"steps":120})).unwrap();
    assert_eq!(clear.sim().state(), cloudy.sim().state());
    assert_ne!(clear.sim().state_hash(), cloudy.sim().state_hash());
    assert_eq!(cloudy.sim().state_hash(), repeat.sim().state_hash());
    let time = cloudy.sim().instrument_time();
    assert!((time.hood_s - 1.).abs() < 1e-12);
    assert!((time.imc_s - 1.).abs() < 1e-12);
    cloudy.execute("view.hood", json!({"enabled":false})).unwrap();
    cloudy.execute("sim.step", json!({"steps":120})).unwrap();
    assert_eq!(cloudy.sim().instrument_time().hood_s, time.hood_s);
    assert!(cloudy.sim().instrument_time().imc_s > time.imc_s);
    cloudy
        .execute("sim.reset", json!({"start":{"kind":"runway"}}))
        .unwrap();
    assert_eq!(cloudy.sim().instrument_time(), Default::default());
    assert!(!cloudy.sim().hood());
}

#[test]
fn report_qnh_matches_the_pressure_instruments() {
    let mut s = Session::new().unwrap();
    s.execute(
        "sim.reset",
        json!({"start":{"kind":"air","height_m":1000.,"airspeed_mps":40.,"heading_rad":0.}}),
    )
    .unwrap();
    s.execute("weather.set", json!({"sea_level_pressure_pa":99000.}))
        .unwrap();
    s.execute("sim.step", json!({"steps":1})).unwrap();
    let error = (s.sim().systems().sensed.altitude_msl_m - s.sim().readout().altitude_msl_m).abs();
    assert!(error > 100.);
    s.execute(
        "systems.set",
        json!({"setting":{"kind":"altimeter","pressure_pa":99000.}}),
    )
    .unwrap();
    s.execute("sim.step", json!({"steps":1})).unwrap();
    assert!((s.sim().systems().sensed.altitude_msl_m - s.sim().readout().altitude_msl_m).abs() < 0.1);
    let before = s.execute("weather.state", json!({})).unwrap();
    for input in [
        json!({"temperature_c":f64::NAN}),
        json!({"temperature_c":5.,"dewpoint_c":6.}),
        json!({"sea_level_pressure_pa":79000.}),
    ] {
        assert!(s.execute("weather.set", input).is_err());
        assert_eq!(s.execute("weather.state", json!({})).unwrap(), before);
    }
}
