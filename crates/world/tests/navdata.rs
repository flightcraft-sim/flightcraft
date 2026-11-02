#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_world::{MAX_NAVDATA_BYTES, Navdata};
use serde_json::json;
fn document() -> serde_json::Value {
    json!({"version":1,"attribution":"Original navigation fixture","effective_date":null,"snapshot_date":"2026-11-01","navaids":[],"fixes":[{"id":"fixture-fix","ident":"EXAMP","name":"Example fix","position":{"latitude_deg":41.0,"longitude_deg":-75.0},"effective_unix_ms":null}],"ils":[],"routes":[],"holds":[]})
}
#[test]
fn bounded_navdata_rejects_hostile_fields_and_duplicate_ids() {
    Navdata::from_json(&document().to_string()).unwrap();
    for bad in [
        json!(null),
        json!({}),
        {
            let mut d = document();
            d["extra"] = json!(true);
            d
        },
        {
            let mut d = document();
            d["fixes"][0]["position"]["latitude_deg"] = json!(91);
            d
        },
        {
            let mut d = document();
            let f = d["fixes"][0].clone();
            d["fixes"].as_array_mut().unwrap().push(f);
            d
        },
        {
            let mut d = document();
            d["snapshot_date"] = json!("2026-02-29");
            d
        },
        {
            let mut d = document();
            d["snapshot_date"] = json!("2026-1-001");
            d
        },
        {
            let mut d = document();
            d["effective_date"] = json!("not a date");
            d
        },
    ] {
        assert!(Navdata::from_json(&bad.to_string()).is_err());
    }
    assert!(Navdata::from_json(&" ".repeat(MAX_NAVDATA_BYTES + 1)).is_err());
}
#[test]
fn geographic_distance_and_destination_handle_antimeridian_and_coincidence() {
    use flightcraft_world::GeoPoint;
    let a = GeoPoint {
        latitude_deg: 0.0,
        longitude_deg: 179.9,
    };
    let b = a.destination(90.0, 50000.0);
    assert!(b.longitude_deg < 0.0);
    assert!((a.inverse(b).0 - 50000.0).abs() < 1e-6);
    assert_eq!(a.inverse(a).0, 0.0);
}
