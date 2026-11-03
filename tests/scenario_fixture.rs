// Real FAA catalogues on original, deliberately flat test terrain. Never navigation scenery.
#[allow(dead_code)]
fn scenario_fixture() -> flightcraft_world::World {
    use flightcraft_world::*;
    let mut r:Region=serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"),"/../../assets/terrain/eastern-pa/region.json"))).unwrap();
    r.id="scenario-fixture".into(); r.name="Original scenario test terrain".into();
    r.southwest_m=[-100000.,-110000.]; r.tiles_north=1; r.tiles_east=1; r.spacing_m=1000.; r.tile_size=257; r.tiles=vec!["000.fcht".into()];
    r.source="Original flat fixture".into();r.licence="Apache-2.0".into();r.attribution="Original test terrain; FAA catalogues, public domain".into();
    let tile=HeightTile::new(r.southwest_m,1000.,257,vec![100.;257*257],vec![Surface::Grass;256*256]).unwrap();
    World::from_region(r,vec![tile]).unwrap()
        .with_airports(serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"),"/../../assets/terrain/eastern-pa/airports.json"))).unwrap()).unwrap()
        .with_navdata(Navdata::from_json(include_str!(concat!(env!("CARGO_MANIFEST_DIR"),"/../../assets/terrain/eastern-pa/navdata.json"))).unwrap()).unwrap()
        .with_procedures(Procedures::from_json(include_str!(concat!(env!("CARGO_MANIFEST_DIR"),"/../../assets/terrain/eastern-pa/procedures.json"))).unwrap()).unwrap()
        .with_airspace(AirspaceFile::from_json(include_str!(concat!(env!("CARGO_MANIFEST_DIR"),"/../../assets/terrain/eastern-pa/airspace.json"))).unwrap()).unwrap()
}
#[allow(dead_code)]
fn scenario_session() -> flightcraft_control::Session {
    let mut s=flightcraft_control::Session::new().unwrap();
    s.install_region(scenario_fixture()).unwrap();
    s.execute("sim.reset",serde_json::json!({"world":"scenario-fixture","start":{"kind":"air","height_m":1200.,"airspeed_mps":33.,"heading_rad":0.}})).unwrap();
    s
}
