#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_aero::{self as aero, Aircraft};
use flightcraft_sim::{Controls, Sim, Start};
use flightcraft_world::{self as world, GRAVITY_MPS2 as G};
use std::f64::consts::{PI, TAU};

fn air(speed: f64) -> Sim {
    Sim::new_with_aircraft(
        &Start::Air {
            height_m: 1000.0,
            airspeed_mps: speed,
            heading_rad: 0.0,
        },
        Aircraft::larrow().unwrap(),
    )
    .unwrap()
}

#[test]
fn stall_speed_matches_weight_and_clmax() {
    let a = Aircraft::larrow().unwrap();
    let rho = world::atmosphere(0.0).density_kg_m3;
    let weight = a.mass.mass_kg * G;
    let theory = libm::sqrt(2.0 * weight / (rho * a.geometry.wing_area_m2 * a.aerodynamics.cl_max));
    let mut measured = None;
    for k in 2000..4000 {
        let speed = f64::from(k) * 0.01;
        let mut support: f64 = 0.0;
        for step in 0..=100 {
            let alpha = f64::from(step) * 0.25 * PI / 180.0;
            let loads = aero::forces(
                &a,
                aero::Airflow {
                    velocity_body_mps: [speed * libm::cos(alpha), 0.0, speed * libm::sin(alpha)],
                    angular_rate_rps: [0.0; 3],
                },
                aero::Controls::default(),
                rho,
            );
            let [fx, _, fz] = loads.force_n;
            support = support.max(fx * libm::sin(alpha) - fz * libm::cos(alpha));
        }
        if support >= weight {
            measured = Some(speed);
            break;
        }
    }
    let measured = measured.unwrap();
    println!("stall measured={measured:.2}, theory={theory:.2}");
    assert!((measured / theory - 1.0).abs() < 0.05);
}

fn wrap(angle: f64) -> f64 {
    (angle + PI).rem_euclid(TAU) - PI
}

#[test]
fn full_aileron_roll_and_recovery() {
    let mut sim = air(50.0);
    let trim = sim.controls();
    let initial_height = sim.readout().altitude_msl_m;
    let mut roll = 0.0;
    let mut previous = sim.readout().roll_rad;
    for _ in 0..480 {
        sim.set_controls(Controls {
            aileron: 1.0,
            elevator: 0.0,
            ..trim
        });
        sim.step(1);
        let r = sim.readout();
        roll += wrap(r.roll_rad - previous);
        previous = r.roll_rad;
        assert!(!r.crashed);
        if roll >= TAU {
            break;
        }
    }
    println!("roll={roll}, duration={}", sim.state().time_s);
    assert!(roll >= TAU && sim.state().time_s < 4.0);
    for _ in 0..600 {
        let r = sim.readout();
        let [p, q, yaw] = sim.state().angular_rate_rps;
        let pitch = ((initial_height - r.altitude_msl_m) * 0.003 + 0.10).clamp(-0.1, 0.25);
        let cm = aero::coefficients(sim.aircraft(), r.alpha_rad, 0.0).pitch;
        sim.set_controls(Controls {
            aileron: -1.2 * r.roll_rad - 0.35 * p,
            elevator: (-cm + 2.0 * (pitch - r.pitch_rad) - 0.8 * q) / 0.85,
            rudder: -0.8 * r.beta_rad - 0.6 * yaw,
            ..trim
        });
        sim.step(2);
        assert!(!sim.readout().crashed);
    }
    let r = sim.readout();
    println!("recovered {r:?}");
    assert!(r.roll_rad.abs() < 0.05 && r.beta_rad.abs() < 0.05);
    assert!(r.altitude_msl_m > initial_height - 100.0);
    assert!(r.pitch_rad.abs() < 0.15 && r.vertical_speed_mps.abs() < 1.0);
    assert!(r.true_airspeed_mps > 40.0);
    assert!(sim.state().angular_rate_rps.iter().all(|rate| rate.abs() < 0.1));
}

#[test]
fn dynamic_modes_damp_without_holds() {
    for lateral in [false, true] {
        let mut sim = air(45.0);
        let trim = sim.controls();
        // A brief surface pulse excites the mode without a state-restoration API.
        sim.set_controls(if lateral {
            Controls { rudder: 0.25, ..trim }
        } else {
            Controls {
                elevator: trim.elevator + 0.1,
                ..trim
            }
        });
        sim.step(24);
        sim.set_controls(trim);
        let mut early: f64 = 0.0;
        let mut late: f64 = 0.0;
        for tick in 0..400 {
            sim.step(6);
            let r = sim.readout();
            let magnitude = if lateral {
                r.beta_rad.abs() + sim.state().angular_rate_rps[2].abs()
            } else {
                sim.state().angular_rate_rps[1].abs()
            };
            if tick < 60 {
                early = early.max(magnitude);
            }
            if tick > 240 {
                late = late.max(magnitude);
            }
            assert!(!r.crashed);
        }
        println!("lateral={lateral}, early={early}, late={late}");
        assert!(early > 0.01 && late < early * 0.3);
    }
}

#[test]
fn tailwheel_takeoff_climb_and_landing() {
    let mut sim = Sim::new_with_aircraft(&Start::Runway, Aircraft::larrow().unwrap()).unwrap();
    assert!(sim.readout().pitch_rad > 0.15);
    sim.step(240);
    assert!(sim.readout().on_ground && !sim.readout().crashed);
    let mut raised = false;
    let mut airborne = false;
    let mut max_height: f64 = 0.0;
    let mut landed = false;
    for tick in 0..16000 {
        let r = sim.readout();
        let [p, q, yaw] = sim.state().angular_rate_rps;
        let speed = r.true_airspeed_mps;
        // With wings level, measure the aft wheel's clearance directly, before liftoff.
        let tail = sim.aircraft().geometry.gear.last().unwrap();
        let [tx, _, tz] = sim.aircraft().relative_point(tail.point_m);
        let tail_height = r.altitude_msl_m + tx * libm::sin(r.pitch_rad) - tz * libm::cos(r.pitch_rad);
        if !airborne && speed > 15.0 && r.on_ground && tail_height > 0.25 {
            raised = true;
        }
        if r.height_agl_m > 1.0 {
            airborne = true;
        }
        max_height = max_height.max(r.height_agl_m);
        let returning = r.time_s > 40.0;
        if returning && airborne && r.on_ground {
            landed = true;
        }
        let climb = if returning {
            if r.height_agl_m < 7.0 {
                -0.35 - 0.10 * r.height_agl_m
            } else {
                -2.5
            }
        } else {
            3.0
        };
        let rho = world::atmosphere(r.altitude_msl_m).density_kg_m3;
        let alpha = ((2.0 * 650.0 * G / (rho * (speed.max(30.0) * speed.max(30.0)) * 11.0) - 0.04) / 4.6)
            .clamp(0.0, 0.23);
        let mut pitch = alpha + climb / speed.max(25.0);
        let target_speed = if returning { 33.0 } else { 42.0 };
        let mut throttle = (0.45 + 0.08 * (target_speed - speed) + 0.06 * climb).clamp(0.0, 1.0);
        if !airborne {
            pitch = if speed < 34.0 { 0.0 } else { 0.25 };
            throttle = 1.0;
        }
        if returning && r.height_agl_m < 4.0 {
            throttle = 0.0;
        }
        if landed {
            pitch = 0.18;
            throttle = 0.0;
        }
        let cm = aero::coefficients(sim.aircraft(), r.alpha_rad, 0.0).pitch;
        sim.set_controls(Controls {
            elevator: (-cm + 2.0 * (pitch - r.pitch_rad) - 0.8 * q) / 0.85,
            aileron: -1.2 * r.roll_rad - 0.35 * p,
            rudder: -0.8 * r.beta_rad - 0.6 * yaw,
            throttle,
            brake: if landed { 0.25 } else { 0.0 },
            flaps: 0.0,
        });
        sim.step(3);
        if tick % 400 == 0 {
            println!(
                "flight t={:.1} speed={speed:.2} h={:.2} pitch={:.3} climb={:.2} ground={}",
                r.time_s, r.height_agl_m, r.pitch_rad, r.vertical_speed_mps, r.on_ground
            );
        }
        assert!(!sim.readout().crashed, "crash {:?}", sim.readout());
        if landed && sim.readout().ground_speed_mps < 0.2 {
            break;
        }
    }
    println!(
        "raised={raised}, airborne={airborne}, max_height={max_height}, landed={landed}, final={:?}",
        sim.readout()
    );
    assert!(raised && airborne && max_height > 70.0 && landed);
    assert!(sim.readout().on_ground && sim.readout().ground_speed_mps < 0.2);
}

#[test]
fn phugoid_damps_with_fixed_trim_controls() {
    let mut sim = air(45.0);
    let trim = sim.controls();
    sim.set_controls(Controls {
        throttle: 1.0,
        ..trim
    });
    sim.step(120);
    sim.set_controls(trim);
    let mut early: f64 = 0.0;
    let mut late: f64 = 0.0;
    for tick in 0..1800 {
        sim.step(12);
        let r = sim.readout();
        let error = (r.true_airspeed_mps - 45.0).abs();
        if tick < 400 {
            early = early.max(error);
        }
        if tick > 1200 {
            late = late.max(error);
        }
        assert!(!r.crashed);
    }
    println!("phugoid early={early}, late={late}");
    assert!(early > 0.5 && late < early * 0.6);
}

#[test]
fn positive_rudder_steers_the_tailwheel_nose_right() {
    let mut sim = Sim::new_with_aircraft(&Start::Runway, Aircraft::larrow().unwrap()).unwrap();
    sim.set_controls(Controls {
        throttle: 0.12,
        rudder: 0.5,
        ..Default::default()
    });
    sim.step(120 * 8);
    let r = sim.readout();
    assert!(!r.crashed && r.on_ground, "{r:?}");
    assert!(sim.state().position_m[1] > 0.2);
    assert!((0.02..PI).contains(&r.heading_rad), "{r:?}");
}

#[test]
fn larrow_lands_and_stops_on_the_valley_runway() {
    let mut sim = Sim::new_with_aircraft_weather_world(
        &Start::Air {
            height_m: 15.0,
            airspeed_mps: 33.0,
            heading_rad: 0.0,
        },
        Aircraft::larrow().unwrap(),
        world::Weather::default(),
        world::World::load(world::WorldChoice::TrainingValley).unwrap(),
    )
    .unwrap();
    assert!(!sim.readout().on_ground);
    let mut touchdown = None;
    for _ in 0..2400 {
        let r = sim.readout();
        let [p, q, yaw] = sim.state().angular_rate_rps;
        let [north, east, _] = sim.state().position_m;
        if r.on_ground && touchdown.is_none() {
            assert_eq!(sim.world().ground(north, east).surface, world::Surface::Pavement);
            touchdown = Some(*sim.state());
        }
        let landed = touchdown.is_some();
        let climb = if r.height_agl_m < 7.0 {
            -0.35 - 0.10 * r.height_agl_m
        } else {
            -2.5
        };
        let speed = r.true_airspeed_mps;
        let aircraft = sim.aircraft();
        let rho = world::atmosphere(r.altitude_msl_m).density_kg_m3;
        let alpha = ((2.0 * aircraft.mass.mass_kg * G
            / (rho * speed.max(30.0) * speed.max(30.0) * aircraft.geometry.wing_area_m2)
            - 0.04)
            / 4.6)
            .clamp(0.0, 0.23);
        let pitch = if landed {
            0.18
        } else {
            alpha + climb / speed.max(25.0)
        };
        let cm = aero::coefficients(aircraft, r.alpha_rad, 0.0).pitch;
        let throttle = if landed || r.height_agl_m < 4.0 {
            0.0
        } else {
            (0.45 + 0.08 * (33.0 - speed) + 0.06 * climb).clamp(0.0, 1.0)
        };
        sim.set_controls(Controls {
            elevator: (-cm + 2.0 * (pitch - r.pitch_rad) - 0.8 * q) / 0.85,
            aileron: -1.2 * r.roll_rad - 0.35 * p,
            rudder: -0.8 * r.beta_rad - 0.6 * yaw,
            throttle,
            brake: if landed { 0.25 } else { 0.0 },
            flaps: 0.0,
        });
        sim.step(3);
        assert!(!sim.readout().crashed, "{:?}", sim.readout());
        if landed && sim.readout().on_ground && sim.readout().ground_speed_mps < 0.2 {
            break;
        }
    }
    let touchdown = touchdown.unwrap();
    let [north, east, _] = sim.state().position_m;
    println!(
        "valley touchdown={:?}; stopped={:?}",
        touchdown.position_m,
        sim.state().position_m
    );
    assert!(sim.readout().on_ground && sim.readout().ground_speed_mps < 0.2);
    assert_eq!(sim.world().ground(north, east).surface, world::Surface::Pavement);
    assert_eq!(sim.world().choice(), world::WorldChoice::TrainingValley);
}
