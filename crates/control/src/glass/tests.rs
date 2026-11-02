#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
#[cfg(test)]
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/region_data.rs"));
use super::*;
#[test]
fn terrain_clearance_thresholds() {
    assert_eq!(terrain_band(3000. * 0.3048, 1999. * 0.3048), TerrainBand::Clear);
    assert_eq!(terrain_band(3000. * 0.3048, 2000. * 0.3048), TerrainBand::Caution);
    assert_eq!(terrain_band(3000. * 0.3048, 2900. * 0.3048), TerrainBand::Warning);
    assert_eq!(terrain_band(3000. * 0.3048, 4000. * 0.3048), TerrainBand::Warning);
}
#[test]
fn commands_validate_atomically_and_install_glass() {
    let mut session = Session::new().unwrap();
    session
        .execute(
            "aircraft.load",
            serde_json::json!({"source":{"kind":"installed","id":"corvid-glass"},"reset":true}),
        )
        .unwrap();
    assert!(!session.sim.systems().vacuum_installed);
    assert!(
        session
            .set_glass(GlassSetting::Transponder {
                code: "1280".into(),
                mode: TransponderMode::Alt
            })
            .is_err()
    );
    assert_eq!(session.glass_state().settings.transponder_code, "1200");
    session
        .set_glass(GlassSetting::Transponder {
            code: "0456".into(),
            mode: TransponderMode::On,
        })
        .unwrap();
    assert_eq!(session.glass_state().settings.transponder_code, "0456");
    assert!(
        session
            .set_glass(GlassSetting::MapRange { range_nm: f64::NAN })
            .is_err()
    );
}
fn regional_session() -> Session {
    let mut s = Session::new().unwrap();
    s.execute(
        "aircraft.load",
        serde_json::json!({"source":{"kind":"installed","id":"corvid-glass"},"reset":true}),
    )
    .unwrap();
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/terrain/new-york");
    s.install_region(crate::read_region(&directory).unwrap()).unwrap();
    s.execute("sim.reset",serde_json::json!({"world":"new-york","start":{"kind":"air","height_m":914.4,"airspeed_mps":50.0,"heading_rad":1.0}})).unwrap();
    s
}
// Independent vector central-angle oracle; no production geodesy is called here.
fn oracle(a: [f64; 2], b: [f64; 2]) -> (f64, f64) {
    let [a, b1] = a.map(f64::to_radians);
    let [c, d] = b.map(f64::to_radians);
    let dot = a.sin() * c.sin() + a.cos() * c.cos() * (d - b1).cos();
    let distance = dot.clamp(-1., 1.).acos() * 6_371_008.8 / 1852.;
    let bearing = ((d - b1).sin() * c.cos())
        .atan2(a.cos() * c.sin() - a.sin() * c.cos() * (d - b1).cos())
        .rem_euclid(std::f64::consts::TAU);
    (distance, bearing)
}
#[test]
fn new_york_leg_math_scaling_airspace_and_snapshot_schema() {
    require_region_data!();
    let mut s = regional_session();
    let world = s.sim.world();
    let a = crate::regions::airport(world, "1N7").unwrap();
    let b = crate::regions::airport(world, "KMMU").unwrap();
    let start = [a.latitude_deg, a.longitude_deg];
    let end = [b.latitude_deg, b.longitude_deg];
    let expected = oracle(start, end);
    s.set_glass(GlassSetting::FlightPlan {
        airports: vec!["1N7".into(), "KMMU".into()],
    })
    .unwrap();
    s.set_glass(GlassSetting::ActiveLeg { index: 1 }).unwrap();
    let state = s.glass_state();
    let leg = state.gps.legs.get(1).unwrap();
    assert!((leg.distance_nm - expected.0).abs() < 0.1);
    assert!(
        (leg.bearing_rad - s.sim.heading_reference().true_to_magnetic(expected.1))
            .to_degrees()
            .abs()
            < 1.
    );
    assert!(state.outlines.iter().any(
        |o| o.class == Some(flightcraft_world::AirspaceClass::B) && o.rings.iter().any(|r| r.len() > 10)
    ));
    assert!(state.terrain.len() > 16000);
    assert_eq!(state.airports.len(), 26);
    let position = s
        .sim
        .world()
        .geodetic(s.sim.state().position_m[0], s.sim.state().position_m[1])
        .unwrap();
    let distance = oracle(position, end).0;
    assert!((state.gps.distance_nm - distance).abs() < 0.1);
    assert_eq!(state.gps.cdi_full_scale_nm, if distance < 30. { 1. } else { 2. });
    // Right-of-track case: latitude shifted south of an eastward leg gives a left correction.
    let (xt, _) = track(start, end, [start[0] - 0.01, start[1]]);
    let angular = oracle(start, [start[0] - 0.01, start[1]]).0 * 1852. / 6_371_008.8;
    let bearing = oracle(start, [start[0] - 0.01, start[1]]).1;
    let expected_xt = (angular.sin() * (bearing - expected.1).sin()).asin() * 6_371_008.8 / 1852.;
    assert!((xt / 1852. - expected_xt).abs() < 0.001);
    let info = Session::commands()
        .into_iter()
        .find(|c| c.name == "avionics.state")
        .unwrap();
    let out = s.execute("avionics.state", serde_json::json!({})).unwrap();
    assert!(
        jsonschema::validator_for(&info.output_schema)
            .unwrap()
            .is_valid(&out)
    );
}
#[test]
fn independent_failure_reversion_and_backup_endurance() {
    require_region_data!();
    let mut s = regional_session();
    s.sim.step(5401);
    assert!(s.glass_state().air_data.attitude_valid);
    s.set_glass(GlassSetting::Failure {
        target: GlassFailure::Ahrs,
        failed: true,
    })
    .unwrap();
    let state = s.glass_state();
    assert!(!state.air_data.attitude_valid && state.air_data.air_data_valid);
    assert!(state.standby.attitude_valid);
    s.set_glass(GlassSetting::Failure {
        target: GlassFailure::Adc,
        failed: true,
    })
    .unwrap();
    assert!(!s.glass_state().air_data.air_data_valid);
    s.set_glass(GlassSetting::Failure {
        target: GlassFailure::Mfd,
        failed: true,
    })
    .unwrap();
    assert!(s.glass_state().reversion);
    s.sim
        .set_systems(flightcraft_sim::systems::SystemSetting::Switch {
            switch: flightcraft_sim::systems::SystemSwitch::Master,
            on: false,
        })
        .unwrap();
    // Electrical state changes on a fixed step, including independent standby power.
    s.sim.step(1);
    let standby = s
        .sim
        .systems()
        .electrical
        .loads
        .iter()
        .find(|l| l.id == "glass_standby")
        .unwrap();
    assert!(standby.on_backup && standby.powered);
    assert!(s.glass_state().standby.attitude_valid);
}
#[test]
fn blocked_ports_remain_unflagged_and_freeze_when_both_are_sealed() {
    require_region_data!();
    let mut s = regional_session();
    s.sim.step(10);
    s.execute(
        "failure.set",
        serde_json::json!({"failure":{"kind":"pitot_blocked"}}),
    )
    .unwrap();
    s.execute(
        "failure.set",
        serde_json::json!({"failure":{"kind":"static_blocked"}}),
    )
    .unwrap();
    let before = s.sim.systems().sensed;
    s.sim.step(120);
    let state = s.glass_state();
    assert!(state.air_data.air_data_valid);
    assert!((state.air_data.indicated_airspeed_mps - before.indicated_airspeed_mps).abs() < 1e-9);
    assert!((state.air_data.altitude_msl_m - before.altitude_msl_m).abs() < 1e-9);
    assert_eq!(state.air_data.vertical_speed_mps, 0.);
}
#[test]
fn actual_cdi_matches_signed_geodesic_and_terminal_transition() {
    require_region_data!();
    let mut s = regional_session();
    s.set_glass(GlassSetting::FlightPlan {
        airports: vec!["1N7".into(), "KMMU".into()],
    })
    .unwrap();
    s.set_glass(GlassSetting::ActiveLeg { index: 1 }).unwrap();
    let a = crate::regions::airport(s.sim.world(), "1N7").unwrap();
    let b = crate::regions::airport(s.sim.world(), "KMMU").unwrap();
    let start = [a.latitude_deg, a.longitude_deg];
    let end = [b.latitude_deg, b.longitude_deg];
    let course = oracle(start, end).1;
    let origin = s.sim.world().region().unwrap().origin.clone();
    let projection =
        flightcraft_world::geo::LocalProjection::new(origin.latitude_deg, origin.longitude_deg).unwrap();
    for position in [[40.90, -74.95], [40.82, -74.60], [40.92, -74.60]] {
        let mut state = *s.sim.state();
        let [n, e] = projection.project(position[0], position[1]).unwrap();
        state.position_m = [n, e, -914.4];
        s.sim = flightcraft_sim::Sim::from_state_in_world(state, s.sim.controls(), s.sim.world().clone())
            .unwrap();
        let result = s.glass_state().gps;
        let (distance, bearing) = oracle(start, position);
        let expected_xt = (distance * 1852. / 6_371_008.8)
            .sin()
            .mul_add((bearing - course).sin(), 0.)
            .asin()
            * 6_371_008.8
            / 1852.;
        let full_scale = if oracle(position, end).0 < 30. { 1. } else { 2. };
        assert_eq!(result.cdi_full_scale_nm, full_scale);
        assert!((result.cross_track_nm - expected_xt).abs() < 0.001);
        assert!((result.cdi_deviation - (-expected_xt / full_scale).clamp(-1., 1.)).abs() < 0.001);
    }
}
#[test]
fn glass_settings_validate_against_schemas_and_queries_do_not_advance_sensors() {
    require_region_data!();
    let mut s = regional_session();
    let before = s.sim.glass_computers().clone();
    let hash = s.sim.state_hash();
    for _ in 0..4 {
        s.execute("avionics.state", serde_json::json!({})).unwrap();
    }
    assert_eq!(s.sim.glass_computers(), &before);
    assert_eq!(s.sim.state_hash(), hash);
    s.set_glass(GlassSetting::Failure {
        target: GlassFailure::Ahrs,
        failed: true,
    })
    .unwrap();
    assert_ne!(s.sim.state_hash(), hash);
    let info = Session::commands()
        .into_iter()
        .find(|c| c.name == "avionics.set")
        .unwrap();
    let validator = jsonschema::validator_for(&info.input_schema).unwrap();
    for setting in [
        serde_json::json!({"kind":"map_range","range_nm":0}),
        serde_json::json!({"kind":"transponder","code":"1280","mode":"alt"}),
    ] {
        assert!(!validator.is_valid(&serde_json::json!({"setting":{"kind":"glass","setting":setting}})));
    }
}
