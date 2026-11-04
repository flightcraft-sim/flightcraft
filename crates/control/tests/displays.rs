//! Two-screen glass installs: the left screen is the PFD and the right the MFD, per display pages
//! persist for the variant, the seat swaps the defaults, and a lost screen reverts the other.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_aero::modern::{DataArea, PilotSeat, TouchLayout};
use flightcraft_control::*;
use serde_json::{Value, json};

/// The equipment demonstrator, optionally with its panel edited (for example to declare an MFD).
fn session(patch: &dyn Fn(&mut Value)) -> Session {
    let mut s = Session::new().unwrap();
    let manifest =
        PackManifest::from_json(include_str!("../../../examples/packs/equipment-demo/pack.json")).unwrap();
    let mut panel: Value =
        serde_json::from_str(include_str!("../../../examples/packs/equipment-demo/panel.json")).unwrap();
    patch(&mut panel);
    let files = [
        (
            "engine.json",
            include_str!("../../../examples/packs/equipment-demo/engine.json").to_owned(),
        ),
        (
            "systems.json",
            include_str!("../../../examples/packs/equipment-demo/systems.json").to_owned(),
        ),
        (
            "variant.json",
            include_str!("../../../examples/packs/equipment-demo/variant.json").to_owned(),
        ),
        ("panel.json", serde_json::to_string(&panel).unwrap()),
        (
            "aircraft.json",
            include_str!("../../../examples/packs/equipment-demo/aircraft.json").to_owned(),
        ),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect();
    s.install_pack(&PackContents {
        manifest,
        files,
        assets: Default::default(),
    })
    .unwrap();
    s.execute(
        "aircraft.load",
        json!({"source":{"kind":"installed","id":"equipment-demonstrator"},"reset":true}),
    )
    .unwrap();
    s
}
/// What the Sling 2 pack declares: PFD on the left, MFD on the right.
fn two_screen(panel: &mut Value) {
    for e in panel["equipment"]["elements"].as_array_mut().unwrap() {
        match e["id"].as_str() {
            Some("left_display") => e["element"]["layout"] = json!("integrated_pfd"),
            Some("right_display") => {
                e["element"]["layout"] = json!("integrated_pfd");
                e["element"]["default_page"] = json!("mfd");
            }
            _ => {}
        }
    }
}
fn glass(s: &mut Session, setting: Value) -> Result<Value, CommandError> {
    s.execute(
        "avionics.set",
        json!({"setting":{"kind":"glass","setting":setting}}),
    )
}
fn pages(s: &Session) -> Vec<TouchLayout> {
    s.touch_state().displays.iter().map(|d| d.layout).collect()
}
fn equipment(s: &mut Session, setting: Value) {
    s.execute("equipment.set", json!({"setting":setting})).unwrap();
}

#[test]
fn a_two_screen_install_defaults_to_pfd_left_and_mfd_right() {
    let s = session(&two_screen);
    let touch = s.touch_state();
    assert_eq!(touch.displays[0].id, "left_display");
    assert_eq!(
        pages(&s),
        [TouchLayout::IntegratedPfd, TouchLayout::Mfd],
        "left is the primary flight display, right the multi-function display"
    );
    assert_eq!(touch.displays[1].data_area, DataArea::Engine);
    assert_eq!(touch.pilot_seat, PilotSeat::Left);
    assert!(!touch.manual_reversion && !touch.displays.iter().any(|d| d.reversion));
    assert!(
        touch
            .displays
            .iter()
            .all(|d| d.installed_layout == TouchLayout::IntegratedPfd)
    );
}

#[test]
fn packs_without_a_default_page_are_unchanged() {
    let s = session(&|_| {});
    // The demonstrator declares full_pfd and pfd_map and no default page: both show their layout.
    assert_eq!(pages(&s), [TouchLayout::FullPfd, TouchLayout::PfdMap]);
    for d in &s.touch_state().displays {
        assert_eq!(d.layout, d.installed_layout);
        assert_eq!(d.layout, d.default_page);
    }
    assert!(s.display_preferences().is_empty());
    // A layout change through the old equipment path still works.
    let mut s = s;
    equipment(
        &mut s,
        json!({"kind":"display","id":"left_display","action":{"kind":"layout","layout":"map_engine"}}),
    );
    assert_eq!(pages(&s), [TouchLayout::MapEngine, TouchLayout::PfdMap]);
}

#[test]
fn per_display_page_changes_persist_and_round_trip() {
    let mut s = session(&two_screen);
    glass(
        &mut s,
        json!({"kind":"display_page","display":"right_display","page":"nearest"}),
    )
    .unwrap();
    glass(
        &mut s,
        json!({"kind":"display_page","display":"left_display","page":"svt"}),
    )
    .unwrap();
    assert_eq!(pages(&s), [TouchLayout::Svt, TouchLayout::Nearest]);
    assert!(
        s.touch_state().displays[0].synthetic_vision,
        "the SVT page shows synthetic vision"
    );
    // Back on the default, the choice is not stored.
    glass(
        &mut s,
        json!({"kind":"display_page","display":"left_display","page":"integrated_pfd"}),
    )
    .unwrap();
    glass(
        &mut s,
        json!({"kind":"display_page","display":"right_display","page":"mfd"}),
    )
    .unwrap();
    assert!(s.display_preferences().is_empty());
    glass(
        &mut s,
        json!({"kind":"display_page","display":"right_display","page":"traffic"}),
    )
    .unwrap();
    glass(
        &mut s,
        json!({"kind":"display_data","display":"left_display","area":"flight_plan"}),
    )
    .unwrap();
    assert!(
        glass(
            &mut s,
            json!({"kind":"display_page","display":"nope","page":"mfd"})
        )
        .is_err()
    );
    assert!(
        glass(
            &mut s,
            json!({"kind":"display_page","display":"right_display","page":"bogus"})
        )
        .is_err()
    );
    // The same through the equipment path the soft keys use.
    equipment(
        &mut s,
        json!({"kind":"display","id":"right_display","action":{"kind":"data_area","area":"nearest"}}),
    );
    let saved = serde_json::to_string(s.display_preferences()).unwrap();
    // A flight reset keeps the pilot's pages.
    s.execute("sim.reset", json!({"start":{"kind":"runway"}}))
        .unwrap();
    assert_eq!(pages(&s), [TouchLayout::IntegratedPfd, TouchLayout::Traffic]);
    // A new launch restores them from the preference file, for this variant only.
    let mut reopened = session(&two_screen);
    assert_eq!(pages(&reopened), [TouchLayout::IntegratedPfd, TouchLayout::Mfd]);
    reopened
        .restore_display_preferences(serde_json::from_str(&saved).unwrap())
        .unwrap();
    assert_eq!(
        pages(&reopened),
        [TouchLayout::IntegratedPfd, TouchLayout::Traffic]
    );
    assert_eq!(reopened.touch_state().displays[1].data_area, DataArea::Nearest);
    assert_eq!(
        serde_json::to_string(reopened.display_preferences()).unwrap(),
        saved
    );
    // A damaged file is refused whole and changes nothing.
    assert!(
        reopened
            .restore_display_preferences(serde_json::from_value(json!({"Not A Variant":{}})).unwrap())
            .is_err()
    );
    assert_eq!(
        pages(&reopened),
        [TouchLayout::IntegratedPfd, TouchLayout::Traffic]
    );
}

#[test]
fn right_seat_swaps_the_defaults_and_saves() {
    let mut s = session(&two_screen);
    glass(
        &mut s,
        json!({"kind":"display_page","display":"left_display","page":"engine"}),
    )
    .unwrap();
    glass(&mut s, json!({"kind":"pilot_seat","seat":"right"})).unwrap();
    assert_eq!(s.touch_state().pilot_seat, PilotSeat::Right);
    assert_eq!(
        pages(&s),
        [TouchLayout::Mfd, TouchLayout::IntegratedPfd],
        "PFD on the right, and the pilot's earlier page choice is dropped"
    );
    let saved = serde_json::to_string(s.display_preferences()).unwrap();
    let mut reopened = session(&two_screen);
    reopened
        .restore_display_preferences(serde_json::from_str(&saved).unwrap())
        .unwrap();
    assert_eq!(pages(&reopened), [TouchLayout::Mfd, TouchLayout::IntegratedPfd]);
    glass(&mut reopened, json!({"kind":"pilot_seat","seat":"left"})).unwrap();
    assert_eq!(pages(&reopened), [TouchLayout::IntegratedPfd, TouchLayout::Mfd]);
    assert!(reopened.display_preferences().is_empty());
}

#[test]
fn losing_the_pfd_screen_reverts_the_mfd_and_it_returns_on_restore() {
    let mut s = session(&two_screen);
    equipment(
        &mut s,
        json!({"kind":"display","id":"left_display","action":{"kind":"failure","failed":true}}),
    );
    let touch = s.touch_state();
    assert!(touch.displays[0].failed && !touch.displays[0].reversion);
    assert!(
        touch.displays[1].reversion,
        "the MFD shows the PFD with an engine strip"
    );
    assert_eq!(
        touch.displays[1].layout,
        TouchLayout::Mfd,
        "its chosen page is kept for the return"
    );
    equipment(
        &mut s,
        json!({"kind":"display","id":"left_display","action":{"kind":"failure","failed":false}}),
    );
    assert!(!s.touch_state().displays.iter().any(|d| d.reversion));
    // Losing the left screen's power does the same.
    s.execute(
        "systems.set",
        json!({"setting":{"kind":"breaker","load":"touch_left","closed":false}}),
    )
    .unwrap();
    s.execute("sim.step", json!({"steps":1})).unwrap();
    assert!(!s.touch_state().displays[0].powered);
    assert!(s.touch_state().displays[1].reversion);
    s.execute(
        "systems.set",
        json!({"setting":{"kind":"breaker","load":"touch_left","closed":true}}),
    )
    .unwrap();
    s.execute("sim.step", json!({"steps":1})).unwrap();
    assert!(!s.touch_state().displays.iter().any(|d| d.reversion));
}

#[test]
fn display_backup_reverts_only_the_mfd_and_resets_with_the_flight() {
    let mut s = session(&two_screen);
    glass(&mut s, json!({"kind":"display_reversion","enabled":true})).unwrap();
    let touch = s.touch_state();
    assert!(touch.manual_reversion);
    assert!(!touch.displays[0].reversion, "the PFD screen keeps the PFD");
    assert!(touch.displays[1].reversion);
    glass(&mut s, json!({"kind":"display_reversion","enabled":false})).unwrap();
    assert!(!s.touch_state().displays.iter().any(|d| d.reversion));
    glass(&mut s, json!({"kind":"display_reversion","enabled":true})).unwrap();
    s.execute("sim.reset", json!({"start":{"kind":"runway"}}))
        .unwrap();
    assert!(!s.touch_state().manual_reversion);
}

#[test]
fn corvid_glass_has_the_pfd_left_and_the_mfd_map_right() {
    let mut s = Session::new().unwrap();
    s.execute(
        "aircraft.load",
        json!({"source":{"kind":"installed","id":"corvid-glass"},"reset":true}),
    )
    .unwrap();
    let panel = s.aircraft_configuration().unwrap().panel.clone();
    let x = |module: &str| {
        panel
            .instruments
            .iter()
            .find(|i| i.module == module)
            .unwrap_or_else(|| panic!("{module} missing"))
            .position[0]
    };
    assert!(x("glass_pfd") < x("glass_mfd"), "PFD left of the MFD");
    let state = s.glass_state();
    assert_eq!(state.settings.page, GlassPage::Map, "the MFD opens on the map");
    assert!(!state.pfd_failed && !state.mfd_failed);
    // Losing the MFD moves its engine strip to the PFD; losing the PFD is a display failure of its own.
    glass(&mut s, json!({"kind":"failure","target":"mfd","failed":true})).unwrap();
    assert!(s.glass_state().reversion);
}
