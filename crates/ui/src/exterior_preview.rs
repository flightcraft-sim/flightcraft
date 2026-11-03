//! Actual desktop/WebGL-limit GPU captures without a window or downloaded terrain.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::{
    instruments::{InstrumentInput, InstrumentPanel},
    test_capture::Capture,
    theme,
};
use flightcraft_control::Session;
use flightcraft_render::{CameraMode, Scene};
use flightcraft_world::{Conditions, TimeOfDay};

#[test]
#[ignore = "requires an offscreen GPU; optionally set FLIGHTCRAFT_SLING_PACK"]
fn render_exterior_director_shots() {
    pollster::block_on(async {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let out = root.join("out/exterior-after");
        std::fs::create_dir_all(&out).unwrap();
        let mut capture = Capture::new([1280, 800]).await;
        let mut names = vec!["corvid", "larrow"];
        let sling = std::env::var("FLIGHTCRAFT_SLING_PACK").ok();
        if sling.is_some() {
            names.push("sling2");
        }
        for name in names {
            let mut session = Session::new().unwrap();
            if name == "sling2" {
                session
                    .install_pack(
                        &flightcraft_control::read_pack(std::path::Path::new(sling.as_ref().unwrap()))
                            .unwrap(),
                    )
                    .unwrap();
            }
            session
                .execute(
                    "aircraft.load",
                    serde_json::json!({"source":{"kind":"installed","id":name},"reset":true}),
                )
                .unwrap();
            for (label, camera, position) in [
                ("cockpit-takeoff", CameraMode::Cockpit, [0., 0., -1.2]),
                ("wing-climb", CameraMode::Wing, [120., 0., -75.]),
                ("flyby-cruise", CameraMode::Flyby, [320., 0., -150.]),
                ("tower-final", CameraMode::Tower, [-160., 0., -25.]),
                ("night-lights", CameraMode::Chase, [0., 0., -1.2]),
            ] {
                let mut scene = Scene::default();
                scene.set_aircraft(name);
                if let Some(asset) = session.aircraft_catalog().exterior(name) {
                    scene.set_exterior(asset);
                }
                let configuration = session.aircraft_configuration().unwrap();
                let start = if label == "cockpit-takeoff" || label == "night-lights" {
                    flightcraft_sim::Start::Runway
                } else {
                    flightcraft_sim::Start::AirAt {
                        position_m: position,
                        airspeed_mps: 45.,
                        heading_rad: 0.,
                    }
                };
                let mut animation =
                    flightcraft_sim::Sim::new_with_aircraft(&start, configuration.aircraft.clone()).unwrap();
                animation.configure_systems(configuration).unwrap();
                animation.set_controls(flightcraft_sim::Controls {
                    throttle: 0.7,
                    aileron: 0.12,
                    elevator: 0.08,
                    rudder: 0.04,
                    flaps: if label == "tower-final" { 0.6 } else { 0. },
                    ..Default::default()
                });
                animation.step(1);
                scene.set_exterior_state(&animation);
                scene.set_propeller_power(0.7);
                scene.airspace_visible = false;
                scene.camera = camera;
                let mut pose = *animation.state();
                if label == "cockpit-takeoff" || label == "night-lights" {
                    pose = *session.sim().state();
                    scene.set_exterior_state(session.sim());
                }
                let night = label == "night-lights";
                let mut dark = None;
                if night {
                    scene.set_weather(
                        &Conditions {
                            time_of_day: TimeOfDay::Night,
                            ..Default::default()
                        },
                        false,
                    );
                    capture.draw(|ui| scene.show(ui, &pose, 1. / 60., theme::scene()));
                    dark = Some(capture.image());
                }
                if night {
                    // Exercise the real switches and bus, rather than injecting emissive geometry.
                    if name == "sling2" {
                        for light in ["nav", "strobe", "beacon", "landing"] {
                            session.execute("systems.set",serde_json::json!({"setting":{"kind":"equipment","setting":{"kind":"light","light":light,"on":true}}})).unwrap();
                        }
                    } else {
                        session
                            .execute(
                                "systems.set",
                                serde_json::json!({"setting":{"kind":"switch","switch":"lights","on":true}}),
                            )
                            .unwrap();
                    }
                    session.sim_mut().step(1);
                    scene.set_exterior_state(session.sim());
                    scene.set_weather(
                        &Conditions {
                            time_of_day: TimeOfDay::Night,
                            ..Default::default()
                        },
                        false,
                    );
                }
                if camera == CameraMode::Flyby {
                    let mut approach = pose;
                    approach.position_m[0] -= 320.;
                    capture.draw(|ui| scene.show(ui, &approach, 1. / 60., theme::scene()));
                }
                let mut panel = InstrumentPanel::default();
                for _ in 0..3 {
                    capture.draw(|ui| {
                        if camera != CameraMode::Cockpit {
                            scene.show(ui, &pose, 1. / 60., theme::scene());
                            return;
                        }
                        let full = ui.max_rect();
                        let split = full.top() + full.height() * 0.56;
                        ui.scope_builder(
                            eframe::egui::UiBuilder::new().max_rect(eframe::egui::Rect::from_min_max(
                                full.min,
                                eframe::egui::pos2(full.right(), split),
                            )),
                            |ui| scene.show(ui, &pose, 1. / 60., theme::scene()),
                        );
                        ui.scope_builder(
                            eframe::egui::UiBuilder::new().max_rect(eframe::egui::Rect::from_min_max(
                                eframe::egui::pos2(full.left(), split),
                                full.max,
                            )),
                            |ui| {
                                ui.painter().rect_filled(ui.max_rect(), 0., theme::PANEL);
                                let configuration = session.aircraft_configuration().unwrap();
                                panel
                                    .show(
                                        ui,
                                        &configuration.panel,
                                        InstrumentInput::new(session.sim().systems(), 0.)
                                            .with_aircraft(configuration),
                                        session.flight_generation(),
                                    )
                                    .unwrap();
                            },
                        );
                    });
                }
                let image = capture.image();
                let colorful = image.pixels().filter(|p| p[0].abs_diff(p[2]) > 8).count();
                assert!(colorful > 300, "{name} {label} must contain a visible scene");
                if let Some(dark) = dark {
                    assert!(
                        image
                            .pixels()
                            .zip(dark.pixels())
                            .filter(|(a, b)| a.0.iter().zip(b.0).any(|(a, b)| a.abs_diff(b) > 8))
                            .count()
                            > 20,
                        "night switches must change actual pixels"
                    );
                }
                image.save(out.join(format!("{name}-{label}.png"))).unwrap();
            }
        }
        capture.validate().await;
    });
}
