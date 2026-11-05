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

fn chocked(extra: GroundHazards) -> GroundHazards {
    GroundHazards {
        chocks: true,
        ..extra
    }
}

fn run_static(hazards: GroundHazards, seconds: u32) -> Sim {
    takeoff_roll(chocked(hazards), seconds)
}

#[test]
fn low_oil_drops_pressure_heats_the_oil_and_wears_the_engine() {
    let healthy = run_static(GroundHazards::default(), 240);
    let low = run_static(
        GroundHazards {
            low_oil: true,
            ..Default::default()
        },
        240,
    );
    let (h, l) = (healthy.systems(), low.systems());
    assert!(h.oil.fill > 0.99 && h.oil.wear == 0.0, "{:?}", h.oil);
    assert!(
        l.oil.fill < 0.85,
        "the finding leaves the tank under MIN: {:?}",
        l.oil
    );
    assert!(l.oil.wear > 0.0);
    assert!(
        l.oil_pressure_pa < 0.8 * h.oil_pressure_pa,
        "{} vs {}",
        l.oil_pressure_pa,
        h.oil_pressure_pa
    );
    assert!(l.oil_temperature_c > h.oil_temperature_c + 5.0);
    assert!(l.engine_running, "four minutes is not enough to seize it");
}

#[test]
fn low_oil_flown_long_enough_seizes_the_engine_and_stays_seized() {
    let mut sim = Sim::new(&Start::Runway).unwrap();
    sim.set_hazards(chocked(GroundHazards {
        low_oil: true,
        ..Default::default()
    }));
    sim.set_controls(Controls {
        throttle: 1.,
        ..Default::default()
    });
    let mut minutes = 0;
    while !sim.systems().oil.seized && minutes < 120 {
        sim.step(120 * 60);
        minutes += 1;
    }
    let s = sim.systems();
    assert!(s.oil.seized, "wear {}", s.oil.wear);
    assert!(minutes > 10, "damage must take a while, not {minutes} min");
    assert!(!s.engine_running);
    assert_eq!(s.engine_power_fraction, 0.0);
    // Topping up fixes the oil but not the engine.
    sim.set_hazards(chocked(GroundHazards::default()));
    assert!((sim.systems().oil.fill - 1.0).abs() < 1e-12);
    assert!(sim.systems().oil.seized);
    sim.step(120 * 5);
    assert!(!sim.systems().engine_running);
}

#[test]
fn topping_up_the_oil_restores_the_level_and_stops_the_wear() {
    let mut sim = run_static(
        GroundHazards {
            low_oil: true,
            ..Default::default()
        },
        60,
    );
    let wear = sim.systems().oil.wear;
    assert!(wear > 0.0);
    sim.set_hazards(chocked(GroundHazards::default()));
    assert!((sim.systems().oil.fill - 1.0).abs() < 1e-12);
    sim.step(120 * 60);
    assert_eq!(sim.systems().oil.wear, wear);
}

#[test]
fn a_full_tank_keeps_the_original_oil_numbers() {
    let a = run_static(GroundHazards::default(), 60);
    let s = a.systems();
    assert_eq!(s.oil.wear, 0.0);
    assert!(!s.oil.disturbed);
    assert!(s.oil_pressure_pa > 300_000.0, "{}", s.oil_pressure_pa);
}

#[test]
fn damaged_tyre_rolls_harder_and_pulls_to_its_side() {
    let run = |tyre: Option<flightcraft_aero::Wheel>| {
        let mut sim = Sim::new(&Start::Runway).unwrap();
        sim.set_hazards(GroundHazards {
            damaged_tyre: tyre,
            ..Default::default()
        });
        let start_heading = sim.readout().heading_rad;
        sim.set_controls(Controls {
            throttle: 0.6,
            ..Default::default()
        });
        sim.step(120 * 8);
        let r = sim.readout();
        let turned = (r.heading_rad - start_heading + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU)
            - std::f64::consts::PI;
        (r.ground_speed_mps, turned)
    };
    let (v0, h0) = run(None);
    let (vl, hl) = run(Some(flightcraft_aero::Wheel::Left));
    let (vr, hr) = run(Some(flightcraft_aero::Wheel::Right));
    assert!(vl < v0 && vr < v0, "{v0} {vl} {vr}");
    assert!(hl - h0 < -0.01, "a left tyre pulls left: {h0} {hl}");
    assert!(hr - h0 > 0.01, "a right tyre pulls right: {h0} {hr}");
}

#[test]
fn damaged_tyre_wears_out_and_bursts_on_the_landing_roll() {
    let mut tyre = TyreWear::default();
    // Taxi 200 m, then a 350 m take-off roll.
    for _ in 0..40 {
        tyre.step(1.0, 5., None);
    }
    for _ in 0..14 {
        tyre.step(1.0, 25., None);
    }
    assert!(!tyre.flat && tyre.integrity > 0.3, "{tyre:?}");
    // A normal landing leaves it close; the roll-out bursts it.
    tyre.step(0.01, 30., Some(0.6));
    assert!(!tyre.flat, "{tyre:?}");
    let mut rolled = 0.;
    while !tyre.flat && rolled < 600. {
        tyre.step(1.0, 20., None);
        rolled += 20.;
    }
    assert!(tyre.flat && rolled <= 300., "burst after {rolled} m");
    // A firm landing on a fresh damaged tyre bursts it at once.
    let mut fresh = TyreWear::default();
    fresh.step(0.01, 30., Some(1.4));
    assert!(fresh.flat);
}

#[test]
fn burst_tyre_drags_far_harder_than_a_damaged_one() {
    let mut f = Failures::default();
    GroundHazards {
        damaged_tyre: Some(flightcraft_aero::Wheel::Right),
        ..Default::default()
    }
    .apply(&mut f, false);
    assert_eq!(f.damaged_tyre, Some(flightcraft_aero::Wheel::Right));
    assert!(!f.tyre_flat);
    let speed_after = |flat: bool| {
        let mut sim = Sim::new(&Start::Runway).unwrap();
        sim.set_hazards(GroundHazards {
            damaged_tyre: Some(flightcraft_aero::Wheel::Right),
            ..Default::default()
        });
        sim.force_tyre_flat(flat);
        sim.set_controls(Controls {
            throttle: 0.6,
            ..Default::default()
        });
        sim.step(120 * 8);
        sim.readout().ground_speed_mps
    };
    assert!(speed_after(true) < speed_after(false) - 1.0);
}

#[test]
fn damaged_tyre_in_the_sim_wears_with_the_roll() {
    let sim = takeoff_roll(
        GroundHazards {
            damaged_tyre: Some(flightcraft_aero::Wheel::Left),
            ..Default::default()
        },
        10,
    );
    let wear = sim.tyre_wear();
    assert!(wear.integrity < 1.0, "{wear:?}");
}

fn fly_fast(hazards: GroundHazards, seconds: u32) -> Sim {
    fly_at(hazards, 0.8, seconds)
}

fn fly_at(hazards: GroundHazards, throttle: f64, seconds: u32) -> Sim {
    let mut sim = Sim::new(&Start::Air {
        height_m: 600.,
        airspeed_mps: 52.,
        heading_rad: 0.,
    })
    .unwrap();
    sim.set_hazards(hazards);
    sim.set_controls(Controls {
        throttle,
        ..Default::default()
    });
    sim.step(120 * seconds);
    sim
}

#[test]
fn loose_cowl_vibrates_under_power_and_comes_off_at_speed() {
    let loose = GroundHazards {
        loose_cowl: true,
        ..Default::default()
    };
    let early = fly_fast(loose, 5);
    assert!(early.systems().cowl.vibration);
    assert!(!early.systems().cowl.separated);
    let late = fly_fast(loose, 150);
    assert!(late.systems().cowl.separated);
    assert!(late.failures().cowl_separated);
    assert!(!late.systems().cowl.vibration);
    let healthy = fly_fast(GroundHazards::default(), 150);
    assert!(!healthy.systems().cowl.separated);
    // Open engine bay: more drag and hotter.
    assert!(late.systems().cht_c > healthy.systems().cht_c + 20.);
    assert!(late.readout().true_airspeed_mps < healthy.readout().true_airspeed_mps);
}

#[test]
fn loose_cowl_on_the_ground_never_comes_off() {
    let sim = run_static(
        GroundHazards {
            loose_cowl: true,
            ..Default::default()
        },
        120,
    );
    assert!(sim.systems().cowl.vibration);
    assert!(!sim.systems().cowl.separated);
    assert_eq!(cowl_stress_rate(0.), 0.);
    assert!(cowl_stress_rate(60.) > cowl_stress_rate(45.));
}

#[test]
fn cracked_fairing_adds_a_little_drag_only() {
    let cracked = GroundHazards {
        fairing_crack: true,
        ..Default::default()
    };
    // Coast with the engine at idle so drag alone sets the speed.
    let a = fly_at(GroundHazards::default(), 0.0, 20);
    let b = fly_at(cracked, 0.0, 20);
    // Drag shows as lost total energy (height plus speed head), whatever the phugoid is doing.
    let energy = |s: &Sim| {
        let r = s.readout();
        r.altitude_msl_m + r.true_airspeed_mps * r.true_airspeed_mps / (2. * 9.80665)
    };
    let (va, vb) = (energy(&a), energy(&b));
    assert!(vb < va - 0.2, "{va} {vb}");
    assert!(va - vb < 15., "a cosmetic crack must not matter much: {va} {vb}");
    assert!(b.systems().engine_running);
}

#[test]
fn new_findings_leave_older_hashes_alone_when_absent() {
    let base = GroundHazards {
        chocks: true,
        ..Default::default()
    };
    assert_eq!(base.hash_words().len(), 5);
    let with = GroundHazards {
        fairing_crack: true,
        ..base
    };
    assert_eq!(with.hash_words().len(), 9);
    let mut b = Sim::new(&Start::Runway).unwrap();
    b.set_hazards(base);
    let mut c = Sim::new(&Start::Runway).unwrap();
    c.set_hazards(with);
    b.step(120);
    c.step(120);
    assert_ne!(b.state_hash(), c.state_hash());
}
