#![allow(clippy::unwrap_used)]
use flightcraft_control::Session;
use serde_json::json;
fn set(s: &mut Session, setting: serde_json::Value) {
    s.execute("avionics.set", json!({"setting":setting})).unwrap();
}
fn selected(s: &Session) -> f64 {
    s.sim().systems().controls.heading_bug_rad.to_degrees()
}
#[test]
fn one_heading_wraps_and_drives_every_display_and_heading_hold() {
    for id in ["corvid", "corvid-standby", "corvid-glass"] {
        let mut s = Session::new().unwrap();
        s.execute(
            "aircraft.load",
            json!({"source":{"kind":"installed","id":id},"reset":true}),
        )
        .unwrap();
        s.execute("autopilot.set", json!({"heading_rad":0.5})).unwrap();
        set(&mut s, json!({"kind":"heading_bug","degrees":359.}));
        set(&mut s, json!({"kind":"heading_bug_step","detents":2}));
        assert!((selected(&s) - 1.).abs() < 1e-10);
        assert!((s.sim().systems().avionics.electronic_hsi.heading_bug_deg - 1.).abs() < 1e-10);
        assert!((s.glass_state().settings.heading_bug_rad.to_degrees() - 1.).abs() < 1e-10);
        assert!((s.sim().readout().autopilot.heading_rad.unwrap().to_degrees() - 1.).abs() < 1e-10);
        set(
            &mut s,
            json!({"kind":"glass","setting":{"kind":"heading_bug","heading_rad":57_f64.to_radians()}}),
        );
        assert!((selected(&s) - 57.).abs() < 1e-10);
        assert!((s.sim().systems().avionics.electronic_hsi.heading_bug_deg - 57.).abs() < 1e-10);
    }
}
#[test]
fn encoder_acceleration_wraps_both_directions_and_syncs_without_stepping() {
    let mut s = Session::new().unwrap();
    s.execute(
        "sim.reset",
        json!({"start":{"kind":"air","height_m":1000.,"airspeed_mps":45.,"heading_rad":1.2}}),
    )
    .unwrap();
    set(&mut s, json!({"kind":"heading_bug","degrees":359.}));
    for (detents, interval, expected) in [(1, 80, 0.), (1, 79, 10.), (-2, 20, 350.), (1, 200, 351.)] {
        set(
            &mut s,
            json!({"kind":"heading_bug_step","detents":detents,"interval_ms":interval}),
        );
        assert!((selected(&s) - expected).abs() < 1e-10);
    }
    let time = s.sim().state().time_s;
    let expected = libm::round(s.sim().systems().sensed.heading_rad.to_degrees()).rem_euclid(360.);
    set(&mut s, json!({"kind":"heading_bug_sync"}));
    assert!((selected(&s) - expected).abs() < 1e-10);
    assert_eq!(s.sim().state().time_s, time);
    let before = selected(&s);
    assert!(
        s.execute(
            "avionics.set",
            json!({"setting":{"kind":"heading_bug_step","detents":361}})
        )
        .is_err()
    );
    assert_eq!(selected(&s), before);
}
