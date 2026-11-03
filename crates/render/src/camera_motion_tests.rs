//! Exercise the viewport's camera math against moving fixed-step simulation states.
#![allow(clippy::unwrap_used)]
use super::*;
use flightcraft_sim::{Controls, Sim, Start};

fn flight() -> Sim {
    let mut sim = Sim::new(&Start::Air {
        height_m: 300.,
        airspeed_mps: 38.,
        heading_rad: 0.7,
    })
    .unwrap();
    sim.set_controls(Controls {
        throttle: 0.65,
        ..Default::default()
    });
    sim.set_autopilot(flightcraft_sim::autopilot::Targets {
        vertical_speed_mps: Some(2.),
        heading_rad: Some(0.7),
        airspeed_mps: Some(38.),
        ..Default::default()
    })
    .unwrap();
    sim
}

#[test]
fn chase_follows_takeoff_speed_at_regular_sparse_and_zero_presentation_steps() {
    // Include screenshot-only cadence, zero elapsed time, and a presentation step
    // unrelated to the owner's fixed-step clock (the old page used a constant 1/60).
    for (steps, dt) in [
        (1, 1. / 120.),
        (2, 1. / 60.),
        (12, 0.1),
        (156, 1.3),
        (156, 1. / 60.),
        (156, 0.),
    ] {
        let mut sim = flight();
        let mut scene = Scene::default();
        scene.set_chase_offsets(7., 1.2);
        scene.camera_frame(sim.state(), dt);
        let start = Vec3::from_array(sim.state().position_m.map(|x| x as f32));
        let mut max_distance = 0_f32;
        for _ in 0..(120 * 15 / steps) {
            sim.step(steps);
            assert!(!sim.readout().crashed);
            let (eye, view, model, _) = scene.camera_frame(sim.state(), dt);
            let aircraft = model.w_axis.truncate();
            let distance = eye.distance(aircraft);
            max_distance = max_distance.max(distance);
            assert!(
                distance < 1.5 * 7.,
                "{steps} sim steps, dt={dt}, t={}: eye-to-aircraft distance {distance} m",
                sim.state().time_s
            );
            assert!(distance > 0.5 * 7.);
            assert!(view.is_finite());
        }
        assert!(Vec3::from_array(sim.state().position_m.map(|x| x as f32)).distance(start) > 300.);
        println!("{steps} sim steps per paint, dt={dt}: maximum chase distance {max_distance:.3} m");
    }
}

#[test]
fn wing_tracks_tower_stays_at_airport_and_flyby_repositions_after_pass() {
    for mode in [CameraMode::Wing, CameraMode::Tower, CameraMode::Flyby] {
        let mut sim = flight();
        let mut scene = Scene {
            camera: mode,
            ..Default::default()
        };
        let (initial_eye, initial_view, _, _) = scene.camera_frame(sim.state(), 0.);
        let mut previous_eye = initial_eye;
        let mut repositioned = false;
        for _ in 0..40 {
            sim.step(120);
            assert!(!sim.readout().crashed);
            let (eye, view, model, fov) = scene.camera_frame(sim.state(), 1.);
            let aircraft = model.w_axis.truncate();
            match mode {
                CameraMode::Wing => {
                    let [w, x, y, z] = sim.state().attitude.map(|v| v as f32);
                    let attitude = Quat::from_xyzw(x, y, z, w).normalize();
                    let offset = drawing(attitude * Vec3::new(-1., -7., -1.4));
                    assert!(eye.distance(aircraft + offset) < 0.001);
                }
                CameraMode::Tower => assert_eq!(eye, initial_eye),
                CameraMode::Flyby => {
                    if eye != previous_eye {
                        assert!(previous_eye.distance(aircraft) >= 800.);
                        assert!((eye.distance(aircraft) - Vec3::new(350., 30., -10.).length()).abs() < 0.01);
                        repositioned = true;
                    }
                    assert!(eye.distance(aircraft) < 800.);
                }
                _ => {}
            }
            assert!(view.is_finite());
            let clip = Mat4::perspective_rh(fov, 1.6, 0.015, 16000.) * view * aircraft.extend(1.);
            assert!(
                clip.w > 0. && clip.x.abs() < clip.w && clip.y.abs() < clip.w,
                "{mode:?} must keep the aircraft in frame at t={}: {clip:?}",
                sim.state().time_s
            );
            assert_ne!(view, initial_view);
            previous_eye = eye;
        }
        if mode == CameraMode::Flyby {
            assert!(
                repositioned,
                "flight must pass the observer and trigger another flyby"
            );
        }
    }
}

#[test]
fn chase_smooths_offset_and_attitude_changes_then_converges() {
    let mut scene = Scene::default();
    let mut state = State {
        position_m: [0., 0., -100.],
        ..Default::default()
    };
    scene.camera_frame(&state, 0.);
    scene.set_chase_offsets(7., 1.2);
    let desired = drawing(Vec3::new(-7., 0., -1.2));
    let (eye, _, model, _) = scene.camera_frame(&state, 1. / 60.);
    let offset = eye - model.w_axis.truncate();
    assert!(offset.distance(desired) > 1.);
    assert!(offset.distance(desired) < Vec3::new(-19., 0., -7.).distance(Vec3::new(-7., 0., -1.2)));
    for _ in 0..120 {
        scene.camera_frame(&state, 1. / 60.);
    }
    let (eye, _, model, _) = scene.camera_frame(&state, 1. / 60.);
    assert!((eye - model.w_axis.truncate()).distance(desired) < 0.001);
    let attitude = Quat::from_rotation_z(0.7) * Quat::from_rotation_y(0.15);
    state.attitude = [
        f64::from(attitude.w),
        f64::from(attitude.x),
        f64::from(attitude.y),
        f64::from(attitude.z),
    ];
    state.position_m = [50., 15., -105.];
    let desired = drawing(attitude * Vec3::new(-7., 0., -1.2));
    let (eye, _, model, _) = scene.camera_frame(&state, 1. / 60.);
    assert!((eye - model.w_axis.truncate()).distance(desired) > 1.);
    for _ in 0..120 {
        scene.camera_frame(&state, 1. / 60.);
    }
    let (eye, _, model, _) = scene.camera_frame(&state, 1. / 60.);
    assert!((eye - model.w_axis.truncate()).distance(desired) < 0.001);
    for dt in [f32::NAN, f32::INFINITY, -1.] {
        let (eye, view, _, _) = scene.camera_frame(&state, dt);
        assert!(eye.is_finite() && view.is_finite());
    }
    // A long paint gap must allow the offset to converge across the full elapsed time.
    scene.set_chase_offsets(12., 3.);
    let (eye, _, model, _) = scene.camera_frame(&state, 1.3);
    let new_offset = drawing(attitude * Vec3::new(-12., 0., -3.));
    assert!((eye - model.w_axis.truncate()).distance(new_offset) < 0.001);
    scene.set_chase_offsets(7., 1.2);
    // Observer positions must not leak into chase when a director/user changes views.
    for mode in [CameraMode::Tower, CameraMode::Flyby, CameraMode::Wing] {
        scene.camera = mode;
        scene.camera_frame(&state, 0.);
        scene.camera = CameraMode::Chase;
        let (eye, _, model, _) = scene.camera_frame(&state, 0.);
        assert!((eye - model.w_axis.truncate()).distance(desired) < 0.001);
    }
}
