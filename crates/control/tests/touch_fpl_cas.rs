//! The touch EFIS's flight-plan editing and crew-alert acknowledgement through `equipment.set`.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_control::touch_efis::Overlay;
use flightcraft_control::*;
use serde_json::{Value, json};
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/region_data.rs"));

fn session() -> Session {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut pack = read_pack(&root.join("../../examples/packs/equipment-demo")).unwrap();
    for source in pack.files.values_mut() {
        let mut document: Value = serde_json::from_str(source).unwrap();
        if document["kind"] != "panel" {
            continue;
        }
        for e in document["equipment"]["elements"].as_array_mut().unwrap() {
            if e["element"]["kind"] == "touch_display" {
                e["appearance"] = json!({"bezel":{"model":"landscape_touch_efis"}});
            }
        }
        *source = serde_json::to_string(&document).unwrap();
    }
    let mut s = Session::new().unwrap();
    s.install_pack(&pack).unwrap();
    s.execute(
        "aircraft.load",
        json!({"source":{"kind":"installed","id":"equipment-demonstrator"},"reset":true}),
    )
    .unwrap();
    let dir = root.join("../../assets/terrain/new-york");
    s.install_region(read_region(&dir).unwrap()).unwrap();
    s.execute(
        "sim.reset",
        json!({"world":"new-york","start":{"runway":{"airport":"1N7","end":"07"}}}),
    )
    .unwrap();
    s
}

fn display(s: &mut Session, action: Value) -> Result<Value, CommandError> {
    s.execute(
        "equipment.set",
        json!({"setting":{"kind":"display","id":"right_display","action":action}}),
    )
}
fn fpl(s: &mut Session, edit: Value) -> Result<Value, CommandError> {
    display(s, json!({"kind":"flight_plan","edit":edit}))
}
fn plan(s: &Session) -> Vec<String> {
    s.glass_state().settings.flight_plan
}
fn overlay(s: &Session) -> Overlay {
    s.touch_state()
        .displays
        .into_iter()
        .find(|d| d.id == "right_display")
        .unwrap()
        .overlay
}
fn set_plan(s: &mut Session, airports: &[&str]) {
    s.execute(
        "avionics.set",
        json!({"setting":{"kind":"glass","setting":{"kind":"flight_plan","airports":airports}}}),
    )
    .unwrap();
}

#[test]
fn touch_editing_inserts_removes_and_activates_legs_of_the_existing_flight_plan() {
    require_region_data!();
    let mut s = session();
    set_plan(&mut s, &["1N7", "KMMU", "KFRG"]);
    assert_eq!(plan(&s).len(), 3);
    // A waypoint outside the plan cannot be selected.
    assert!(fpl(&mut s, json!({"kind":"select","index":9})).is_err());
    // Select KMMU and insert KTEB before it: the new waypoint is selected.
    fpl(&mut s, json!({"kind":"select","index":1})).unwrap();
    assert_eq!(s.glass_state().settings.fpl_cursor, 1);
    fpl(&mut s, json!({"kind":"insert","after":false,"ident":"KTEB"})).unwrap();
    let p = plan(&s);
    assert_eq!(p.len(), 4, "{p:?}");
    assert!(p[1].contains("TEB") && p[2].contains("MMU"), "{p:?}");
    assert_eq!(s.glass_state().settings.fpl_cursor, 1);
    // Insert After puts the next one behind it.
    fpl(&mut s, json!({"kind":"insert","after":true,"ident":"KHPN"})).unwrap();
    let p = plan(&s);
    assert_eq!(p.len(), 5);
    assert!(p[2].contains("HPN"), "{p:?}");
    // Activate Leg: the selected waypoint becomes the one flown to.
    fpl(&mut s, json!({"kind":"select","index":3})).unwrap();
    fpl(&mut s, json!({"kind":"activate"})).unwrap();
    assert_eq!(s.glass_state().settings.active_leg, 3);
    assert_eq!(s.glass_state().gps.active_leg, 3);
    // Removing a waypoint before the active one keeps the same waypoint active.
    fpl(&mut s, json!({"kind":"select","index":1})).unwrap();
    fpl(&mut s, json!({"kind":"remove"})).unwrap();
    let p = plan(&s);
    assert_eq!(p.len(), 4);
    assert!(!p.iter().any(|w| w.contains("TEB")), "{p:?}");
    assert_eq!(s.glass_state().settings.active_leg, 2);
    // The data the glass computes follows the edited plan.
    assert_eq!(s.glass_state().gps.legs.len(), 4);
    // An unknown identifier is refused and the plan is untouched.
    assert!(fpl(&mut s, json!({"kind":"insert","after":true,"ident":"KZZZ9"})).is_err());
    assert_eq!(plan(&s), p);
    // Removing everything leaves an empty plan, and Remove then has nothing to act on.
    for _ in 0..4 {
        fpl(&mut s, json!({"kind":"remove"})).unwrap();
    }
    assert!(plan(&s).is_empty());
    assert!(fpl(&mut s, json!({"kind":"remove"})).is_err());
    // Insert into an empty plan makes it the plan.
    fpl(&mut s, json!({"kind":"insert","after":true,"ident":"KMMU"})).unwrap();
    assert_eq!(plan(&s).len(), 1);
}

#[test]
fn the_insert_keypads_edit_the_plan_beside_the_selected_waypoint() {
    require_region_data!();
    let mut s = session();
    set_plan(&mut s, &["1N7", "KFRG"]);
    fpl(&mut s, json!({"kind":"select","index":0})).unwrap();
    display(
        &mut s,
        json!({"kind":"open_overlay","overlay":{"keypad":"fpl_after"}}),
    )
    .unwrap();
    assert_eq!(overlay(&s), Overlay::Keypad(touch_entry::EntryTarget::FplAfter));
    display(&mut s, json!({"kind":"enter","target":"fpl_after","text":"kmmu"})).unwrap();
    let p = plan(&s);
    assert!(p.len() == 3 && p[1].contains("MMU"), "{p:?}");
    assert_eq!(overlay(&s), Overlay::None, "Enter closes the keypad");
    // A bad identifier keeps the keypad up and changes nothing.
    display(
        &mut s,
        json!({"kind":"open_overlay","overlay":{"keypad":"fpl_before"}}),
    )
    .unwrap();
    assert!(display(&mut s, json!({"kind":"enter","target":"fpl_before","text":"K M"})).is_err());
    assert_eq!(plan(&s).len(), 3);
    assert_eq!(overlay(&s), Overlay::Keypad(touch_entry::EntryTarget::FplBefore));
}

#[test]
fn touching_the_cas_window_or_pressing_back_acknowledges_the_master_lamps() {
    require_region_data!();
    let mut s = session();
    let master = |s: &Session| s.sim().systems().modern.as_ref().unwrap().master_caution;
    let door = |s: &mut Session, open: bool| {
        s.execute(
            "equipment.set",
            json!({"setting":{"kind":"systems","setting":{"kind":"door","open":open}}}),
        )
        .unwrap();
        s.execute("sim.step", json!({"steps":4})).unwrap();
    };
    door(&mut s, true);
    assert!(master(&s));
    display(&mut s, json!({"kind":"acknowledge_cas"})).unwrap();
    assert!(!master(&s));
    // BACK acknowledges before it does anything else, and does not change the page.
    door(&mut s, false);
    door(&mut s, true);
    assert!(master(&s));
    let page = |s: &Session| {
        let d = s
            .touch_state()
            .displays
            .into_iter()
            .find(|d| d.id == "right_display")
            .unwrap();
        (d.layout, d.mfd_page)
    };
    let before = page(&s);
    display(&mut s, json!({"kind":"key","key":"back","hold":false})).unwrap();
    assert!(!master(&s));
    assert_eq!(page(&s), before);
}

fn glass_set(s: &mut Session, setting: Value) -> Result<Value, CommandError> {
    s.execute(
        "avionics.set",
        json!({"setting":{"kind":"glass","setting":setting}}),
    )
}
fn systems_set(s: &mut Session, setting: Value) {
    s.execute(
        "equipment.set",
        json!({"setting":{"kind":"systems","setting":setting}}),
    )
    .unwrap();
}

#[test]
fn the_cas_list_acknowledges_one_message_at_a_time_and_keeps_it_until_it_clears() {
    require_region_data!();
    let mut s = session();
    let master = |s: &Session| s.sim().systems().modern.as_ref().unwrap().master_caution;
    systems_set(&mut s, json!({"kind":"door","open":true}));
    s.execute("sim.step", json!({"steps":4})).unwrap();
    systems_set(
        &mut s,
        json!({"kind":"failure","component":"lane_a","failed":true}),
    );
    s.execute("sim.step", json!({"steps":240})).unwrap();
    let list = s.glass_state().cas.list();
    assert!(list.len() >= 2, "{list:?}");
    assert!(list.iter().all(|m| !m.acknowledged));
    assert!(s.glass_state().cas.unacknowledged && master(&s));
    let first = list[0].text.clone();
    glass_set(&mut s, json!({"kind":"cas_acknowledge","index":0})).unwrap();
    let after = s.glass_state().cas.list();
    assert_eq!(after.len(), list.len(), "an acknowledged message stays listed");
    assert!(after.iter().find(|m| m.text == first).unwrap().acknowledged);
    assert!(after.iter().any(|m| !m.acknowledged), "the others still flash");
    assert!(master(&s), "the lamp stays lit while a caution is unacknowledged");
    display(&mut s, json!({"kind":"acknowledge_cas"})).unwrap();
    assert!(s.glass_state().cas.list().iter().all(|m| m.acknowledged));
    assert!(!master(&s));
    glass_set(&mut s, json!({"kind":"cas_acknowledge","index":0})).unwrap();
    assert!(glass_set(&mut s, json!({"kind":"cas_acknowledge","index":99})).is_err());
    // A message that clears leaves the list, and flashes again when it returns.
    systems_set(&mut s, json!({"kind":"door","open":false}));
    s.execute("sim.step", json!({"steps":4})).unwrap();
    let gone = s.glass_state().cas.list();
    assert!(gone.len() < list.len(), "{gone:?}");
    systems_set(&mut s, json!({"kind":"door","open":true}));
    s.execute("sim.step", json!({"steps":4})).unwrap();
    let again = s.glass_state().cas.list();
    assert_eq!(again.len(), list.len());
    assert!(again.iter().any(|m| !m.acknowledged), "{again:?}");
}

#[test]
fn the_cas_page_is_selectable_from_the_display_and_through_avionics_set() {
    require_region_data!();
    let mut s = session();
    display(&mut s, json!({"kind":"mfd_page","page":"cas"})).unwrap();
    let d = s
        .touch_state()
        .displays
        .into_iter()
        .find(|d| d.id == "right_display")
        .unwrap();
    assert_eq!(
        touch_efis::effective_page(d.layout, d.mfd_page),
        flightcraft_aero::modern::MfdPage::Cas
    );
    assert!(
        touch_efis::select_page_entries(flightcraft_aero::modern::MfdPage::Map).contains(
            &touch_efis::PageChoice::Main(flightcraft_aero::modern::MfdPage::Cas)
        )
    );
    glass_set(&mut s, json!({"kind":"page","page":"cas"})).unwrap();
    assert_eq!(s.glass_state().settings.page, GlassPage::Cas);
    systems_set(&mut s, json!({"kind":"door","open":true}));
    s.execute("sim.step", json!({"steps":4})).unwrap();
    glass_set(&mut s, json!({"kind":"cas_acknowledge","index":0})).unwrap();
    assert!(!s.sim().systems().modern.as_ref().unwrap().master_caution);
}

#[test]
fn ident_lasts_eighteen_seconds_and_needs_the_transponder_on() {
    require_region_data!();
    let mut s = session();
    let remaining = |s: &Session| s.glass_state().settings.ident_remaining_s;
    assert_eq!(remaining(&s), 0.);
    glass_set(
        &mut s,
        json!({"kind":"transponder","code":"1200","mode":"standby"}),
    )
    .unwrap();
    assert!(
        glass_set(&mut s, json!({"kind":"ident"})).is_err(),
        "no ident in standby"
    );
    glass_set(&mut s, json!({"kind":"transponder","code":"4321","mode":"alt"})).unwrap();
    glass_set(&mut s, json!({"kind":"ident"})).unwrap();
    assert!((remaining(&s) - 18.).abs() < 0.02, "{}", remaining(&s));
    s.execute("sim.step", json!({"steps":120 * 10})).unwrap();
    assert!((remaining(&s) - 8.).abs() < 0.1, "{}", remaining(&s));
    glass_set(&mut s, json!({"kind":"transponder","code":"1200","mode":"on"})).unwrap();
    assert!(remaining(&s) > 7.);
    s.execute("sim.step", json!({"steps":120 * 9})).unwrap();
    assert_eq!(remaining(&s), 0.);
    glass_set(&mut s, json!({"kind":"ident"})).unwrap();
    glass_set(
        &mut s,
        json!({"kind":"transponder","code":"1200","mode":"standby"}),
    )
    .unwrap();
    assert_eq!(remaining(&s), 0.);
}
