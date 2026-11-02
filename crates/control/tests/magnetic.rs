#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_control::{Session, Step, read_region};
use flightcraft_sim::autopilot::wrap;
use flightcraft_world::{
    World,
    geomag::{MagneticModel, SimulationDate},
};
use serde_json::{Value, json};
fn world() -> World {
    read_region(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/terrain/new-york"))
        .unwrap()
}
fn set(s: &mut Session, setting: Value) {
    s.execute("avionics.set", json!({"setting":setting})).unwrap();
}
fn panel(s: &mut Session, setting: Value) {
    set(s, json!({"kind":"autopilot","setting":setting}));
}
fn flight(aircraft: &str) -> Session {
    let mut s = Session::new().unwrap();
    s.execute(
        "aircraft.load",
        json!({"source":{"kind":"installed","id":aircraft},"reset":true}),
    )
    .unwrap();
    s.install_region(world()).unwrap();
    s
}
#[test]
fn wmm_sensor_compass_date_and_free_gyro_references() {
    let mut s = flight("corvid-glass");
    // Commands use radians, with degrees reserved for explicit degrees fields.
    s.execute("sim.reset",json!({"world":"new-york","start":{"kind":"air","height_m":1000.,"airspeed_mps":40.,"heading_rad":core::f64::consts::FRAC_PI_2}})).unwrap();
    let r = s.sim().readout();
    assert!(wrap(r.heading_rad - core::f64::consts::FRAC_PI_2).abs() < 1e-8);
    let field = MagneticModel::wmm2025()
        .unwrap()
        .field(
            r.latitude_deg.unwrap(),
            r.longitude_deg.unwrap(),
            1000.,
            s.simulation_date().decimal_year().unwrap(),
        )
        .unwrap();
    assert!((field.declination_deg - r.magnetic_declination_rad.to_degrees()).abs() < 1e-10);
    assert!(wrap(r.heading_true_rad - r.heading_rad - r.magnetic_declination_rad).abs() < 1e-10);
    let compass = s.execute("avionics.state", json!({})).unwrap();
    assert!((compass["magnetic_compass"]["heading_deg"].as_f64().unwrap() - 90.).abs() < 1e-8);
    let gyro = s.sim().systems().sensed.heading_rad;
    s.execute("sim.set_date", json!({"date":{"year":2029,"month":7,"day":1}}))
        .unwrap();
    assert_ne!(s.sim().readout().heading_rad, r.heading_rad);
    assert_eq!(s.sim().systems().sensed.heading_rad, gyro);
    let hash = s.sim().state_hash();
    assert!(
        s.execute("sim.set_date", json!({"date":{"year":2030,"month":1,"day":1}}))
            .is_err()
    );
    assert_eq!(s.sim().state_hash(), hash);
    s.execute("sim.reset",json!({"world":"new-york","start":{"kind":"air","height_m":1000.,"airspeed_mps":40.,"heading_rad":core::f64::consts::FRAC_PI_2}})).unwrap();
    assert_eq!(
        s.sim().simulation_date(),
        SimulationDate {
            year: 2029,
            month: 7,
            day: 1
        }
    );
    assert!(wrap(s.sim().systems().sensed.heading_rad - core::f64::consts::FRAC_PI_2).abs() < 1e-8);
    s.execute(
        "assist.set",
        json!({"altitude_m":1000.,"heading_rad":core::f64::consts::FRAC_PI_2,"airspeed_mps":40.}),
    )
    .unwrap();
    s.run(Step { steps: 5520 }).unwrap();
    let a = s.glass_state().air_data;
    assert!(a.attitude_valid);
    assert!(wrap(a.heading_rad - s.sim().readout().heading_rad).abs() < 0.01);
    let flat = flightcraft_sim::Sim::new(&flightcraft_sim::Start::Air {
        height_m: 1000.,
        airspeed_mps: 40.,
        heading_rad: 1.,
    })
    .unwrap();
    assert_eq!(flat.heading_reference().declination_rad, 0.);
    assert_eq!(flat.readout().heading_rad, flat.readout().heading_grid_rad);
}
#[test]
fn hdg_and_trk_fly_magnetic_and_map_rotation_stays_in_the_grid() {
    let mut s = flight("corvid-glass");
    s.execute("sim.reset",json!({"world":"new-york","start":{"kind":"air","height_m":1000.,"airspeed_mps":40.,"heading_rad":80_f64.to_radians()}})).unwrap();
    s.execute(
        "assist.set",
        json!({"altitude_m":1000.,"heading_rad":80_f64.to_radians(),"airspeed_mps":40.}),
    )
    .unwrap();
    s.run(Step { steps: 5520 }).unwrap();
    s.execute("assist.off", json!({})).unwrap();
    set(&mut s, json!({"kind":"heading_bug","degrees":90.}));
    panel(&mut s, json!({"kind":"engage","on":true}));
    panel(&mut s, json!({"kind":"vertical","mode":"alt"}));
    panel(&mut s, json!({"kind":"lateral","mode":"hdg"}));
    s.run(Step { steps: 120 * 120 }).unwrap();
    let r = s.sim().readout();
    assert!(
        wrap(r.heading_rad - 90_f64.to_radians()).abs() < 3_f64.to_radians(),
        "{r:?}"
    );
    assert!((r.heading_true_rad.to_degrees() - 77.).abs() < 3.);
    panel(&mut s, json!({"kind":"track","track_rad":270_f64.to_radians()}));
    panel(&mut s, json!({"kind":"lateral","mode":"trk"}));
    s.run(Step { steps: 180 * 120 }).unwrap();
    let g = s.glass_state();
    assert!(wrap(g.ground_track_rad - 270_f64.to_radians()).abs() < 3_f64.to_radians());
    set(
        &mut s,
        json!({"kind":"glass","setting":{"kind":"orientation","orientation":"track_up"}}),
    );
    assert!(s.glass_state().aircraft_heading_relative_rad.abs() < 0.05);
    set(
        &mut s,
        json!({"kind":"glass","setting":{"kind":"orientation","orientation":"heading_up"}}),
    );
    assert!(s.glass_state().aircraft_heading_relative_rad.abs() < 0.05);
}
#[test]
fn rnav07_final_track_and_leg_labels_are_magnetic() {
    let w = world();
    let d = w.procedures().unwrap();
    let p = d
        .procedures
        .iter()
        .find(|p| p.airport == "1N7" && p.ident == "R07")
        .unwrap();
    let r = p.routes.iter().find(|r| r.transition.is_empty()).unwrap();
    let published = r
        .legs
        .iter()
        .find(|l| l.path == flightcraft_world::procedures::PathTerminator::CA)
        .unwrap()
        .course_deg
        .unwrap();
    let faf = d
        .point(
            r.legs
                .iter()
                .find(|l| l.waypoint_description.ends_with('F'))
                .unwrap()
                .fix
                .as_deref()
                .unwrap(),
        )
        .unwrap()
        .position;
    let end = d
        .point(
            r.legs
                .iter()
                .find(|l| l.waypoint_description.ends_with('M'))
                .unwrap()
                .fix
                .as_deref()
                .unwrap(),
        )
        .unwrap()
        .position;
    let start = end.destination(faf.inverse(end).1 + 180., 7. * 1852.);
    let region = w.region().unwrap();
    let projection =
        flightcraft_world::geo::LocalProjection::new(region.origin.latitude_deg, region.origin.longitude_deg)
            .unwrap();
    let [n, e] = projection
        .project(start.latitude_deg, start.longitude_deg)
        .unwrap();
    let mut s = flight("corvid-glass");
    s.execute("sim.reset",json!({"world":"new-york","start":{"kind":"geographic_air","position":start,"height_agl_m":1200.-w.ground(n,e).height_msl_m,"airspeed_mps":40.,"heading_rad":published.to_radians()}})).unwrap();
    set(
        &mut s,
        json!({"kind":"load_procedure","airport":"1N7","procedure":"R07","vectors_to_final":true}),
    );
    set(&mut s, json!({"kind":"activate_approach"}));
    s.run(Step { steps: 1 }).unwrap();
    let expected = s
        .sim()
        .heading_reference()
        .true_to_magnetic(faf.inverse(end).1.to_radians());
    assert!(wrap(s.glass_state().gps.desired_track_rad - expected).abs() < 0.3_f64.to_radians());
    let final_leg = s
        .sim()
        .procedure_receiver()
        .legs
        .iter()
        .find(|l| l.leg.waypoint_description.ends_with('M'))
        .unwrap();
    assert!((final_leg.course_magnetic_deg.unwrap() - published).abs() < 1.);
}
#[test]
fn sbj_radials_use_station_declination_instead_of_wmm() {
    let w = world();
    let n = w
        .navdata()
        .unwrap()
        .navaids
        .iter()
        .find(|n| n.ident == "SBJ")
        .unwrap();
    let station = &n.components[0];
    let variation = station.magnetic_variation_deg.unwrap();
    let p = station.position.destination(90. + variation, 10. * 1852.);
    let region = w.region().unwrap();
    let projection =
        flightcraft_world::geo::LocalProjection::new(region.origin.latitude_deg, region.origin.longitude_deg)
            .unwrap();
    let [north, east] = projection.project(p.latitude_deg, p.longitude_deg).unwrap();
    let state = flightcraft_sim::State {
        position_m: [north, east, -1200.],
        ..Default::default()
    };
    let radio = flightcraft_sim::avionics::radio::NavRadio {
        active_hz: 112900000,
        course_deg: 270.,
        ..Default::default()
    };
    let output = radio.sample(&w, state, true);
    assert!((output.radial_magnetic_deg.unwrap() - 90.).abs() < 0.01);
    let wmm = MagneticModel::wmm2025()
        .unwrap()
        .declination(p.latitude_deg, p.longitude_deg, Default::default())
        .unwrap();
    assert!((wmm - variation).abs() > 1.);
}

#[test]
fn wind_commands_and_readouts_are_magnetic() {
    let mut s = flight("corvid-glass");
    s.execute("sim.reset",json!({"world":"new-york","start":{"kind":"air","height_m":1000.,"airspeed_mps":40.,"heading_rad":1.}})).unwrap();
    let out = s
        .execute(
            "weather.set",
            json!({"wind_from_deg":90.,"wind_speed_mps":5.,"shear":false,"turbulence":"off","seed":0}),
        )
        .unwrap();
    assert!((out["weather"]["wind_from_deg"].as_f64().unwrap() - 90.).abs() < 1e-8);
    assert!((s.sim().readout().weather.wind_from_deg - 90.).abs() < 1e-8);
    assert!(wrap(s.glass_state().wind_from_rad - 90_f64.to_radians()).abs() < 1e-8);
    let raw = s.sim().weather().wind_from_deg.to_radians();
    assert!(wrap(raw - s.sim().heading_reference().magnetic_to_grid(90_f64.to_radians())).abs() < 1e-8);
}
