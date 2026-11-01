#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_control::{AppRequest, CommandError, Session, Step};
use serde_json::json;

#[test]
fn app_commands_validate_then_require_the_app() {
    let mut session = Session::new().unwrap();
    for (name, input) in [
        ("view.set_camera", json!({"mode":"chase"})),
        ("app.screenshot", json!({"directory":"captures"})),
        ("sim.wait", json!({"seconds":1})),
    ] {
        let error = session.execute(name, input).unwrap_err();
        assert!(error.to_string().contains("needs the app"), "{error}");
    }
    session.enable_app();
    let camera = session
        .execute(
            "view.set_camera",
            json!({"mode":"cockpit","distance_m":30,"height_m":12}),
        )
        .unwrap();
    assert_eq!(camera["distance_m"], 30.0);
    assert_eq!(camera["mode"], "cockpit");
    let previous = session.camera();
    assert!(
        session
            .execute("view.set_camera", json!({"mode":"chase","height_m":-1}))
            .is_err()
    );
    assert_eq!(session.camera(), previous);
    for input in [
        json!({}),
        json!({"base64":false}),
        json!({"directory":""}),
        json!({"base64":true,"extra":1}),
    ] {
        assert!(matches!(
            session.execute("app.screenshot", input),
            Err(CommandError::InvalidInput { .. })
        ));
    }
    assert!(matches!(
        session.execute("app.screenshot", json!({"base64":true})),
        Err(CommandError::AppPending(AppRequest::Screenshot(_)))
    ));
    for seconds in [-1.0, 61.0] {
        assert!(session.execute("sim.wait", json!({"seconds":seconds})).is_err());
    }
    assert!(
        session
            .execute("sim.wait", json!({"seconds":1}))
            .unwrap_err()
            .to_string()
            .contains("resume")
    );
}

#[test]
fn app_explicit_steps_require_pause_but_frame_steps_and_realtime_wait_do_not() {
    let mut session = Session::new().unwrap();
    session.enable_app();
    session.execute("sim.resume", json!({})).unwrap();
    for (name, input) in [
        ("sim.step", json!({"steps":120})),
        ("sim.run_until", json!({"conditions":[]})),
    ] {
        assert!(
            session
                .execute(name, input)
                .unwrap_err()
                .to_string()
                .contains("pause first")
        );
        assert_eq!(session.sim().state().time_s, 0.0);
    }
    assert!(matches!(
        session.execute("sim.wait", json!({"seconds":1})),
        Err(CommandError::AppPending(AppRequest::Wait { seconds: 1.0 }))
    ));
    session.run(Step { steps: 120 }).unwrap();
    assert_eq!(session.sim().state().time_s, 1.0);
    session.execute("sim.pause", json!({})).unwrap();
    session.execute("sim.step", json!({"steps":120})).unwrap();
    assert_eq!(session.sim().state().time_s, 2.0);
}

#[test]
fn holds_own_controls_until_disengaged() {
    let mut session = Session::new().unwrap();
    assert!(!session.holds_active());
    session
        .execute("autopilot.set", json!({"bank_limit_rad":0.3}))
        .unwrap();
    assert!(!session.holds_active());
    session
        .execute("autopilot.set", json!({"wings_level":true}))
        .unwrap();
    assert!(session.holds_active());
    session.execute("autopilot.off", json!({})).unwrap();
    assert!(!session.holds_active());
}
