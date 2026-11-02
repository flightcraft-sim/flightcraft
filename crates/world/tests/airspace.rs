#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_world::{srtm::LocalProjection, *};
use serde_json::json;

fn square() -> AirspaceFile {
    AirspaceFile::from_json(
        &json!({"attribution":"Original test geometry", "volumes":[{
            "id":"shelf", "name":"Test shelf", "class":"B", "kind":"class",
            "floor":{"value_ft":1000,"reference":"MSL"},
            "ceiling":{"value_ft":2000,"reference":"MSL"},
            "polygons":[[[[0,0],[0,0.1],[0.1,0.1],[0.1,0]],[[0.04,0.04],[0.04,0.06],[0.06,0.06],[0.06,0.04]]]]
        }]})
        .to_string(),
    )
    .unwrap()
}
#[test]
fn holes_closed_edges_half_open_altitudes_and_nearest_shelf() {
    let p = LocalProjection::new(0.0, 0.0).unwrap();
    let mut file = square();
    assert_eq!(file.volumes[0].polygons[0][0].len(), 5);
    let mut surface = file.volumes[0].clone();
    surface.id = "surface".into();
    surface.class = Some(AirspaceClass::D);
    surface.floor = AirspaceAltitude {
        value_ft: 0.0,
        reference: AltitudeReference::SFC,
    };
    surface.ceiling.value_ft = 1500.0;
    file.volumes.push(surface);
    let airspace = file.project(p).unwrap();
    let [n, e] = p.project(0.02, 0.02).unwrap();
    let query = |alt| airspace.query(n, e, alt, 100.0).unwrap();
    assert_eq!(query(100.0).containing.len(), 1); // SFC follows terrain, including taxiing.
    assert_eq!(query(99.0).containing.len(), 0);
    assert_eq!(query(1000.0 * FOOT_M).containing.len(), 2);
    assert_eq!(query(2000.0 * FOOT_M).containing.len(), 0); // Upper bound excluded.
    let boundary = query(950.0 * FOOT_M).nearest.unwrap();
    assert!(boundary.warning);
    assert_eq!(boundary.lateral_distance_m, 0.0);
    assert!((boundary.floor_above_m - 50.0 * FOOT_M).abs() < 1e-10);
    let [n, e] = p.project(0.05, 0.05).unwrap();
    let hole = airspace.query(n, e, 1200.0 * FOOT_M, 100.0).unwrap();
    assert!(hole.containing.is_empty());
    assert!(hole.nearest.unwrap().lateral_distance_m > 1000.0);
    let boundary = airspace
        .query(-500.0, 500.0, 1200.0 * FOOT_M, 100.0)
        .unwrap()
        .nearest
        .unwrap();
    assert!((boundary.lateral_distance_m - 500.0).abs() < 1.0);
    assert!(boundary.warning);
    assert!(airspace.query(f64::NAN, 0.0, 0.0, 0.0).is_err());
    assert!(airspace.query(0.0, 0.0, f64::INFINITY, 0.0).is_err());
}
#[test]
fn hostile_documents_are_errors() {
    let good = square();
    let mutate = |f: fn(&mut AirspaceFile)| {
        let mut file = good.clone();
        f(&mut file);
        assert!(file.validate().is_err());
    };
    mutate(|f| f.volumes[0].floor.value_ft = f64::NAN);
    mutate(|f| f.volumes[0].ceiling.value_ft = 1000.0);
    mutate(|f| f.volumes[0].polygons[0][0][0][0] = 91.0);
    mutate(|f| f.volumes[0].polygons[0].clear());
    mutate(|f| f.volumes[0].polygons[0][0] = vec![[0.0, 0.0]; 3]);
    mutate(|f| f.volumes[0].polygons[0][0] = vec![[0.0, 0.0]; MAX_AIRSPACE_VERTICES + 1]);
    mutate(|f| f.volumes.push(f.volumes[0].clone()));
    mutate(|f| f.volumes[0].kind = AirspaceKind::ModeC);
    mutate(|f| f.attribution.clear());
    let mut json = serde_json::to_value(good).unwrap();
    json["volumes"][0]["floor"]["reference"] = "AGL".into();
    assert!(AirspaceFile::from_json(&json.to_string()).is_err());
    json["volumes"][0]["floor"]["reference"] = "MSL".into();
    json["volumes"][0]["unknown"] = true.into();
    assert!(AirspaceFile::from_json(&json.to_string()).is_err());
    assert!(AirspaceFile::from_json(&" ".repeat(MAX_AIRSPACE_BYTES + 1)).is_err());
}
#[test]
fn new_york_acceptance_locations() {
    let projection = LocalProjection::new(40.97113, -74.99750).unwrap();
    let file =
        AirspaceFile::from_json(include_str!("../../../assets/terrain/new-york/airspace.json")).unwrap();
    let airspace = file.project(projection).unwrap();
    let at = |lat, lon, ft| {
        let [n, e] = projection.project(lat, lon).unwrap();
        airspace.query(n, e, ft * FOOT_M, 0.0).unwrap()
    };
    let jfk = at(40.6413, -73.7781, 2000.0);
    assert!(
        jfk.containing
            .iter()
            .any(|v| v.class == Some(AirspaceClass::B) && v.name.contains("NEW YORK"))
    );
    let teb = at(40.8501, -74.0608, 1000.0);
    assert!(
        teb.containing
            .iter()
            .any(|v| v.class == Some(AirspaceClass::D) && v.name.contains("TETERBORO"))
    );
    assert!(!teb.containing.iter().any(|v| v.class == Some(AirspaceClass::B)));
    assert!(
        !at(40.97113, -74.99750, 1500.0)
            .containing
            .iter()
            .any(|v| v.class == Some(AirspaceClass::B))
    );
    assert!(
        airspace
            .volumes
            .iter()
            .any(|v| v.info.class == Some(AirspaceClass::C))
    );
    assert!(
        airspace
            .volumes
            .iter()
            .any(|v| v.info.class == Some(AirspaceClass::E))
    );
    assert!(
        airspace
            .volumes
            .iter()
            .any(|v| v.info.kind == AirspaceKind::ModeC)
    );
}

#[test]
fn eastern_pennsylvania_acceptance_locations() {
    let projection = LocalProjection::new(40.97113, -74.99750).unwrap();
    let file =
        AirspaceFile::from_json(include_str!("../../../assets/terrain/eastern-pa/airspace.json")).unwrap();
    let airspace = file.project(projection).unwrap();
    let at = |lat, lon, ft| {
        let [n, e] = projection.project(lat, lon).unwrap();
        airspace.query(n, e, ft * FOOT_M, 0.0).unwrap()
    };
    let abe = at(40.65236, -75.4404, 2500.0);
    assert!(
        abe.containing
            .iter()
            .any(|v| v.class == Some(AirspaceClass::C) && v.name.contains("ALLENTOWN"))
    );
    let rdg = at(40.3785, -75.9652, 1500.0);
    assert!(
        rdg.containing
            .iter()
            .any(|v| v.class == Some(AirspaceClass::D) && v.name.contains("READING"))
    );
    let mpo = at(41.1375, -75.3789, 2500.0);
    assert!(!mpo.containing.iter().any(|v| matches!(
        v.class,
        Some(AirspaceClass::B | AirspaceClass::C | AirspaceClass::D)
    )));
}
