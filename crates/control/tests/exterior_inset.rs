#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
//! The exterior inset: its setting, when it shows, and which part it frames after a control moves.
use flightcraft_control::screens::{GROUND_EXTERIOR_ID, INSET_ID, Layout, PageKind, ScreenSet};
use flightcraft_control::*;
use serde_json::{Value, json};

fn run(s: &mut Session, name: &str, input: Value) -> Value {
    s.execute(name, input).unwrap()
}
fn step(s: &mut Session, seconds: f64) {
    run(s, "sim.step", json!({"steps": (seconds * 120.).round() as u32}));
}

#[test]
fn the_setting_defaults_to_the_ground_and_persists_through_preferences() {
    let mut s = Session::new().unwrap();
    let state = run(&mut s, "view.exterior_inset", json!({}));
    assert_eq!(state["mode"], "ground");
    assert_eq!(state["hold_s"], 4.0);
    run(
        &mut s,
        "view.exterior_inset",
        json!({"mode":"always","hold_s":6.0,"placement":{"x":0.5,"y":0.1,"w":0.3,"h":0.3}}),
    );
    let saved = serde_json::to_string(s.view_preferences()).unwrap();
    let mut reopened = Session::new().unwrap();
    reopened
        .restore_view_preferences(serde_json::from_str(&saved).unwrap())
        .unwrap();
    assert_eq!(reopened.view_preferences(), s.view_preferences());
    let state = run(&mut reopened, "view.exterior_inset", json!({}));
    assert_eq!(state["mode"], "always");
    assert_eq!(state["hold_s"], 6.0);
    // Bad values are refused and leave the setting alone.
    assert!(s.execute("view.exterior_inset", json!({"hold_s": 0.0})).is_err());
    assert!(
        s.execute("view.exterior_inset", json!({"mode": "sometimes"}))
            .is_err()
    );
    assert!(
        serde_json::from_str::<ViewPreferences>(r#"{"framing_hold_s":900.0}"#)
            .unwrap()
            .validate()
            .is_err()
    );
    // Old preference files without the new fields still load, with defaults.
    let old: ViewPreferences = serde_json::from_str("{}").unwrap();
    assert_eq!(old.exterior_inset, InsetMode::Ground);
    // A placement that would leave the outside view is pulled back inside it.
    run(
        &mut s,
        "view.exterior_inset",
        json!({"placement":{"x":0.95,"y":0.95,"w":0.5,"h":0.5}}),
    );
    let p = s.view_preferences().placement;
    assert!(p.x + p.w <= 1.0001 && p.y + p.h <= 1.0001);
}

#[test]
fn the_inset_shows_per_mode_on_the_ground_and_in_flight() {
    let mut s = Session::new().unwrap();
    let visible = |s: &mut Session| run(s, "view.exterior_inset", json!({}))["visible"] == true;
    for (mode, ground, air) in [
        ("off", false, false),
        ("ground", true, false),
        ("always", true, true),
    ] {
        run(&mut s, "sim.reset", json!({"start":{"kind":"runway"}}));
        run(&mut s, "view.exterior_inset", json!({"mode": mode}));
        assert_eq!(visible(&mut s), ground, "{mode} on the ground");
        run(
            &mut s,
            "sim.reset",
            json!({"start":{"kind":"air","height_m":500.0,"airspeed_mps":50.0,"heading_rad":0.0}}),
        );
        assert_eq!(visible(&mut s), air, "{mode} in flight");
    }
}

#[test]
fn moving_a_control_frames_that_part_then_eases_back_to_default() {
    let mut s = Session::new().unwrap();
    run(&mut s, "view.exterior_inset", json!({"hold_s": 2.0}));
    let target = |s: &mut Session| {
        run(s, "view.exterior_inset", json!({}))["target"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    step(&mut s, 0.2);
    assert_eq!(target(&mut s), "default");
    for (controls, expected) in [
        (json!({"flaps": 0.5}), "flaps"),
        (json!({"aileron": 0.8}), "ailerons"),
        (json!({"elevator": 0.6}), "tail"),
        (json!({"brake": 1.0}), "wheels"),
    ] {
        run(&mut s, "sim.set_controls", controls);
        step(&mut s, 0.2);
        assert_eq!(target(&mut s), expected);
        assert!(
            run(&mut s, "view.exterior_inset", json!({}))["remaining_s"]
                .as_f64()
                .unwrap()
                > 1.0
        );
        step(&mut s, 2.5);
        assert_eq!(
            target(&mut s),
            "default",
            "{expected} returns to default after the timeout"
        );
    }
    // The trim wheel frames the tail too; the canopy frames the cockpit.
    run(
        &mut s,
        "systems.set",
        json!({"setting":{"kind":"trim","value":0.1}}),
    );
    step(&mut s, 0.2);
    assert_eq!(target(&mut s), "tail");
}

#[test]
fn ground_layout_splits_the_band_and_the_inset_stays_inside_it() {
    let set = ScreenSet::default();
    let base = &set.windows[0].layout;
    let ground = base.ground(true, false);
    let panel = |l: &Layout| {
        l.elements
            .iter()
            .find(|e| e.page.kind == PageKind::Panel)
            .unwrap()
            .clone()
    };
    assert_eq!(
        panel(&ground).position,
        panel(base).position,
        "the dash is untouched"
    );
    assert_eq!(panel(&ground).size, panel(base).size);
    let checklist = ground
        .elements
        .iter()
        .find(|e| e.page.kind == PageKind::Checklist)
        .unwrap();
    let exterior = ground
        .elements
        .iter()
        .find(|e| e.id == GROUND_EXTERIOR_ID)
        .unwrap();
    assert!(
        checklist.size[0] > exterior.size[0],
        "the exterior is the smaller box"
    );
    assert!((checklist.position[0] + checklist.size[0] - exterior.position[0]).abs() < 0.01);
    assert!((exterior.position[0] + exterior.size[0] - base.canvas[0]).abs() < 0.5);
    ground.validate().unwrap();
    let stacked = base.ground(true, true);
    let c = stacked
        .elements
        .iter()
        .find(|e| e.page.kind == PageKind::Checklist)
        .unwrap();
    let x = stacked
        .elements
        .iter()
        .find(|e| e.id == GROUND_EXTERIOR_ID)
        .unwrap();
    assert!((c.position[1] + c.size[1] - x.position[1]).abs() < 0.01 && c.size[0] == x.size[0]);
    let no_exterior = base.ground(false, false);
    assert!(no_exterior.elements.iter().all(|e| e.id != GROUND_EXTERIOR_ID));
    let inset = base.with_inset(InsetPlacement {
        x: 0.9,
        y: 0.9,
        w: 0.5,
        h: 0.5,
    });
    let b = inset.elements.iter().find(|e| e.id == INSET_ID).unwrap();
    let p = panel(&inset);
    assert!(
        b.position[1] + b.size[1] <= p.position[1] + 0.01,
        "never covers the dash"
    );
    assert!(b.position[0] + b.size[0] <= base.canvas[0] + 0.01);
    inset.validate().unwrap();
}
