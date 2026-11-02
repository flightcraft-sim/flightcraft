#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
#[cfg(test)]
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/region_data.rs"));
use flightcraft_control::{
    FlightPlan, PinsFile, PlanFormat, Session, export_plan, import_plan, parse_coordinates,
};
use serde_json::{Value, json};
fn point(ident: &str, lat: f64, lon: f64, kind: &str) -> Value {
    json!({"waypoint":{"ident":ident,"kind":kind,"position":{"latitude_deg":lat,"longitude_deg":lon}},"altitude_m":1000.0})
}
#[test]
fn coordinate_forms_and_hostile_parser() {
    for input in [
        "40.9711111111 -74.9975",
        "40 58.2666666667N 074 59.85W",
        "N40°58'16\" W74°59'51\"",
        "074 59.85W 40 58.2666666667N",
    ] {
        let p = parse_coordinates(input).unwrap().unwrap();
        assert!((p.latitude_deg - 40.9711111111).abs() < 1e-8, "{input}");
        assert!((p.longitude_deg + 74.9975).abs() < 1e-8);
    }
    for input in [
        "91,0",
        "0,181",
        "40 60N 75 0W",
        "N-40 W75",
        "N40 59 60 W75 0 0",
        "N40 1.1 2 W75 0 0",
    ] {
        assert!(parse_coordinates(input).is_err(), "{input}");
    }
    // Bounded, deterministic arbitrary Unicode and numeric strings, including invalid signs and exponents.
    let alphabet = [
        '\0', '\n', 'N', 'S', 'E', 'W', '0', '9', '-', '+', '.', ',', '/', '°', '\'', '"', '🛩', 'é', ' ',
    ];
    let mut seed = 17u64;
    for len in 0..600 {
        let mut text = String::new();
        for _ in 0..len {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            text.push(alphabet[(seed % alphabet.len() as u64) as usize]);
        }
        if let Ok(Some(p)) = parse_coordinates(&text) {
            p.validate().unwrap();
        }
    }
}
#[test]
fn pins_are_atomic_bounded_and_persisted() {
    let dir = std::env::temp_dir().join(format!("flightcraft-pins-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = Session::new().unwrap();
    s.load_navigation_storage(&dir).unwrap();
    let pin = json!({"id":"example","name":"Example","note":"Original fixture","position":{"latitude_deg":41.0,"longitude_deg":-75.0},"altitude_m":null});
    s.execute("pins.add", json!({"pin":pin})).unwrap();
    assert!(s.execute("pins.add", json!({"pin":pin})).is_err());
    let document = s.execute("pins.export", json!({})).unwrap();
    assert_eq!(document["pins"].as_array().unwrap().len(), 1);
    let mut other = Session::new().unwrap();
    other.load_navigation_storage(&dir).unwrap();
    assert_eq!(other.execute("pins.list", json!({})).unwrap(), document);
    assert!(
        s.execute(
            "pins.import",
            json!({"document":{"version":1,"pins":[pin.clone(),pin]}})
        )
        .is_err()
    );
    assert_eq!(s.execute("pins.list", json!({})).unwrap(), document);
    assert!(PinsFile::from_json("{\"version\":1,\"pins\":[],\"extra\":0}").is_err());
    s.execute("pins.remove", json!({"id":"example"})).unwrap();
    assert!(s.execute("pins.remove", json!({"id":"missing"})).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn search_ranking_radial_nearest_and_opt_in_actions() {
    require_region_data!();
    let mut s = Session::new().unwrap();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/terrain/new-york");
    s.install_region(flightcraft_control::read_region(&dir).unwrap())
        .unwrap();
    s.execute("sim.reset", json!({"world":"new-york","start":{"kind":"runway"}}))
        .unwrap();
    for q in ["MMU", "KMMU", "Morristown"] {
        let out = s.execute("search.query", json!({"text":q})).unwrap();
        assert_eq!(out["results"][0]["kind"], "airport");
        assert_eq!(out["results"][0]["ident"], "MMU");
    }
    let sbj = s.execute("search.query", json!({"text":"SBJ"})).unwrap();
    assert_eq!(sbj["results"][0]["kind"], "navaid");
    let a = s.execute("search.query", json!({"text":"SBJ270015"})).unwrap();
    let b = s.execute("search.query", json!({"text":"SBJ/270/15"})).unwrap();
    assert_eq!(a["results"][0]["position"], b["results"][0]["position"]);
    assert_eq!(a["results"][0]["kind"], "radial");
    let origin: flightcraft_world::GeoPoint =
        serde_json::from_value(sbj["results"][0]["position"].clone()).unwrap();
    let dest: flightcraft_world::GeoPoint =
        serde_json::from_value(a["results"][0]["position"].clone()).unwrap();
    let (distance, bearing) = origin.inverse(dest);
    assert!((distance - 15.0 * 1852.0).abs() < 0.01);
    let variation = s
        .sim()
        .world()
        .navdata()
        .unwrap()
        .navaids
        .iter()
        .find(|n| n.ident == "SBJ")
        .unwrap()
        .components
        .iter()
        .find_map(|c| c.magnetic_variation_deg)
        .unwrap();
    assert!((bearing - 270.0 - variation).abs() < 0.001);
    let near = s
        .execute("navdata.nearest", json!({"position":origin,"limit":5}))
        .unwrap();
    assert_eq!(near["results"][0]["ident"], "SBJ");
    assert_eq!(
        s.execute("search.query", json!({"text":"my location"})).unwrap()["results"][0]["kind"],
        "location_action"
    );
    assert_eq!(
        s.execute("search.query", json!({"text":"100 Example Street"}))
            .unwrap()["results"][0]["kind"],
        "geocode_action"
    );
    assert!(s.execute("location.current", json!({})).is_err());
    assert!(
        s.execute(
            "geocode.lookup",
            json!({"provider":"census","address":"100 Example Street"})
        )
        .is_err()
    );
    assert!(
        s.execute("sim.set_date", json!({"date":{"year":2026,"month":2,"day":29}}))
            .is_err()
    );
    assert_eq!(s.simulation_date(), Default::default());
}
#[test]
fn editing_xml_summary_profile_and_glass_activation() {
    require_region_data!();
    let mut s = Session::new().unwrap();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/terrain/new-york");
    s.install_region(flightcraft_control::read_region(&dir).unwrap())
        .unwrap();
    s.execute("sim.reset",json!({"world":"new-york","start":{"kind":"air","height_m":1000.0,"airspeed_mps":45.0,"heading_rad":0.0}})).unwrap();
    s.execute("plan.new",json!({"name":"Example & <route>","cruise":{"airspeed_mps":50.0,"fuel_flow_kg_s":0.005},"reserve_s":2700.0})).unwrap();
    for (id, lat, lon, kind) in [
        ("START", 40.97, -75.0, "coordinate"),
        ("EXAMP", 40.9, -74.8, "fix"),
        ("PIN", 40.8, -74.7, "pin"),
    ] {
        s.execute("plan.add", json!({"point":point(id,lat,lon,kind)}))
            .unwrap();
    }
    let before = s.execute("plan.export", json!({"format":"json"})).unwrap();
    assert!(s.execute("plan.move", json!({"from":usize::MAX,"to":0})).is_err());
    assert_eq!(
        s.execute("plan.export", json!({"format":"json"})).unwrap(),
        before
    );
    let p = FlightPlan::from_json(before["text"].as_str().unwrap()).unwrap();
    for format in [PlanFormat::Gpx, PlanFormat::Fpl] {
        let text = export_plan(&p, format).unwrap();
        let q = import_plan(&text, format).unwrap();
        assert_eq!(q.name, p.name);
        assert_eq!(q.waypoints.len(), 3);
        for (a, b) in q.waypoints.iter().zip(&p.waypoints) {
            assert_eq!(a.waypoint.position, b.waypoint.position);
            assert_eq!(a.waypoint.ident, b.waypoint.ident);
            assert_eq!(a.altitude_m, b.altitude_m);
        }
    }
    for text in [
        "<!DOCTYPE gpx [<!ENTITY a 'x'>]><gpx>&a;</gpx>",
        "<gpx><rte><rtept lat='NaN' lon='0'/></rte></gpx>",
        "<gpx><rte/><rte/></gpx>",
    ] {
        assert!(import_plan(text, PlanFormat::Gpx).is_err());
    }
    let summary = s.execute("plan.summary", json!({})).unwrap();
    assert_eq!(summary["legs"].as_array().unwrap().len(), 2);
    let fuel = summary["required_fuel_kg"].as_f64().unwrap();
    let trip = summary["trip_fuel_kg"].as_f64().unwrap();
    assert!((fuel - trip - 13.5).abs() < 1e-9);
    let distance = summary["distance_m"].as_f64().unwrap();
    assert!((summary["ete_s"].as_f64().unwrap() - distance / 50.0).abs() < 1e-9);
    let profile = s.execute("plan.profile", json!({"spacing_m":1000.0})).unwrap();
    assert!(!profile["samples"].as_array().unwrap().is_empty());
    s.execute("plan.activate", json!({})).unwrap();
    let gps = s.glass_state();
    assert_eq!(gps.settings.waypoints.len(), 3);
    assert_eq!(gps.settings.flight_plan, vec!["START", "EXAMP", "PIN"]);
    assert!(gps.gps.valid);
    assert_eq!(gps.gps.destination.as_deref(), Some("EXAMP"));
    assert_eq!(gps.gps.active_leg, 1); // The origin is already within 0.3 nm of START.
    assert!(s.execute("avionics.set",json!({"setting":{"kind":"glass","setting":{"kind":"flight_plan_waypoints","waypoints":[{"ident":"BAD","kind":"coordinate","position":{"latitude_deg":99,"longitude_deg":0}}]}}})).is_err());
    assert_eq!(s.glass_state().settings.waypoints.len(), 3);
    s.execute("sim.reset",json!({"world":"new-york","start":{"kind":"geographic_air","position":{"latitude_deg":40.97,"longitude_deg":-75.0},"height_agl_m":600.0,"airspeed_mps":45.0,"heading_rad":0.0}})).unwrap();
    assert!((s.sim().readout().latitude_deg.unwrap() - 40.97).abs() < 1e-8);
}

#[test]
fn named_plan_storage_edits_and_hostile_names() {
    let dir = std::env::temp_dir().join(format!("flightcraft-plan-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = Session::new().unwrap();
    s.load_navigation_storage(&dir).unwrap();
    s.execute("plan.new", json!({"name":"Stored"})).unwrap();
    s.execute("plan.add", json!({"point":point("A",41.0,-75.0,"coordinate")}))
        .unwrap();
    s.execute(
        "plan.insert",
        json!({"index":0,"point":point("B",41.01,-75.01,"navaid")}),
    )
    .unwrap();
    s.execute("plan.move", json!({"from":0,"to":1})).unwrap();
    let p = s
        .execute("plan.set_altitude", json!({"index":0,"altitude_m":1500.0}))
        .unwrap();
    assert_eq!(p["waypoints"][0]["waypoint"]["ident"], "A");
    s.execute("plan.save", json!({"name":"example"})).unwrap();
    s.execute("plan.remove", json!({"index":0})).unwrap();
    let restored = s.execute("plan.load", json!({"name":"example"})).unwrap();
    assert_eq!(restored, p);
    for name in ["../escape", "/absolute", "a/b", "", "a\\b"] {
        assert!(s.execute("plan.save", json!({"name":name})).is_err());
        assert!(s.execute("plan.load", json!({"name":name})).is_err());
    }
    std::fs::write(
        dir.join("plan-bad.json"),
        "{\"version\":1,\"name\":\"Bad\",\"reserve_s\":-1,\"cruise\":null,\"waypoints\":[]}",
    )
    .unwrap();
    assert!(s.execute("plan.load", json!({"name":"bad"})).is_err());
    let out = s.execute("plan.export", json!({"format":"json"})).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(out["text"].as_str().unwrap()).unwrap(),
        p
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn profile_warns_on_terrain_airspace_coverage_and_impossible_wind() {
    require_region_data!();
    let mut s = Session::new().unwrap();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/terrain/new-york");
    s.install_region(flightcraft_control::read_region(&dir).unwrap())
        .unwrap();
    s.execute("sim.reset", json!({"world":"new-york","start":{"kind":"runway"}}))
        .unwrap();
    s.execute(
        "plan.new",
        json!({"name":"Low","cruise":{"airspeed_mps":10.0,"fuel_flow_kg_s":0.005}}),
    )
    .unwrap();
    for (id, lat, lon) in [("A", 40.64, -73.78), ("B", 40.65, -73.76)] {
        let mut p = point(id, lat, lon, "coordinate");
        p["altitude_m"] = json!(10.0);
        s.execute("plan.add", json!({"point":p})).unwrap();
    }
    let profile = s.execute("plan.profile", json!({"spacing_m":100.0})).unwrap();
    let conflicts = profile["conflicts"].as_array().unwrap();
    assert!(conflicts.iter().any(|c| c["kind"] == "terrain"));
    assert!(conflicts.iter().any(|c| c["kind"] == "airspace"));
    s.execute(
        "weather.set",
        json!({"wind_from_deg":0.0,"wind_speed_mps":75.0,"shear":false,"turbulence":"off","seed":1}),
    )
    .unwrap();
    let summary = s.execute("plan.summary", json!({})).unwrap();
    assert!(summary["ete_s"].is_null());
    assert!(
        summary["conflicts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["kind"] == "wind")
    );
    s.execute("plan.add", json!({"point":point("OUT",41.0,-80.0,"coordinate")}))
        .unwrap();
    let profile = s.execute("plan.profile", json!({"spacing_m":10000.0})).unwrap();
    assert!(
        profile["conflicts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["kind"] == "beyond_region")
    );
}

#[test]
fn fpl_exports_standard_waypoint_fields_and_documents_altitude_loss() {
    let p=FlightPlan::from_json(&json!({"version":1,"name":"FPL fixture","reserve_s":2700.0,"cruise":null,"waypoints":[{"waypoint":{"ident":"EXAMP","kind":"fix","position":{"latitude_deg":41.0,"longitude_deg":-75.0}},"altitude_m":1500.0}]}).to_string()).unwrap();
    let text = export_plan(&p, PlanFormat::Fpl).unwrap();
    assert!(!text.contains("<elevation>"));
    let imported = import_plan(&text, PlanFormat::Fpl).unwrap();
    assert_eq!(imported.waypoints[0].altitude_m, 1000.0);
    assert_eq!(imported.waypoints[0].waypoint.ident, "EXAMP");
}
