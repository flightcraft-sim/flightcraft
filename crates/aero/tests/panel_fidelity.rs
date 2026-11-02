#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::unreachable)]
use flightcraft_aero::{modern::*, panel_fidelity::*};
use serde_json::json;
fn panel() -> EquipmentPanel {
    let flightcraft_aero::LayerFile::Panel(p) = flightcraft_aero::LayerFile::from_json(include_str!(
        "../../../examples/packs/equipment-demo/panel.json"
    ))
    .unwrap() else {
        unreachable!()
    };
    p.equipment.unwrap()
}
#[test]
fn essential_panel_region_rejects_invalid_and_outside_bounds() {
    let mut p = panel();
    assert!(p.essential.is_none());
    for region in [
        [0., 0., 0., 10.],
        [-1., 0., 10., 10.],
        [0., 0., 1251., 700.],
        [0., 699., 10., 2.],
        [f64::NAN, 0., 10., 10.],
        [0., 0., f64::INFINITY, 10.],
    ] {
        p.essential = Some(region);
        assert!(p.validate().is_err(), "accepted {region:?}");
    }
    p.essential = Some([80., 90., 1120., 430.]);
    p.validate().unwrap();
    let encoded = serde_json::to_string(&p).unwrap();
    assert_eq!(serde_json::from_str::<EquipmentPanel>(&encoded).unwrap(), p);
}
#[test]
fn old_pack_defaults_and_all_optional_artwork_fields_round_trip() {
    let mut p = panel();
    assert!(
        p.elements
            .iter()
            .all(|e| e.appearance == EquipmentAppearance::default())
    );
    p.shell.as_mut().unwrap().palette = serde_json::from_value(
        json!({"face":"#2b2c2e","glare_shield":"#121212","bezel":"#080808","placard_text":"#ffffff"}),
    )
    .unwrap();
    p.shell.as_mut().unwrap().centre_console = Some(Outline::Polygon {
        points: vec![[500., 500.], [600., 500.], [600., 650.], [500., 650.]],
    });
    let display = p
        .elements
        .iter_mut()
        .find(|e| matches!(e.element, PanelElement::TouchDisplay { .. }))
        .unwrap();
    display.appearance=serde_json::from_value(json!({"synthetic_vision":true,"skin":"private_art","bezel":{"color":"#0b0b0b","edge_trim":"#bbbbbb","margins":[5.,6.,5.,12.],"softkeys":false,"softkey_count":4,"corner_knobs":[[8.,180.],[250.,180.]],"card_slot":true}})).unwrap();
    // Knobs are local mm and checked against this display's real rectangle.
    display.appearance.bezel.as_mut().unwrap().corner_knobs = vec![
        [8., display.size[1] - 8.],
        [display.size[0] - 8., display.size[1] - 8.],
    ];
    let nav = p
        .elements
        .iter_mut()
        .find(|e| matches!(e.element, PanelElement::Navigator { .. }))
        .unwrap();
    nav.appearance.navigator_layout = NavigatorLayout::HomeColumns;
    let ap = p
        .elements
        .iter_mut()
        .find(|e| matches!(e.element, PanelElement::Autopilot))
        .unwrap();
    ap.appearance.controller=Some(serde_json::from_value(json!({"sections":[{"label":"MODES","rect":[0.,0.,10.,10.]}],"controls":[{"action":"ap","rect":[1.,1.,3.,3.],"lit_color":"#70ddaa"},{"action":"heading_knob","rect":[5.,1.,3.,3.]}]})).unwrap());
    p.validate().unwrap();
    let encoded = serde_json::to_value(&p).unwrap();
    let decoded: EquipmentPanel = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, p);
    for (kind, style) in [
        (
            json!({"kind":"rocker","switch":"ems_backup","label":"EMS"}),
            ControlStyle::BlackRocker,
        ),
        (
            json!({"kind":"breaker","load":"touch_left","label":"PFD"}),
            ControlStyle::PushPullBreaker,
        ),
        (json!({"kind":"parachute"}), ControlStyle::RoundedPullHandle),
        (json!({"kind":"key"}), ControlStyle::SilverKey),
        (json!({"kind":"vent"}), ControlStyle::LouvredEyeball),
        (json!({"kind":"elt","label":"ELT"}), ControlStyle::EltPlate),
        (json!({"kind":"fuel_selector"}), ControlStyle::RedTHandle),
        (json!({"kind":"dimmer","label":"LIGHT"}), ControlStyle::ArcDimmer),
        (
            json!({"kind":"push_pull","label":"HEAT"}),
            ControlStyle::CabinHeat,
        ),
    ] {
        let mut p = panel();
        let e = &mut p.elements[0];
        e.element = serde_json::from_value(kind).unwrap();
        e.appearance.style = Some(style);
        p.validate().unwrap();
    }
    for binding in [ControlBinding::LaneA, ControlBinding::LaneB] {
        let mut p = panel();
        p.elements[0].element = PanelElement::Toggle {
            label: "LANE".into(),
            guarded: true,
            lamp: true,
        };
        p.elements[0].appearance.binding = Some(binding);
        p.validate().unwrap();
    }
}
#[test]
fn hostile_artwork_and_unreadable_labels_are_rejected() {
    for color in ["#fff", "#+fffff", "#zzzzzz", "ffffff", "#ffffff00", "#💚abc"] {
        assert!(SrgbHex(color.into()).validate().is_err());
    }
    assert_eq!(
        SrgbHex("#ffffff".into()).contrast(&SrgbHex("#000000".into())),
        Some(21.)
    );
    let mut p = panel();
    p.shell.as_mut().unwrap().palette = PanelPalette {
        face: Some(SrgbHex("#dddddd".into())),
        placard_text: Some(SrgbHex("#cccccc".into())),
        ..Default::default()
    };
    assert!(p.validate().is_err());
    for value in [
        json!({"bezel":{"color":null,"edge_trim":null,"margins":[9999.,0.,0.,0.],"softkeys":true,"softkey_count":4,"corner_knobs":[],"card_slot":false}}),
        json!({"bezel":{"margins":[0.,0.,0.,0.],"softkeys":true,"softkey_count":0,"corner_knobs":[],"card_slot":false}}),
        json!({"controller":{"controls":[{"action":"ap","rect":[0.,0.,1.,1.]},{"action":"ap","rect":[2.,0.,1.,1.]}]}}),
        json!({"controller":{"sections":[{"label":"bad\nlabel","rect":[0.,0.,1.,1.]}],"controls":[]}}),
        json!({"binding":"lane_a"}),
        json!({"style":"red_t_handle"}),
        json!({"navigator_layout":"home_columns"}),
        json!({"skin":"../invalid"}),
    ] {
        let mut p = panel();
        p.elements[0].appearance = serde_json::from_value(value).unwrap();
        assert!(p.validate().is_err());
    }
    let a:EquipmentAppearance=serde_json::from_value(json!({"bezel":{"margins":[0.,0.,0.,0.],"softkeys":true,"softkey_count":2,"corner_knobs":[[-1.,0.]],"card_slot":true}})).unwrap();
    assert!(a.validate([10., 10.]).is_err());
}
#[test]
fn backup_bus_and_wigwag_fields_are_bounded_and_referenced() {
    let mut systems: flightcraft_aero::SystemsDefinition = serde_json::from_value(
        serde_json::from_str::<serde_json::Value>(include_str!(
            "../../../examples/packs/equipment-demo/systems.json"
        ))
        .unwrap()["definition"]
            .clone(),
    )
    .unwrap();
    let modern = systems.modern.as_mut().unwrap();
    modern.backup_buses = vec![BackupBus {
        circuit: BackupCircuit::Efis,
        capacity_ah: 2.,
        voltage: 12.,
        loads: vec!["touch_left".into()],
    }];
    modern.wigwag = Some(Wigwag {
        left_load: "landing".into(),
        right_load: "landing_right".into(),
        half_period_s: 0.5,
    });
    let mut light = systems
        .electrical
        .loads
        .iter()
        .find(|l| l.id == "landing")
        .unwrap()
        .clone();
    light.id = "landing_right".into();
    systems.electrical.loads.push(light);
    systems.validate().unwrap();
    let encoded = serde_json::to_string(&systems).unwrap();
    assert_eq!(systems, serde_json::from_str(&encoded).unwrap());
    systems.modern.as_mut().unwrap().backup_buses[0].loads = vec!["missing".into()];
    assert!(systems.validate().is_err());
    systems.modern.as_mut().unwrap().backup_buses[0].loads = vec!["touch_left".into()];
    systems
        .modern
        .as_mut()
        .unwrap()
        .wigwag
        .as_mut()
        .unwrap()
        .half_period_s = f64::NAN;
    assert!(systems.validate().is_err());
}

#[test]
fn terrain_and_synthetic_vision_inset_content_round_trips() {
    for name in ["terrain", "synthetic_vision"] {
        let inset: PfdInset =
            serde_json::from_value(json!({"corner":"bottom_right","content":name})).unwrap();
        let mut p = panel();
        if let PanelElement::TouchDisplay {
            layout, composition, ..
        } = &mut p.elements[0].element
        {
            *layout = TouchLayout::IntegratedPfd;
            composition.insets = vec![inset.clone()];
        }
        p.validate().unwrap();
        assert_eq!(serde_json::to_value(inset).unwrap()["content"], name);
    }
}
