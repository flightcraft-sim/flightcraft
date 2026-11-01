#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
//! Closed-loop pilot, using only the public simulation interface.
use flightcraft_aero as aero;
use flightcraft_sim::{Controls, Sim, Start};
use flightcraft_world::{self as world, Surface, World, WorldChoice};
use std::f64::consts::{PI, TAU};

fn wrap(x: f64) -> f64 {
    (x + PI).rem_euclid(TAU) - PI
}

#[test]
fn runway_pattern() {
    fly_pattern(WorldChoice::Flat);
}

#[test]
fn valley_runway_pattern() {
    fly_pattern(WorldChoice::TrainingValley);
}

fn fly_pattern(choice: WorldChoice) {
    let aircraft = flightcraft_aero::Aircraft::corvid().unwrap();
    let mut sim = Sim::new_with_world(&Start::Runway, World::load(choice).unwrap()).unwrap();
    let mut phase = 0;
    let mut took_off = false;
    let mut landed = false;
    let mut takeoff_distance = 0.0;
    let mut touchdown = [0.0; 3];
    let mut max_height: f64 = 0.0;
    let mut max_altitude: f64 = 0.0;
    let mut climb_sum = 0.0;
    let mut climb_samples = 0;
    let mut trajectory = 0xcbf29ce484222325u64;
    for tick in 0..7200 {
        let r = sim.readout();
        let [north, east, _] = sim.state().position_m;
        let [p, q, rate_yaw] = sim.state().angular_rate_rps;
        let speed = r.true_airspeed_mps;
        max_height = max_height.max(r.height_agl_m);
        max_altitude = max_altitude.max(r.altitude_msl_m);
        if !took_off && r.height_agl_m > 0.5 {
            took_off = true;
            takeoff_distance = north;
            println!("liftoff t={:.1} x={north:.1} V={speed:.2}", r.time_s);
        }
        if took_off && r.height_agl_m > 50.0 && r.height_agl_m < 250.0 && phase == 0 {
            climb_sum += r.vertical_speed_mps;
            climb_samples += 1;
        }
        if phase == 0 && north > 3000.0 {
            phase = 1;
        }
        if phase == 1 && east > 1200.0 {
            phase = 2;
        }
        if phase == 2 && north < -2400.0 {
            phase = 3;
        }
        if phase == 3 && east < 250.0 {
            phase = 4;
        }
        if took_off && phase == 4 && r.on_ground {
            if !landed {
                touchdown = sim.state().position_m;
                println!(
                    "touchdown t={:.1} at {touchdown:?} V={speed:.2} sink={:.2}",
                    r.time_s, r.vertical_speed_mps
                );
            }
            landed = true;
        }
        let (heading, target_height, target_speed) = match phase {
            0 => ((-east * 0.003).clamp(-0.3, 0.3), 330.0, 36.0),
            1 => (
                PI * 0.5 + ((north - 3300.0) * 0.003).clamp(-0.6, 0.6),
                330.0,
                40.0,
            ),
            2 => (PI + ((east - 1500.0) * 0.003).clamp(-0.6, 0.6), 330.0, 40.0),
            3 => (
                PI * 1.5 - ((north + 2700.0) * 0.003).clamp(-0.6, 0.6),
                220.0,
                35.0,
            ),
            _ => (
                (-east * 0.006).clamp(-0.5, 0.5),
                ((-150.0 - north) * 0.070 + 1.1).max(1.1),
                30.0,
            ),
        };
        let mut climb = ((target_height - r.altitude_msl_m) * 0.12).clamp(-3.0, 4.0);
        if phase == 4 && r.height_agl_m < 8.0 {
            climb = (-0.3 - r.height_agl_m * 0.12).max(-1.2);
        }
        let bank = (wrap(heading - r.heading_rad) * 1.3).clamp(-0.40, 0.40);
        let rho = world::atmosphere(r.altitude_msl_m).density_kg_m3;
        let desired_cl = 2.0 * 1000.0 * world::GRAVITY_MPS2
            / (rho * speed.max(26.0) * speed.max(26.0) * 15.0 * libm::cos(bank));
        let trim_alpha = ((desired_cl - 0.25) / 5.0).clamp(-0.05, 0.23);
        let mut pitch = trim_alpha + climb / speed.max(25.0);
        // Feed forward the independently computed drag and climb power; proportional
        // speed feedback corrects transient errors without a persistent speed bias.
        let target_cl = 2.0 * 1000.0 * world::GRAVITY_MPS2 / (rho * target_speed * target_speed * 15.0);
        let target_alpha = ((target_cl - 0.25) / 5.0).clamp(-0.05, 0.23);
        let drag = 0.5
            * rho
            * target_speed
            * target_speed
            * 15.0
            * aero::coefficients(&aircraft, target_alpha, 0.0).drag;
        let needed = drag + 1000.0 * world::GRAVITY_MPS2 * climb / target_speed;
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..24 {
            let mid = (lo + hi) * 0.5;
            if aero::propulsion(&aircraft, target_speed, mid, rho).0 > needed {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        let mut throttle = ((lo + hi) * 0.5 + 0.07 * (target_speed - speed)).clamp(0.0, 1.0);
        if phase == 0 && !took_off {
            pitch = if speed < 55.0 * 0.514444 { 0.0 } else { 0.22 };
            throttle = 1.0;
        }
        if phase == 4 && r.height_agl_m < 4.0 {
            throttle = 0.0;
        }
        if landed {
            pitch = 0.0;
            throttle = 0.0;
        }
        let controls = Controls {
            elevator: -aero::coefficients(&aircraft, r.alpha_rad, 0.0).pitch + 2.0 * (pitch - r.pitch_rad)
                - 0.8 * q,
            aileron: 1.2 * (bank - r.roll_rad) - 0.35 * p,
            rudder: 0.8 * (world::GRAVITY_MPS2 * libm::tan(r.roll_rad) / speed.max(15.0) - rate_yaw)
                - 0.8 * r.beta_rad,
            throttle,
            brake: if landed { 1.0 } else { 0.0 },
            flaps: 0.0,
        };
        sim.set_controls(controls);
        sim.step(12);
        trajectory ^= sim.state_hash();
        trajectory = trajectory.wrapping_mul(0x100000001b3);
        if tick % 200 == 0 {
            println!(
                "t={:.1} phase={phase} pos=({north:.0},{east:.0}) h={:.1} V={speed:.1} pitch={:.2} roll={:.2} course={:.2}",
                r.time_s, r.height_agl_m, r.pitch_rad, r.roll_rad, r.heading_rad
            );
        }
        assert!(
            !sim.readout().crashed,
            "crash at {:?}, {:?}",
            sim.state(),
            sim.readout()
        );
        if landed && sim.readout().ground_speed_mps < 0.2 {
            break;
        }
    }
    let climb_rate = climb_sum / f64::from(climb_samples);
    println!(
        "trajectory={trajectory:#018x}; state={:#018x}; takeoff={takeoff_distance:.2} m; climb={climb_rate:.3} m/s; max height={max_height:.2} m; touchdown={touchdown:?}",
        sim.state_hash()
    );
    assert!(took_off && max_altitude >= 300.0);
    if choice == WorldChoice::Flat {
        assert!(max_height >= 300.0);
    }
    assert!(landed, "did not land: {:?}", sim.readout());
    assert_eq!(
        world::ground(touchdown[0], touchdown[1]).surface,
        Surface::Pavement
    );
    let [north, east, _] = sim.state().position_m;
    assert_eq!(world::ground(north, east).surface, Surface::Pavement);
    assert!(sim.readout().on_ground && sim.readout().ground_speed_mps < 0.2);
    assert!((100.0..600.0).contains(&takeoff_distance));
    assert!((1.5..6.0).contains(&climb_rate));
    // FNV fold of every 0.1 s state hash, including rollout. Update only after reviewing
    // an intentional physics or pilot change and the flight's quantitative assertions.
    if choice == WorldChoice::Flat {
        assert_eq!(trajectory, 0x4b3c_ea10_0a2f_b15d);
    }
}
