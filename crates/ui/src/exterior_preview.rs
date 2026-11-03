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
fn registration_decal_reads_the_override_on_both_sides() {
    pollster::block_on(async {
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/tail-number");
        std::fs::create_dir_all(&out).unwrap();
        let mut capture = Capture::new([1600, 800]).await;
        let mut names = vec!["corvid", "larrow"];
        let sling = std::env::var("FLIGHTCRAFT_SLING_PACK").ok();
        if sling.is_some() {
            names.push("sling2");
        }
        for name in names {
            let mut session = Session::new().unwrap();
            if let Some(path) = sling.as_ref().filter(|_| name == "sling2") {
                session
                    .install_pack(&flightcraft_control::read_pack(std::path::Path::new(path)).unwrap())
                    .unwrap();
            }
            session
                .execute(
                    "aircraft.load",
                    serde_json::json!({"source":{"kind":"installed","id":name},"reset":true}),
                )
                .unwrap();
            let asset = session
                .aircraft_catalog()
                .exterior(name)
                .or_else(|| flightcraft_render::exterior::builtin(name))
                .unwrap();
            let cg = session.sim().aircraft().mass.cg_m.map(|x| x as f32);
            let mut state = *session.sim().state();
            state.position_m = [0., 0., -20.];
            state.attitude = [1., 0., 0., 0.];
            // Geometry/unit tests separately exercise the >150 m model selection.
            for sign in [-1_f32, 1.] {
                let mut scene = Scene::default();
                scene.set_aircraft(name);
                scene.set_exterior(asset.clone());
                scene.set_exterior_state(session.sim());
                scene.airspace_visible = false;
                scene.set_screen_view(Some(flightcraft_render::ScreenView {
                    heading_offset_rad: 0.,
                    pitch_rad: 0.,
                    fov_rad: 30_f32.to_radians(),
                    fixed_eye_ned_m: Some([0., sign * 6., -20.]),
                }));
                let mut shots = Vec::new();
                for number in ["N111FC", "N888FC"] {
                    session
                        .execute(
                            "comms.callsign",
                            serde_json::json!({"registration":number,"persist":false}),
                        )
                        .unwrap();
                    let callsign = session.pilot_callsign();
                    scene.set_registration(callsign.overridden.then_some(callsign.registration.as_str()));
                    capture.draw(|ui| scene.show(ui, &state, 0., theme::scene()));
                    let image = capture.image();
                    image
                        .save(out.join(format!("{name}-{number}-{sign}.png")))
                        .unwrap();
                    shots.push(image);
                }
                let node = asset
                    .lod0
                    .nodes
                    .iter()
                    .find(|n| n.name == asset.definition.registration.node)
                    .unwrap();
                let patch: Vec<_> = node
                    .vertices
                    .iter()
                    .filter(|v| v.position[1] * sign > 0. && v.normal[1].abs() > 0.1)
                    .collect();
                let low_x = patch.iter().map(|v| v.position[0]).fold(f32::INFINITY, f32::min);
                let high_x = patch
                    .iter()
                    .map(|v| v.position[0])
                    .fold(f32::NEG_INFINITY, f32::max);
                let low_z = patch.iter().map(|v| v.position[2]).fold(f32::INFINITY, f32::min);
                let high_z = patch
                    .iter()
                    .map(|v| v.position[2])
                    .fold(f32::NEG_INFINITY, f32::max);
                let height = high_z - low_z;
                let width = (height * 0.6).min((high_x - low_x) / 7.25);
                let total = width * 7.25;
                let focal = 400. / 15_f32.to_radians().tan();
                // Recognize all three replacement digits by independent stroke/loop probes:
                // a One has a center stem and no left bowl; an Eight has a left bowl and a hole.
                // This checks coverage inside each glyph at the actual pack node, not just image changes.
                for digit in 1..=3 {
                    for (u, one_ink) in [(0.5, true), (0.08, false)] {
                        let x =
                            (low_x + high_x) * 0.5 + sign * (width * (digit as f32 * 1.25 + u) - total * 0.5);
                        let padding = 0.08 * height - width * 0.075;
                        let z = low_z + (0.29 * height - padding) * height / (height - 2. * padding);
                        let nearest = patch
                            .iter()
                            .min_by(|a, b| {
                                let distance = |v: &&flightcraft_aero::exterior::ModelVertex| {
                                    (v.position[0] - x).powi(2) + (v.position[2] - z).powi(2)
                                };
                                distance(a).total_cmp(&distance(b))
                            })
                            .unwrap();
                        let [vx, vy, vz] = nearest.position;
                        let [nx, ny, nz] = nearest.normal;
                        let y = vy - (nx * (x - vx) + nz * (z - vz)) / ny + sign * 0.001;
                        let depth = 6. - sign * (y - cg[1]);
                        let px = (800. + sign * (x - cg[0]) * focal / depth).round() as u32;
                        let py = (400. + (z - cg[2]) * focal / depth).round() as u32;
                        let light = |image: &image::RgbaImage| {
                            let p = image.get_pixel(px, py);
                            u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2])
                        };
                        let (one, eight) = (light(&shots[0]), light(&shots[1]));
                        assert!(
                            if one_ink {
                                one + 40 < eight
                            } else {
                                eight + 40 < one
                            },
                            "{name} side {sign}, digit {digit}, u={u}: One={one}, Eight={eight} at ({px},{py})"
                        );
                    }
                }
                scene.set_registration(None);
                capture.draw(|ui| scene.show(ui, &state, 0., theme::scene()));
                let restored = capture.image();
                let mut original = Scene::default();
                original.set_aircraft(name);
                original.set_exterior(asset.clone());
                original.set_exterior_state(session.sim());
                original.airspace_visible = false;
                original.set_screen_view(Some(flightcraft_render::ScreenView {
                    heading_offset_rad: 0.,
                    pitch_rad: 0.,
                    fov_rad: 30_f32.to_radians(),
                    fixed_eye_ned_m: Some([0., sign * 6., -20.]),
                }));
                capture.draw(|ui| original.show(ui, &state, 0., theme::scene()));
                assert_eq!(
                    restored,
                    capture.image(),
                    "Use pack must restore the baked image exactly"
                );
            }
        }
        capture.validate().await;
    });
}

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
