#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_aero::{AircraftCatalog, DefinitionSource, LayerFile};
use serde_json::{Value, json};

#[test]
fn hostile_systems_definitions_are_rejected() {
    let original: Value =
        serde_json::from_str(include_str!("../../../aircraft/corvid/systems.json")).unwrap();
    for (pointer, value) in [
        ("/definition/electrical/battery_capacity_ah", json!(0)),
        ("/definition/electrical/alternator_voltage", json!(10)),
        ("/definition/electrical/loads/0/breaker_amps", json!(-1)),
        ("/definition/electrical/loads/0/backup_capacity_ah", json!(-1)),
        ("/definition/electrical/loads/0/id", json!("../invalid")),
        ("/definition/tanks/0/unusable_kg", json!(50)),
        ("/definition/tanks/0/capacity_kg", json!(0)),
        ("/definition/tanks/0/gauge_lag_s", json!(0)),
        ("/definition/tanks/1/id", json!("left")),
        ("/definition/tanks/1/wing", json!("left")),
        ("/definition/vacuum/spin_down_s", json!(0)),
        ("/definition/pitot_static/vsi_lag_s", json!(-1)),
        ("/definition/pitot_static/alternate_static_error_pa", json!(10000)),
        ("/definition/engine/start_time_s", json!(0)),
        ("/definition/engine/single_mag_power_fraction", json!(1.1)),
    ] {
        let mut modified = original.clone();
        *modified.pointer_mut(pointer).unwrap() = value;
        assert!(LayerFile::from_json(&modified.to_string()).is_err(), "{pointer}");
    }
    let mut modified = original;
    modified["definition"]["extra"] = json!(1);
    assert!(LayerFile::from_json(&modified.to_string()).is_err());
}

#[test]
fn direct_nonfinite_and_bad_variant_references_are_rejected() {
    let mut catalog = AircraftCatalog::builtins().unwrap();
    let mut resolved = catalog.resolve_aircraft("corvid").unwrap();
    let definition = resolved.installed_systems.definition.as_mut().unwrap();
    definition.electrical.battery_capacity_ah = f64::NAN;
    assert!(definition.validate().is_err());
    let mut variant = resolved.variant;
    variant.identity.id = "bad-capacity".into();
    variant.fuel_capacity_kg = 99.0;
    catalog
        .insert(LayerFile::Variant(variant), DefinitionSource::BuiltIn)
        .unwrap();
    assert!(catalog.validate().is_err());
}
