#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
#[cfg(test)]
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/region_data.rs"));
use flightcraft_control::{Session, Step, read_region};
use flightcraft_sim::{
    autopilot::wrap,
    equipment::{LateralMode, VerticalMode},
};
use serde_json::{Value, json};
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
    s.install_region(
        read_region(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/terrain/new-york"))
            .unwrap(),
    )
    .unwrap();
    s.execute("sim.reset",json!({"world":"new-york","start":{"kind":"air","height_m":1000.,"airspeed_mps":40.,"heading_rad":0.}})).unwrap();
    // An airborne start is already aligned (a parked start's alignment is covered in tests/avionics.rs).
    s.execute(
        "assist.set",
        json!({"altitude_m":1000.,"heading_rad":0.,"airspeed_mps":40.}),
    )
    .unwrap();
    s.run(Step { steps: 5520 }).unwrap();
    s.execute("assist.off", json!({})).unwrap();
    panel(&mut s, json!({"kind":"engage","on":true}));
    panel(&mut s, json!({"kind":"vertical","mode":"alt"}));
    s
}
fn step(s: &mut Session, seconds: u32) {
    s.run(Step { steps: seconds * 120 }).unwrap();
    assert!(!s.sim().readout().crashed);
}
#[test]
fn glass_and_standby_share_bug_and_fly_hdg_alt_after_alignment() {
    require_region_data!();
    for aircraft in ["corvid-glass", "corvid-standby"] {
        let mut s = flight(aircraft);
        panel(&mut s, json!({"kind":"lateral","mode":"hdg"}));
        let altitude = s.sim().readout().altitude_msl_m;
        for degrees in [90.0_f64, 270.] {
            let expected = wrap(degrees.to_radians() - s.sim().readout().heading_rad).signum();
            set(&mut s, json!({"kind":"heading_bug","degrees":degrees}));
            step(&mut s, 12);
            let ap = s.sim().installed_autopilot().unwrap();
            assert_eq!(ap.command_bank_rad.signum(), expected, "{aircraft}: {ap:?}");
            assert!((ap.command_bank_rad.abs() - 25.0_f64.to_radians()).abs() < 0.01);
            step(&mut s, 108);
            let r = s.sim().readout();
            assert!(
                wrap(r.heading_rad - degrees.to_radians()).abs() < 3.0_f64.to_radians(),
                "{aircraft}: {r:?}"
            );
            assert!(
                (r.altitude_msl_m - altitude).abs() < 50. * 0.3048,
                "{aircraft}: {r:?}"
            );
        }
        set(&mut s, json!({"kind":"selected_altitude","feet":4900.}));
        assert!((s.glass_state().settings.selected_altitude_m - 1493.52).abs() < 1e-8);
        assert!(
            (s.sim()
                .systems()
                .avionics
                .electronic_attitude
                .selected_altitude_ft
                - 4900.)
                .abs()
                < 1e-8
        );
        let out = s.execute("avionics.state", json!({})).unwrap();
        assert_eq!(out["autopilot"]["active_lateral"], "hdg");
    }
}
#[test]
fn installed_sensors_disconnect_with_shared_suite_failures() {
    require_region_data!();
    for aircraft in ["corvid-glass", "corvid-standby"] {
        for air_data in [false, true] {
            let mut s = flight(aircraft);
            let fault = if aircraft == "corvid-glass" {
                json!({"kind":"glass","setting":{"kind":"failure","target":if air_data {"adc"} else {"ahrs"},"failed":true}})
            } else if air_data {
                json!({"kind":"air_data_failure","failed":true})
            } else {
                json!({"kind":"ahrs_failure","unit":"electronic_attitude","failed":true})
            };
            set(&mut s, fault);
            step(&mut s, 1);
            let ap = s.sim().installed_autopilot().unwrap();
            assert!(
                !ap.engaged && ap.ap_flash && ap.disconnect_tone,
                "{aircraft}: {ap:?}"
            );
            assert_eq!(ap.disconnect_sequence, 1);
            step(&mut s, 5);
            let ap = s.sim().installed_autopilot().unwrap();
            assert!(!ap.ap_flash && !ap.disconnect_tone);
            assert_eq!(ap.disconnect_sequence, 1);
        }
    }
}
#[test]
fn nav_reads_glass_plan_and_standby_receiver_and_loses_signal() {
    require_region_data!();
    for aircraft in ["corvid-glass", "corvid-standby"] {
        let mut s = flight(aircraft);
        let direct = if aircraft == "corvid-glass" {
            json!({"kind":"glass","setting":{"kind":"flight_plan","airports":["KMMU"]}})
        } else {
            json!({"kind":"direct_to","ident":"KMMU"})
        };
        set(&mut s, direct);
        panel(&mut s, json!({"kind":"lateral","mode":"nav"}));
        for second in 0..300 {
            step(&mut s, 1);
            if second > 150 {
                let cross = if aircraft == "corvid-glass" {
                    s.glass_state().gps.cross_track_nm
                } else {
                    s.sim()
                        .systems()
                        .avionics
                        .electronic_hsi
                        .lateral_deviation_nm
                        .unwrap()
                };
                assert!(cross.abs() < 0.1, "{aircraft}: cross-track={cross}");
            }
        }
        let ap = s.sim().installed_autopilot().unwrap();
        assert_eq!(ap.active_lateral, Some(LateralMode::Nav));
        assert_eq!(ap.active_vertical, Some(VerticalMode::Alt));
        if aircraft == "corvid-glass" {
            set(
                &mut s,
                json!({"kind":"glass","setting":{"kind":"failure","target":"mfd","failed":true}}),
            );
        } else {
            set(&mut s, json!({"kind":"power","unit":"electronic_hsi","on":false}));
        }
        step(&mut s, 1);
        assert!(
            !s.sim().installed_autopilot().unwrap().engaged,
            "{aircraft}: {:?}",
            s.sim().installed_autopilot()
        );
    }
}
