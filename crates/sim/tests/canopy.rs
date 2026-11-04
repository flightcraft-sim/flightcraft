#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_aero::AircraftCatalog;
use flightcraft_sim::{
    Controls, Failures, Sim, Start,
    canopy::{CanopyAction as A, CanopyState},
};
fn sliding(start: Start) -> Sim {
    let mut config = AircraftCatalog::builtins()
        .unwrap()
        .resolve_aircraft("corvid")
        .unwrap();
    config
        .installed_systems
        .definition
        .as_mut()
        .unwrap()
        .sliding_canopy = true;
    let mut sim = Sim::new(&start).unwrap();
    sim.configure_systems(&config).unwrap();
    sim
}
#[test]
fn canopy_parked_runway_air_starts_and_atomic_guards() {
    let mut sim = sliding(Start::ParkedAt {
        position_m: [50., 50., 0.],
        heading_rad: 0.,
    });
    assert_eq!(
        sim.canopy(),
        CanopyState {
            sliding: true,
            open_fraction: 1.,
            latched: false
        }
    );
    assert!(!sim.failures().door_open);
    let before = sim.canopy();
    assert!(sim.set_canopy(A::Latch, None).is_err());
    assert_eq!(sim.canopy(), before);
    for v in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
        assert!(sim.set_canopy(A::Open, Some(v)).is_err());
        assert_eq!(sim.canopy(), before);
    }
    assert!(sim.set_canopy(A::Close, Some(0.)).is_err());
    sim.set_canopy(A::Open, Some(0.5)).unwrap();
    assert_eq!(sim.canopy().open_fraction, 0.5);
    sim.set_canopy(A::Close, None).unwrap();
    assert!(!sim.canopy().latched);
    sim.set_canopy(A::Latch, None).unwrap();
    assert!(sim.canopy().secure());
    assert!(
        sim.set_canopy(A::Open, None)
            .unwrap_err()
            .to_string()
            .contains("unlatch")
    );
    for start in [
        Start::Runway,
        Start::Air {
            height_m: 1000.,
            airspeed_mps: 40.,
            heading_rad: 0.,
        },
    ] {
        let mut sim = sliding(start);
        assert!(sim.canopy().secure());
        sim.set_canopy(A::Unlatch, None).unwrap();
        if !sim.readout().on_ground {
            let before = sim.canopy();
            assert!(
                sim.set_canopy(A::Open, None)
                    .unwrap_err()
                    .to_string()
                    .contains("ground")
            );
            assert_eq!(sim.canopy(), before);
        }
    }
    let mut unsupported = Sim::new(&Start::Runway).unwrap();
    assert!(
        unsupported
            .set_canopy(A::Unlatch, None)
            .unwrap_err()
            .to_string()
            .contains("no sliding canopy")
    );
}
#[test]
fn canopy_speed_guard_and_unlatched_flight_use_existing_failure_drag() {
    let mut fast = sliding(Start::Runway);
    fast.set_canopy(A::Unlatch, None).unwrap();
    fast.set_controls(Controls {
        throttle: 1.,
        brake: 0.,
        ..Controls::default()
    });
    for _ in 0..1800 {
        fast.step(1);
        if fast.readout().ground_speed_mps > 2.1 {
            break;
        }
    }
    assert!(fast.readout().on_ground && fast.readout().ground_speed_mps > 2.);
    assert!(
        fast.set_canopy(A::Open, None)
            .unwrap_err()
            .to_string()
            .contains("speed")
    );
    let mut canopy = sliding(Start::Air {
        height_m: 1000.,
        airspeed_mps: 40.,
        heading_rad: 0.,
    });
    let mut door = canopy.clone();
    canopy.set_canopy(A::Unlatch, None).unwrap();
    door.set_failures(Failures {
        door_open: true,
        ..Failures::default()
    })
    .unwrap();
    assert!(canopy.readout().failures.door_open);
    assert!(!canopy.injected_failures().door_open);
    canopy.step(120);
    door.step(120);
    assert_eq!(canopy.state(), door.state());
    let h = canopy.state_hash();
    let mut repeat = canopy.clone();
    repeat.step(120);
    for _ in 0..120 {
        canopy.step(1);
    }
    assert_ne!(h, canopy.state_hash());
    assert_eq!(canopy.state_hash(), repeat.state_hash());
    canopy.set_canopy(A::Latch, None).unwrap();
    assert!(!canopy.failures().door_open);
}
