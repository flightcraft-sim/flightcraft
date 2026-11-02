#![allow(clippy::unwrap_used)]
use flightcraft_control::Session;
use serde_json::json;

#[test]
fn world_choice_height_and_reset_validation() {
    let mut session = Session::new().unwrap();
    let flat = session
        .execute("world.height", json!({"north_m":2200,"east_m":3000}))
        .unwrap();
    assert_eq!(flat["height_msl_m"], 0.0);
    let valley = session
        .execute(
            "sim.reset",
            json!({"world":"training-valley","start":{"kind":"runway"}}),
        )
        .unwrap();
    assert_eq!(valley["world"], "training-valley");
    let hill = session
        .execute("world.height", json!({"north_m":2200,"east_m":3000}))
        .unwrap();
    assert!(hill["height_msl_m"].as_f64().unwrap() > 500.0);
    assert!(hill["normal_ned"].is_array());
    let runway = session
        .execute("world.height", json!({"north_m":450,"east_m":0}))
        .unwrap();
    assert_eq!(runway["surface"], "pavement");
    assert_eq!(runway["height_msl_m"], 0.0);
    let before = *session.sim().state();
    assert!(
        session
            .execute("sim.reset", json!({"world":"unknown","start":{"kind":"runway"}}))
            .is_err()
    );
    assert_eq!(session.sim().state(), &before);
    assert!(
        session
            .execute("world.height", json!({"north_m":1e100,"east_m":0}))
            .is_err()
    );
    session
        .execute("sim.reset", json!({"start":{"kind":"runway"}}))
        .unwrap();
    assert_eq!(session.execute("sim.state", json!({})).unwrap()["world"], "flat");
}

fn synthetic_region() -> (flightcraft_world::Region, Vec<flightcraft_world::HeightTile>) {
    use flightcraft_world::{HeightTile, Region, RegionOrigin, SRTM_ATTRIBUTION, Surface};
    let region = Region {
        id: "test-field".into(),
        name: "Test field".into(),
        origin: RegionOrigin {
            latitude_deg: 37.0,
            longitude_deg: -123.0,
            field_elevation_m: 750.0,
            runway_heading_deg: 90.0,
            runway_length_m: 900.0,
            runway_width_m: 23.0,
        },
        southwest_m: [-2000.0, -2000.0],
        tiles_north: 2,
        tiles_east: 2,
        spacing_m: 31.25,
        tile_size: 65,
        runway_blend_m: 200.0,
        source: "SRTM".into(),
        licence: "public domain".into(),
        attribution: SRTM_ATTRIBUTION.into(),
        airports: None,
        tiles: (0..4).map(|i| format!("{i:03}.fcht")).collect(),
    };
    let mut tiles = Vec::new();
    for e in 0..2 {
        for n in 0..2 {
            tiles.push(
                HeightTile::new(
                    [-2000.0 + 2000.0 * f64::from(n), -2000.0 + 2000.0 * f64::from(e)],
                    31.25,
                    65,
                    vec![750.0; 65 * 65],
                    vec![Surface::Grass; 64 * 64],
                )
                .unwrap(),
            );
        }
    }
    (region, tiles)
}
#[test]
fn installed_region_reset_heading_elevation_attribution_and_atomic_failure() {
    use flightcraft_world::{SRTM_ATTRIBUTION, World};
    let mut session = Session::new().unwrap();
    let (region, tiles) = synthetic_region();
    let world = World::from_region(region.clone(), tiles.clone()).unwrap();
    session.install_region(world.clone()).unwrap();
    assert!(session.install_region(world).is_err());
    let choices = session.execute("world.list", json!({})).unwrap();
    assert_eq!(choices["worlds"].as_array().unwrap().len(), 3);
    assert_eq!(choices["worlds"][2]["id"], "test-field");
    assert_eq!(choices["worlds"][2]["attribution"], SRTM_ATTRIBUTION);
    let state = session
        .execute(
            "sim.reset",
            json!({"world":"test-field","start":{"kind":"runway"}}),
        )
        .unwrap();
    assert_eq!(state["world"], "test-field");
    let runway = session
        .execute("world.height", json!({"north_m":0,"east_m":450}))
        .unwrap();
    assert_eq!(runway["height_msl_m"], 750.0);
    assert_eq!(runway["surface"], "pavement");
    assert!(session.sim().state().position_m[2] < -750.0);
    assert!((session.sim().readout().heading_rad - std::f64::consts::FRAC_PI_2).abs() < 1e-10);
    assert_eq!(session.sim().world().attribution(), SRTM_ATTRIBUTION);
    let before = *session.sim().state();
    assert!(
        session
            .execute("sim.reset", json!({"world":"absent","start":{"kind":"runway"}}))
            .is_err()
    );
    assert_eq!(session.sim().state(), &before);
    assert!(World::from_region(region, tiles.into_iter().take(3).collect()).is_err());
    session
        .execute("sim.reset", json!({"start":{"kind":"runway"}}))
        .unwrap();
    assert!(session.sim().world().attribution().contains("Apache-2.0"));
}
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn native_regions_require_all_tiles_and_skip_bad_neighbours() {
    let root = std::env::temp_dir().join(format!("flightcraft-regions-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let good = root.join("good");
    let bad = root.join("bad");
    std::fs::create_dir(&good).unwrap();
    std::fs::create_dir(&bad).unwrap();
    let (region, tiles) = synthetic_region();
    let metadata = serde_json::to_string(&region).unwrap();
    std::fs::write(good.join("region.json"), &metadata).unwrap();
    std::fs::write(bad.join("region.json"), &metadata).unwrap();
    assert!(flightcraft_control::read_region(&good).is_err());
    for (name, tile) in region.tiles.iter().zip(tiles) {
        std::fs::write(good.join(name), tile.write()).unwrap();
    }
    let mut session = Session::new().unwrap();
    assert_eq!(session.load_regions_directory(&root).len(), 1);
    assert!(
        session
            .execute(
                "sim.reset",
                json!({"world":"test-field","start":{"kind":"runway"}})
            )
            .is_ok()
    );
    let mut malicious = region.clone();
    malicious.tiles = vec!["../escape.fcht".into()];
    std::fs::write(
        bad.join("region.json"),
        serde_json::to_string(&malicious).unwrap(),
    )
    .unwrap();
    assert!(flightcraft_control::read_region(&bad).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn airspace_commands_state_membership_visibility_and_atomic_sidecar_loading() {
    use flightcraft_control::attach_region_airspace;
    use flightcraft_world::World;
    let text = json!({"attribution":"Original test airspace", "volumes":[{
        "id":"surface-d", "name":"Test field", "class":"D", "kind":"class",
        "floor":{"value_ft":0,"reference":"SFC"},
        "ceiling":{"value_ft":4000,"reference":"MSL"},
        "polygons":[[[[36.99,-123.01],[36.99,-122.99],[37.01,-122.99],[37.01,-123.01]]]]
    }, {
        "id":"upper-b", "name":"Test shelf", "class":"B", "kind":"class",
        "floor":{"value_ft":3000,"reference":"MSL"},
        "ceiling":{"value_ft":7000,"reference":"MSL"},
        "polygons":[[[[36.99,-123.01],[36.99,-122.99],[37.01,-122.99],[37.01,-123.01]]]]
    }]})
    .to_string();
    let (region, tiles) = synthetic_region();
    let bare = World::from_region(region.clone(), tiles.clone()).unwrap();
    let world = attach_region_airspace(bare.clone(), &text).unwrap();
    assert_ne!(bare.fingerprint(), world.fingerprint());
    assert!(attach_region_airspace(bare, "{}").is_err());
    let mut session = Session::new().unwrap();
    assert!(
        session
            .execute("sim.state", json!({}))
            .unwrap()
            .get("airspace")
            .is_none()
    );
    assert!(
        session.execute("airspace.list", json!({})).unwrap()["volumes"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    session.install_region(world).unwrap();
    let reset = session
        .execute(
            "sim.reset",
            json!({"world":"test-field","start":{"kind":"runway"}}),
        )
        .unwrap();
    assert_eq!(reset["airspace"], json!([{"id":"surface-d","class":"D"}]));
    let list = session.execute("airspace.list", json!({})).unwrap();
    assert_eq!(list["volumes"].as_array().unwrap().len(), 2);
    assert!(list["volumes"][0].get("polygons").is_none());
    let schema = schemars::schema_for!(flightcraft_control::AirspaceAt);
    let validator = jsonschema::validator_for(schema.as_value()).unwrap();
    assert!(validator.is_valid(&json!({})));
    assert!(!validator.is_valid(&json!({"north_m":0})));
    assert!(!validator.is_valid(&json!({"north_m":null,"east_m":0,"altitude_msl_m":0})));
    let query = session.execute("airspace.at", json!({})).unwrap();
    assert_eq!(query["containing"][0]["id"], "surface-d");
    assert_eq!(query["nearest"]["id"], "upper-b");
    assert!((query["nearest"]["floor_msl_m"].as_f64().unwrap() - 914.4).abs() < 1e-10);
    let at = session
        .execute(
            "airspace.at",
            json!({"north_m":0,"east_m":0,"altitude_msl_m":1000}),
        )
        .unwrap();
    assert_eq!(at["containing"][0]["class"], "B");
    for input in [
        json!({"north_m":0}),
        json!({"north_m":null}),
        json!({"north_m":0,"east_m":0,"altitude_msl_m":1e100}),
        json!({"unknown":0}),
    ] {
        assert!(session.execute("airspace.at", input).is_err());
    }
    assert!(session.airspace_visible());
    session
        .execute("view.set_airspace", json!({"visible":false}))
        .unwrap();
    assert!(!session.airspace_visible());
    assert!(
        session
            .execute("view.set_airspace", json!({"visible":null}))
            .is_err()
    );
    let root = std::env::temp_dir().join(format!("flightcraft-airspace-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("region.json"), serde_json::to_string(&region).unwrap()).unwrap();
    for (name, tile) in region.tiles.iter().zip(tiles) {
        std::fs::write(root.join(name), tile.write()).unwrap();
    }
    assert!(
        flightcraft_control::read_region(&root)
            .unwrap()
            .airspace()
            .is_none()
    );
    std::fs::write(root.join("airspace.json"), &text).unwrap();
    assert!(
        flightcraft_control::read_region(&root)
            .unwrap()
            .airspace()
            .is_some()
    );
    std::fs::write(root.join("airspace.json"), "{}").unwrap();
    assert!(flightcraft_control::read_region(&root).is_err());
    std::fs::remove_dir_all(root).unwrap();
}
