#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_aero as aero;
use flightcraft_sim::{Controls, Sim, Start, State};
use flightcraft_world::{self as world, GRAVITY_MPS2 as G};
use std::f64::consts::PI;
fn air(speed: f64) -> Sim {
    Sim::new(&Start::Air {
        height_m: 1000.0,
        airspeed_mps: speed,
        heading_rad: 0.0,
    })
    .unwrap()
}

#[test]
fn level_trim_and_drag_polar() {
    let aircraft = flightcraft_aero::Aircraft::corvid().unwrap();
    for speed in [35.0, 40.0, 50.0] {
        let mut s = air(speed);
        let initial = s.readout();
        let rho = world::atmosphere(1000.0).density_kg_m3;
        let c = aero::coefficients(&aircraft, initial.alpha_rad, 0.0);
        let drag = 0.5 * rho * speed * speed * 15.0 * c.drag;
        let (thrust, _) = aero::propulsion(
            &aircraft,
            speed * libm::cos(initial.alpha_rad),
            s.controls().throttle,
            rho,
        );
        assert!((thrust * libm::cos(initial.alpha_rad) - drag).abs() < 0.01);
        s.step(120 * 60);
        println!("trim {speed}: {:?}", s.readout());
        assert!(!s.readout().crashed);
        assert!((s.readout().true_airspeed_mps / speed - 1.0).abs() < 0.05);
        assert!((s.readout().altitude_msl_m - 1000.0).abs() < 2.0);
        assert!(s.readout().vertical_speed_mps.abs() < 0.1);
    }
}

#[test]
fn stall_speed() {
    let aircraft = flightcraft_aero::Aircraft::corvid().unwrap();
    let rho = world::atmosphere(0.0).density_kg_m3;
    let theory = libm::sqrt(2.0 * 1000.0 * G / (rho * 15.0 * 1.50));
    // Measure maximum vertical support across the entire incidence table, then find
    // the minimum supporting speed. This includes the propeller's vertical component.
    let mut measured = 0.0;
    for k in 2000..3500 {
        let v = f64::from(k) * 0.01;
        let mut support: f64 = 0.0;
        for deg in 0..=40 {
            let alpha = f64::from(deg) * 0.5 * PI / 180.0;
            let c = aero::coefficients(&aircraft, alpha, 0.0);
            let loads = aero::forces(
                &aircraft,
                aero::Airflow {
                    velocity_body_mps: [v * libm::cos(alpha), 0.0, v * libm::sin(alpha)],
                    angular_rate_rps: [0.0; 3],
                },
                aero::Controls {
                    elevator: -c.pitch,
                    throttle: 0.0,
                    ..Default::default()
                },
                rho,
            );
            let [fx, _, fz] = loads.force_n;
            support = support.max(fx * libm::sin(alpha) - fz * libm::cos(alpha));
        }
        if support >= 1000.0 * G {
            measured = v;
            break;
        }
    }
    println!(
        "stall speed={measured:.2} m/s ({:.2} kt), theoretical={theory:.3}",
        measured / 0.514444
    );
    assert!((measured / theory - 1.0).abs() < 0.05);
}

#[test]
fn phugoid_period() {
    let s = air(40.0);
    let controls = s.controls();
    let mut state = *s.state();
    state.velocity_mps[0] += 1.5;
    let mut s = Sim::from_state(state, controls).unwrap();
    let mut peaks = Vec::new();
    let mut prev = 0.0;
    let mut prev_diff = 0.0;
    for _ in 0..2400 {
        s.step(12);
        let speed = s.readout().true_airspeed_mps;
        let diff = speed - prev;
        if diff < 0.0 && prev_diff >= 0.0 && s.state().time_s > 8.0 {
            peaks.push(s.state().time_s - 0.1);
        }
        prev = speed;
        prev_diff = diff;
    }
    println!("phugoid peaks={peaks:?}");
    assert!(peaks.len() >= 3);
    let period = peaks[2] - peaks[1];
    let expected = PI * libm::sqrt(2.0) * 40.0 / G;
    println!("phugoid period={period:.3} expected={expected:.3}");
    assert!((period / expected - 1.0).abs() < 0.15);
}

#[test]
fn short_period_and_dutch_roll_damped() {
    for lateral in [false, true] {
        let s = air(40.0);
        let controls = s.controls();
        let mut state = *s.state();
        if lateral {
            state.velocity_mps[1] = 2.0;
            state.angular_rate_rps[2] = 0.1;
        } else {
            state.angular_rate_rps[1] = 0.15;
        }
        let mut s = Sim::from_state(state, controls).unwrap();
        let mut early: f64 = 0.0;
        let mut late: f64 = 0.0;
        for _ in 0..400 {
            s.step(6);
            let r = s.readout();
            let magnitude = if lateral {
                r.beta_rad.abs() + s.state().angular_rate_rps[2].abs()
            } else {
                s.state().angular_rate_rps[1].abs()
            };
            if r.time_s < 3.0 {
                early = early.max(magnitude);
            }
            if r.time_s > 12.0 {
                late = late.max(magnitude);
            }
            assert!(!r.crashed);
        }
        println!("lateral={lateral} early={early} late={late}");
        assert!(late < early * 0.3);
    }
}

#[test]
fn hostile_controls_and_starts_stay_finite() {
    for value in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::MAX,
        -f64::MAX,
        -1.0,
        1.0,
    ] {
        let mut s = air(40.0);
        s.set_controls(Controls {
            elevator: value,
            aileron: value,
            rudder: value,
            throttle: value,
            flaps: value,
            brake: value,
        });
        s.step(120 * 60);
        let state = s.state();
        assert!(
            state
                .position_m
                .iter()
                .chain(&state.velocity_mps)
                .chain(&state.attitude)
                .chain(&state.angular_rate_rps)
                .all(|v| v.is_finite())
        );
        let r = s.readout();
        for v in [
            r.indicated_airspeed_mps,
            r.true_airspeed_mps,
            r.ground_speed_mps,
            r.altitude_msl_m,
            r.height_agl_m,
            r.vertical_speed_mps,
            r.heading_rad,
            r.pitch_rad,
            r.roll_rad,
            r.alpha_rad,
            r.beta_rad,
            r.load_factor_g,
            r.engine_rpm,
        ] {
            assert!(v.is_finite());
        }
        let [w, x, y, z] = state.attitude;
        assert!((w * w + x * x + y * y + z * z - 1.0).abs() < 1e-12);
    }
    for (h, v, heading) in [
        (f64::NAN, 40.0, 0.0),
        (1000.0, f64::MAX, 0.0),
        (1000.0, 40.0, f64::INFINITY),
        (-1.0, 40.0, 0.0),
    ] {
        assert!(
            Sim::new(&Start::Air {
                height_m: h,
                airspeed_mps: v,
                heading_rad: heading
            })
            .is_err()
        );
    }
    assert!(
        Sim::from_state(State::default(), Controls::default())
            .unwrap()
            .readout()
            .crashed
    );
    let mut state = *air(40.0).state();
    state.attitude = [0.0; 4];
    assert!(Sim::from_state(state, Controls::default()).is_err());
    // All u32 values are accepted. A ground impact terminates a maximal call early.
    let mut s = Sim::new(&Start::Air {
        height_m: 3.0,
        airspeed_mps: 1.0,
        heading_rad: 0.0,
    })
    .unwrap();
    s.step(u32::MAX);
    assert!(s.readout().crashed);
    let state = *s.state();
    s.step(u32::MAX);
    assert_eq!(state, *s.state());
}

#[test]
fn runway_idle_and_brakes() {
    let mut s = Sim::new(&Start::Runway).unwrap();
    s.step(120 * 20);
    assert!(!s.readout().crashed && s.readout().on_ground);
    assert!(s.readout().ground_speed_mps < 0.01);
    assert!(s.state().position_m[0].abs() < 0.1);
    s.set_controls(Controls {
        throttle: 1.0,
        brake: 1.0,
        ..Default::default()
    });
    s.step(120 * 5);
    assert!(s.readout().ground_speed_mps < 0.1);
}

#[test]
fn contact_crashes_steering_and_step_grouping() {
    let mut s = air(40.0);
    let mut grouped = s.clone();
    s.step(1200);
    for _ in 0..1200 {
        grouped.step(1);
    }
    assert_eq!(s.state(), grouped.state());
    assert_eq!(s.state_hash(), grouped.state_hash());

    // A hard vertical arrival triggers the contact threshold before a belly strike.
    let mut state = *air(40.0).state();
    state.position_m = [100.0, 0.0, -1.4];
    state.velocity_mps = [25.0, 0.0, 6.0];
    let mut hard = Sim::from_state(state, Controls::default()).unwrap();
    hard.step(120);
    assert!(hard.readout().crashed);
    let frozen = *hard.state();
    hard.step(120);
    assert_eq!(frozen, *hard.state());

    // Detect a just-over-limit arrival even if suspension damping reduces the
    // rate below the limit within the first contact step.
    let mut state = *air(40.0).state();
    state.position_m = [100.0, 0.0, -1.1505];
    state.attitude = [1.0, 0.0, 0.0, 0.0];
    state.velocity_mps = [0.0, 0.0, 4.55];
    let mut hard = Sim::from_state(state, Controls::default()).unwrap();
    hard.step(1);
    assert!(hard.readout().crashed);

    // A low wingtip is a crash even when no wheel has touched.
    let mut state = *air(40.0).state();
    state.position_m = [100.0, 0.0, -2.0];
    state.attitude = [libm::cos(PI / 8.0), libm::sin(PI / 8.0), 0.0, 0.0];
    assert!(
        Sim::from_state(state, Controls::default())
            .unwrap()
            .readout()
            .crashed
    );

    let mut taxi = Sim::new(&Start::Runway).unwrap();
    taxi.set_controls(Controls {
        throttle: 0.15,
        rudder: 0.5,
        ..Default::default()
    });
    taxi.step(120 * 10);
    assert!(!taxi.readout().crashed && taxi.readout().on_ground);
    assert!(taxi.state().position_m[1] > 1.0);
    assert!((0.05..PI).contains(&taxi.readout().heading_rad));
}
