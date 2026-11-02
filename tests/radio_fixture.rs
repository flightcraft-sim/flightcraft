// Test-only circuit: real FAA 1N7 runway geometry, original flat/ridge terrain and COM frequencies.
// The synthetic frequency is deliberately not asserted to be an operational 1N7 frequency.
#[allow(dead_code)]
fn radio_fixture(ridge: bool) -> flightcraft_world::World {
    use flightcraft_world::*;
    let airports: Airports = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/terrain/new-york/airports.json"
    )))
    .unwrap();
    let airport = airports.airports.into_iter().find(|a| a.ident == "1N7").unwrap();
    let runway = airport.runways.first().unwrap();
    let end = runway.ends.iter().find(|e| e.ident == "25").unwrap();
    let elevation = airport.elevation_m;
    let region = Region {
        id: "radio-fixture".into(),
        name: "1N7 radio fixture · synthetic terrain and COM frequency".into(),
        origin: RegionOrigin {
            latitude_deg: end.latitude_deg,
            longitude_deg: end.longitude_deg,
            field_elevation_m: elevation,
            runway_heading_deg: end.heading_deg,
            runway_length_m: runway.length_m,
            runway_width_m: runway.width_m,
        },
        southwest_m: [-12000.0, -12000.0],
        tiles_north: 1,
        tiles_east: 1,
        spacing_m: 100.0,
        tile_size: 257,
        runway_blend_m: 100.0,
        source: "Original radio test terrain".into(),
        licence: "Apache-2.0".into(),
        attribution: "Original fixture".into(),
        tiles: vec!["000.fcht".into()],
        airports: Some("airports.json".into()),
    };
    let projection = geo::LocalProjection::new(end.latitude_deg, end.longitude_deg).unwrap();
    let other = runway.ends.iter().find(|e| e.ident != end.ident).unwrap();
    let [far_n, far_e] = projection
        .project(other.latitude_deg, other.longitude_deg)
        .unwrap();
    let length = far_n.hypot(far_e);
    let samples = (0..257 * 257)
        .map(|i| {
            let north = -12000.0 + f64::from(i % 257) * 100.0;
            let east = -12000.0 + f64::from(i / 257) * 100.0;
            let along = (north * far_n + east * far_e) / length;
            let apron =
                end.elevation_m + (along / length).clamp(0.0, 1.0) * (other.elevation_m - end.elevation_m);
            if ridge && (north - 4000.0).abs() < 300.0 {
                (apron + 900.0) as f32
            } else {
                apron as f32
            }
        })
        .collect();
    let tile = HeightTile::new(
        region.southwest_m,
        100.0,
        257,
        samples,
        vec![Surface::Grass; 256 * 256],
    )
    .unwrap();
    World::from_region(region, vec![tile])
        .unwrap()
        .with_airports(Airports {
            attribution: airports.attribution,
            airports: vec![airport],
        })
        .unwrap()
}
#[allow(dead_code)]
fn radio_station(kind: flightcraft_world::FacilityKind, channel_khz: u32) -> flightcraft_world::ComStation {
    use flightcraft_world::*;
    ComStation {
        metadata: None,
        id: format!("fixture-{kind:?}"),
        airport: "1N7".into(),
        name: "Blairstown".into(),
        kind,
        channel_khz,
        position: GeoPoint {
            latitude_deg: 40.971133,
            longitude_deg: -74.997504,
        },
        elevation_m: 113.2,
        antenna_agl_m: 10.0,
        range_nm: 25.0,
        source: "Original synthetic test frequency; not operational FAA frequency data".into(),
    }
}
#[allow(dead_code)]
fn radio_session() -> flightcraft_control::Session {
    use flightcraft_control::*;
    let mut s = Session::new().unwrap();
    s.install_region(radio_fixture(false)).unwrap();
    s.execute("sim.reset", serde_json::json!({"world":"radio-fixture","start":{"kind":"airport_runway","airport":"1N7","end":"25"}})).unwrap();
    s.run(CommsFacilities {
        stations: Some(vec![
            radio_station(flightcraft_world::FacilityKind::Ctaf, 122800),
            radio_station(flightcraft_world::FacilityKind::Awos, 123000),
        ]),
    })
    .unwrap();
    s
}
