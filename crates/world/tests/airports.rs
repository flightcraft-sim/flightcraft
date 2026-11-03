#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_world::{
    Airport, AirportRunway, Airports, Region, RunwayEnd, RunwaySurface, Surface, World, geo::LocalProjection,
};
use serde_json::json;
fn region() -> Region {
    Region::from_json(&json!({"id":"airport-test","name":"Airport test","origin":{"latitude_deg":40.0,"longitude_deg":-75.0,"field_elevation_m":100,"runway_heading_deg":0,"runway_length_m":1000,"runway_width_m":20},"southwest_m":[-2000,-2000],"tiles_north":1,"tiles_east":1,"tile_size":65,"spacing_m":100,"runway_blend_m":200,"source":"Test","licence":"public domain","attribution":"Test","tiles":["000.fcht"],"airports":"airports.json"}).to_string()).unwrap()
}
fn catalogue(r: &Region) -> Airports {
    let p = LocalProjection::new(r.origin.latitude_deg, r.origin.longitude_deg).unwrap();
    let end = |ident: &str, n, e, height, heading| {
        let [lat, lon] = p.unproject(n, e).unwrap();
        RunwayEnd {
            lighting: None,
            ident: ident.into(),
            latitude_deg: lat,
            longitude_deg: lon,
            elevation_m: height,
            heading_deg: heading,
        }
    };
    Airports {
        attribution: "Original test data".into(),
        airports: vec![Airport {
            ident: "TEST".into(),
            icao: None,
            name: "Test Field".into(),
            traffic_pattern_altitude_ft: None,
            elevation_m: 100.0,
            latitude_deg: 40.0,
            longitude_deg: -75.0,
            runways: vec![
                AirportRunway {
                    designator: "18/36".into(),
                    length_m: 1000.0,
                    width_m: 20.0,
                    surface: RunwaySurface::Asphalt,
                    ends: [
                        end("18", 1000.0, 0.0, 110.0, 180.0),
                        end("36", 0.0, 0.0, 100.0, 0.0),
                    ],
                },
                AirportRunway {
                    designator: "09/27".into(),
                    length_m: 1000.0,
                    width_m: 20.0,
                    surface: RunwaySurface::Turf,
                    ends: [
                        end("09", 500.0, -500.0, 105.0, 90.0),
                        end("27", 500.0, 500.0, 105.0, 270.0),
                    ],
                },
            ],
        }],
    }
}
#[test]
fn sloped_runways_crossing_pavement_and_smooth_apron() {
    let r = region();
    let c = catalogue(&r);
    let tile = flightcraft_world::HeightTile::new(
        [-2000.0, -2000.0],
        100.0,
        65,
        vec![200.0; 65 * 65],
        vec![Surface::Grass; 64 * 64],
    )
    .unwrap();
    let world = World::from_region(r, vec![tile])
        .unwrap()
        .with_airports(c)
        .unwrap();
    for n in [1.0, 100.0, 500.0, 999.0] {
        let g = world.ground(n, 0.0);
        assert_eq!(g.surface, Surface::Pavement);
        assert!((g.height_msl_m - (100.0 + n * 0.01)).abs() < 1e-7);
        assert!((g.normal_ned[0] + 0.01 / libm::sqrt(1.0001)).abs() < 1e-9);
    }
    assert_eq!(world.ground(500.0, 100.0).surface, Surface::Grass);
    assert!((world.ground(500.0, 100.0).height_msl_m - 105.0).abs() < 1e-7);
    // Both runway planes agree at their crossing, including the prepared apron.
    assert!((world.prepared_airport_ground(500.0, 0.0).height_msl_m - 105.0).abs() < 1e-7);
    let at = world.prepared_airport_ground(250.0, 210.0).height_msl_m;
    let near = world.prepared_airport_ground(250.0, 210.01).height_msl_m;
    assert!((at - near).abs() < 1e-5);
    assert!((world.prepared_airport_ground(-1000.0, 1000.0).height_msl_m - 200.0).abs() < 1e-6);
}
#[test]
fn hostile_catalogues_are_errors() {
    let r = region();
    let valid = catalogue(&r);
    valid.validate(&r).unwrap();
    for mutation in 0..8 {
        let mut c = valid.clone();
        match mutation {
            0 => c.airports[0].latitude_deg = f64::NAN,
            1 => c.airports[0].runways[0].width_m = -1.0,
            2 => c.airports[0].runways[0].ends[0].heading_deg = 360.0,
            3 => c.airports[0].runways[0].ends[0].elevation_m = f64::INFINITY,
            4 => c.airports[0].runways[0].ends[0].longitude_deg = -80.0,
            5 => c.airports[0].runways[0].length_m = 100.0,
            6 => c.airports.push(c.airports[0].clone()),
            _ => c.airports[0].runways = vec![c.airports[0].runways[0].clone(); 2001],
        }
        assert!(c.validate(&r).is_err());
    }
    assert!(Airports::from_json(b"{}", &r).is_err());
    let mut region = r;
    region.airports = Some("../airports.json".into());
    assert!(region.validate().is_err());
}
