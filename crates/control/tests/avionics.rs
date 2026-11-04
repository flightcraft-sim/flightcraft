#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
#[cfg(test)]
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/region_data.rs"));
use flightcraft_control::{Session, read_region};
use flightcraft_sim::avionics::{AvionicsSetting, Unit};
use serde_json::{Value, json};
fn standby() -> Session {
    let mut s = Session::new().unwrap();
    s.execute(
        "aircraft.load",
        json!({"source":{"kind":"installed","id":"corvid-standby"},"reset":true}),
    )
    .unwrap();
    s.execute("sim.set_controls", json!({"brake":1.})).unwrap();
    s
}
fn set(s: &mut Session, setting: Value) -> Value {
    s.execute("avionics.set", json!({"setting":setting})).unwrap()
}
#[test]
fn commands_are_atomic_schema_checked_and_observations_consume_no_time() {
    let mut s = standby();
    let hash = s.sim().state_hash();
    let before = s.execute("avionics.state", json!({})).unwrap();
    assert!(before["installed"].as_bool().unwrap());
    assert!(before["electronic_attitude"]["aligning"].as_bool().unwrap());
    assert!(!before["electronic_hsi"]["gps_fix"].as_bool().unwrap());
    for _ in 0..100 {
        assert_eq!(before, s.execute("avionics.state", json!({})).unwrap());
    }
    assert_eq!(hash, s.sim().state_hash());
    let commands = Session::commands();
    for setting in [
        json!({"kind":"heading_bug","degrees":127.}),
        json!({"kind":"course","degrees":85.}),
        json!({"kind":"baro","pressure_pa":100500.}),
        json!({"kind":"selected_altitude","feet":4500.}),
        json!({"kind":"reversion","enabled":true}),
        json!({"kind":"air_data_failure","failed":true}),
        json!({"kind":"ahrs_failure","unit":"electronic_attitude","failed":true}),
        json!({"kind":"power","unit":"electronic_attitude","on":false}),
    ] {
        let command = commands.iter().find(|c| c.name == "avionics.set").unwrap();
        let input = json!({"setting":setting});
        assert!(
            jsonschema::validator_for(&command.input_schema)
                .unwrap()
                .is_valid(&input)
        );
        let output = s.execute("avionics.set", input).unwrap();
        assert!(
            jsonschema::validator_for(&command.output_schema)
                .unwrap()
                .is_valid(&output)
        );
        assert_eq!(s.sim().state().time_s, 0.);
        assert_eq!(
            output["electronic_attitude"]["power"]["battery_remaining_s"],
            before["electronic_attitude"]["power"]["battery_remaining_s"]
        );
    }
    let hash = s.sim().state_hash();
    for setting in [
        json!({"kind":"course","degrees":-1}),
        json!({"kind":"selected_altitude","feet":30001}),
        json!({"kind":"direct_to","ident":"UNKNOWN"}),
        json!({"kind":"baro","pressure_pa":0}),
        json!({"kind":"power","unit":"invalid","on":false}),
        json!({"kind":"course","degrees":2,"extra":true}),
    ] {
        assert!(s.execute("avionics.set", json!({"setting":setting})).is_err());
        assert_eq!(hash, s.sim().state_hash());
    }
    assert!(
        s.sim_mut()
            .set_avionics(AvionicsSetting::Course { degrees: f64::NAN })
            .is_err()
    );
}
#[test]
fn alignment_bus_switchover_pitot_hold_and_independent_reversion() {
    let mut s = standby();
    s.execute("sim.step", json!({"steps":5399})).unwrap();
    assert!(!s.sim().systems().avionics.electronic_attitude.attitude_valid);
    s.execute("sim.step", json!({"steps":1})).unwrap();
    assert!(s.sim().systems().avionics.electronic_attitude.attitude_valid);
    assert!(
        s.sim()
            .systems()
            .avionics
            .electronic_hsi
            .reversion_attitude
            .attitude_valid
    );
    let energy = s
        .sim()
        .systems()
        .avionics
        .electronic_attitude
        .power
        .battery_remaining_s;
    assert_eq!(energy, 3600.);
    s.execute(
        "systems.set",
        json!({"setting":{"kind":"switch","switch":"master","on":false}}),
    )
    .unwrap();
    s.execute("sim.step", json!({"steps":120})).unwrap();
    let a = &s.sim().systems().avionics.electronic_attitude;
    assert!(a.power.on_battery && a.attitude_valid);
    assert!((a.power.battery_remaining_s - 3599.).abs() < 1e-6);
    let output = set(
        &mut s,
        json!({"kind":"ahrs_failure","unit":"electronic_attitude","failed":true}),
    );
    assert!(!output["electronic_attitude"]["attitude_valid"].as_bool().unwrap());
    assert!(output["electronic_hsi"]["reversion"].as_bool().unwrap());
    assert!(
        output["electronic_hsi"]["reversion_attitude"]["attitude_valid"]
            .as_bool()
            .unwrap()
    );
    let output = set(
        &mut s,
        json!({"kind":"power","unit":"electronic_attitude","on":false}),
    );
    assert!(
        !output["electronic_attitude"]["power"]["powered"]
            .as_bool()
            .unwrap()
    );
    let energy = s
        .sim()
        .systems()
        .avionics
        .electronic_attitude
        .power
        .battery_remaining_s;
    s.execute("sim.step", json!({"steps":120})).unwrap();
    assert_eq!(
        energy,
        s.sim()
            .systems()
            .avionics
            .electronic_attitude
            .power
            .battery_remaining_s
    );
    assert!(!s.sim().readout().crashed);
}
#[test]
fn new_york_direct_to_geodetic_distance_bearing_and_terminal_scaling() {
    require_region_data!();
    let mut s = standby();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/terrain/new-york");
    s.install_region(read_region(&dir).unwrap()).unwrap();
    s.execute(
        "sim.reset",
        json!({"world":"new-york","start":{"runway":{"airport":"1N7","end":"07"}}}),
    )
    .unwrap();
    let output = set(&mut s, json!({"kind":"direct_to","ident":"KMMU"}));
    let dest = s
        .sim()
        .world()
        .airports()
        .unwrap()
        .airports
        .iter()
        .find(|a| a.ident == "MMU")
        .unwrap()
        .clone();
    let from = s.sim().readout();
    // Independent spherical haversine and forward azimuth, without the production inverse.
    let p = from.latitude_deg.unwrap().to_radians();
    let q = dest.latitude_deg.to_radians();
    let dl = (dest.longitude_deg - from.longitude_deg.unwrap()).to_radians();
    let hav = libm::pow(libm::sin((q - p) / 2.), 2.)
        + libm::cos(p) * libm::cos(q) * libm::pow(libm::sin(dl / 2.), 2.);
    let nm = 2. * 6371008.8 * libm::asin(libm::sqrt(hav)) / 1852.;
    let brg = libm::atan2(
        libm::sin(dl) * libm::cos(q),
        libm::cos(p) * libm::sin(q) - libm::sin(p) * libm::cos(q) * libm::cos(dl),
    )
    .to_degrees()
    .rem_euclid(360.);
    let h = &output["electronic_hsi"];
    assert!((h["distance_nm"].as_f64().unwrap() - nm).abs() < 0.1);
    assert!(
        (h["bearing_deg"].as_f64().unwrap()
            - (brg - from.magnetic_declination_rad.to_degrees()).rem_euclid(360.))
        .abs()
            < 1.
    );
    assert_eq!(h["destination"], "MMU");
    assert_eq!(h["full_scale_nm"], if nm <= 30. { 1. } else { 5. });
    assert_eq!(h["cdi"], 0.);
    s.execute("sim.reset",json!({"world":"new-york","start":{"air":{"airport":"MMU","distance_m":18520.,"bearing_deg":270.,"height_m":1000.,"airspeed_mps":45.,"heading_rad":0.}}})).unwrap();
    let output = set(&mut s, json!({"kind":"direct_to","ident":"KMMU"}));
    assert_eq!(output["electronic_hsi"]["full_scale_nm"], 1.);
    assert!((output["electronic_hsi"]["distance_nm"].as_f64().unwrap() - 10.).abs() < 0.02);
    let before = s.execute("avionics.state", json!({})).unwrap();
    assert!(
        s.execute(
            "avionics.set",
            json!({"setting":{"kind":"direct_to","ident":"KZZZ"}})
        )
        .is_err()
    );
    assert_eq!(before, s.execute("avionics.state", json!({})).unwrap());
    s.sim_mut()
        .set_avionics(AvionicsSetting::Power {
            unit: Unit::ElectronicHsi,
            on: false,
        })
        .unwrap();
    assert!(!s.sim().systems().avionics.electronic_hsi.gps_fix);
}

#[test]
fn mcp_reports_the_same_failure_flags_as_the_panel() {
    let mut s = standby();
    set(
        &mut s,
        json!({"kind":"ahrs_failure","unit":"electronic_attitude","failed":true}),
    );
    let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"avionics_state","arguments":{}}});
    let response: Value =
        serde_json::from_str(&flightcraft_control::mcp::handle_line(&mut s, &request.to_string()).unwrap())
            .unwrap();
    assert_eq!(response["result"]["isError"], false);
    assert_eq!(
        response["result"]["structuredContent"],
        s.execute("avionics.state", json!({})).unwrap()
    );
    assert!(
        response["result"]["structuredContent"]["electronic_hsi"]["reversion"]
            .as_bool()
            .unwrap()
    );
}

#[test]
fn runway_reset_is_ready_for_departure_without_advancing_time() {
    for id in ["corvid-glass", "corvid-standby"] {
        let mut s = Session::new().unwrap();
        s.execute(
            "aircraft.load",
            json!({"source":{"kind":"installed","id":id},"reset":true}),
        )
        .unwrap();
        for _ in 0..2 {
            s.execute("sim.reset", json!({"start":{"kind":"runway"}}))
                .unwrap();
            assert_eq!(s.sim().state().time_s, 0.);
            let systems = s.sim().systems();
            assert!(systems.engine_running && systems.controls.master && systems.controls.avionics);
            if id == "corvid-glass" {
                let state = s.glass_state();
                assert!(state.air_data.powered && state.air_data.attitude_valid);
                assert!(state.standby.powered && state.standby.attitude_valid);
                assert_eq!(state.air_data.alignment_remaining_s, 0.);
                assert_eq!(state.standby.alignment_remaining_s, 0.);
                assert!(!state.pfd_failed);
            } else {
                let state = s.execute("avionics.state", json!({})).unwrap();
                assert_eq!(state["electronic_attitude"]["attitude_valid"], true);
                assert_eq!(state["electronic_attitude"]["aligning"], false);
                assert_eq!(
                    state["electronic_hsi"]["reversion_attitude"]["attitude_valid"],
                    true
                );
            }
            s.execute(
                "systems.set",
                json!({"setting":{"kind":"switch","switch":"avionics","on":false}}),
            )
            .unwrap();
        }
    }
}

#[test]
fn named_airport_runway_reset_aligns_sources_at_the_runway_attitude() {
    let terrain = if let Some(path) = std::env::var_os("FLIGHTCRAFT_TEST_TERRAIN") {
        std::path::PathBuf::from(path)
    } else {
        require_region_data!();
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/terrain")
    };
    let root = terrain.join("eastern-pa");
    let mut s = Session::new().unwrap();
    s.install_region(read_region(&root).unwrap()).unwrap();
    s.execute(
        "aircraft.load",
        json!({"source":{"kind":"installed","id":"corvid-glass"}}),
    )
    .unwrap();
    s.execute(
        "sim.reset",
        json!({"world":"eastern-pa","start":{"kind":"airport_runway","airport":"1N7","end":"07"}}),
    )
    .unwrap();
    let state = s.glass_state();
    assert!(state.air_data.attitude_valid && state.standby.attitude_valid);
    assert!((state.air_data.heading_rad - s.sim().readout().heading_rad).abs() < 1e-6);
    assert_eq!(s.sim().state().time_s, 0.);
}

#[test]
fn airborne_start_is_aligned_and_running_but_a_parked_start_aligns_over_time() {
    let mut s = Session::new().unwrap();
    s.execute(
        "aircraft.load",
        json!({"source":{"kind":"installed","id":"corvid-glass"},"reset":true}),
    )
    .unwrap();
    s.execute(
        "sim.reset",
        json!({"start":{"kind":"air","height_m":900.,"airspeed_mps":52.,"heading_rad":1.047},"paused":false}),
    )
    .unwrap();
    assert_eq!(s.sim().state().time_s, 0.);
    let systems = s.sim().systems();
    assert!(systems.engine_running && systems.controls.master && systems.controls.avionics);
    let state = s.glass_state();
    for unit in [&state.air_data, &state.standby] {
        assert!(unit.powered && unit.attitude_valid, "{unit:?}");
        assert_eq!(unit.alignment_remaining_s, 0.);
    }
    assert!(!state.pfd_failed);
    // A cold, parked start keeps the realistic alignment delay.
    s.execute(
        "sim.reset",
        json!({"start":{"kind":"parked","north_m":0.,"east_m":0.,"heading_rad":0.5}}),
    )
    .unwrap();
    s.execute(
        "systems.set",
        json!({"setting":{"kind":"switch","switch":"avionics","on":true}}),
    )
    .unwrap();
    s.execute("sim.step", json!({"steps":10})).unwrap();
    let state = s.glass_state();
    assert!(!state.air_data.attitude_valid && state.air_data.alignment_remaining_s > 0.);
    s.execute("sim.step", json!({"steps":5400})).unwrap();
    assert!(s.glass_state().air_data.attitude_valid);
}
