#![allow(clippy::unwrap_used)]

use flightcraft_sim::{
    Controls, Sim, Start,
    autopilot::{AirspeedMode, Targets, wrap},
};

fn disturbed() -> Sim {
    let trimmed = Sim::new(&Start::Air {
        height_m: 350.0,
        airspeed_mps: 36.0,
        heading_rad: 6.1,
    })
    .unwrap();
    let mut state = *trimmed.state();
    state.angular_rate_rps = [0.08, -0.04, 0.03];
    state.velocity_mps[2] = 1.0;
    Sim::from_state(state, trimmed.controls()).unwrap()
}

#[test]
fn altitude_heading_and_throttle_speed_converge_and_stay() {
    let mut sim = disturbed();
    sim.set_autopilot(Targets {
        altitude_m: Some(450.0),
        heading_rad: Some(0.3),
        airspeed_mps: Some(40.0),
        bank_limit_rad: 0.3,
        ..Targets::default()
    })
    .unwrap();
    for second in 0..180 {
        sim.step(120);
        let r = sim.readout();
        assert!(!r.crashed, "{r:?}");
        if second > 120 {
            assert!((r.altitude_msl_m - 450.0).abs() < 15.0, "{r:?}");
            assert!(wrap(r.heading_rad - 0.3).abs() < 3.0_f64.to_radians(), "{r:?}");
            assert!((r.true_airspeed_mps - 40.0).abs() < 3.0 * 0.514444, "{r:?}");
        }
    }
}

#[test]
fn vertical_speed_and_wings_level_converge_and_stay() {
    let mut sim = disturbed();
    sim.set_autopilot(Targets {
        vertical_speed_mps: Some(2.0),
        wings_level: true,
        airspeed_mps: Some(36.0),
        ..Targets::default()
    })
    .unwrap();
    for second in 0..120 {
        sim.step(120);
        let r = sim.readout();
        assert!(!r.crashed, "{r:?}");
        if second > 70 {
            assert!((r.vertical_speed_mps - 2.0).abs() < 0.3, "{r:?}");
            assert!(r.roll_rad.abs() < 0.02, "{r:?}");
            assert!((r.true_airspeed_mps - 36.0).abs() < 3.0 * 0.514444, "{r:?}");
        }
    }
}

#[test]
fn pitch_attitude_converges_and_does_not_own_pilot_throttle() {
    let mut sim = disturbed();
    let mut c = sim.controls();
    c.throttle = 0.8;
    c.flaps = 0.1;
    sim.set_controls(c);
    sim.set_autopilot(Targets {
        pitch_rad: Some(0.12),
        wings_level: true,
        ..Targets::default()
    })
    .unwrap();
    for second in 0..90 {
        sim.step(120);
        let r = sim.readout();
        assert!(!r.crashed, "{r:?}");
        assert_eq!(r.controls.throttle, 0.8);
        assert_eq!(r.controls.flaps, 0.1);
        if second > 45 {
            assert!((r.pitch_rad - 0.12).abs() < 0.01, "{r:?}");
        }
    }
}

#[test]
fn pitch_speed_climbs_at_full_power_and_stays_on_speed() {
    let mut sim = disturbed();
    sim.set_autopilot(Targets {
        airspeed_mps: Some(36.0),
        airspeed_mode: AirspeedMode::Pitch,
        wings_level: true,
        ..Targets::default()
    })
    .unwrap();
    for second in 0..120 {
        sim.step(120);
        let r = sim.readout();
        assert!(!r.crashed, "{r:?}");
        assert_eq!(r.controls.throttle, 1.0);
        if second > 60 {
            assert!((r.true_airspeed_mps - 36.0).abs() < 3.0 * 0.514444, "{r:?}");
            assert!(r.vertical_speed_mps > 1.0, "{r:?}");
        }
    }
}

#[test]
fn hostile_and_conflicting_targets_are_atomic_errors() {
    let mut sim = disturbed();
    let original = sim.readout();
    for value in [f64::NAN, f64::INFINITY, -1e100, 1e100] {
        for targets in [
            Targets {
                pitch_rad: Some(value),
                ..Targets::default()
            },
            Targets {
                altitude_m: Some(value),
                ..Targets::default()
            },
            Targets {
                vertical_speed_mps: Some(value),
                ..Targets::default()
            },
            Targets {
                heading_rad: Some(value),
                ..Targets::default()
            },
            Targets {
                airspeed_mps: Some(value),
                ..Targets::default()
            },
            Targets {
                bank_limit_rad: value,
                ..Targets::default()
            },
        ] {
            assert!(sim.set_autopilot(targets).is_err());
            assert_eq!(sim.readout(), original);
        }
    }
    assert!(
        sim.set_autopilot(Targets {
            pitch_rad: Some(0.1),
            altitude_m: Some(300.0),
            ..Targets::default()
        })
        .is_err()
    );
    assert!(
        sim.set_autopilot(Targets {
            heading_rad: Some(0.0),
            wings_level: true,
            ..Targets::default()
        })
        .is_err()
    );
    assert!(
        sim.set_autopilot(Targets {
            altitude_m: Some(300.0),
            airspeed_mps: Some(36.0),
            airspeed_mode: AirspeedMode::Pitch,
            ..Targets::default()
        })
        .is_err()
    );
    sim.set_controls(Controls::default());
}

#[test]
fn disengaged_holds_preserve_the_pilot_trajectory_and_fixed_step_batching() {
    let mut pilot = disturbed();
    let mut off = pilot.clone();
    off.set_autopilot(Targets {
        heading_rad: Some(0.2),
        ..Targets::default()
    })
    .unwrap();
    off.autopilot_off();
    pilot.step(1200);
    for _ in 0..1200 {
        off.step(1);
    }
    assert_eq!(pilot.state(), off.state());
    assert_eq!(pilot.readout(), off.readout());
    // Selecting HDG moves the shared bug; disengaging leaves that cockpit selection in place.
    assert!((off.systems().controls.heading_bug_rad - 0.2).abs() < 1e-12);
    assert_ne!(pilot.state_hash(), off.state_hash());
    let mut a = disturbed();
    a.set_autopilot(Targets {
        altitude_m: Some(400.0),
        heading_rad: Some(0.2),
        airspeed_mps: Some(40.0),
        ..Targets::default()
    })
    .unwrap();
    let mut b = a.clone();
    a.step(1200);
    for _ in 0..1200 {
        b.step(1);
    }
    assert_eq!(a.readout(), b.readout());
    assert_eq!(a.state_hash(), b.state_hash());
}
