#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_aero::{
    Aircraft, AircraftCatalog, DefinitionSource, FileKind, LayerFile, Load, MAX_AIRCRAFT_BYTES,
    MAX_PACK_TEXT_BYTES, PackManifest, StationLoad, migrate_v1,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn demo() -> (PackManifest, BTreeMap<String, String>) {
    let manifest =
        PackManifest::from_json(include_str!("../../../examples/packs/demo-pack/pack.json")).unwrap();
    let files = [
        (
            "variant.json",
            include_str!("../../../examples/packs/demo-pack/variant.json"),
        ),
        (
            "engine.json",
            include_str!("../../../examples/packs/demo-pack/engine.json"),
        ),
        (
            "panel.json",
            include_str!("../../../examples/packs/demo-pack/panel.json"),
        ),
        (
            "aircraft.json",
            include_str!("../../../examples/packs/demo-pack/aircraft.json"),
        ),
    ]
    .into_iter()
    .map(|(p, text)| (p.into(), text.into()))
    .collect();
    (manifest, files)
}

#[test]
fn migration_round_trip_preserves_both_flight_models() {
    for text in [flightcraft_aero::CORVID_JSON, flightcraft_aero::LARROW_JSON] {
        let original = Aircraft::from_json(text).unwrap();
        let migrated = migrate_v1(text).unwrap();
        let mut catalog = AircraftCatalog::default();
        for file in migrated {
            let encoded = serde_json::to_string(&file).unwrap();
            let decoded = LayerFile::from_json(&encoded).unwrap();
            assert_eq!(file, decoded);
            catalog.insert(decoded, DefinitionSource::BuiltIn).unwrap();
        }
        catalog.validate().unwrap();
        assert_eq!(
            catalog.resolve_aircraft(&original.identity.id).unwrap().aircraft,
            original
        );
    }
}

#[test]
fn builtins_resolve_to_the_original_exact_mass_geometry_and_tables() {
    for (text, resolved) in [
        (flightcraft_aero::CORVID_JSON, Aircraft::corvid().unwrap()),
        (flightcraft_aero::LARROW_JSON, Aircraft::larrow().unwrap()),
    ] {
        assert_eq!(resolved, Aircraft::from_json(text).unwrap());
    }
    let catalog = AircraftCatalog::builtins().unwrap();
    let a = catalog.resolve_aircraft("corvid").unwrap();
    assert!(a.weight_balance.in_limits);
    assert_eq!(a.panel.instruments.len(), 19);
    assert_eq!(a.load.stations.len(), 3);
    let reference = include_str!("../../../aircraft/corvid/aircraft.json");
    assert_eq!(Aircraft::from_json(reference).unwrap(), a.aircraft);
}

#[test]
fn demo_pack_composes_with_builtins_and_has_independent_equipment() {
    let (manifest, files) = demo();
    let mut catalog = AircraftCatalog::builtins().unwrap();
    catalog.install_pack(&manifest, &files).unwrap();
    let a = catalog.resolve_aircraft("corvid-tourer").unwrap();
    assert_eq!(a.aircraft.geometry, Aircraft::corvid().unwrap().geometry);
    assert_eq!(a.aircraft.propulsion.max_power_w, 135_000.0);
    assert_eq!(a.engine.induction, flightcraft_aero::Induction::Injection);
    assert_eq!(a.panel.instruments.len(), 3);
    assert!(a.weight_balance.in_limits);
    assert_eq!(
        catalog.source(FileKind::Aircraft, "corvid-tourer"),
        Some(&DefinitionSource::Pack {
            id: "demo-pack".into()
        })
    );
}

fn sample_variant() -> flightcraft_aero::Variant {
    let mut v = AircraftCatalog::builtins()
        .unwrap()
        .variant("corvid-standard")
        .unwrap()
        .clone();
    v.empty_mass.mass_kg = 600.0;
    v.empty_mass.cg_m = [1.0, 0.0, 0.2];
    v.min_mass_kg = 600.0;
    v.max_mass_kg = 1000.0;
    v.cg_envelope = vec![[0.8, 600.0], [1.6, 600.0], [1.5, 1000.0], [1.0, 1000.0]];
    for (s, p) in v.stations.iter_mut().zip([
        [2.0, -0.5, 0.0],
        [2.0, 0.5, 0.0],
        [4.0, 0.0, 0.0],
        [1.0, 0.0, 0.2],
    ]) {
        s.position_m = p;
    }
    v
}
fn sample_load() -> Load {
    Load {
        stations: [
            ("pilot", 80.0),
            ("passenger", 60.0),
            ("baggage", 20.0),
            ("fuel", 40.0),
        ]
        .into_iter()
        .map(|(station, mass_kg)| StationLoad {
            station: station.into(),
            mass_kg,
        })
        .collect(),
    }
}

#[test]
fn weight_and_balance_matches_hand_computed_moments() {
    let balance = sample_variant().weight_balance(&sample_load()).unwrap();
    // 800 kg; moments (1000, -10, 128) kg m; g = 9.80665 m/s².
    assert_eq!(balance.mass_kg, 800.0);
    assert_eq!(balance.cg_m, [1.25, -0.0125, 0.16]);
    assert!((balance.weight_n - 7845.32).abs() < 1e-9);
    assert!(balance.in_limits);
}

#[test]
fn polygon_edges_are_inclusive_and_outside_cg_and_weight_are_reported() {
    let mut variant = sample_variant();
    let mut load = sample_load();
    variant.cg_envelope = vec![[1.0, 600.0], [1.25, 600.0], [1.25, 1000.0], [1.0, 1000.0]];
    assert!(variant.weight_balance(&load).unwrap().in_limits); // right edge
    variant.stations[2].position_m[0] = 5.0;
    let result = variant.weight_balance(&load).unwrap();
    assert!(!result.in_limits);
    assert_eq!(result.violations, vec!["CG outside envelope"]);
    load.stations[2].mass_kg = 500.0;
    let result = variant.weight_balance(&load).unwrap();
    assert!(result.violations.iter().any(|s| s.contains("station baggage")));
    assert!(result.violations.iter().any(|s| s.contains("mass outside")));
    load.stations[3].mass_kg = 200.0;
    assert!(
        variant
            .weight_balance(&load)
            .unwrap()
            .violations
            .iter()
            .any(|s| s.contains("fuel capacity"))
    );
    load.stations[0].mass_kg = f64::NAN;
    assert!(variant.weight_balance(&load).is_err());
}

#[test]
fn loads_reject_unknown_duplicate_negative_and_oversized_station_lists() {
    let variant = sample_variant();
    let mut load = sample_load();
    load.stations[0].station = "missing".into();
    assert!(variant.weight_balance(&load).is_err());
    load = sample_load();
    load.stations.push(load.stations[0].clone());
    assert!(variant.weight_balance(&load).is_err());
    load = sample_load();
    load.stations[0].mass_kg = -1.0;
    assert!(variant.weight_balance(&load).is_err());
    load.stations = vec![load.stations[0].clone(); 65];
    assert!(variant.weight_balance(&load).is_err());
}

#[test]
fn hostile_manifests_reject_traversal_unknown_kinds_and_missing_metadata() {
    let (manifest, _) = demo();
    let original = serde_json::to_value(manifest).unwrap();
    for path in [
        "../escape.json",
        "/tmp/out.json",
        "a/../b.json",
        "a//b.json",
        "a\\b.json",
        "https://host/a.json",
        "C:bad.json",
        ".hidden.json",
        "a/./b.json",
        "pack.json",
        "a.js",
        "",
    ] {
        let mut bad = original.clone();
        bad["files"][0]["path"] = json!(path);
        assert!(PackManifest::from_json(&bad.to_string()).is_err(), "{path}");
    }
    for field in ["id", "name", "version", "licence", "affiliation"] {
        let mut bad = original.clone();
        bad[field] = json!("");
        assert!(PackManifest::from_json(&bad.to_string()).is_err(), "{field}");
    }
    let mut bad = original.clone();
    bad.as_object_mut().unwrap().remove("affiliation");
    assert!(PackManifest::from_json(&bad.to_string()).is_err());
    let mut bad = original.clone();
    bad["files"][0]["kind"] = json!("executable");
    assert!(PackManifest::from_json(&bad.to_string()).is_err());
    let mut bad = original.clone();
    bad["files"][1]["path"] = bad["files"][0]["path"].clone();
    assert!(PackManifest::from_json(&bad.to_string()).is_err());
    let mut bad = original;
    bad["surprise"] = json!(true);
    assert!(PackManifest::from_json(&bad.to_string()).is_err());
    assert!(PackManifest::from_json(&" ".repeat(flightcraft_aero::MAX_MANIFEST_BYTES + 1)).is_err());
}

#[test]
fn hostile_pack_files_are_rejected_atomically_even_when_unused() {
    let (manifest, files) = demo();
    let mutate = |name: &str, pointer: &str, replacement: Value| {
        let mut files = files.clone();
        let mut value: Value = serde_json::from_str(&files[name]).unwrap();
        *value.pointer_mut(pointer).unwrap() = replacement;
        files.insert(name.into(), value.to_string());
        files
    };
    let cases = vec![
        mutate("variant.json", "/engine", json!("missing")),
        mutate("variant.json", "/format_version", json!(999)),
        mutate(
            "variant.json",
            "/empty_mass/inertia_kg_m2",
            json!([[1, 2, 0], [2, 1, 0], [0, 0, 1]]),
        ),
        mutate(
            "variant.json",
            "/cg_envelope",
            json!([[0, 500], [1, 1200], [0, 1200], [1, 500]]),
        ),
        mutate("variant.json", "/flap_stages/0/max_speed_mps", json!(-1)),
        mutate("engine.json", "/fuel_flow_kg_s/points", json!([[0, 0], [1, -1]])),
        mutate("engine.json", "/propulsion/max_power_w", json!(-1)),
        mutate("panel.json", "/instruments/0/module", json!("run_script")),
        mutate("panel.json", "/instruments/0/size", json!([-1, 1])),
        mutate("aircraft.json", "/load/stations/0/station", json!("missing")),
        mutate("aircraft.json", "/identity/id", json!("corvid")),
    ];
    for files in cases {
        let mut catalog = AircraftCatalog::builtins().unwrap();
        let builtin_count = catalog.aircraft_definitions().count();
        assert!(catalog.install_pack(&manifest, &files).is_err());
        assert_eq!(catalog.aircraft_definitions().count(), builtin_count);
        assert!(catalog.get(FileKind::Engine, "tourer-injection").is_err());
    }
    let mut catalog = AircraftCatalog::builtins().unwrap();
    let mut bad = files.clone();
    bad.insert("extra.json".into(), "{}".into());
    assert!(catalog.install_pack(&manifest, &bad).is_err());
    bad = files.clone();
    bad.remove("engine.json");
    assert!(catalog.install_pack(&manifest, &bad).is_err());
    bad = files.clone();
    bad.insert("engine.json".into(), " ".repeat(MAX_AIRCRAFT_BYTES + 1));
    assert!(catalog.install_pack(&manifest, &bad).is_err());
    let mut wrong_kind = manifest.clone();
    wrong_kind.files[0].kind = FileKind::Engine;
    assert!(catalog.install_pack(&wrong_kind, &files).is_err());
    // Bounded total size is checked before any JSON decoding.
    let mut large = manifest.clone();
    large.files.clear();
    let mut large_files = BTreeMap::new();
    for n in 0..MAX_PACK_TEXT_BYTES / MAX_AIRCRAFT_BYTES + 1 {
        let path = format!("file-{n}.json");
        large.files.push(flightcraft_aero::PackFile {
            path: path.clone(),
            kind: FileKind::Panel,
        });
        large_files.insert(path, " ".repeat(MAX_AIRCRAFT_BYTES));
    }
    assert_eq!(
        catalog.install_pack(&large, &large_files).unwrap_err().field,
        "pack_size"
    );
    catalog.install_pack(&manifest, &files).unwrap();
    assert!(catalog.install_pack(&manifest, &files).is_err());
}

#[test]
fn loaded_station_mass_updates_inertia_about_the_loaded_cg() {
    let catalog = AircraftCatalog::builtins().unwrap();
    let mut v = catalog.variant("corvid-standard").unwrap().clone();
    v.identity.id = "inertia-test".into();
    v.empty_mass.mass_kg = 800.0;
    v.empty_mass.cg_m = [0.0; 3];
    v.stations[0].position_m = [2.0, 0.0, 0.0];
    let empty_inertia = v.empty_mass.inertia_kg_m2;
    let mut catalog = catalog;
    catalog
        .insert(LayerFile::Variant(v), DefinitionSource::BuiltIn)
        .unwrap();
    let load = Load {
        stations: vec![StationLoad {
            station: "pilot".into(),
            mass_kg: 200.0,
        }],
    };
    let resolved = catalog.resolve("inertia-test", "corvid-six", &load).unwrap();
    assert_eq!(resolved.aircraft.mass.cg_m, [0.4, 0.0, 0.0]);
    // 800 * 0.4² + 200 * 1.6² = 640 kg m², added to pitch and yaw inertia.
    for (i, empty_row) in empty_inertia.iter().enumerate() {
        assert!(
            (resolved.aircraft.mass.inertia_kg_m2[i][i] - empty_row[i] - if i == 0 { 0.0 } else { 640.0 })
                .abs()
                < 1e-9
        );
    }
}

#[test]
fn concave_envelopes_work_in_both_vertex_orders() {
    let mut variant = sample_variant();
    variant.cg_envelope = vec![
        [0.0, 600.0],
        [2.0, 600.0],
        [2.0, 1000.0],
        [1.0, 1000.0],
        [1.0, 800.0],
        [0.0, 800.0],
    ];
    variant.empty_mass.mass_kg = 900.0;
    variant.empty_mass.cg_m = [0.5, 0.0, 0.0];
    assert!(!variant.weight_balance(&Load::default()).unwrap().in_limits);
    variant.empty_mass.cg_m = [1.5, 0.0, 0.0];
    assert!(variant.weight_balance(&Load::default()).unwrap().in_limits);
    variant.cg_envelope.reverse();
    assert!(variant.weight_balance(&Load::default()).unwrap().in_limits);
}

#[test]
fn unused_parts_and_oversized_typed_manifests_cannot_bypass_validation() {
    let (manifest, mut files) = demo();
    let mut variant: Value = serde_json::from_str(&files["variant.json"]).unwrap();
    variant["engine"] = json!("corvid-engine");
    files.insert("variant.json".into(), variant.to_string());
    let mut engine: Value = serde_json::from_str(&files["engine.json"]).unwrap();
    engine["propulsion"]["max_power_w"] = json!(-1.0);
    files.insert("engine.json".into(), engine.to_string());
    let mut catalog = AircraftCatalog::builtins().unwrap();
    assert!(catalog.install_pack(&manifest, &files).is_err());
    let mut huge = manifest;
    huge.authors = vec!["a".repeat(4096); 32];
    assert_eq!(
        catalog.install_pack(&huge, &files).unwrap_err().field,
        "pack.json"
    );
}

#[test]
fn built_in_station_arms_change_cg_and_default_mass_properties_preserve_every_bit() {
    let catalog = AircraftCatalog::builtins().unwrap();
    for text in [flightcraft_aero::CORVID_JSON, flightcraft_aero::LARROW_JSON] {
        let old = Aircraft::from_json(text).unwrap();
        let new = catalog.resolve_aircraft(&old.identity.id).unwrap().aircraft;
        assert_eq!(new.mass.mass_kg.to_bits(), old.mass.mass_kg.to_bits());
        for (a, b) in new.mass.cg_m.iter().zip(old.mass.cg_m) {
            assert_eq!(a.to_bits(), b.to_bits());
        }
        for (a, b) in new
            .mass
            .inertia_kg_m2
            .iter()
            .flatten()
            .zip(old.mass.inertia_kg_m2.iter().flatten())
        {
            assert_eq!(a.to_bits(), b.to_bits());
        }
    }
    let v = catalog.variant("corvid-standard").unwrap();
    let pilot_only = Load {
        stations: vec![StationLoad {
            station: "pilot".into(),
            mass_kg: 80.0,
        }],
    };
    let balance = v.weight_balance(&pilot_only).unwrap();
    assert_eq!(balance.cg_m, [-25.0 / 880.0, -20.0 / 880.0, 0.0]);
    assert!(balance.in_limits);
}

#[test]
fn exterior_manifest_version_and_installation_are_transactional() {
    let (mut manifest, files) = demo();
    let mut definition: flightcraft_aero::exterior::Exterior =
        serde_json::from_str(include_str!("../../../aircraft/corvid/exterior.json")).unwrap();
    definition.aircraft = "corvid-tourer".into();
    definition.registration.text = LayerFile::from_json(&files["aircraft.json"])
        .unwrap()
        .identity()
        .registration
        .clone();
    manifest.exterior = vec![definition];
    assert!(
        manifest.validate().is_err(),
        "legacy manifests must reject exteriors"
    );
    manifest.format_version = 2;
    assert!(manifest.validate().is_ok());
    let assets = BTreeMap::from([
        (
            "exterior.glb".into(),
            include_bytes!("../../../aircraft/corvid/exterior.glb").to_vec(),
        ),
        (
            "exterior-lod1.glb".into(),
            include_bytes!("../../../aircraft/corvid/exterior-lod1.glb").to_vec(),
        ),
    ]);
    let mut catalog = AircraftCatalog::builtins().unwrap();
    let mut bad = assets.clone();
    bad.insert("exterior-lod1.glb".into(), vec![0; 100]);
    assert!(catalog.install_pack_with_assets(&manifest, &files, &bad).is_err());
    assert!(catalog.resolve_aircraft("corvid-tourer").is_err());
    assert!(catalog.exterior("corvid-tourer").is_none());
    catalog
        .install_pack_with_assets(&manifest, &files, &assets)
        .unwrap();
    assert!(catalog.exterior("corvid-tourer").is_some());
    let mut future = manifest.clone();
    future.format_version = 4;
    assert!(future.validate().is_err());
    let mut traversal = manifest;
    traversal.exterior[0].model = "../escape.glb".into();
    assert!(traversal.validate().is_err());
}
