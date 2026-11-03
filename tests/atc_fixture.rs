// Real bundled NASR/CIFP/airport records on original flat test terrain; no scenery coverage claim.
#[allow(dead_code)]
fn atc_world() -> flightcraft_world::World {
    use flightcraft_world::*;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/terrain");
    let mut airports: Airports =
        serde_json::from_str(&std::fs::read_to_string(root.join("eastern-pa/airports.json")).unwrap())
            .unwrap();
    let ny: Airports =
        serde_json::from_str(&std::fs::read_to_string(root.join("new-york/airports.json")).unwrap()).unwrap();
    for a in ny.airports {
        if !airports.airports.iter().any(|v| v.ident == a.ident) {
            airports.airports.push(a);
        }
    }
    airports.airports.retain(|a| {
        ["KABE", "KMMU", "KEWR"].contains(&a.ident.as_str())
            || a.icao
                .as_deref()
                .is_some_and(|id| ["KABE", "KMMU", "KEWR"].contains(&id))
    });
    let mut nav: Navdata =
        serde_json::from_str(&std::fs::read_to_string(root.join("eastern-pa/navdata.json")).unwrap())
            .unwrap();
    let ny: Navdata =
        serde_json::from_str(&std::fs::read_to_string(root.join("new-york/navdata.json")).unwrap()).unwrap();
    for s in ny.comms {
        if !nav.comms.iter().any(|v| v.id == s.id) {
            nav.comms.push(s);
        }
    }
    let procedures: Procedures =
        serde_json::from_str(&std::fs::read_to_string(root.join("eastern-pa/procedures.json")).unwrap())
            .unwrap();
    let airspace: AirspaceFile =
        serde_json::from_str(&std::fs::read_to_string(root.join("eastern-pa/airspace.json")).unwrap())
            .unwrap();
    let region = Region {
        id: "atc-fixture".into(),
        name: "ATC training fixture: original flat terrain".into(),
        origin: RegionOrigin {
            latitude_deg: 40.65,
            longitude_deg: -75.44,
            field_elevation_m: 0.,
            runway_heading_deg: 60.,
            runway_length_m: 2300.,
            runway_width_m: 45.,
        },
        southwest_m: [-128000., -128000.],
        tiles_north: 1,
        tiles_east: 1,
        spacing_m: 1000.,
        tile_size: 257,
        runway_blend_m: 100.,
        source: "Original flat test terrain".into(),
        licence: "Apache-2.0".into(),
        attribution: "Original test fixture".into(),
        tiles: vec!["000.fcht".into()],
        airports: Some("airports.json".into()),
    };
    let tile = HeightTile::new(
        region.southwest_m,
        1000.,
        257,
        vec![0.; 257 * 257],
        vec![Surface::Grass; 256 * 256],
    )
    .unwrap();
    World::from_region(region, vec![tile])
        .unwrap()
        .with_airports(airports)
        .unwrap()
        .with_navdata(nav)
        .unwrap()
        .with_procedures(procedures)
        .unwrap()
        .with_airspace(airspace)
        .unwrap()
}
#[allow(dead_code)]
fn atc_session(airport: &str) -> flightcraft_control::Session {
    use flightcraft_control::Session;
    let mut s = Session::new().unwrap();
    s.install_region(atc_world()).unwrap();
    let runway = if airport == "KABE" {
        "06"
    } else if airport == "KEWR" {
        "04L"
    } else {
        "05"
    };
    s.execute("sim.reset",serde_json::json!({"world":"atc-fixture","start":{"kind":"airport_runway","airport":airport,"end":runway}})).unwrap();
    // Stage the parked aircraft inside the pavement, away from the fixture threshold boundary.
    let mut pose = *s.sim().state();
    let heading = s.sim().readout().heading_grid_rad;
    pose.position_m[0] += 25. * heading.cos();
    pose.position_m[1] += 25. * heading.sin();
    let [n, e, _] = pose.position_m;
    pose.position_m[2] = -(s.sim().world().ground(n, e).height_msl_m + 1.0);
    let staged =
        flightcraft_sim::Sim::from_state_in_world(pose, s.sim().controls(), s.sim().world().clone()).unwrap();
    *s.sim_mut() = staged;
    assert!(
        !s.sim().readout().crashed,
        "fixture parked start must be clear of terrain"
    );
    s.execute(
        "comms.callsign",
        serde_json::json!({"registration":"N371FC","spoken_type":"Sling"}),
    )
    .unwrap();
    s.execute("sim.set_controls", serde_json::json!({"throttle":0.,"brake":1.}))
        .unwrap();
    s.run(flightcraft_control::Step { steps: 120 }).unwrap();
    assert!(
        !s.sim().readout().crashed,
        "parked fixture {:?}",
        s.sim().readout()
    );
    s.execute(
        "atc.configure",
        serde_json::json!({"airport":airport,"destination":if airport=="KABE" {"KMMU"} else {"KABE"}}),
    )
    .unwrap();
    s
}
