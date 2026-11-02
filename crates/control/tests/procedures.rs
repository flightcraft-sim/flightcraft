#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
#[cfg(test)]
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/region_data.rs"));
use flightcraft_control::{Session, Step, read_region};
use flightcraft_sim::{
    State,
    avionics::{procedures::HoldEntry, radio::NavRadio},
    equipment::{LateralMode, VerticalMode},
};
use flightcraft_world::{GeoPoint, World};
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
fn point_state(world: &World, p: GeoPoint, alt: f64) -> State {
    let r = world.region().unwrap();
    let proj =
        flightcraft_world::geo::LocalProjection::new(r.origin.latitude_deg, r.origin.longitude_deg).unwrap();
    let [n, e] = proj.project(p.latitude_deg, p.longitude_deg).unwrap();
    let sim = flightcraft_sim::Sim::new(&flightcraft_sim::Start::Air {
        height_m: 1000.,
        airspeed_mps: 40.,
        heading_rad: 0.,
    })
    .unwrap();
    let mut state = *sim.state();
    state.position_m = [n, e, -alt];
    state.velocity_mps = [40., 0., 0.];
    state
}
fn flight() -> Session {
    let mut s = Session::new().unwrap();
    s.execute(
        "aircraft.load",
        json!({"source":{"kind":"installed","id":"corvid-glass"},"reset":true}),
    )
    .unwrap();
    s.install_region(world()).unwrap();
    s
}
/// Shared by the integration flight and the actual PFD preview.
pub fn ils_flight() -> Session {
    ils_flight_for(false)
}
fn ils_flight_for(legacy: bool) -> Session {
    let mut s = flight();
    if legacy {
        let load = s.aircraft_configuration().unwrap().load.clone();
        s.execute(
            "aircraft.load",
            json!({"variant":"corvid-standard","panel":"corvid-legacy","load":load,"reset":true}),
        )
        .unwrap();
    }
    let w = world();
    let loc = w
        .procedures()
        .unwrap()
        .localizers
        .iter()
        .find(|l| l.airport == "KMMU" && l.runway == "RW23")
        .unwrap();
    let gs = loc.gs_position.unwrap();
    let inbound = loc.course_magnetic_deg + loc.magnetic_variation_deg;
    let start = gs.destination(inbound + 180., 9. * 1852.);
    let state = point_state(&w, start, 1000.);
    let [n, e, _] = state.position_m;
    let r = w.region().unwrap();
    let p =
        flightcraft_world::geo::LocalProjection::new(r.origin.latitude_deg, r.origin.longitude_deg).unwrap();
    let p2 = p.unproject(n + 10., e).unwrap();
    let convergence =
        flightcraft_sim::avionics::navigation::inverse([start.latitude_deg, start.longitude_deg], p2).1;
    let heading = (inbound.to_radians() - convergence).rem_euclid(core::f64::consts::TAU);
    let altitude = loc.gs_elevation_m.unwrap()
        + (9. * 1852. - 46. * 40.) * libm::tan(loc.gs_angle_deg.unwrap().to_radians());
    let heading = flightcraft_sim::magnetic::reference_at(&w, [n, e, -altitude], s.simulation_date())
        .unwrap()
        .grid_to_magnetic(heading);
    s.execute("sim.reset",json!({"world":"new-york","start":{"kind":"geographic_air","position":start,"height_agl_m":altitude-w.ground(n,e).height_msl_m,"airspeed_mps":40.,"heading_rad":heading}})).unwrap();
    s.execute(
        "assist.set",
        json!({"altitude_m":altitude,"heading_rad":heading,"airspeed_mps":40.}),
    )
    .unwrap();
    s.run(Step { steps: 5520 }).unwrap();
    s.execute("assist.off", json!({})).unwrap();
    set(
        &mut s,
        json!({"kind":"load_procedure","airport":"KMMU","procedure":"I23-Y","vectors_to_final":true}),
    );
    set(&mut s, json!({"kind":"vectors_to_final"}));
    set(&mut s, json!({"kind":"cdi_source","source":"nav1"}));
    panel(&mut s, json!({"kind":"engage","on":true}));
    panel(&mut s, json!({"kind":"vertical","mode":"alt"}));
    panel(&mut s, json!({"kind":"lateral","mode":"apr"}));
    s.run(Step { steps: 240 }).unwrap();
    s
}
#[test]
fn kmmu_ils_captures_and_descends_on_the_published_localizer_and_glideslope() {
    require_region_data!();
    let mut s = ils_flight();
    let ap = s.sim().installed_autopilot().unwrap();
    let signal = &s.sim().radio_state()[0];
    assert_eq!(ap.active_lateral, Some(LateralMode::Apr));
    assert_eq!(ap.active_vertical, Some(VerticalMode::Gs));
    assert!((s.glass_state().gps.desired_track_rad.to_degrees() - 229.1).abs() < 1.);
    assert!(signal.lateral_dots.unwrap().abs() < 1., "{signal:?}");
    let mut captured = 0;
    for second in 0..430 {
        s.run(Step { steps: 120 }).unwrap();
        let signal = &s.sim().radio_state()[0];
        let r = s.sim().readout();
        assert!(!r.crashed, "crash at {second}: {r:?}");
        if second > 30 && signal.vertical_dots.is_some() {
            if second > 60 {
                assert!(
                    (r.heading_rad.to_degrees() - 229.1).abs() < 1.,
                    "heading {second}: {r:?}"
                );
            }
            assert!(
                signal.lateral_dots.unwrap().abs() < 0.5,
                "LOC {second}: {signal:?}"
            );
            assert!(
                signal.vertical_dots.unwrap().abs() < 0.5,
                "GS {second}: {signal:?}; {r:?}"
            );
            captured += 1;
        }
        // FAA ILS Y OR LOC Y RWY 23, cycle 2611: unstarred S-ILS DA is 436 ft MSL.
        if r.altitude_msl_m < 436. * 0.3048 {
            break;
        }
    }
    assert!(captured > 200, "insufficient stabilized descent: {captured}");
    assert!(s.sim().readout().altitude_msl_m < 436. * 0.3048);
}
#[test]
fn rnav_transition_cdi_and_missed_approach_are_shared_and_observational() {
    require_region_data!();
    let mut s = flight();
    s.execute("sim.reset",json!({"world":"new-york","start":{"kind":"air","height_m":1200.,"airspeed_mps":40.,"heading_rad":1.}})).unwrap();
    set(
        &mut s,
        json!({"kind":"load_procedure","airport":"1N7","procedure":"R07","transition":"KIMKE"}),
    );
    set(&mut s, json!({"kind":"activate_approach"}));
    s.run(Step { steps: 1 }).unwrap();
    let r = s.sim().procedure_receiver();
    assert_eq!(r.loaded[0].transition.as_deref(), Some("KIMKE"));
    assert_eq!(r.cdi_full_scale_nm, 0.3);
    let a = s.execute("avionics.state", json!({})).unwrap();
    let b = s.execute("avionics.state", json!({})).unwrap();
    assert_eq!(a, b);
    assert!(
        a["procedures"]["loaded"][0]["plate"]["pdf"]
            .as_str()
            .unwrap()
            .ends_with(".PDF")
    );
    set(&mut s, json!({"kind":"missed_approach"}));
    s.run(Step { steps: 1 }).unwrap();
    let r = s.sim().procedure_receiver();
    assert!(r.missed_approach);
    assert!(r.legs[r.active_leg].leg.missed);
    assert!(r.vertical_deviation_m.is_none());
}
#[test]
fn published_hold_enters_and_hm_suspends() {
    require_region_data!();
    let w = world();
    let d = w.procedures().unwrap();
    let p = d
        .procedures
        .iter()
        .find(|p| p.airport == "1N7" && p.ident == "R07")
        .unwrap();
    let hold = p
        .routes
        .iter()
        .flat_map(|r| &r.legs)
        .find(|l| l.path == flightcraft_world::procedures::PathTerminator::HM)
        .unwrap();
    let fix = d.point(hold.fix.as_deref().unwrap()).unwrap();
    let state = point_state(&w, fix.position, 1000.);
    let mut receiver = flightcraft_sim::avionics::procedures::ProcedureReceiver::default();
    receiver
        .load(&w, state, "1N7", "R07", Some("ZUCRY"), false)
        .unwrap();
    receiver.missed().unwrap();
    receiver.active_leg = receiver.legs.len() - 1;
    receiver.step(&w, state, true);
    assert!(receiver.suspended);
    assert_eq!(receiver.hold_entry, Some(HoldEntry::Direct));
    let active = receiver.active_leg;
    for _ in 0..120 {
        receiver.step(&w, state, true);
    }
    assert_eq!(receiver.active_leg, active);
    receiver.hold(false, true).unwrap();
    assert!(!receiver.suspended);
}
#[test]
fn sbj_radials_slant_range_cone_and_to_from() {
    require_region_data!();
    let w = world();
    let station = w
        .navdata()
        .unwrap()
        .navaids
        .iter()
        .find(|n| n.ident == "SBJ")
        .unwrap();
    let c = &station.components[0];
    let var = c.magnetic_variation_deg.unwrap();
    let radio = NavRadio {
        active_hz: 112900000,
        course_deg: 270.,
        ..Default::default()
    };
    let east = station.position.destination(90. + var, 10. * 1852.);
    let west = station.position.destination(270. + var, 10. * 1852.);
    let a = radio.sample(&w, point_state(&w, east, 1200.), true);
    let b = radio.sample(&w, point_state(&w, west, 1200.), true);
    assert_eq!(a.to_from, Some(flightcraft_sim::nav_source::ToFrom::To));
    assert_eq!(b.to_from, Some(flightcraft_sim::nav_source::ToFrom::From));
    assert!(a.lateral_dots.unwrap().abs() < 0.01);
    assert!(a.dme_nm.unwrap() > 10.);
    let overhead = radio.sample(&w, point_state(&w, station.position, 1200.), true);
    assert!(overhead.cone_of_confusion);
    assert!(!overhead.signal_valid);
    assert!(overhead.guidance.is_none());
    assert!(
        !radio
            .sample(
                &w,
                point_state(&w, station.position.destination(90., 60. * 1852.), 1200.),
                true
            )
            .signal_valid
    );
}

#[test]
fn autopilot_intercepts_the_sbj_obs_radial() {
    require_region_data!();
    let mut s = flight();
    let w = world();
    let station = w
        .navdata()
        .unwrap()
        .navaids
        .iter()
        .find(|n| n.ident == "SBJ")
        .unwrap();
    let variation = station.components[0].magnetic_variation_deg.unwrap();
    let course = 270. + variation;
    let start = station
        .position
        .destination(course + 180., 12. * 1852.)
        .destination(course + 90., 1.5 * 1852.);
    let state = point_state(&w, start, 1200.);
    let [n, e, _] = state.position_m;
    let r = w.region().unwrap();
    let projection =
        flightcraft_world::geo::LocalProjection::new(r.origin.latitude_deg, r.origin.longitude_deg).unwrap();
    let convergence = flightcraft_sim::avionics::navigation::inverse(
        [start.latitude_deg, start.longitude_deg],
        projection.unproject(n + 10., e).unwrap(),
    )
    .1;
    let heading = (course.to_radians() - convergence).rem_euclid(core::f64::consts::TAU);
    let heading = flightcraft_sim::magnetic::reference_at(&w, [n, e, -1000.], s.simulation_date())
        .unwrap()
        .grid_to_magnetic(heading);
    s.execute("sim.reset",json!({"world":"new-york","start":{"kind":"geographic_air","position":start,"height_agl_m":1200.-w.ground(n,e).height_msl_m,"airspeed_mps":40.,"heading_rad":heading}})).unwrap();
    s.execute(
        "assist.set",
        json!({"altitude_m":1200.,"heading_rad":heading,"airspeed_mps":40.}),
    )
    .unwrap();
    s.run(Step { steps: 5520 }).unwrap();
    s.execute("assist.off", json!({})).unwrap();
    for setting in [
        json!({"kind":"nav_radio","setting":{"kind":"swap","radio":1}}),
        json!({"kind":"nav_radio","setting":{"kind":"course","radio":1,"degrees":270.}}),
        json!({"kind":"cdi_source","source":"nav1"}),
    ] {
        set(&mut s, setting);
    }
    panel(&mut s, json!({"kind":"engage","on":true}));
    panel(&mut s, json!({"kind":"vertical","mode":"alt"}));
    panel(&mut s, json!({"kind":"lateral","mode":"nav"}));
    s.run(Step { steps: 180 * 120 }).unwrap();
    let signal = &s.sim().radio_state()[0];
    assert!(signal.signal_valid);
    assert_eq!(signal.ident.as_deref(), Some("SBJ"));
    assert!(
        signal.guidance.unwrap().cross_track_m.abs() < 0.1 * 1852.,
        "{signal:?}"
    );
    assert!(!s.sim().readout().crashed);
}

#[test]
fn catalogues_match_the_published_schema_and_reject_hostile_edits() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let schema = serde_json::to_value(schemars::schema_for!(flightcraft_world::Procedures)).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    for region in ["new-york", "eastern-pa"] {
        let text =
            std::fs::read_to_string(root.join(format!("assets/terrain/{region}/procedures.json"))).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert!(validator.is_valid(&value));
        let mut data = flightcraft_world::Procedures::from_json(&text).unwrap();
        assert!(
            data.procedures
                .iter()
                .any(|p| p.airport == "KMMU" && p.ident == "I23-Y")
        );
        assert!(
            data.procedures
                .iter()
                .any(|p| p.airport == "1N7" && p.ident == "R07")
        );
        assert!(
            data.procedures
                .iter()
                .filter(|p| p.kind == flightcraft_world::procedures::ProcedureKind::Approach)
                .all(|p| p.plate.is_some())
        );
        data.procedures[0].routes[0].legs[0].fix = Some("unresolved".into());
        assert!(data.validate().is_err());
        let mut unknown = value;
        unknown["unknown"] = json!(true);
        assert!(!validator.is_valid(&unknown));
        assert!(flightcraft_world::Procedures::from_json(&unknown.to_string()).is_err());
    }
}
#[test]
fn invalid_receiver_commands_leave_the_flight_unchanged() {
    require_region_data!();
    let mut s = flight();
    s.execute("sim.reset",json!({"world":"new-york","start":{"kind":"air","height_m":1200.,"airspeed_mps":40.,"heading_rad":1.}})).unwrap();
    set(
        &mut s,
        json!({"kind":"load_procedure","airport":"1N7","procedure":"R07","transition":"KIMKE"}),
    );
    for setting in [
        json!({"kind":"load_procedure","airport":"1N7","procedure":"R07","transition":"UNKNOWN"}),
        json!({"kind":"nav_radio","setting":{"kind":"standby","radio":1,"frequency_hz":1}}),
        json!({"kind":"nav_radio","setting":{"kind":"swap","radio":0}}),
        json!({"kind":"nav_radio","setting":{"kind":"course","radio":3,"degrees":90.}}),
    ] {
        let hash = s.sim().state_hash();
        assert!(s.execute("avionics.set", json!({"setting":setting})).is_err());
        assert_eq!(s.sim().state_hash(), hash);
    }
}

#[test]
fn published_lpv_glidepath_drives_the_digital_autopilot() {
    require_region_data!();
    let mut s = flight();
    let w = world();
    let d = w.procedures().unwrap();
    let proc = d
        .procedures
        .iter()
        .find(|p| p.airport == "KMMU" && p.ident == "R05")
        .unwrap();
    let final_leg = proc
        .routes
        .iter()
        .flat_map(|r| &r.legs)
        .find(|l| l.vertical_angle_deg.is_some())
        .unwrap();
    let end = d.point(final_leg.fix.as_deref().unwrap()).unwrap().position;
    let faf = d
        .points
        .iter()
        .find(|p| p.airport.as_deref() == Some("KMMU") && p.ident == "HOSBE")
        .unwrap()
        .position;
    let course = faf.inverse(end).1;
    let start = end.destination(course + 180., 8. * 1852.);
    let [n, e, _] = point_state(&w, start, 1000.).position_m;
    let region = w.region().unwrap();
    let p =
        flightcraft_world::geo::LocalProjection::new(region.origin.latitude_deg, region.origin.longitude_deg)
            .unwrap();
    let convergence = flightcraft_sim::avionics::navigation::inverse(
        [start.latitude_deg, start.longitude_deg],
        p.unproject(n + 10., e).unwrap(),
    )
    .1;
    let heading = (course.to_radians() - convergence).rem_euclid(core::f64::consts::TAU);
    let angle = -final_leg.vertical_angle_deg.unwrap();
    let altitude = final_leg.altitude1_m.unwrap() + (8. * 1852. - 46. * 40.) * libm::tan(angle.to_radians());
    let heading = flightcraft_sim::magnetic::reference_at(&w, [n, e, -1000.], s.simulation_date())
        .unwrap()
        .grid_to_magnetic(heading);
    s.execute("sim.reset",json!({"world":"new-york","start":{"kind":"geographic_air","position":start,"height_agl_m":altitude-w.ground(n,e).height_msl_m,"airspeed_mps":40.,"heading_rad":heading}})).unwrap();
    s.execute(
        "assist.set",
        json!({"altitude_m":altitude,"heading_rad":heading,"airspeed_mps":40.}),
    )
    .unwrap();
    s.run(Step { steps: 5520 }).unwrap();
    s.execute("assist.off", json!({})).unwrap();
    set(
        &mut s,
        json!({"kind":"load_procedure","airport":"KMMU","procedure":"R05","vectors_to_final":true}),
    );
    set(&mut s, json!({"kind":"vectors_to_final"}));
    panel(&mut s, json!({"kind":"engage","on":true}));
    panel(&mut s, json!({"kind":"vertical","mode":"alt"}));
    panel(&mut s, json!({"kind":"lateral","mode":"apr"}));
    s.run(Step { steps: 3600 }).unwrap();
    assert_eq!(
        s.sim().procedure_receiver().mode,
        flightcraft_sim::avionics::procedures::ApproachMode::LPV
    );
    assert_eq!(
        s.sim().installed_autopilot().unwrap().active_vertical,
        Some(VerticalMode::Gp)
    );
    assert!(s.sim().procedure_receiver().vertical_deviation_m.unwrap().abs() < 15.);
}

#[test]
fn legacy_apr_couples_the_same_published_ils_glideslope() {
    require_region_data!();
    let mut s = ils_flight_for(true);
    s.run(Step { steps: 3600 }).unwrap();
    let ap = s.sim().installed_autopilot().unwrap();
    assert_eq!(ap.active_lateral, Some(LateralMode::Apr));
    assert_eq!(ap.active_vertical, Some(VerticalMode::Gs));
    let signal = &s.sim().radio_state()[0];
    assert!(signal.vertical_dots.unwrap().abs() < 0.5, "{signal:?}");
}
#[test]
fn localizer_sectors_back_course_and_glideslope_lobes() {
    require_region_data!();
    let w = world();
    let l = w
        .procedures()
        .unwrap()
        .localizers
        .iter()
        .find(|l| l.airport == "KMMU" && l.runway == "RW23")
        .unwrap();
    let course = l.course_magnetic_deg + l.magnetic_variation_deg;
    let gs = l.gs_position.unwrap();
    let p = l.position.destination(course + 180., 6. * 1852.);
    let (distance, bearing) = gs.inverse(p);
    let along = distance * libm::cos((bearing - course - 180.).to_radians());
    let elevation = l.gs_elevation_m.unwrap();
    let altitude = elevation + along * libm::tan(l.gs_angle_deg.unwrap().to_radians());
    let radio = NavRadio::default();
    let signal = radio.sample(&w, point_state(&w, p, altitude), true);
    assert!(signal.signal_valid);
    assert!(signal.vertical_dots.unwrap().abs() < 0.01);
    let false_lobe = radio.sample(
        &w,
        point_state(&w, p, elevation + along * libm::tan(9_f64.to_radians())),
        true,
    );
    assert!(false_lobe.signal_valid);
    assert!(false_lobe.vertical_dots.is_none());
    let off_sector = radio.sample(
        &w,
        point_state(
            &w,
            l.position.destination(course + 180. + 40., 6. * 1852.),
            altitude,
        ),
        true,
    );
    assert!(!off_sector.signal_valid);
    let bc = NavRadio {
        back_course: true,
        ..radio.clone()
    };
    let back = bc.sample(
        &w,
        point_state(&w, l.position.destination(course, 6. * 1852.), altitude),
        true,
    );
    assert!(back.signal_valid);
    assert!(back.vertical_dots.is_none());
    assert_eq!(
        back.guidance.unwrap().kind,
        flightcraft_sim::nav_source::NavKind::BackCourse
    );
    assert!(!radio.sample(&w, point_state(&w, p, altitude), false).signal_valid);
}

#[test]
fn standby_hsi_uses_the_shared_procedure_and_loses_it_with_receiver_power() {
    require_region_data!();
    let mut s = Session::new().unwrap();
    s.execute(
        "aircraft.load",
        json!({"source":{"kind":"installed","id":"corvid-standby"},"reset":true}),
    )
    .unwrap();
    s.install_region(world()).unwrap();
    s.execute("sim.reset",json!({"world":"new-york","start":{"kind":"air","height_m":1200.,"airspeed_mps":40.,"heading_rad":1.}})).unwrap();
    set(
        &mut s,
        json!({"kind":"load_procedure","airport":"1N7","procedure":"R07","transition":"KIMKE"}),
    );
    set(&mut s, json!({"kind":"activate_approach"}));
    s.run(Step { steps: 1 }).unwrap();
    let receiver = s.sim().procedure_receiver();
    let hsi = &s.sim().systems().avionics.electronic_hsi;
    assert!(receiver.valid && hsi.gps_fix);
    assert_eq!(hsi.full_scale_nm, 0.3);
    assert_eq!(hsi.lateral_deviation_nm, Some(receiver.cross_track_nm));
    set(&mut s, json!({"kind":"power","unit":"electronic_hsi","on":false}));
    s.run(Step { steps: 1 }).unwrap();
    assert!(!s.sim().procedure_receiver().valid);
    assert!(!s.sim().systems().avionics.electronic_hsi.gps_fix);
}
