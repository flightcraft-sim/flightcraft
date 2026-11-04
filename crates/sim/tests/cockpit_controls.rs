//! The throttle, flap, hand brake, and park-brake settings reach the same pilot controls that
//! `sim.set_controls`, the keyboard, and the dash levers use.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_sim::systems::SystemSetting;
use flightcraft_sim::{Sim, Start};

fn parked() -> Sim {
    let mut sim = Sim::new(&Start::Runway).unwrap();
    sim.step(120);
    sim
}

#[test]
fn throttle_flap_and_brake_settings_set_the_pilot_controls() {
    let mut sim = parked();
    for (setting, check) in [
        (
            SystemSetting::Throttle { value: 0.7 },
            (|c: flightcraft_sim::Controls| c.throttle) as fn(_) -> f64,
        ),
        (SystemSetting::Flaps { value: 0.5 }, |c| c.flaps),
        (SystemSetting::WheelBrake { value: 1. }, |c| c.brake),
    ] {
        let before = sim.controls();
        sim.set_systems(setting).unwrap();
        assert!(check(sim.controls()) > check(before));
    }
    // The other controls were not disturbed.
    let c = sim.controls();
    assert!((c.throttle - 0.7).abs() < 1e-9 && c.flaps > 0.4 && c.brake > 0.9);
    for bad in [-0.1, 1.1, f64::NAN] {
        assert!(sim.set_systems(SystemSetting::Throttle { value: bad }).is_err());
        assert!(sim.set_systems(SystemSetting::Flaps { value: bad }).is_err());
        assert!(sim.set_systems(SystemSetting::WheelBrake { value: bad }).is_err());
    }
    assert_eq!(sim.controls(), c, "a rejected setting changes nothing");
}

#[test]
fn the_park_brake_holds_the_wheels_with_the_lever_released_and_replays() {
    let roll = |park: bool| {
        let mut sim = parked();
        sim.set_systems(SystemSetting::ParkBrake { on: park }).unwrap();
        sim.set_systems(SystemSetting::Throttle { value: 0.5 }).unwrap();
        sim.step(600);
        (
            sim.readout().ground_speed_mps,
            sim.state_hash(),
            sim.controls().brake,
        )
    };
    let (free, _, _) = roll(false);
    let (held, hash, lever) = roll(true);
    assert!(free > 3. && held < 1., "free {free}, parked {held}");
    assert!(lever < 0.01, "the lever itself is released");
    assert_eq!(roll(true).1, hash, "deterministic");
    let mut sim = parked();
    let clean = sim.state_hash();
    sim.set_systems(SystemSetting::ParkBrake { on: true }).unwrap();
    assert_ne!(sim.state_hash(), clean, "the valve is part of the hashed state");
    assert!(sim.systems().controls.park_brake);
}
