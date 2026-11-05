//! Vertical navigation end to end: altitude constraints on flight-plan waypoints become a vertical
//! path, the glass shows the deviation, and the digital autopilot's GP mode tracks it down to the
//! constraint and holds the level there. The approach glidepath is a separate path and is tested
//! in `procedures.rs`.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
#[cfg(test)]
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/region_data.rs"));
use flightcraft_control::{Session, Step, read_region};
use flightcraft_sim::equipment::VerticalMode;
use serde_json::{Value, json};

fn set(s: &mut Session, setting: Value) {
    s.execute("avionics.set", json!({"setting":setting})).unwrap();
}
fn panel(s: &mut Session, setting: Value) {
    set(s, json!({"kind":"autopilot","setting":setting}));
}
fn altitude_m(s: &Session) -> f64 {
    s.glass_state().air_data.altitude_msl_m
}

/// A glass-equipped flight level at `altitude` m, heading north, with a plan to a fix `miles`
/// due north that must be crossed at `constraint_m`, then a second fix 4 NM farther on, and the
/// selected altitude at `selected_m`.
fn flight(altitude: f64, miles: f64, constraint_m: f64, selected_m: f64) -> Session {
    let mut s = Session::new().unwrap();
    s.execute(
        "aircraft.load",
        json!({"source":{"kind":"installed","id":"corvid-glass"},"reset":true}),
    )
    .unwrap();
    s.install_region(
        read_region(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/terrain/new-york"))
            .unwrap(),
    )
    .unwrap();
    s.execute(
        "sim.reset",
        json!({"world":"new-york","start":{"kind":"air","height_m":altitude,"airspeed_mps":45.,"heading_rad":0.}}),
    )
    .unwrap();
    s.execute(
        "assist.set",
        json!({"altitude_m":altitude,"heading_rad":0.,"airspeed_mps":45.}),
    )
    .unwrap();
    s.run(Step { steps: 3600 }).unwrap();
    s.execute("assist.off", json!({})).unwrap();
    let [n, e, _] = s.sim().state().position_m;
    let [lat, lon] = s.sim().world().geodetic(n, e).unwrap();
    set(
        &mut s,
        json!({"kind":"glass","setting":{"kind":"flight_plan_waypoints","waypoints":[{
            "ident":"DESCNT","kind":"fix",
            "position":{"latitude_deg": lat + miles / 60., "longitude_deg": lon},
            "altitude_constraint":{"kind":"at","altitude_m":constraint_m}
        },{
            "ident":"BEYOND","kind":"fix",
            "position":{"latitude_deg": lat + (miles + 4.) / 60., "longitude_deg": lon}
        }]}}),
    );
    set(
        &mut s,
        json!({"kind":"glass","setting":{"kind":"selected_altitude","altitude_m":selected_m}}),
    );
    s
}

#[test]
fn gp_mode_tracks_the_constraint_path_down_and_holds_the_constraint_altitude_past_the_fix() {
    require_region_data!();
    // A 3 degree path to 600 m over 8 NM is above 1,200 m until about 3.5 km before the fix. The
    // selected altitude is below the constraint, so only the path stops the descent.
    let mut s = flight(1200., 8., 600., 300.);
    let start = altitude_m(&s);
    assert!((start - 1200.).abs() < 40., "{start}");
    let v = s.glass_state().vnav.unwrap();
    assert_eq!(v.target_ident, "DESCNT");
    assert!(v.deviation_m < 0., "below the path to start with: {v:?}");
    panel(&mut s, json!({"kind":"engage","on":true}));
    panel(&mut s, json!({"kind":"vertical","mode":"alt"}));
    panel(&mut s, json!({"kind":"lateral","mode":"nav"}));
    // VNAV arms the path mode; it captures when the aircraft meets the path.
    panel(&mut s, json!({"kind":"vertical","mode":"gp"}));
    let mut captured_at = None;
    let mut min_altitude = f64::MAX;
    // A step is 1/120 s, so 60 steps are half a second: the 8 NM take about 700 of these.
    for round in 0..900u32 {
        s.run(Step { steps: 60 }).unwrap();
        let ap = s.sim().installed_autopilot().unwrap();
        if captured_at.is_none() && ap.active_vertical == Some(VerticalMode::Gp) {
            captured_at = Some(round);
        }
        assert!(ap.engaged, "the autopilot stayed engaged at round {round}");
        min_altitude = min_altitude.min(altitude_m(&s));
        // Once tracking, the aircraft stays near the path until the last mile before the fix.
        if captured_at.is_some_and(|c| round > c + 20)
            && let Some(v) = s.glass_state().vnav
            && v.target_distance_nm > 1.
        {
            assert!(v.deviation_m.abs() < 60., "round {round}: {v:?}");
        }
    }
    let captured = captured_at.expect("GP captured the path");
    assert!(
        captured > 100,
        "captured only on meeting the path, not at once: round {captured}"
    );
    // Past the fix the plan has moved on to the next leg, which has no constraint: the aircraft
    // came down to the constraint altitude and holds it there with GP still flying.
    assert_eq!(s.glass_state().gps.active_leg, 1, "the first fix was sequenced");
    let end = altitude_m(&s);
    assert!((end - 600.).abs() < 40., "ended at {end} m");
    assert!(
        min_altitude > 600. - 40.,
        "never far below the constraint: {min_altitude}"
    );
    let ap = s.sim().installed_autopilot().unwrap();
    assert!(ap.engaged && ap.active_vertical == Some(VerticalMode::Gp));
}

#[test]
fn a_plan_without_constraints_leaves_gp_armed_and_the_autopilot_alone() {
    require_region_data!();
    let mut s = flight(1200., 8., 600., 300.);
    set(
        &mut s,
        json!({"kind":"glass","setting":{"kind":"waypoint_altitude","index":0,"constraint":null}}),
    );
    assert!(s.glass_state().vnav.is_none());
    panel(&mut s, json!({"kind":"engage","on":true}));
    panel(&mut s, json!({"kind":"vertical","mode":"alt"}));
    panel(&mut s, json!({"kind":"vertical","mode":"gp"}));
    s.run(Step { steps: 60 * 60 }).unwrap();
    let ap = s.sim().installed_autopilot().unwrap();
    assert_eq!(
        ap.active_vertical,
        Some(VerticalMode::Alt),
        "GP never captures without a path"
    );
    assert_eq!(ap.armed_vertical, Some(VerticalMode::Gp));
    assert!((altitude_m(&s) - 1200.).abs() < 40.);
}
