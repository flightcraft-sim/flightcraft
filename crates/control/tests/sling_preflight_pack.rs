//! The real Sling pack (when `FLIGHTCRAFT_SLING_PACK` points at it): installs at pack format 4,
//! matches the checked-in fixture, and walks through the session. Skipped without the pack.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::expect_used)]
use flightcraft_aero::preflight::Preflight;
use flightcraft_control::{
    PreflightAdvance, PreflightBegin, PreflightPerform, PreflightState, Session, preflight_lesson_library,
    read_pack,
};

fn pack_dir() -> Option<std::path::PathBuf> {
    std::env::var("FLIGHTCRAFT_SLING_PACK").ok().map(Into::into)
}

#[test]
fn the_sling_pack_installs_and_matches_the_fixture() {
    let Some(dir) = pack_dir() else {
        eprintln!("skipped: FLIGHTCRAFT_SLING_PACK is not set");
        return;
    };
    let mut session = Session::new().unwrap();
    let contents = read_pack(&dir).unwrap();
    assert_eq!(contents.manifest.format_version, 4);
    // An older manifest version refuses the section.
    let mut old = contents.clone();
    old.manifest.format_version = 3;
    let err = Session::new().unwrap().install_pack(&old).unwrap_err();
    assert!(err.to_string().contains("format version 4"), "{err}");
    session.install_pack(&contents).unwrap();
    session
        .execute(
            "aircraft.load",
            serde_json::json!({"source":{"kind":"installed","id":"sling2"},"reset":true}),
        )
        .unwrap();
    let state = session
        .run(PreflightBegin {
            lesson: None,
            seed: None,
            random_findings: None,
            findings: vec![],
            covers_on: None,
            planned_fuel_l: None,
            speed: Some(16.),
        })
        .unwrap();
    assert_eq!(state.items.len(), 67);
    assert!(state.items.iter().all(|i| i.poh_ref.starts_with("4.4 (")));
    // The checked-in fixture is a copy of the pack's section.
    let fixture: Preflight = serde_json::from_str(include_str!("fixtures/sling-preflight.json")).unwrap();
    let systems: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("systems.json")).unwrap()).unwrap();
    let pack: Preflight = serde_json::from_value(systems["preflight"].clone()).unwrap();
    assert_eq!(
        pack, fixture,
        "refresh crates/control/tests/fixtures/sling-preflight.json"
    );
    // Real fuel tanks, real exterior nodes.
    assert_eq!(state.fuel.len(), 2);
    assert!(state.fuel.iter().all(|f| f.covers_bottom && f.litres > 30.));
    let asset = session.aircraft_catalog().exterior("sling2").unwrap();
    for part in pack.driven_parts() {
        assert!(
            asset.definition.moving_parts.iter().any(|m| m.node == part),
            "{part}"
        );
    }
    session
        .run(PreflightPerform {
            item: "aileron_left".into(),
        })
        .unwrap();
    let mut rocked = false;
    for _ in 0..4000 {
        let st: PreflightState = session.run(PreflightAdvance { seconds: 0.1 }).unwrap();
        rocked |= st
            .scene
            .surfaces
            .iter()
            .any(|s| s.node == "aileron_left" && s.value.abs() > 0.3);
        if st.current_item.is_none() {
            break;
        }
    }
    assert!(rocked);
    // The lessons start on the installed pack when the region is present; without it they say why.
    let lessons = preflight_lesson_library().unwrap();
    let err = session
        .run(PreflightBegin {
            lesson: Some(lessons[0].id.clone()),
            seed: None,
            random_findings: None,
            findings: vec![],
            covers_on: None,
            planned_fuel_l: None,
            speed: None,
        })
        .map(|_| ())
        .unwrap_err();
    assert!(err.to_string().contains("region"), "{err}");
}

/// With the Sling pack and the eastern-pa region (`FLIGHTCRAFT_TEST_TERRAIN` or the repository's
/// assets), the lessons park the aircraft at 1N7 and the random variant is deterministic.
#[test]
fn lessons_park_the_sling_at_1n7_and_random_findings_follow_the_seed() {
    let (Some(dir), terrain) = (
        pack_dir(),
        std::env::var_os("FLIGHTCRAFT_TEST_TERRAIN")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/terrain")),
    ) else {
        eprintln!("skipped: FLIGHTCRAFT_SLING_PACK is not set");
        return;
    };
    let region = terrain.join("eastern-pa");
    let Ok(contents) = flightcraft_control::read_region(&region) else {
        eprintln!("skipped: no eastern-pa region data at {}", region.display());
        return;
    };
    let start = |lesson: &str, seed: Option<u64>| {
        let mut session = Session::new().unwrap();
        session.install_pack(&read_pack(&dir).unwrap()).unwrap();
        session.install_region(contents.clone()).unwrap();
        let state = session
            .run(PreflightBegin {
                lesson: Some(lesson.into()),
                seed,
                random_findings: None,
                findings: vec![],
                covers_on: None,
                planned_fuel_l: None,
                speed: None,
            })
            .unwrap();
        (session, state)
    };
    let (session, state) = start("sling-preflight", None);
    assert_eq!(session.sim().aircraft().identity.id, "sling2");
    assert!(session.sim().readout().on_ground);
    assert_eq!(state.lesson.as_deref(), Some("sling-preflight"));
    assert_eq!(state.planned_fuel_l, Some(40.));
    assert!(state.hazards.pitot_cover && state.hazards.chocks && state.hazards.tie_downs);
    assert!(
        session
            .preflight_state(true)
            .items
            .iter()
            .all(|i| i.fault_seeded.is_none())
    );
    let seeded = |seed: Option<u64>| {
        let (session, _) = start("sling-preflight-findings", seed);
        session
            .preflight_state(true)
            .items
            .into_iter()
            .filter_map(|i| i.fault_seeded.map(|k| (i.id, k)))
            .collect::<Vec<_>>()
    };
    let a = seeded(None);
    assert_eq!(a.len(), 2);
    assert_eq!(
        a,
        seeded(None),
        "the lesson's own seed always seeds the same findings"
    );
    assert_eq!(seeded(Some(5)), seeded(Some(5)));
    assert!(
        (6..30).any(|seed| seeded(Some(seed)) != a),
        "another seed seeds other findings"
    );
}
