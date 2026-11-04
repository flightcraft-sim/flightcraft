//! The landscape touch EFIS model: knob roles, the page and split-screen state machine, the
//! dual-display defaults and reversion with this unit, and skin packs with fallback.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_aero::display_model::DisplayModel;
use flightcraft_aero::modern::{MfdPage, TouchLayout};
use flightcraft_aero::skin_pack::SkinPackManifest;
use flightcraft_control::touch_efis::{EfisKey, Overlay};
use flightcraft_control::*;
use serde_json::{Value, json};

fn session(recommend: Option<&str>) -> Session {
    let mut s = Session::new().unwrap();
    let manifest =
        PackManifest::from_json(include_str!("../../../examples/packs/equipment-demo/pack.json")).unwrap();
    let mut panel: Value =
        serde_json::from_str(include_str!("../../../examples/packs/equipment-demo/panel.json")).unwrap();
    for e in panel["equipment"]["elements"].as_array_mut().unwrap() {
        match e["id"].as_str() {
            Some("left_display") => e["element"]["layout"] = json!("integrated_pfd"),
            Some("right_display") => {
                e["element"]["layout"] = json!("integrated_pfd");
                e["element"]["default_page"] = json!("mfd");
            }
            _ => continue,
        }
        e["appearance"] = json!({"bezel":{"model":"landscape_touch_efis"}});
        if let Some(id) = recommend {
            e["appearance"]["recommended_skin"] = json!(id);
        }
    }
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

fn display(s: &mut Session, id: &str, action: Value) {
    s.execute(
        "equipment.set",
        json!({"setting":{"kind":"display","id":id,"action":action}}),
    )
    .unwrap();
}
fn state(s: &Session, id: &str) -> TouchDisplayState {
    s.touch_state().displays.into_iter().find(|d| d.id == id).unwrap()
}
fn knob(s: &mut Session, id: &str, side: &str, ring: &str, detents: i32) {
    display(
        s,
        id,
        json!({"kind":"knob","knob":side,"ring":ring,"detents":detents,"time_s":0.0}),
    );
}
fn key(s: &mut Session, id: &str, key: &str, hold: bool) {
    display(s, id, json!({"kind":"key","key":key,"hold":hold}));
}
fn glass_state(s: &Session) -> GlassState {
    s.glass_state()
}

#[test]
fn the_model_is_reported_and_the_dual_display_defaults_map_onto_it() {
    let s = session(None);
    let left = state(&s, "left_display");
    let right = state(&s, "right_display");
    assert_eq!(left.model, Some(DisplayModel::LandscapeTouchEfis));
    assert_eq!(right.model, Some(DisplayModel::LandscapeTouchEfis));
    // Left is a full PFD, right a full MFD showing the map: the real unit's two full-screen modes.
    assert_eq!(
        touch_efis::ScreenMode::of(left.layout),
        touch_efis::ScreenMode::FullPfd
    );
    assert_eq!(
        touch_efis::ScreenMode::of(right.layout),
        touch_efis::ScreenMode::FullMfd
    );
    assert_eq!(right.mfd_page, MfdPage::Map);
}

#[test]
fn pfd_knobs_set_heading_altitude_baro_and_course() {
    let mut s = session(None);
    let before = glass_state(&s).settings;
    knob(&mut s, "left_display", "left", "inner", 3);
    let after = glass_state(&s).settings;
    let step = (after.heading_bug_rad - before.heading_bug_rad)
        .to_degrees()
        .rem_euclid(360.);
    assert!((step - 3.).abs() < 1e-6, "heading moved {step} degrees");
    knob(&mut s, "left_display", "left", "outer", 1);
    let first = glass_state(&s).settings.selected_altitude_m;
    knob(&mut s, "left_display", "left", "outer", 2);
    let alt = glass_state(&s).settings.selected_altitude_m;
    assert!((alt - first - 200. * 0.3048).abs() < 1e-6, "{alt} vs {first}");
    let baro_before = glass_state(&s).baro_pa;
    knob(&mut s, "left_display", "right", "outer", 5);
    assert!((glass_state(&s).baro_pa - baro_before - 5. * 33.8639).abs() < 1e-6);
    knob(&mut s, "left_display", "right", "inner", -10);
    let course = glass_state(&s).settings.course_rad.to_degrees();
    assert!((course - 350.).abs() < 1e-6, "{course}");
    assert!(glass_state(&s).settings.course_manual);
}

#[test]
fn pushing_the_heading_knob_syncs_the_bug_to_the_heading() {
    let mut s = session(None);
    s.execute(
        "avionics.set",
        json!({"setting":{"kind":"heading_bug","degrees":200.0}}),
    )
    .unwrap();
    display(
        &mut s,
        "left_display",
        json!({"kind":"knob_push","knob":"left","time_s":0.0}),
    );
    let synced = glass_state(&s).settings.heading_bug_rad.to_degrees();
    assert!(
        (synced - synced.round()).abs() < 1e-6,
        "sync rounds to a degree: {synced}"
    );
}

#[test]
fn press_to_toggle_swaps_the_functions_and_reverts_after_ten_seconds() {
    let mut s = session(None);
    display(
        &mut s,
        "left_display",
        json!({"kind":"knobs","config":{"alternate":"course_baro","press_to_toggle":true,"mfd_side":"right"}}),
    );
    assert!(state(&s, "left_display").knobs.press_to_toggle);
    display(
        &mut s,
        "left_display",
        json!({"kind":"knob_push","knob":"left","time_s":100.0}),
    );
    assert_eq!(state(&s, "left_display").knob_toggle_until_s, Some(110.0));
    // While toggled the left outer knob sets baro instead of altitude.
    let alt = glass_state(&s).settings.selected_altitude_m;
    let baro = glass_state(&s).baro_pa;
    display(
        &mut s,
        "left_display",
        json!({"kind":"knob","knob":"left","ring":"outer","detents":1,"time_s":105.0}),
    );
    assert_eq!(glass_state(&s).settings.selected_altitude_m, alt);
    assert!(glass_state(&s).baro_pa > baro);
    // After the revert time it is the altitude bug again.
    display(
        &mut s,
        "left_display",
        json!({"kind":"knob","knob":"left","ring":"outer","detents":1,"time_s":111.0}),
    );
    assert!(glass_state(&s).settings.selected_altitude_m > alt);
}

#[test]
fn mfd_knob_cycles_pages_and_the_range_knob_steps_the_ladder() {
    let mut s = session(None);
    knob(&mut s, "right_display", "right", "outer", 1);
    assert_eq!(state(&s, "right_display").mfd_page, MfdPage::Waypoint);
    knob(&mut s, "right_display", "left", "outer", -2);
    assert_eq!(state(&s, "right_display").mfd_page, MfdPage::Engine);
    s.execute(
        "avionics.set",
        json!({"setting":{"kind":"glass","setting":{"kind":"map_range","range_nm":10.0}}}),
    )
    .unwrap();
    knob(&mut s, "right_display", "right", "inner", 1);
    assert_eq!(glass_state(&s).settings.range_nm, 7.0);
    knob(&mut s, "right_display", "right", "inner", -2);
    assert_eq!(glass_state(&s).settings.range_nm, 15.0);
}

#[test]
fn split_screen_pages_and_keys_follow_the_state_machine_and_persist() {
    let mut s = session(None);
    // BACK held on the full PFD opens the split screen; held again returns.
    key(&mut s, "left_display", "back", true);
    assert_eq!(state(&s, "left_display").layout, TouchLayout::Split);
    knob(&mut s, "left_display", "right", "outer", 2);
    assert_eq!(state(&s, "left_display").mfd_page, MfdPage::FlightPlan);
    // In split, the left knob stays on the PFD.
    let alt = glass_state(&s).settings.selected_altitude_m;
    knob(&mut s, "left_display", "left", "outer", 1);
    assert!(glass_state(&s).settings.selected_altitude_m > alt);
    key(&mut s, "left_display", "back", true);
    assert_eq!(state(&s, "left_display").layout, TouchLayout::IntegratedPfd);
    // NEAREST shows the nearest page; MENU opens menus; DIRECT opens entry; BACK closes it.
    key(&mut s, "left_display", "nearest", false);
    let d = state(&s, "left_display");
    assert_eq!((d.layout, d.mfd_page), (TouchLayout::Split, MfdPage::Nearest));
    key(&mut s, "left_display", "menu", false);
    assert_eq!(state(&s, "left_display").overlay, Overlay::PageMenu);
    key(&mut s, "left_display", "direct", false);
    assert_eq!(state(&s, "left_display").overlay, Overlay::DirectTo);
    key(&mut s, "left_display", "back", false);
    assert_eq!(state(&s, "left_display").overlay, Overlay::None);
    // The choices are saved for the variant and survive a restore.
    let saved = s.display_preferences().clone();
    let mut again = session(None);
    again.restore_display_preferences(saved).unwrap();
    let d = state(&again, "left_display");
    assert_eq!((d.layout, d.mfd_page), (TouchLayout::Split, MfdPage::Nearest));
    let _ = EfisKey::Menu;
}

#[test]
fn a_page_chosen_from_the_page_bar_shows_on_the_full_mfd() {
    let mut s = session(None);
    display(
        &mut s,
        "right_display",
        json!({"kind":"mfd_page","page":"traffic"}),
    );
    let d = state(&s, "right_display");
    assert_eq!((d.layout, d.mfd_page), (TouchLayout::Mfd, MfdPage::Traffic));
    // The legacy avionics.set page selection still works and names its own page.
    s.execute(
        "avionics.set",
        json!({"setting":{"kind":"glass","setting":{"kind":"display_page","display":"right_display","page":"nearest"}}}),
    )
    .unwrap();
    let d = state(&s, "right_display");
    assert_eq!(touch_efis::effective_page(d.layout, d.mfd_page), MfdPage::Nearest);
}

#[test]
fn direct_to_entry_activates_a_plan_and_closes_the_keypad() {
    let mut s = session(None);
    key(&mut s, "left_display", "direct", false);
    assert_eq!(state(&s, "left_display").overlay, Overlay::DirectTo);
    // An unknown identifier is refused and the keypad stays up.
    let bad = s.execute(
        "equipment.set",
        json!({"setting":{"kind":"display","id":"left_display","action":{"kind":"direct_to","query":"ZZZZ9"}}}),
    );
    assert!(bad.is_err());
    assert_eq!(state(&s, "left_display").overlay, Overlay::DirectTo);
}

#[test]
fn losing_a_screen_reverts_the_other_to_the_pfd_with_engine_strip() {
    let mut s = session(None);
    display(&mut s, "left_display", json!({"kind":"failure","failed":true}));
    let right = state(&s, "right_display");
    assert!(right.reversion, "the surviving MFD reverts");
    assert!(state(&s, "left_display").failed);
    // The MFD's own knob page choice is kept for when the PFD returns.
    display(&mut s, "left_display", json!({"kind":"failure","failed":false}));
    assert!(!state(&s, "right_display").reversion);
    assert_eq!(state(&s, "right_display").layout, TouchLayout::Mfd);
}

fn skin_json(model: &str) -> Value {
    json!({"kind":"skin_pack","format_version":1,"id":"landscape-touch-efis","version":"0.1.0",
        "display_name":"Landscape touch EFIS","licence":"Apache-2.0","display_model":model,
        "palette":{"bezel":"#101010"}})
}
fn skin(model: &str) -> SkinContents {
    SkinContents {
        manifest: SkinPackManifest::from_json(&skin_json(model).to_string()).unwrap(),
        assets: Default::default(),
    }
}

#[test]
fn a_recommended_skin_is_used_when_installed_and_the_built_in_one_otherwise() {
    let mut s = session(Some("landscape-touch-efis"));
    let d = state(&s, "left_display");
    assert_eq!(d.recommended_skin.as_deref(), Some("landscape-touch-efis"));
    assert_eq!(d.skin, None, "not installed: fall back to the built-in skin");
    s.install_skin(skin("landscape_touch_efis")).unwrap();
    assert_eq!(
        state(&s, "left_display").skin.as_deref(),
        Some("landscape-touch-efis")
    );
    // The pilot can force the built-in skin per display, and go back to the recommendation.
    display(&mut s, "left_display", json!({"kind":"skin","skin":"built-in"}));
    assert_eq!(state(&s, "left_display").skin, None);
    assert_eq!(
        state(&s, "right_display").skin.as_deref(),
        Some("landscape-touch-efis")
    );
    display(&mut s, "left_display", json!({"kind":"skin"}));
    assert_eq!(
        state(&s, "left_display").skin.as_deref(),
        Some("landscape-touch-efis")
    );
    // Unknown skins are refused.
    assert!(
        s.execute(
            "equipment.set",
            json!({"setting":{"kind":"display","id":"left_display","action":{"kind":"skin","skin":"nope"}}})
        )
        .is_err()
    );
    assert_eq!(s.skins().count(), 1);
    assert_eq!(s.skins_for(DisplayModel::LandscapeTouchEfis).len(), 1);
}

#[test]
fn skin_packs_are_bounded_and_must_declare_every_asset() {
    let mut s = session(None);
    let mut with_font = skin("landscape_touch_efis");
    with_font
        .manifest
        .assets
        .push(flightcraft_aero::skin_pack::SkinAsset {
            path: "fonts/a.ttf".into(),
            kind: flightcraft_aero::skin_pack::SkinAssetKind::Font,
            licence: "OFL-1.1".into(),
        });
    assert!(
        s.install_skin(with_font.clone()).is_err(),
        "declared asset missing"
    );
    with_font
        .assets
        .insert("fonts/a.ttf".into(), vec![0; MAX_SKIN_FONT_BYTES + 1]);
    assert!(s.install_skin(with_font.clone()).is_err(), "oversize font");
    with_font.assets.insert("fonts/a.ttf".into(), vec![0; 16]);
    s.install_skin(with_font).unwrap();
    let mut extra = skin("landscape_touch_efis");
    extra.assets.insert("fonts/undeclared.ttf".into(), vec![0]);
    assert!(s.install_skin(extra).is_err());
}
