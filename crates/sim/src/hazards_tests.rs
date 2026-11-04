#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::hazards::*;
use crate::{Controls, Sim, Start};
use flightcraft_aero::Failures;

fn takeoff_roll(hazards: GroundHazards, seconds: u32) -> Sim {
    let mut sim = Sim::new(&Start::Runway).unwrap();
    sim.set_hazards(hazards);
    sim.set_controls(Controls {
        throttle: 1.,
        ..Default::default()
    });
    sim.step(120 * seconds);
    sim
}

#[test]
fn healthy_aircraft_accelerates_and_reads_airspeed() {
    let sim = takeoff_roll(GroundHazards::default(), 12);
    assert!(sim.readout().ground_speed_mps > 15.);
    assert!(sim.systems().sensed.indicated_airspeed_mps > 15.);
}

#[test]
fn pitot_cover_left_on_reads_zero_airspeed_in_flight() {
    let mut sim = Sim::new(&Start::Runway).unwrap();
    sim.set_hazards(GroundHazards {
        pitot_cover: true,
        ..Default::default()
    });
    sim.set_controls(Controls {
        throttle: 1.,
        ..Default::default()
    });
    sim.step(120 * 12);
    sim.set_controls(Controls {
        throttle: 1.,
        elevator: 0.3,
        ..Default::default()
    });
    sim.step(120 * 6);
    let r = sim.readout();
    assert!(!r.on_ground, "the aircraft must actually be airborne: {r:?}");
    assert!(r.true_airspeed_mps > 20., "{r:?}");
    assert!(r.failures.pitot_blocked);
    let shown = sim.systems().sensed.indicated_airspeed_mps;
    assert!(shown < 3., "a covered pitot must read about zero, not {shown}");
}

#[test]
fn chocks_and_ropes_keep_the_aircraft_where_it_is() {
    for hazards in [
        GroundHazards {
            chocks: true,
            ..Default::default()
        },
        GroundHazards {
            tie_downs: true,
            ..Default::default()
        },
    ] {
        let sim = takeoff_roll(hazards, 20);
        let r = sim.readout();
        assert!(r.ground_speed_mps < 0.1, "{hazards:?} {r:?}");
        assert!(r.on_ground);
        assert!(hazards.restrained());
    }
    let free = takeoff_roll(GroundHazards::default(), 20);
    assert!(!GroundHazards::default().restrained());
    assert!(free.readout().ground_speed_mps > 20.);
}

#[test]
fn hardware_defects_use_the_existing_failure_paths() {
    let mut f = Failures::default();
    GroundHazards {
        flap_hardware: true,
        aileron_hardware: true,
        ..Default::default()
    }
    .apply(&mut f, false);
    assert!(f.flap_asymmetry);
    assert_eq!(f.aileron, None, "an aileron only sticks once airborne");
    GroundHazards {
        aileron_hardware: true,
        ..Default::default()
    }
    .apply(&mut f, true);
    assert_eq!(f.aileron, Some(AILERON_HARDWARE_JAM));
}

#[test]
fn default_hazards_keep_the_golden_hash() {
    let mut a = Sim::new(&Start::Runway).unwrap();
    let mut b = Sim::new(&Start::Runway).unwrap();
    b.set_hazards(GroundHazards::default());
    a.step(240);
    b.step(240);
    assert_eq!(a.state_hash(), b.state_hash());
    b.set_hazards(GroundHazards {
        pitot_cover: true,
        ..Default::default()
    });
    assert_ne!(a.state_hash(), b.state_hash());
}
