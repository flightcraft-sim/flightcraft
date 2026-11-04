#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::unreachable)]
use flightcraft_aero::{LayerFile, modern::*};

fn demo_panel() -> EquipmentPanel {
    let LayerFile::Panel(p) =
        LayerFile::from_json(include_str!("../../../examples/packs/equipment-demo/panel.json")).unwrap()
    else {
        unreachable!()
    };
    p.equipment.unwrap()
}

fn with_placard_text(text: &str) -> EquipmentPanel {
    let mut panel = demo_panel();
    let placard = panel
        .elements
        .iter_mut()
        .find(|e| matches!(e.element, PanelElement::Placard { .. }))
        .unwrap();
    let PanelElement::Placard { text: t, .. } = &mut placard.element else {
        unreachable!()
    };
    *t = text.into();
    panel
}

#[test]
fn registration_token_is_replaced_by_the_current_registration() {
    assert_eq!(expand_placard_text("{registration}", "N371FC"), "N371FC");
    // The pilot's own tail number replaces the pack's.
    assert_eq!(expand_placard_text("{registration}", "N824RX"), "N824RX");
    assert_eq!(expand_placard_text("ID {registration}", "D-EABC"), "ID D-EABC");
    assert_eq!(expand_placard_text("NO TOKEN", "N371FC"), "NO TOKEN");
}

#[test]
fn validation_accepts_the_token_and_still_limits_the_text() {
    for ok in ["{registration}", "ID: {registration}", &"x".repeat(128)] {
        with_placard_text(ok).validate().unwrap();
    }
    for bad in [
        "",
        "tab\there",
        &"x".repeat(129),
        &format!("{}{{registration}}", "x".repeat(120)),
    ] {
        assert!(with_placard_text(bad).validate().is_err(), "accepted {bad:?}");
    }
    // Expansion cannot push validated text past the limit: registrations are shorter than the token.
    let longest = "A".repeat(MAX_REGISTRATION_LEN);
    let text = "{registration}".repeat(9);
    with_placard_text(&text).validate().unwrap();
    assert!(expand_placard_text(&text, &longest).len() <= 128);
}

#[test]
fn every_builtin_panel_has_a_registration_placard() {
    for (name, json) in [
        ("corvid", include_str!("../../../aircraft/corvid/panel.json")),
        (
            "corvid-digital",
            include_str!("../../../aircraft/corvid/panel-digital.json"),
        ),
        (
            "corvid-legacy",
            include_str!("../../../aircraft/corvid/panel-legacy.json"),
        ),
        (
            "corvid-leveler",
            include_str!("../../../aircraft/corvid/panel-leveler.json"),
        ),
        ("larrow", include_str!("../../../aircraft/larrow/panel.json")),
    ] {
        let LayerFile::Panel(panel) = LayerFile::from_json(json).unwrap() else {
            unreachable!()
        };
        let placards = panel
            .instruments
            .iter()
            .filter(|i| i.module == "registration")
            .count();
        assert_eq!(placards, 1, "{name} needs exactly one registration placard");
        // The placard must not overlap any other instrument.
        let p = panel
            .instruments
            .iter()
            .find(|i| i.module == "registration")
            .unwrap();
        for other in panel.instruments.iter().filter(|i| i.module != "registration") {
            let overlap = p.position[0] < other.position[0] + other.size[0]
                && other.position[0] < p.position[0] + p.size[0]
                && p.position[1] < other.position[1] + other.size[1]
                && other.position[1] < p.position[1] + p.size[1];
            assert!(!overlap, "{name}: registration overlaps {}", other.id);
        }
    }
}
