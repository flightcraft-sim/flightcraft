#![allow(clippy::unwrap_used)]
use flightcraft_sim::{Controls, Sim, Start, State};
use flightcraft_world::GRAVITY_MPS2 as G;

fn attitude(roll: f64, pitch: f64, heading: f64) -> [f64; 4] {
    let (sr, cr) = (libm::sin(roll * 0.5), libm::cos(roll * 0.5));
    let (sp, cp) = (libm::sin(pitch * 0.5), libm::cos(pitch * 0.5));
    let (sh, ch) = (libm::sin(heading * 0.5), libm::cos(heading * 0.5));
    [
        ch * cp * cr + sh * sp * sr,
        ch * cp * sr - sh * sp * cr,
        ch * sp * cr + sh * cp * sr,
        sh * cp * cr - ch * sp * sr,
    ]
}

#[test]
fn steady_coordinated_turn_kinematics_match_bank_and_speed() {
    // Analytical constant-bank, constant-pitch turn fixtures, in both directions.
    // Body rates are the inverse rotation of the known earth-vertical angular rate.
    for bank_deg in [-30.0_f64, -15.0, 15.0, 30.0] {
        for pitch in [0.0, 0.12] {
            for speed in [35.0, 50.0] {
                let bank = bank_deg.to_radians();
                let rate = G * libm::tan(bank) / speed;
                let state = State {
                    position_m: [0.0, 0.0, -1000.0],
                    velocity_mps: [speed, 0.0, 0.0],
                    attitude: attitude(bank, pitch, 1.2),
                    angular_rate_rps: [
                        -rate * libm::sin(pitch),
                        rate * libm::sin(bank) * libm::cos(pitch),
                        rate * libm::cos(bank) * libm::cos(pitch),
                    ],
                    ..State::default()
                };
                let sim = Sim::from_state(state, Controls::default()).unwrap();
                let hash = sim.state_hash();
                let before = *sim.state();
                let r = sim.readout();
                assert!((r.turn_rate_rps / rate - 1.0).abs() < 0.02, "{r:?}");
                assert_eq!(
                    sim.state_hash(),
                    hash,
                    "observing instruments cannot change state"
                );
                assert_eq!(*sim.state(), before);
            }
        }
    }
}

#[test]
fn lateral_specific_force_is_centred_when_coordinated_and_responds_to_rudder_slip() {
    let mut sim = Sim::new(&Start::Air {
        height_m: 1000.0,
        airspeed_mps: 40.0,
        heading_rad: 0.0,
    })
    .unwrap();
    let trim = sim.controls();
    assert!(sim.readout().lateral_acceleration_g.abs() < 1e-12);
    for direction in [-1.0, 1.0] {
        let mut slipping = Sim::from_state(*sim.state(), trim).unwrap();
        slipping.set_controls(Controls {
            rudder: direction * 0.5,
            ..trim
        });
        let initial = slipping.readout().lateral_acceleration_g;
        assert!(
            initial * direction > 0.02,
            "rudder side force must move the ball opposite the force"
        );
        slipping.step(30);
        let r = slipping.readout();
        assert!(r.beta_rad.abs() > 0.001, "rudder must develop a sideslip: {r:?}");
        assert!(r.lateral_acceleration_g.abs() > 0.005, "{r:?}");
    }
    sim.step(1200);
    assert!(sim.readout().lateral_acceleration_g.abs() < 1e-12);
}

#[test]
fn turn_rate_stays_finite_at_vertical_pitch() {
    for pitch in [std::f64::consts::FRAC_PI_2, -std::f64::consts::FRAC_PI_2] {
        let sim = Sim::from_state(
            State {
                position_m: [0.0, 0.0, -1000.0],
                attitude: attitude(0.3, pitch, 0.8),
                angular_rate_rps: [0.1, 0.2, 0.3],
                ..State::default()
            },
            Controls::default(),
        )
        .unwrap();
        assert_eq!(sim.readout().turn_rate_rps, 0.0);
    }
}

#[test]
fn sustained_turn_matches_coordinated_turn_relation() {
    use flightcraft_sim::autopilot::Targets;
    for direction in [-1.0, 1.0] {
        let mut sim = Sim::new(&Start::Air {
            height_m: 1000.0,
            airspeed_mps: 40.0,
            heading_rad: 0.0,
        })
        .unwrap();
        sim.set_autopilot(Targets {
            altitude_m: Some(1000.0),
            airspeed_mps: Some(40.0),
            ..Targets::default()
        })
        .unwrap();
        for tick in 0..900 {
            let r = sim.readout();
            let aircraft = sim.aircraft();
            let dynamic = 0.5
                * flightcraft_world::atmosphere(r.altitude_msl_m).density_kg_m3
                * r.true_airspeed_mps
                * r.true_airspeed_mps
                * aircraft.geometry.wing_area_m2;
            let rudder_gain = dynamic * aircraft.aerodynamics.derivatives.cy_rudder * libm::cos(r.beta_rad)
                / (aircraft.mass.mass_kg * G);
            let [p, _, _] = sim.state().angular_rate_rps;
            // Fly a fixed bank, using rudder to null lateral specific force. The
            // longitudinal holds leave these two pilot-controlled surfaces alone.
            sim.set_controls(Controls {
                aileron: 1.4 * (direction * 0.3 - r.roll_rad) - 0.35 * p,
                rudder: r.controls.rudder - r.lateral_acceleration_g / rudder_gain,
                ..r.controls
            });
            sim.step(12);
            if tick > 700 {
                let r = sim.readout();
                let expected = G * libm::tan(r.roll_rad) / r.true_airspeed_mps;
                assert!(!r.crashed && r.roll_rad * direction > 0.25, "{r:?}");
                assert!(
                    (r.turn_rate_rps / expected - 1.0).abs() < 0.02,
                    "{r:?}, expected {expected}"
                );
                assert!(r.lateral_acceleration_g.abs() < 0.01, "coordinated ball: {r:?}");
            }
        }
    }
}
