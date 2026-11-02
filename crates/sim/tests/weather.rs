#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_aero as aero;
use flightcraft_sim::{
    Failures, Sim, Start, TurbulenceLevel, Weather,
    autopilot::{AirspeedMode, Targets},
};

fn air(weather: Weather) -> Sim {
    Sim::new_with_weather(
        &Start::Air {
            height_m: 1000.0,
            airspeed_mps: 38.0,
            heading_rad: 0.0,
        },
        weather,
    )
    .unwrap()
}

#[test]
fn hands_off_glide_drifts_with_the_air_mass() {
    let mut calm = air(Weather::default());
    let mut windy = air(Weather {
        wind_from_deg: 270.0,
        wind_speed_mps: 5.0,
        ..Weather::default()
    });
    for sim in [&mut calm, &mut windy] {
        sim.set_failures(Failures {
            engine_power_fraction: 0.0,
            ..Failures::default()
        })
        .unwrap();
        sim.step(120 * 60);
        assert!(!sim.readout().crashed);
        assert!(sim.readout().vertical_speed_mps < 0.0);
    }
    let drift = windy.state().position_m[1] - calm.state().position_m[1];
    assert!((drift / (5.0 * 60.0) - 1.0).abs() < 0.05, "drift {drift}");
    assert!((calm.readout().true_airspeed_mps - windy.readout().true_airspeed_mps).abs() < 1e-8);
    assert!((windy.readout().wind_ned_mps[1] - 5.0).abs() < 1e-12);
}

#[test]
fn modest_crosswind_takeoff_with_holds_stays_on_the_runway() {
    for from in [90.0, 270.0] {
        let mut sim = Sim::new_with_weather(
            &Start::Runway,
            Weather {
                wind_from_deg: from,
                wind_speed_mps: 3.0,
                ..Weather::default()
            },
        )
        .unwrap();
        sim.set_controls(flightcraft_sim::Controls {
            throttle: 1.0,
            ..Default::default()
        });
        let mut airborne = false;
        for _ in 0..400 {
            let r = sim.readout();
            sim.set_autopilot(Targets {
                heading_rad: Some(0.0),
                pitch_rad: Some(if r.true_airspeed_mps < 28.0 { 0.0 } else { 0.22 }),
                ..Targets::default()
            })
            .unwrap();
            sim.step(12);
            let r = sim.readout();
            assert!(!r.crashed, "{r:?}");
            let [north, east, _] = sim.state().position_m;
            assert!(east.abs() < 11.5, "off runway at {north}, {east}: {r:?}");
            if r.height_agl_m > 5.0 {
                assert!(north < 900.0);
                airborne = true;
                break;
            }
        }
        assert!(airborne, "{:?}", sim.readout());
    }
}

#[test]
fn seeded_gusts_replay_across_batching_readouts_and_clones() {
    let weather = Weather {
        turbulence: TurbulenceLevel::Moderate,
        seed: 42,
        ..Weather::default()
    };
    let mut a = air(weather);
    let mut b = a.clone();
    let mut c = air(Weather { seed: 43, ..weather });
    a.step(2400);
    c.step(2400);
    for _ in 0..2400 {
        assert_eq!(b.readout(), b.readout());
        b.step(1);
    }
    assert!(!a.readout().crashed && !b.readout().crashed && !c.readout().crashed);
    assert_eq!(a.state_hash(), b.state_hash());
    assert_eq!(a.readout(), b.readout());
    assert_ne!(a.state(), c.state());
    assert_ne!(a.state_hash(), c.state_hash());
    // The seed and filter memory belong to the hash even before gusts move the aircraft.
    assert_ne!(
        air(weather).state_hash(),
        air(Weather { seed: 43, ..weather }).state_hash()
    );
}

#[test]
fn failed_engine_glides_near_best_glide_speed_under_pitch_speed_hold() {
    let aircraft = flightcraft_aero::Aircraft::corvid().unwrap();
    let mut sim = air(Weather::default());
    let rho = flightcraft_world::atmosphere(1000.0).density_kg_m3;
    // Independently find maximum L/D including the failed prop's drag area.
    let mut best = (0.0_f64, 0.0_f64);
    for i in 0..240 {
        let coeff = aero::coefficients(&aircraft, f64::from(i) * 0.001, 0.0);
        let ratio = coeff.lift / (coeff.drag + 0.10 / 15.0);
        if ratio > best.0 {
            best = (ratio, coeff.lift);
        }
    }
    let target = libm::sqrt(2.0 * 1000.0 * flightcraft_world::GRAVITY_MPS2 / (rho * 15.0 * best.1));
    sim.set_failures(Failures {
        engine_power_fraction: 0.0,
        ..Failures::default()
    })
    .unwrap();
    sim.set_autopilot(Targets {
        airspeed_mps: Some(target),
        airspeed_mode: AirspeedMode::Pitch,
        wings_level: true,
        ..Targets::default()
    })
    .unwrap();
    for second in 0..120 {
        sim.step(120);
        let r = sim.readout();
        assert!(!r.crashed, "{r:?}");
        if second > 60 {
            assert!(
                (r.true_airspeed_mps / target - 1.0).abs() < 0.05,
                "target {target}: {r:?}"
            );
            assert!(r.vertical_speed_mps < -1.0, "{r:?}");
        }
    }
    assert!(sim.readout().altitude_msl_m < 800.0);
}

#[test]
fn failures_are_persistent_hash_state_and_actual_surfaces_override_holds() {
    let mut sim = air(Weather::default());
    let initial = sim.state_hash();
    let f = Failures {
        engine_power_fraction: 0.5,
        elevator: Some(0.1),
        aileron: Some(0.02),
        rudder: Some(-0.03),
        flaps: Some(0.2),
        ..Failures::default()
    };
    sim.set_failures(f).unwrap();
    assert_ne!(sim.state_hash(), initial);
    sim.set_autopilot(Targets {
        pitch_rad: Some(0.15),
        heading_rad: Some(0.2),
        ..Targets::default()
    })
    .unwrap();
    sim.step(120);
    let r = sim.readout();
    assert_eq!(r.failures, f);
    assert_eq!(r.effective_controls.elevator, 0.1);
    assert_eq!(r.effective_controls.aileron, 0.02);
    assert_eq!(r.effective_controls.rudder, -0.03);
    assert_eq!(r.effective_controls.flaps, 0.2);
    let before = sim.readout();
    for value in [f64::NAN, f64::INFINITY, -1.0, 2.0] {
        assert!(
            sim.set_failures(Failures {
                engine_power_fraction: value,
                ..f
            })
            .is_err()
        );
        assert_eq!(sim.readout(), before);
    }
}
