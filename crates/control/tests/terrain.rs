#![allow(clippy::unwrap_used)]
use flightcraft_control::Session;
use serde_json::json;

#[test]
fn world_choice_height_and_reset_validation() {
    let mut session = Session::new().unwrap();
    let flat = session
        .execute("world.height", json!({"north_m":2200,"east_m":3000}))
        .unwrap();
    assert_eq!(flat["height_msl_m"], 0.0);
    let valley = session
        .execute(
            "sim.reset",
            json!({"world":"training-valley","start":{"kind":"runway"}}),
        )
        .unwrap();
    assert_eq!(valley["world"], "training-valley");
    let hill = session
        .execute("world.height", json!({"north_m":2200,"east_m":3000}))
        .unwrap();
    assert!(hill["height_msl_m"].as_f64().unwrap() > 500.0);
    assert!(hill["normal_ned"].is_array());
    let runway = session
        .execute("world.height", json!({"north_m":450,"east_m":0}))
        .unwrap();
    assert_eq!(runway["surface"], "pavement");
    assert_eq!(runway["height_msl_m"], 0.0);
    let before = *session.sim().state();
    assert!(
        session
            .execute("sim.reset", json!({"world":"unknown","start":{"kind":"runway"}}))
            .is_err()
    );
    assert_eq!(session.sim().state(), &before);
    assert!(
        session
            .execute("world.height", json!({"north_m":1e100,"east_m":0}))
            .is_err()
    );
    session
        .execute("sim.reset", json!({"start":{"kind":"runway"}}))
        .unwrap();
    assert_eq!(session.execute("sim.state", json!({})).unwrap()["world"], "flat");
}
