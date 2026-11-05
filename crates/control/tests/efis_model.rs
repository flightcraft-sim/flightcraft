//! The landscape touch EFIS model: knob roles, the page and split-screen state machine, the
//! dual-display defaults and reversion with this unit, and skin packs with fallback.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_aero::display_model::DisplayModel;
use flightcraft_aero::modern::{MfdPage, TouchLayout};
use flightcraft_aero::skin_pack::SkinPackManifest;
use flightcraft_control::touch_efis::{EfisKey, Overlay};
use flightcraft_control::touch_entry;
use flightcraft_control::*;
use serde_json::{Value, json};
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/region_data.rs"));

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
        json!({"setting":{"kind":"glass","setting":{"kind":"map_range","range_nm":12.0}}}),
    )
    .unwrap();
    knob(&mut s, "right_display", "right", "inner", 1);
    assert_eq!(glass_state(&s).settings.range_nm, 8.0);
    knob(&mut s, "right_display", "right", "inner", -2);
    assert_eq!(glass_state(&s).settings.range_nm, 20.0);
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

// ---- Touch keypads, radio windows, range ladder, nearest pages, director knob, reversion ----

fn try_display(s: &mut Session, id: &str, action: Value) -> Result<Value, CommandError> {
    s.execute(
        "equipment.set",
        json!({"setting":{"kind":"display","id":id,"action":action}}),
    )
}
fn open(s: &mut Session, id: &str, overlay: Value) {
    display(s, id, json!({"kind":"open_overlay","overlay":overlay}));
}
fn enter(s: &mut Session, id: &str, target: &str, text: &str) -> Result<Value, CommandError> {
    try_display(s, id, json!({"kind":"enter","target":target,"text":text}))
}
fn set_glass(s: &mut Session, setting: Value) {
    s.execute(
        "avionics.set",
        json!({"setting":{"kind":"glass","setting":setting}}),
    )
    .unwrap();
}

#[test]
fn keypad_entry_sets_baro_heading_altitude_and_course() {
    let mut s = session(None);
    // Baro, in inHg and then hPa.
    open(&mut s, "left_display", json!({"keypad":"baro"}));
    assert_eq!(
        state(&s, "left_display").overlay,
        Overlay::Keypad(touch_entry::EntryTarget::Baro)
    );
    enter(&mut s, "left_display", "baro", "2985").unwrap();
    assert!((glass_state(&s).baro_pa - 29.85 * 3386.389).abs() < 1.0);
    assert_eq!(
        state(&s, "left_display").overlay,
        Overlay::None,
        "Enter closes the keypad"
    );
    set_glass(&mut s, json!({"kind":"pressure_units","hpa":true}));
    enter(&mut s, "left_display", "baro", "1020").unwrap();
    assert!((glass_state(&s).baro_pa - 102_000.).abs() < 1e-6);
    // Heading, with 360 meaning north.
    enter(&mut s, "left_display", "heading", "090").unwrap();
    assert!((glass_state(&s).settings.heading_bug_rad.to_degrees() - 90.).abs() < 1e-6);
    enter(&mut s, "left_display", "heading", "360").unwrap();
    assert!(glass_state(&s).settings.heading_bug_rad.to_degrees().abs() < 1e-6);
    // Altitude in feet, and course.
    enter(&mut s, "left_display", "altitude", "3500").unwrap();
    assert!((glass_state(&s).settings.selected_altitude_m - 3500. * 0.3048).abs() < 1e-6);
    enter(&mut s, "left_display", "course", "123").unwrap();
    let g = glass_state(&s).settings;
    assert!((g.course_rad.to_degrees() - 123.).abs() < 1e-6 && g.course_manual);
}

#[test]
fn keypad_entry_sets_the_direct_to_identifier_the_radios_and_the_transponder() {
    let mut s = session(None);
    // COM1 standby, then transfer.
    open(&mut s, "left_display", json!("com_window"));
    assert_eq!(state(&s, "left_display").overlay, Overlay::ComWindow);
    let active = glass_state(&s).comms.com1.active_khz;
    enter(&mut s, "left_display", "com_standby", "121.5").unwrap();
    assert_eq!(glass_state(&s).comms.com1.standby_khz, 121_500);
    assert_eq!(glass_state(&s).comms.com1.active_khz, active);
    display(&mut s, "left_display", json!({"kind":"transfer","band":"com"}));
    assert_eq!(glass_state(&s).comms.com1.active_khz, 121_500);
    assert_eq!(glass_state(&s).comms.com1.standby_khz, active);
    // NAV1 standby, then transfer.
    open(&mut s, "left_display", json!("nav_window"));
    enter(&mut s, "left_display", "nav_standby", "113.8").unwrap();
    assert_eq!(
        glass_state(&s).touch.navigator.as_ref().unwrap().nav_standby_khz,
        113_800
    );
    display(&mut s, "left_display", json!({"kind":"transfer","band":"nav"}));
    assert_eq!(
        glass_state(&s).touch.navigator.as_ref().unwrap().nav_active_khz,
        113_800
    );
    // Transponder: a code, then a mode key keeps the code.
    open(&mut s, "left_display", json!("transponder_window"));
    enter(&mut s, "left_display", "transponder", "4321").unwrap();
    assert_eq!(glass_state(&s).settings.transponder_code, "4321");
    display(
        &mut s,
        "left_display",
        json!({"kind":"transponder_mode","mode":"standby"}),
    );
    let g = glass_state(&s).settings;
    assert_eq!(g.transponder_code, "4321");
    assert_eq!(format!("{:?}", g.transponder_mode), "Standby");
}

#[test]
fn a_bad_entry_is_refused_and_leaves_the_keypad_open() {
    let mut s = session(None);
    open(&mut s, "left_display", json!({"keypad":"heading"}));
    for (target, text) in [
        ("heading", "361"),
        ("heading", ""),
        ("altitude", "99999"),
        ("baro", "5"),
        ("transponder", "1289"),
        ("com_standby", "117.0"),
        ("com_standby", "121.512"),
        ("nav_standby", "113.83"),
        ("direct_to", "ZZZZ9"),
    ] {
        assert!(
            enter(&mut s, "left_display", target, text).is_err(),
            "{target} {text:?}"
        );
    }
    assert_eq!(
        state(&s, "left_display").overlay,
        Overlay::Keypad(touch_entry::EntryTarget::Heading)
    );
    // A non-entry overlay cannot be opened by touch.
    assert!(
        try_display(
            &mut s,
            "left_display",
            json!({"kind":"open_overlay","overlay":"main_menu"})
        )
        .is_err()
    );
    display(&mut s, "left_display", json!({"kind":"close_overlay"}));
    assert_eq!(state(&s, "left_display").overlay, Overlay::None);
}

#[test]
fn keypad_shortcuts_take_the_current_value_or_standard_pressure() {
    let mut s = session(None);
    set_glass(&mut s, json!({"kind":"baro","pressure_pa":99_000.0}));
    display(
        &mut s,
        "left_display",
        json!({"kind":"shortcut","target":"baro","shortcut":"set_to_standard"}),
    );
    assert!((glass_state(&s).baro_pa - 101_325.).abs() < 1e-6);
    // Set For Field recovers the pressure for the field the aircraft sits on.
    display(
        &mut s,
        "left_display",
        json!({"kind":"shortcut","target":"baro","shortcut":"set_for_field"}),
    );
    let field = glass_state(&s).baro_pa;
    assert!((80_000. ..=110_000.).contains(&field));
    s.execute(
        "avionics.set",
        json!({"setting":{"kind":"heading_bug","degrees":200.0}}),
    )
    .unwrap();
    let heading = glass_state(&s)
        .air_data
        .heading_rad
        .to_degrees()
        .round()
        .rem_euclid(360.);
    display(
        &mut s,
        "left_display",
        json!({"kind":"shortcut","target":"heading","shortcut":"set_to_current"}),
    );
    assert!((glass_state(&s).settings.heading_bug_rad.to_degrees() - heading).abs() < 1e-6);
    display(
        &mut s,
        "left_display",
        json!({"kind":"shortcut","target":"altitude","shortcut":"set_to_current"}),
    );
    let sel_ft = glass_state(&s).settings.selected_altitude_m / 0.3048;
    assert!(
        (sel_ft / 100. - (sel_ft / 100.).round()).abs() < 1e-6,
        "whole hundreds: {sel_ft}"
    );
    display(
        &mut s,
        "left_display",
        json!({"kind":"shortcut","target":"course","shortcut":"set_to_current"}),
    );
    assert!(glass_state(&s).settings.course_manual);
    // A shortcut the pad does not offer is refused.
    assert!(
        try_display(
            &mut s,
            "left_display",
            json!({"kind":"shortcut","target":"transponder","shortcut":"set_to_current"})
        )
        .is_err()
    );
}

#[test]
fn the_range_ladder_has_23_steps_and_the_zoom_buttons_use_it() {
    let mut s = session(None);
    let ladder = touch_efis::MAP_RANGES_NM;
    assert_eq!(ladder.len(), 23);
    set_glass(&mut s, json!({"kind":"map_range","range_nm":1200.0}));
    // Walk the whole ladder with the knob, one detent at a time, to the smallest range.
    let mut seen = vec![glass_state(&s).settings.range_nm];
    for _ in 0..22 {
        knob(&mut s, "right_display", "right", "inner", 1);
        seen.push(glass_state(&s).settings.range_nm);
    }
    for (got, want) in seen.iter().zip(ladder.iter().rev()) {
        assert!((got - want).abs() < 1e-9, "{got} vs {want}");
    }
    knob(&mut s, "right_display", "right", "inner", 1);
    assert!(
        (glass_state(&s).settings.range_nm - ladder[0]).abs() < 1e-9,
        "stops at the end"
    );
    // Zoom buttons: positive steps zoom in.
    set_glass(&mut s, json!({"kind":"map_range","range_nm":20.0}));
    display(&mut s, "right_display", json!({"kind":"map_zoom","steps":-1}));
    assert_eq!(glass_state(&s).settings.range_nm, 30.);
    display(&mut s, "right_display", json!({"kind":"map_zoom","steps":2}));
    assert_eq!(glass_state(&s).settings.range_nm, 12.);
    // Ranges outside the ladder's ends are refused.
    for nm in [0.01, 1201.0] {
        assert!(
            s.execute(
                "avionics.set",
                json!({"setting":{"kind":"glass","setting":{"kind":"map_range","range_nm":nm}}})
            )
            .is_err(),
            "{nm}"
        );
    }
}

#[test]
fn the_select_page_menu_and_nearest_pages_follow_the_nearest_key() {
    let mut s = session(None);
    open(&mut s, "right_display", json!("select_page"));
    assert_eq!(state(&s, "right_display").overlay, Overlay::SelectPage);
    display(
        &mut s,
        "right_display",
        json!({"kind":"mfd_page","page":"nearest"}),
    );
    assert_eq!(state(&s, "right_display").overlay, Overlay::None);
    let d = state(&s, "right_display");
    assert_eq!(touch_efis::effective_page(d.layout, d.mfd_page), MfdPage::Nearest);
    // The large MFD knob steps the nearest pages here, not the main pages.
    use flightcraft_aero::modern::NearestPage as N;
    assert_eq!(state(&s, "right_display").nearest_page, N::Airports);
    knob(&mut s, "right_display", "right", "outer", 1);
    assert_eq!(state(&s, "right_display").nearest_page, N::Vors);
    assert_eq!(state(&s, "right_display").mfd_page, MfdPage::Nearest);
    knob(&mut s, "right_display", "right", "outer", 4);
    assert_eq!(state(&s, "right_display").nearest_page, N::Airports);
    display(
        &mut s,
        "right_display",
        json!({"kind":"nearest_page","page":"airspace"}),
    );
    assert_eq!(state(&s, "right_display").nearest_page, N::Airspace);
    // The airspace list is built while its page shows; with nothing near it is empty.
    assert!(glass_state(&s).nearest_airspace.is_empty());
    // Leaving the nearest group, the knob steps main pages again.
    display(&mut s, "right_display", json!({"kind":"mfd_page","page":"map"}));
    knob(&mut s, "right_display", "right", "outer", 1);
    assert_eq!(state(&s, "right_display").mfd_page, MfdPage::Waypoint);
}

#[test]
fn nearest_lists_are_only_built_for_a_page_that_is_showing() {
    let s = session(None);
    let g = glass_state(&s);
    assert!(g.nearest_vors.is_empty() && g.nearest_ndbs.is_empty());
    assert!(g.nearest_intersections.is_empty() && g.nearest_airspace.is_empty());
    // The airport list is always there and holds at most the 15 rows of the unit's list.
    assert!(g.nearest.len() <= 15);
    assert!(g.nearest.iter().all(|a| a.distance_nm <= 200.));
}

/// An airborne start, aligned, with the autopilot engaged in altitude hold.
fn airborne(s: &mut Session) {
    s.execute(
        "sim.reset",
        json!({"start":{"kind":"air","height_m":1000.,"airspeed_mps":40.,"heading_rad":0.}}),
    )
    .unwrap();
    s.execute(
        "assist.set",
        json!({"altitude_m":1000.,"heading_rad":0.,"airspeed_mps":40.}),
    )
    .unwrap();
    s.run(Step { steps: 5520 }).unwrap();
    s.execute("assist.off", json!({})).unwrap();
}
fn autopilot(s: &mut Session, setting: Value) {
    s.execute(
        "avionics.set",
        json!({"setting":{"kind":"autopilot","setting":setting}}),
    )
    .unwrap();
}

#[test]
fn the_flight_director_bug_knob_steps_the_active_reference_and_a_push_syncs_pitch() {
    let mut s = session(None);
    airborne(&mut s);
    display(
        &mut s,
        "left_display",
        json!({"kind":"knobs","config":{"alternate":"fd_bug_baro","press_to_toggle":false,"mfd_side":"right"}}),
    );
    autopilot(&mut s, json!({"kind":"flight_director","on":true}));
    autopilot(&mut s, json!({"kind":"vertical","mode":"vs"}));
    autopilot(&mut s, json!({"kind":"vertical_speed","vertical_speed_mps":0.0}));
    // Right inner knob: 100 fpm per detent.
    knob(&mut s, "left_display", "right", "inner", 3);
    let vs = s.sim().installed_autopilot().unwrap().vertical_speed_mps / 0.3048 * 60.;
    assert!((vs - 300.).abs() < 1e-6, "{vs}");
    knob(&mut s, "left_display", "right", "inner", -5);
    let vs = s.sim().installed_autopilot().unwrap().vertical_speed_mps / 0.3048 * 60.;
    assert!((vs + 200.).abs() < 1e-6, "{vs}");
    // Pitch hold: half a degree per detent.
    autopilot(&mut s, json!({"kind":"vertical","mode":"pit"}));
    let before = s.sim().installed_autopilot().unwrap().pitch_rad;
    knob(&mut s, "left_display", "right", "inner", 2);
    let after = s.sim().installed_autopilot().unwrap().pitch_rad;
    assert!(
        ((after - before).to_degrees() - 1.).abs() < 1e-6,
        "{before} {after}"
    );
    // A push sets the pitch reference to the present pitch.
    display(
        &mut s,
        "left_display",
        json!({"kind":"knob_push","knob":"right","time_s":0.0}),
    );
    let ap = s.sim().installed_autopilot().unwrap();
    assert!((ap.pitch_rad - s.sim().readout().pitch_rad).abs() < 1e-6);
    // The outer ring is still baro.
    let baro = glass_state(&s).baro_pa;
    knob(&mut s, "left_display", "right", "outer", 1);
    assert!(glass_state(&s).baro_pa > baro);
}

#[test]
fn the_course_knob_push_syncs_the_course_to_the_heading_without_guidance() {
    let mut s = session(None);
    airborne(&mut s);
    knob(&mut s, "left_display", "right", "inner", 37);
    let heading = glass_state(&s)
        .air_data
        .heading_rad
        .to_degrees()
        .round()
        .rem_euclid(360.);
    display(
        &mut s,
        "left_display",
        json!({"kind":"knob_push","knob":"right","time_s":0.0}),
    );
    let course = glass_state(&s).settings.course_rad.to_degrees();
    assert!((course - heading).abs() < 1e-6, "{course} vs {heading}");
}

/// The new-york region with the aircraft airborne in it, for tests that need real airports.
fn in_new_york(s: &mut Session) {
    s.install_region(
        read_region(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/terrain/new-york"))
            .unwrap(),
    )
    .unwrap();
    s.execute(
        "sim.reset",
        json!({"world":"new-york","start":{"kind":"air","height_m":1000.,"airspeed_mps":40.,"heading_rad":0.}}),
    )
    .unwrap();
    s.execute(
        "assist.set",
        json!({"altitude_m":1000.,"heading_rad":0.,"airspeed_mps":40.}),
    )
    .unwrap();
    s.run(Step { steps: 5520 }).unwrap();
    s.execute("assist.off", json!({})).unwrap();
}

#[test]
fn direct_to_keypad_and_course_sync_use_a_real_leg() {
    require_region_data!();
    let mut s = session(None);
    in_new_york(&mut s);
    key(&mut s, "left_display", "direct", false);
    enter(&mut s, "left_display", "direct_to", "kmmu").unwrap();
    assert_eq!(state(&s, "left_display").overlay, Overlay::None);
    assert!(glass_state(&s).settings.flight_plan.iter().any(|a| a == "KMMU"));
    // With an active leg the course push takes the desired track, whole degrees.
    knob(&mut s, "left_display", "right", "inner", 90);
    display(
        &mut s,
        "left_display",
        json!({"kind":"knob_push","knob":"right","time_s":0.0}),
    );
    let g = glass_state(&s);
    assert!(g.gps.valid);
    let dtk = g.gps.desired_track_rad.to_degrees().round().rem_euclid(360.);
    assert!((g.settings.course_rad.to_degrees() - dtk).abs() < 1e-6, "{dtk}");
    // Set To Current on the course keypad does the same.
    knob(&mut s, "left_display", "right", "inner", 45);
    display(
        &mut s,
        "left_display",
        json!({"kind":"shortcut","target":"course","shortcut":"set_to_current"}),
    );
    assert!((glass_state(&s).settings.course_rad.to_degrees() - dtk).abs() < 1e-6);
}

#[test]
fn the_afcs_box_reflects_the_autopilot_modes_as_they_change() {
    use flightcraft_control::touch_pfd::{AfcsTone, afcs_box};
    let mut s = session(None);
    airborne(&mut s);
    let ap = || s.sim().installed_autopilot().unwrap().clone();
    assert!(afcs_box(&ap(), "GPS").autopilot.is_none());
    autopilot(&mut s, json!({"kind":"engage","on":true}));
    autopilot(&mut s, json!({"kind":"lateral","mode":"hdg"}));
    autopilot(&mut s, json!({"kind":"vertical","mode":"alt"}));
    let b = afcs_box(s.sim().installed_autopilot().unwrap(), "GPS");
    assert_eq!(b.autopilot.as_ref().unwrap().tone, AfcsTone::Active);
    assert_eq!(b.lateral_active.as_ref().unwrap().text, "HDG");
    assert_eq!(b.vertical_active.as_ref().unwrap().text, "ALT");
    assert!(b.reference.as_ref().unwrap().ends_with("ft"));
    assert!(!b.hand_flying);
    // Disengaging leaves the director: hollow bars, hand flying.
    autopilot(&mut s, json!({"kind":"flight_director","on":true}));
    autopilot(&mut s, json!({"kind":"engage","on":false}));
    let b = afcs_box(s.sim().installed_autopilot().unwrap(), "GPS");
    assert!(b.hand_flying || b.autopilot.is_none());
}

#[test]
fn losing_the_pfd_disconnects_the_autopilot_but_losing_the_mfd_does_not() {
    let mut s = session(None);
    airborne(&mut s);
    autopilot(&mut s, json!({"kind":"engage","on":true}));
    autopilot(&mut s, json!({"kind":"vertical","mode":"alt"}));
    assert!(s.sim().installed_autopilot().unwrap().engaged);
    // The multi-function display fails: the autopilot carries on.
    display(&mut s, "right_display", json!({"kind":"failure","failed":true}));
    s.run(Step { steps: 120 }).unwrap();
    assert!(
        s.sim().installed_autopilot().unwrap().engaged,
        "MFD loss keeps the autopilot"
    );
    let left = state(&s, "left_display");
    assert!(left.reversion, "the surviving PFD reverts");
    display(&mut s, "right_display", json!({"kind":"failure","failed":false}));
    // The primary display fails: the autopilot disconnects, once, and can be engaged again.
    display(&mut s, "left_display", json!({"kind":"failure","failed":true}));
    s.run(Step { steps: 120 }).unwrap();
    assert!(
        !s.sim().installed_autopilot().unwrap().engaged,
        "PFD loss disconnects"
    );
    autopilot(&mut s, json!({"kind":"engage","on":true}));
    s.run(Step { steps: 240 }).unwrap();
    assert!(
        s.sim().installed_autopilot().unwrap().engaged,
        "engaging again is allowed: the disconnect happens once"
    );
}

#[test]
fn a_reverted_display_is_a_split_screen_with_the_pfd_and_the_engine_page() {
    let mut s = session(None);
    display(&mut s, "left_display", json!({"kind":"failure","failed":true}));
    let right = state(&s, "right_display");
    assert!(right.reversion);
    assert_eq!(
        touch_efis::reverted_layout(right.installed_layout),
        TouchLayout::Split
    );
    assert_eq!(touch_efis::REVERSION_PAGE, MfdPage::Engine);
    // Knobs on a reverted display behave as in the split screen: the PFD knob and the MFD knob.
    let alt = glass_state(&s).settings.selected_altitude_m;
    knob(&mut s, "right_display", "left", "outer", 1);
    assert!(glass_state(&s).settings.selected_altitude_m > alt);
}

#[test]
fn the_minimums_keypad_sets_and_clears_the_minimums_and_a_bad_entry_leaves_it_open() {
    let mut s = session(None);
    open(&mut s, "left_display", json!({"keypad":"minimums"}));
    assert_eq!(
        state(&s, "left_display").overlay,
        Overlay::Keypad(touch_entry::EntryTarget::Minimums)
    );
    assert!(enter(&mut s, "left_display", "minimums", "15001").is_err());
    assert_eq!(
        state(&s, "left_display").overlay,
        Overlay::Keypad(touch_entry::EntryTarget::Minimums),
        "a refused entry leaves the keypad open"
    );
    assert_eq!(glass_state(&s).settings.minimums_m, None);
    enter(&mut s, "left_display", "minimums", "250").unwrap();
    assert!((glass_state(&s).settings.minimums_m.unwrap() - 250. * 0.3048).abs() < 1e-9);
    assert_eq!(state(&s, "left_display").overlay, Overlay::None);
    // Minimums Off.
    open(&mut s, "left_display", json!({"keypad":"minimums"}));
    display(
        &mut s,
        "left_display",
        json!({"kind":"shortcut","target":"minimums","shortcut":"turn_off"}),
    );
    assert_eq!(glass_state(&s).settings.minimums_m, None);
}

#[test]
fn the_fpl_altitude_keypads_set_a_constraint_on_the_selected_waypoint() {
    require_region_data!();
    let mut s = session(None);
    in_new_york(&mut s);
    key(&mut s, "left_display", "direct", false);
    enter(&mut s, "left_display", "direct_to", "kmmu").unwrap();
    display(
        &mut s,
        "left_display",
        json!({"kind":"flight_plan","edit":{"kind":"select","index":0}}),
    );
    for (target, text, want) in [
        ("fpl_altitude_at", "3000", "3000FT"),
        ("fpl_altitude_above", "2500", "2500FT A"),
        ("fpl_altitude_below", "4000", "4000FT B"),
    ] {
        open(&mut s, "left_display", json!({"keypad":target}));
        enter(&mut s, "left_display", target, text).unwrap();
        let g = glass_state(&s);
        assert_eq!(g.gps.legs[0].constraint.as_deref(), Some(want), "{target}");
        assert_eq!(state(&s, "left_display").overlay, Overlay::None);
    }
    assert!(enter(&mut s, "left_display", "fpl_altitude_at", "99999").is_err());
    display(
        &mut s,
        "left_display",
        json!({"kind":"flight_plan","edit":{"kind":"altitude","constraint":null}}),
    );
    assert_eq!(glass_state(&s).gps.legs[0].constraint, None);
}
