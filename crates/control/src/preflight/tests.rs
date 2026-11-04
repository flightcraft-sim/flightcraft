#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::expect_used)]
use super::*;
use flightcraft_aero::preflight::{Action, DefectKind};
use flightcraft_sim::Controls;

fn fixture() -> Preflight {
    serde_json::from_str(include_str!("../../tests/fixtures/sling-preflight.json")).unwrap()
}

fn begun(options: BeginOptions) -> Session {
    let mut s = Session::new().unwrap();
    s.begin_preflight_with(
        fixture(),
        0.85,
        BeginOptions {
            speed: Some(16.),
            ..options
        },
    )
    .unwrap();
    s
}

fn state(s: &Session) -> PreflightState {
    s.preflight_state(true)
}

fn perform(s: &mut Session, item: &str) {
    s.run(PreflightPerform { item: item.into() }).unwrap();
}

/// Run the walkaround clock until the figure is idle with nothing queued.
fn settle(s: &mut Session) -> Vec<PreflightState> {
    let mut samples = Vec::new();
    for _ in 0..40000 {
        let st = s.run(PreflightAdvance { seconds: 0.03 }).unwrap();
        let idle = st.queue.is_empty() && st.current_item.is_none();
        samples.push(st);
        if idle {
            return samples;
        }
    }
    panic!("the walkaround never finished");
}

fn do_item(s: &mut Session, item: &str) -> Vec<PreflightState> {
    perform(s, item);
    settle(s)
}

fn item<'a>(st: &'a PreflightState, id: &str) -> &'a ItemView {
    st.items.iter().find(|i| i.id == id).unwrap()
}

fn all_required(s: &mut Session) {
    all_required_except(s, &[]);
}

fn all_required_except(s: &mut Session, skip: &[&str]) {
    let ids: Vec<_> = fixture()
        .items
        .iter()
        .filter(|i| !i.optional && !skip.contains(&i.id.as_str()))
        .map(|i| i.id.clone())
        .collect();
    for id in ids {
        perform(s, &id);
    }
    settle(s);
}

fn throttle_up(s: &mut Session) {
    s.sim_mut().set_controls(Controls {
        throttle: 1.,
        ..Default::default()
    });
}

#[test]
fn fixture_validates_and_matches_the_handbook_list() {
    let def = fixture();
    def.validate().unwrap();
    let counts: Vec<_> = def
        .stations
        .iter()
        .map(|st| {
            (
                st.id.as_str(),
                def.items.iter().filter(|i| i.station == st.id).count(),
            )
        })
        .collect();
    assert_eq!(
        counts,
        [
            ("cabin", 13),
            ("fuselage", 6),
            ("empennage", 6),
            ("right_wing", 13),
            ("nose", 15),
            ("left_wing", 14),
        ]
    );
    // Walk order is the handbook's: cabin, fuselage, empennage, right wing, nose, left wing.
    assert!(def.source.contains("4.4"));
    for i in &def.items {
        assert!(
            i.poh_ref.starts_with("4.4 ("),
            "{} lacks a handbook reference",
            i.id
        );
        assert!(!i.text.is_empty());
    }
    // The handbook's two warnings and its caution are carried on their items.
    for id in ["fuel_level_right", "fuel_level_left"] {
        let w = def.item(id).unwrap().warning.as_deref().unwrap();
        assert!(w.contains("30 litres"));
    }
    assert!(
        def.item("propeller_turn")
            .unwrap()
            .warning
            .as_deref()
            .unwrap()
            .contains("palm")
    );
    assert!(def.item("propeller_turn").unwrap().optional);
    assert!((def.fuel.covers_bottom_l - 30.).abs() < 1e-9);
}

#[test]
fn state_lists_stations_items_camera_and_the_figure() {
    let s = begun(BeginOptions::default());
    let st = s.preflight_state(false);
    assert!(st.active);
    assert_eq!(st.stations.len(), 6);
    assert_eq!(st.stations[0].id, "cabin");
    assert_eq!(st.stations[0].order, 1);
    assert_eq!(st.items.len(), 67);
    assert_eq!(st.current_station.as_deref(), Some("cabin"));
    let camera = st.camera.unwrap();
    assert_eq!(camera.station, "cabin");
    assert!(camera.eye_offset_m[2] < 0., "the view is elevated (z is down)");
    assert_eq!(st.action, FigureAction::Idle);
    assert!(
        st.items
            .iter()
            .all(|i| !i.done && i.finding.is_none() && i.fault_seeded.is_none())
    );
    assert!(st.hazards.pitot_cover && st.hazards.chocks && st.hazards.tie_downs);
    assert_eq!(st.scene.props.len(), 8);
}

#[test]
fn perform_walks_to_the_station_and_rocks_the_aileron() {
    let mut s = begun(BeginOptions::default());
    perform(&mut s, "aileron_right");
    let samples = settle(&mut s);
    let st = state(&s);
    assert!(item(&st, "aileron_right").done);
    assert_eq!(st.current_station.as_deref(), Some("right_wing"));
    assert_eq!(st.camera.as_ref().unwrap().station, "right_wing");
    let def = fixture();
    let node = def.node("right_aileron").unwrap();
    let p = st.figure.position_m;
    let d = ((p[0] as f64 - node.position_m[0]).powi(2) + (p[1] as f64 - node.position_m[1]).powi(2)).sqrt();
    assert!(d < 0.05, "figure stands {d} m from the aileron station");
    assert!(samples.iter().any(|x| x.action == FigureAction::Walking));
    let rocked: Vec<f32> = samples
        .iter()
        .flat_map(|x| x.scene.surfaces.iter())
        .filter(|o| o.node == "aileron_right")
        .map(|o| o.value)
        .collect();
    assert!(
        rocked.iter().any(|v| *v > 0.3) && rocked.iter().any(|v| *v < -0.3),
        "{rocked:?}"
    );
    assert!(samples.iter().any(|x| x.action == FigureAction::MovingControl));
    assert!(
        st.scene.surfaces.is_empty(),
        "the surface is released when the check ends"
    );
    // The figure never enters a keep-out while walking.
    for x in &samples {
        let q = x.figure.position_m;
        for k in &def.keepouts {
            let inside = (q[0] as f64) > k.min_m[0]
                && (q[0] as f64) < k.max_m[0]
                && (q[1] as f64) > k.min_m[1]
                && (q[1] as f64) < k.max_m[1];
            assert!(!inside || x.figure.seated, "figure entered {}", k.id);
        }
    }
}

#[test]
fn the_walk_takes_a_realistic_pace_and_speeds_up() {
    let mut slow = Session::new().unwrap();
    slow.begin_preflight_with(fixture(), 0.85, BeginOptions::default())
        .unwrap();
    slow.run(PreflightPerform {
        item: "nav_lights_right".into(),
    })
    .unwrap();
    let mut t = 0.;
    loop {
        let st = slow.run(PreflightAdvance { seconds: 0.5 }).unwrap();
        t += 0.5;
        if st.current_item.is_none() {
            break;
        }
        assert!(t < 120.);
    }
    // About 25 m of walking at 1.3 m/s plus a three-second look.
    assert!((15. ..60.).contains(&t), "took {t} s");
    let mut fast = begun(BeginOptions::default());
    perform(&mut fast, "nav_lights_right");
    let mut tf = 0.;
    loop {
        let st = fast.run(PreflightAdvance { seconds: 0.5 }).unwrap();
        tf += 0.5;
        if st.current_item.is_none() {
            break;
        }
    }
    assert!(tf < t / 4.);
}

#[test]
fn seeded_faults_are_revealed_only_by_their_item() {
    let mut s = begun(BeginOptions::default());
    s.run(PreflightFault {
        item: "oil_coolant".into(),
        kind: DefectKind::LowOil,
    })
    .unwrap();
    let hidden = s.preflight_state(false);
    assert!(
        hidden.items.iter().all(|i| i.fault_seeded.is_none()),
        "students never see seeds"
    );
    assert!(hidden.grade.seeded.is_none());
    assert_eq!(
        item(&state(&s), "oil_coolant").fault_seeded,
        Some(DefectKind::LowOil)
    );
    // Other items, even nearby ones, reveal nothing.
    do_item(&mut s, "nose_cowling");
    do_item(&mut s, "fuel_electrical");
    assert!(state(&s).items.iter().all(|i| i.finding.is_none()));
    do_item(&mut s, "oil_coolant");
    let st = state(&s);
    let found = item(&st, "oil_coolant").finding.clone().unwrap();
    assert_eq!(found.kind, DefectKind::LowOil);
    assert_eq!(found.severity, Severity::NoGo);
    assert!(found.description.contains("MIN"));
    assert_eq!(st.items.iter().filter(|i| i.finding.is_some()).count(), 1);
    assert!(st.oil_level.unwrap() < 0., "the dipstick reads below MIN");
    assert!(st.scene.cues.iter().all(|c| c.kind != DefectKind::LowOil) || !st.scene.cues.is_empty());
}

#[test]
fn a_finding_must_be_allowed_on_its_item() {
    let mut s = begun(BeginOptions::default());
    let err = s
        .run(PreflightFault {
            item: "tail_strobe".into(),
            kind: DefectKind::LowOil,
        })
        .unwrap_err();
    assert!(err.to_string().contains("cannot be seeded"));
    let err = s
        .run(PreflightFault {
            item: "oil_coolant".into(),
            kind: DefectKind::PitotCover,
        })
        .unwrap_err();
    assert!(err.to_string().contains("does not remove"));
}

#[test]
fn hand_checks_show_the_actual_fuel_level() {
    let mut s = begun(BeginOptions::default());
    for tank in ["left", "right"] {
        s.sim_mut()
            .set_systems(SystemSetting::TankQuantity {
                tank: tank.into(),
                mass_kg: 30.,
            })
            .unwrap();
    }
    let samples = do_item(&mut s, "fuel_level_right");
    let view = samples
        .iter()
        .find_map(|x| x.scene.fuel_view.clone())
        .expect("the filler cap is opened and the tank is shown");
    assert_eq!(view.tank, "right");
    assert!((view.litres - 30. / 0.72).abs() < 0.2, "{view:?}");
    assert!(view.covers_bottom);
    assert!(samples.iter().any(|x| x.scene.caps.iter().any(|c| c.open)));
    assert!(
        state(&s).scene.caps.iter().all(|c| !c.open),
        "the cap goes back on"
    );
    // A tank seeded low shows fuel that does not cover the bottom, and the reading is real.
    s.run(PreflightFault {
        item: "fuel_level_left".into(),
        kind: DefectKind::LowFuel,
    })
    .unwrap();
    let samples = do_item(&mut s, "fuel_level_left");
    let view = samples.iter().find_map(|x| x.scene.fuel_view.clone()).unwrap();
    assert!(!view.covers_bottom && view.litres < 30., "{view:?}");
    let st = state(&s);
    assert!(!st.fuel.iter().find(|f| f.tank == "left").unwrap().covers_bottom);
    assert!(item(&st, "fuel_level_left").finding.is_some());
    // Rectifying restores the fuel.
    s.run(PreflightResolve {
        item: "fuel_level_left".into(),
    })
    .unwrap();
    assert!(state(&s).fuel.iter().find(|f| f.tank == "left").unwrap().litres > 30.);
}

#[test]
fn covers_ropes_and_chocks_are_props_the_items_remove() {
    let mut s = begun(BeginOptions::default());
    assert_eq!(s.preflight_state(false).scene.props.len(), 8);
    do_item(&mut s, "pitot");
    let st = s.preflight_state(false);
    assert!(!st.hazards.pitot_cover && st.hazards.chocks);
    assert!(st.scene.props.iter().all(|p| p.id != "pitot_cover"));
    do_item(&mut s, "tail_rope");
    do_item(&mut s, "chocks_ropes");
    let st = s.preflight_state(false);
    assert!(st.scene.props.is_empty());
    assert_eq!(st.hazards, GroundHazards::default());
    // The figure carries a prop while it walks it off.
    let mut s = begun(BeginOptions::default());
    let samples = do_item(&mut s, "pitot");
    assert!(
        samples
            .iter()
            .any(|x| x.figure.carrying == Some(PropKind::PitotCover))
    );
}

#[test]
fn a_chock_left_on_stops_the_aircraft_with_a_clear_message() {
    let mut s = begun(BeginOptions::default());
    throttle_up(&mut s);
    s.sim_mut().step(120 * 15);
    assert!(s.sim().readout().ground_speed_mps < 0.2);
    let st = s.preflight_state(false);
    let message = st.blocked.unwrap();
    assert!(message.contains("will not move"), "{message}");
    assert!(!s.preflight_state(false).grade.passed);
    // Take them off (skipping nothing else) and the aircraft rolls.
    do_item(&mut s, "tail_rope");
    do_item(&mut s, "chocks_ropes");
    s.sim_mut().step(120 * 15);
    assert!(s.sim().readout().ground_speed_mps > 10.);
    assert!(s.preflight_state(false).blocked.is_none());
}

#[test]
fn a_pitot_cover_left_on_reads_zero_airspeed_in_flight() {
    let fly = |remove: bool| {
        let mut s = begun(BeginOptions::default());
        do_item(&mut s, "tail_rope");
        do_item(&mut s, "chocks_ropes");
        if remove {
            do_item(&mut s, "pitot");
        }
        throttle_up(&mut s);
        s.sim_mut().step(120 * 12);
        s.sim_mut().set_controls(Controls {
            throttle: 1.,
            elevator: 0.3,
            ..Default::default()
        });
        s.sim_mut().step(120 * 6);
        let r = s.sim().readout();
        assert!(!r.on_ground && r.true_airspeed_mps > 20., "{r:?}");
        s.sim().systems().sensed.indicated_airspeed_mps
    };
    assert!(fly(false) < 3., "a covered pitot reads about zero");
    assert!(fly(true) > 20., "a clear pitot reads the real airspeed");
}

#[test]
fn hinge_hardware_findings_use_the_existing_failure_models() {
    let mut s = begun(BeginOptions::default());
    s.run(PreflightFault {
        item: "flap_hinges_left".into(),
        kind: DefectKind::FlapHardwareMissing,
    })
    .unwrap();
    s.run(PreflightFault {
        item: "aileron_hinges_right".into(),
        kind: DefectKind::AileronHardwareMissing,
    })
    .unwrap();
    assert!(s.sim().failures().flap_asymmetry);
    assert_eq!(
        s.sim().failures().aileron,
        None,
        "the aileron only sticks in the air"
    );
    do_item(&mut s, "flap_hinges_left");
    s.run(PreflightResolve {
        item: "flap_hinges_left".into(),
    })
    .unwrap();
    assert!(!s.sim().failures().flap_asymmetry);
}

#[test]
fn grading_counts_found_missed_left_on_and_flown() {
    // Everything done, everything found: a pass.
    let mut s = begun(BeginOptions {
        findings: vec![
            SeedSpec {
                item: "wheel_brakes_left".into(),
                kind: DefectKind::DamagedTyre,
            },
            SeedSpec {
                item: "pitot".into(),
                kind: DefectKind::PitotCover,
            },
        ],
        ..Default::default()
    });
    all_required(&mut s);
    let st = s.preflight_state(true);
    assert_eq!(st.grade.required_done, st.grade.required_total);
    assert_eq!(st.grade.seeded_total, 2);
    assert_eq!(st.grade.seeded_found, 2);
    assert!(st.grade.props_left.is_empty());
    // A no-go tyre is found but not rectified: flying with it fails.
    assert_eq!(st.grade.no_go_open, 1);
    assert!(!st.grade.passed || !st.grade.flew_with_no_go);
    s.run(PreflightResolve {
        item: "wheel_brakes_left".into(),
    })
    .unwrap();
    let st = s.preflight_state(true);
    assert_eq!(st.grade.no_go_open, 0);
    assert!(st.grade.passed, "{:?}", st.grade);
    // A seeded finding that is never looked at is missed.
    let mut s = begun(BeginOptions {
        findings: vec![SeedSpec {
            item: "wheel_brakes_left".into(),
            kind: DefectKind::DamagedTyre,
        }],
        ..Default::default()
    });
    do_item(&mut s, "wheel_brakes_right");
    let st = s.preflight_state(true);
    assert_eq!(st.grade.seeded_found, 0);
    assert!(!st.grade.passed);
    assert!(st.grade.items_missed.contains(&"wheel_brakes_left".to_string()));
    let seeded = st.grade.seeded.unwrap();
    assert_eq!(
        (seeded[0].found, seeded[0].kind),
        (false, DefectKind::DamagedTyre)
    );
}

#[test]
fn flying_with_a_no_go_finding_is_graded_as_such() {
    let mut s = begun(BeginOptions {
        covers_on: Some(false),
        findings: vec![SeedSpec {
            item: "wheel_brakes_left".into(),
            kind: DefectKind::DamagedTyre,
        }],
        ..Default::default()
    });
    // Ignition stays on so the engine can run: only the physics of departing matters here.
    all_required_except(&mut s, &["ignition_off"]);
    assert!(s.preflight_state(true).grade.seeded_found == 1);
    throttle_up(&mut s);
    s.run(crate::Step { steps: 120 * 10 }).unwrap();
    let st = s.preflight_state(true);
    assert!(st.grade.departed);
    assert!(st.grade.flew_with_no_go);
    assert!(!st.grade.passed);
    // Rectifying afterwards does not undo the departure.
    s.run(PreflightResolve {
        item: "wheel_brakes_left".into(),
    })
    .unwrap();
    assert!(!s.preflight_state(true).grade.passed);
}

#[test]
fn the_propeller_is_only_handled_with_ignition_off() {
    let mut s = begun(BeginOptions::default());
    let err = s
        .run(PreflightPerform {
            item: "propeller_turn".into(),
        })
        .unwrap_err();
    assert!(err.to_string().contains("ignition must be off"), "{err}");
    do_item(&mut s, "ignition_off");
    s.sim_mut().step(240);
    let samples = do_item(&mut s, "propeller_turn");
    assert!(
        samples
            .iter()
            .any(|x| x.action == FigureAction::HandlingPropeller)
    );
    assert!(
        samples
            .iter()
            .any(|x| x.scene.propeller_phase.is_some_and(|p| p > 0.5))
    );
    assert!(item(&state(&s), "propeller_turn").done);
    assert!(state(&s).scene.propeller_phase.is_none());
}

#[test]
fn random_findings_are_deterministic_per_seed() {
    let def = fixture();
    let a = select_findings(&def, 20_261_104, 2);
    assert_eq!(a, select_findings(&def, 20_261_104, 2));
    assert_eq!(a.len(), 2);
    assert_ne!(a[0].item, a[1].item);
    let mut distinct = std::collections::BTreeSet::new();
    for seed in 0..40 {
        let pick = select_findings(&def, seed, 2);
        assert_eq!(pick.len(), 2);
        for f in &pick {
            let item = def.item(&f.item).unwrap();
            assert!(item.defects.iter().any(|d| d.kind == f.kind));
        }
        distinct.insert(pick);
    }
    assert!(distinct.len() > 10, "seeds should vary the findings");
    // Beginning twice with the same seed seeds the same findings in the session.
    let seeded = |seed: u64| {
        let mut s = Session::new().unwrap();
        let cmd = PreflightBegin {
            lesson: None,
            seed: Some(seed),
            random_findings: Some(2),
            findings: vec![],
            covers_on: None,
            planned_fuel_l: None,
            speed: None,
        };
        let def = fixture();
        s.begin_preflight_with(
            def.clone(),
            0.85,
            BeginOptions {
                seed,
                findings: select_findings(&def, seed, usize::from(cmd.random_findings.unwrap())),
                ..Default::default()
            },
        )
        .unwrap();
        s.preflight_state(true)
            .items
            .into_iter()
            .filter_map(|i| i.fault_seeded.map(|k| (i.id, k)))
            .collect::<Vec<_>>()
    };
    assert_eq!(seeded(7), seeded(7));
    assert_eq!(seeded(7).len(), 2);
}

#[test]
fn lessons_parse_and_the_findings_variant_is_random() {
    let lessons = preflight_lesson_library().unwrap();
    assert_eq!(
        lessons.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(),
        ["sling-preflight", "sling-preflight-findings"]
    );
    for l in &lessons {
        l.validate().unwrap();
        assert_eq!(
            (l.aircraft.as_str(), l.airport.as_str(), l.runway_end.as_str()),
            ("sling2", "1N7", "07")
        );
        assert!(l.covers_on);
    }
    assert_eq!(lessons[0].random_findings, 0);
    assert_eq!(lessons[1].random_findings, 2);
}

#[test]
fn cabin_items_put_the_figure_in_the_seat_and_work_the_switches() {
    let mut s = begun(BeginOptions::default());
    let samples = do_item(&mut s, "ignition_off");
    assert!(samples.iter().any(|x| x.action == FigureAction::Boarding));
    let st = state(&s);
    assert!(st.figure.seated);
    assert_eq!(st.figure.action, FigureAction::InCockpit);
    assert_eq!(s.sim().systems().controls.magnetos, Magnetos::Off);
    do_item(&mut s, "master_on");
    assert!(s.sim().systems().controls.master);
    do_item(&mut s, "flaps_full");
    assert_eq!(s.sim().controls().flaps, 1.);
    do_item(&mut s, "master_off");
    assert!(!s.sim().systems().controls.master);
    // Leaving the cockpit for the next station climbs out first.
    let samples = do_item(&mut s, "fuselage_surface");
    assert!(samples.iter().any(|x| x.action == FigureAction::Boarding));
    assert!(!state(&s).figure.seated);
}

#[test]
fn the_camera_frames_the_station_and_moves_smoothly() {
    let mut s = begun(BeginOptions::default());
    perform(&mut s, "wing_tip_left");
    let mut previous = s.preflight_state(false).camera.unwrap().eye_m;
    let mut largest_jump = 0.0_f64;
    let mut last = None;
    for _ in 0..4000 {
        let st = s.run(PreflightAdvance { seconds: 0.1 }).unwrap();
        let camera = st.camera.clone().unwrap();
        let jump = (0..3)
            .map(|i| (camera.eye_m[i] - previous[i]).powi(2))
            .sum::<f64>()
            .sqrt();
        largest_jump = largest_jump.max(jump);
        previous = camera.eye_m;
        last = Some(camera);
        if st.current_item.is_none() {
            break;
        }
    }
    // Real time is 0.1 s per sample at speed 16; the eye never teleports.
    assert!(largest_jump < 4., "{largest_jump}");
    let camera = last.unwrap();
    assert_eq!(camera.station, "left_wing");
    assert!(camera.target_m[1] < -2., "looking at the left wing: {camera:?}");
    assert!(camera.eye_m[2] < camera.target_m[2], "slightly elevated");
}

#[test]
fn begin_requires_pack_data_and_state_is_empty_without_one() {
    let s = Session::new().unwrap();
    let st = s.preflight_state(false);
    assert!(!st.active && st.items.is_empty());
    let mut s = Session::new().unwrap();
    let err = s.run(PreflightPerform { item: "x".into() }).unwrap_err();
    assert!(err.to_string().contains("preflight.begin"));
    let err = s
        .run(PreflightBegin {
            lesson: None,
            seed: None,
            random_findings: None,
            findings: vec![],
            covers_on: None,
            planned_fuel_l: None,
            speed: None,
        })
        .unwrap_err();
    assert!(err.to_string().contains("no preflight section"), "{err}");
}

#[test]
fn every_action_kind_has_an_animation() {
    // Perform each distinct action kind once and confirm each completes and names its action.
    let def = fixture();
    let mut seen = std::collections::BTreeSet::new();
    let mut s = begun(BeginOptions::default());
    do_item(&mut s, "ignition_off");
    s.sim_mut().step(240);
    for it in &def.items {
        let key = match &it.action {
            Action::Inspect { stance, touch, .. } => format!("inspect-{stance:?}-{touch}"),
            Action::MoveControl { .. } => "control".to_string(),
            Action::FuelCap { .. } => "fuel".to_string(),
            Action::OilCheck { .. } => "oil".to_string(),
            Action::PalmProp { .. } => "prop".to_string(),
            Action::Remove { .. } => "remove".to_string(),
            Action::Cabin { .. } => "cabin".to_string(),
        };
        if seen.insert(key) {
            let samples = do_item(&mut s, &it.id);
            assert!(
                samples.iter().any(|x| x.action != FigureAction::Idle),
                "{}",
                it.id
            );
            assert!(item(&state(&s), &it.id).done, "{}", it.id);
        }
    }
    assert!(seen.len() >= 9);
}

#[test]
fn the_whole_list_at_real_pace_takes_minutes_not_seconds() {
    let mut s = Session::new().unwrap();
    s.begin_preflight_with(fixture(), 0.85, BeginOptions::default())
        .unwrap();
    let ids: Vec<_> = fixture().items.iter().map(|i| i.id.clone()).collect();
    for id in &ids {
        // The optional propeller item queues behind the ignition item that makes it safe.
        perform(&mut s, id);
    }
    let mut seconds = 0.;
    loop {
        let st = s.run(PreflightAdvance { seconds: 1.0 }).unwrap();
        s.sim_mut().step(120);
        seconds += 1.;
        if st.queue.is_empty() && st.current_item.is_none() {
            break;
        }
        assert!(seconds < 5000.);
    }
    eprintln!("full walkaround at 1x: {seconds} s");
    assert!((300. ..2400.).contains(&seconds), "{seconds}");
    let st = s.preflight_state(false);
    assert_eq!(st.grade.required_done, 64);
    assert!(
        item(&st, "propeller_turn").done,
        "ignition was off by the time it came up"
    );
}
