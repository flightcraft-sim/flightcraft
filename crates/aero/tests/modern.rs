#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::unreachable)]
use flightcraft_aero::modern::*;
use flightcraft_aero::*;
fn panel() -> LayerFile {
    LayerFile::from_json(include_str!("../../../examples/packs/equipment-demo/panel.json")).unwrap()
}
#[test]
fn bounds_and_hostile_tables() {
    let LayerFile::Panel(mut p) = panel() else {
        unreachable!()
    };
    assert!(LayerFile::Panel(p.clone()).validate().is_ok());
    p.equipment.as_mut().unwrap().elements[0].size = [1501., 1.];
    assert!(LayerFile::Panel(p).validate().is_err());
    let LayerFile::Engine(mut e) =
        LayerFile::from_json(include_str!("../../../examples/packs/equipment-demo/engine.json")).unwrap()
    else {
        unreachable!()
    };
    e.gearbox_ratio = f64::NAN;
    assert!(LayerFile::Engine(e.clone()).validate().is_err());
    e.gearbox_ratio = 2.4;
    if let EngineControl::ElectronicDualLane { coolant_lag_s, .. } = &mut e.control {
        *coolant_lag_s = 0.;
    }
    assert!(LayerFile::Engine(e).validate().is_err());
}

#[test]
fn millimetre_shell_and_composition_reject_hostile_artwork() {
    let LayerFile::Panel(mut p) = panel() else {
        unreachable!()
    };
    p.equipment
        .as_mut()
        .unwrap()
        .shell
        .as_mut()
        .unwrap()
        .screws
        .push([f64::NAN, 0.]);
    assert!(LayerFile::Panel(p).validate().is_err());
    let LayerFile::Panel(mut p) = panel() else {
        unreachable!()
    };
    let equipment = p.equipment.as_mut().unwrap();
    equipment.shell.as_mut().unwrap().outline = Outline::Polygon {
        points: vec![[0., 0.], [1250., 0.], [1250., 700.], [0., 700.]],
    };
    assert!(LayerFile::Panel(p.clone()).validate().is_ok());
    if let PanelElement::TouchDisplay { composition, .. } =
        &mut p.equipment.as_mut().unwrap().elements[0].element
    {
        composition.engine_strip_fraction = 0.95;
    }
    assert!(LayerFile::Panel(p).validate().is_err());
}

#[test]
fn navigator_audio_host_defaults_off_and_validates_shared_installation() {
    let LayerFile::Panel(mut p) = panel() else {
        unreachable!()
    };
    let navigator = p
        .equipment
        .as_mut()
        .unwrap()
        .elements
        .iter_mut()
        .find(|e| matches!(e.element, PanelElement::Navigator { .. }))
        .unwrap();
    let PanelElement::Navigator { audio_panel, .. } = &mut navigator.element else {
        unreachable!()
    };
    assert!(!*audio_panel);
    *audio_panel = true;
    let encoded = serde_json::to_string(&LayerFile::Panel(p.clone())).unwrap();
    let round_trip = LayerFile::from_json(&encoded).unwrap();
    assert_eq!(round_trip, LayerFile::Panel(p.clone()));
    let LayerFile::Engine(engine) =
        LayerFile::from_json(include_str!("../../../examples/packs/equipment-demo/engine.json")).unwrap()
    else {
        unreachable!()
    };
    let LayerFile::Systems(systems) = LayerFile::from_json(include_str!(
        "../../../examples/packs/equipment-demo/systems.json"
    ))
    .unwrap() else {
        unreachable!()
    };
    validate_installation(&engine, systems.definition.as_ref(), &p).unwrap();
    for field in ["power_load", "com1_load", "com2_load"] {
        let mut invalid = p.clone();
        match field {
            "power_load" => invalid.audio_panel.power_load = "missing".into(),
            "com1_load" => invalid.audio_panel.com1_load = "missing".into(),
            _ => invalid.audio_panel.com2_load = "missing".into(),
        }
        assert!(validate_installation(&engine, systems.definition.as_ref(), &invalid).is_err());
    }
    assert!(validate_installation(&engine, None, &p).is_err());
}
