#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
#[cfg(test)]
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/region_data.rs"));
use flightcraft_control::{Session, read_region};
use flightcraft_world::{Surface, geo::LocalProjection};
use serde_json::json;
#[test]
fn bundled_new_york_geography_airports_starts_and_commands() {
    require_region_data!();
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/terrain/new-york");
    let world = read_region(&directory).unwrap();
    let region = world.region().unwrap();
    assert_eq!(region.id, "new-york");
    assert_eq!(world.tiles().len(), 88);
    assert_eq!(region.spacing_m, 60.0);
    assert_eq!(region.tile_size, 257);
    let p = LocalProjection::new(region.origin.latitude_deg, region.origin.longitude_deg).unwrap();
    for lat in [40.25, 41.30] {
        for lon in [-75.20, -73.25] {
            flightcraft_world::inside(region, p.project(lat, lon).unwrap()).unwrap();
        }
    }
    let catalogue = world.airports().unwrap();
    assert!(catalogue.attribution.contains("FAA"));
    for ident in ["1N7", "JFK", "EWR", "LGA", "TEB", "CDW", "MMU"] {
        assert!(catalogue.airports.iter().any(|a| a.ident == ident), "{ident}");
    }
    for (ident, feet) in [
        ("1N7", 371.5),
        ("JFK", 13.0),
        ("LGA", 21.0),
        ("EWR", 18.0),
        ("TEB", 8.0),
    ] {
        let airport = catalogue.airports.iter().find(|a| a.ident == ident).unwrap();
        assert!((airport.elevation_m - feet * 0.3048).abs() < 0.2);
        for runway in &airport.runways {
            for end in &runway.ends {
                let [n, e] = p.project(end.latitude_deg, end.longitude_deg).unwrap();
                assert!(
                    (world.ground(n, e).height_msl_m - feet * 0.3048).abs() < 10.0,
                    "{ident} {}",
                    end.ident
                );
            }
        }
    }
    let jfk = catalogue.airports.iter().find(|a| a.ident == "JFK").unwrap();
    assert_eq!(
        jfk.runways
            .iter()
            .map(|r| r.designator.as_str())
            .collect::<Vec<_>>(),
        ["04L/22R", "04R/22L", "13L/31R", "13R/31L"]
    );
    assert!((jfk.runways[3].length_m - 4423.0).abs() < 1.0);
    let blairstown = catalogue.airports.iter().find(|a| a.ident == "1N7").unwrap();
    let threshold = blairstown.runways[0].ends[0].clone();
    assert_eq!(threshold.ident, "07");
    assert!((threshold.elevation_m - 113.0).abs() < 3.0);
    assert_eq!(threshold.elevation_m, region.origin.field_elevation_m);
    let [n, e] = p
        .project(threshold.latitude_deg, threshold.longitude_deg)
        .unwrap();
    assert!(libm::hypot(n, e) < 0.001);
    assert!((threshold.heading_deg - region.origin.runway_heading_deg).abs() < 0.001);
    // A high point on Kittatinny Ridge, northwest of Blairstown.
    let [n, e] = p.project(40.982, -75.088).unwrap();
    assert!(world.ground(n, e).height_msl_m > 400.0);
    let mut session = Session::new().unwrap();
    assert!(
        session
            .execute("world.geodetic", json!({"north_m":0,"east_m":0}))
            .is_err()
    );
    session.install_region(world).unwrap();
    let state = session
        .execute("sim.reset", json!({"world":"new-york","start":{"kind":"runway"}}))
        .unwrap();
    assert!((state["latitude_deg"].as_f64().unwrap() - threshold.latitude_deg).abs() < 1e-8);
    assert!(state["readout"]["longitude_deg"].is_number());
    for (name, input) in [
        ("world.airports", json!({"nearest_to_m":[0,0]})),
        ("world.geodetic", json!({"north_m":0,"east_m":0})),
        (
            "world.geodetic",
            json!({"latitude_deg":40.7,"longitude_deg":-74.0}),
        ),
    ] {
        let command = Session::commands().into_iter().find(|c| c.name == name).unwrap();
        assert!(
            jsonschema::validator_for(&command.input_schema)
                .unwrap()
                .is_valid(&input)
        );
        let result = session.execute(name, input).unwrap();
        assert!(
            jsonschema::validator_for(&command.output_schema)
                .unwrap()
                .is_valid(&result)
        );
    }
    let coords = session
        .execute(
            "world.geodetic",
            json!({"latitude_deg":40.7,"longitude_deg":-74.0}),
        )
        .unwrap();
    let back = session
        .execute(
            "world.geodetic",
            json!({"north_m":coords["north_m"],"east_m":coords["east_m"]}),
        )
        .unwrap();
    assert!((back["latitude_deg"].as_f64().unwrap() - 40.7).abs() < 1e-9);
    let list = session
        .execute("world.airports", json!({"nearest_to_m":[0,0]}))
        .unwrap();
    assert_eq!(list["airports"][0]["ident"], "1N7");
    for ident in ["1N7", "JFK", "EWR", "LGA", "TEB", "CDW", "MMU"] {
        let state=session.execute("sim.reset",json!({"world":"new-york","start":{"runway":{"airport":ident,"end":match ident {"1N7"=>"07","JFK"|"EWR"=>"04L","LGA"|"CDW"=>"04","TEB"=>"01",_=>"05"}}}})).unwrap();
        let position = session.sim().state().position_m;
        assert_eq!(
            session.sim().world().ground(position[0], position[1]).surface,
            Surface::Pavement
        );
        assert!(state["latitude_deg"].is_number());
    }
    let before = *session.sim().state();
    assert!(
        session
            .execute(
                "sim.reset",
                json!({"world":"new-york","start":{"runway":{"airport":"JFK","end":"99"}}})
            )
            .is_err()
    );
    assert_eq!(*session.sim().state(), before);
    let air=session.execute("sim.reset",json!({"world":"new-york","start":{"air":{"airport":"KJFK","distance_m":5000,"bearing_deg":220,"height_m":600,"airspeed_mps":45,"heading_rad":0.7}}})).unwrap();
    assert!((air["readout"]["altitude_msl_m"].as_f64().unwrap() - (600.0 + 13.0 * 0.3048)).abs() < 1e-8);
    session
        .execute("sim.reset", json!({"start":{"kind":"runway"}}))
        .unwrap();
    let flat = session.execute("sim.state", json!({})).unwrap();
    assert!(flat["latitude_deg"].is_null());
    assert!(flat["readout"]["longitude_deg"].is_null());
}

#[test]
fn bundled_eastern_pennsylvania_airports_elevations_and_shared_budget() {
    require_region_data!();
    let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/terrain");
    let world = read_region(&assets.join("eastern-pa")).unwrap();
    let region = world.region().unwrap();
    assert_eq!(region.name, "Eastern Pennsylvania and New Jersey");
    assert_eq!(world.tiles().len(), 110);
    let p = LocalProjection::new(region.origin.latitude_deg, region.origin.longitude_deg).unwrap();
    for lat in [40.20, 41.50] {
        for lon in [-76.20, -74.30] {
            flightcraft_world::inside(region, p.project(lat, lon).unwrap()).unwrap();
        }
    }
    let airports = world.airports().unwrap();
    for (ident, feet) in [
        ("KABE", 393.7),
        ("KMPO", 1915.2),
        ("KAVP", 961.7),
        ("KRDG", 343.4),
    ] {
        let a = airports
            .airports
            .iter()
            .find(|a| a.icao.as_deref() == Some(ident))
            .unwrap();
        assert!((a.elevation_m - feet * 0.3048).abs() < 0.1);
        let mut highest = f64::NEG_INFINITY;
        for runway in &a.runways {
            for end in &runway.ends {
                let [n, e] = p.project(end.latitude_deg, end.longitude_deg).unwrap();
                let height = world.ground(n, e).height_msl_m;
                highest = highest.max(height);
                if ident == "KMPO" {
                    assert!(height > 550.0);
                }
            }
        }
        // Field elevation is the highest usable landing surface; long, graded runways
        // can have substantially lower thresholds at their opposite ends.
        assert!(
            (highest - feet * 0.3048).abs() < 10.0,
            "{ident}: highest threshold {highest}"
        );
    }

    for ident in ["1N7", "12N", "N05", "N40", "N51", "47N", "39N", "TTN", "FWN"] {
        assert!(airports.airports.iter().any(|a| a.ident == ident), "{ident}");
    }
    let ny = read_region(&assets.join("new-york")).unwrap();
    let ny_origin = &ny.region().unwrap().origin;
    assert_eq!(region.origin.latitude_deg, ny_origin.latitude_deg);
    assert_eq!(region.origin.longitude_deg, ny_origin.longitude_deg);
    assert!(region.tile_bytes() + ny.region().unwrap().tile_bytes() < flightcraft_world::MAX_TERRAIN_BYTES);
    let mut session = Session::new().unwrap();
    session.install_region(world).unwrap();
    session.install_region(ny).unwrap();
    for ident in ["KABE", "KMPO", "KAVP", "KRDG"] {
        let world=session.execute("sim.reset",json!({"world":"eastern-pa","start":{"air":{"airport":ident,"distance_m":0,"bearing_deg":0,"height_m":500,"airspeed_mps":45,"heading_rad":0}}})).unwrap();
        assert!(world["latitude_deg"].is_number());
    }
}
