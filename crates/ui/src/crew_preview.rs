//! Screenshots use recorded crew flights and the real egui/wgpu presentation.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::{crew::CrewUi, keymap::Keymap, test_capture::Capture};
use flightcraft_control::*;
use flightcraft_render::{CameraMode, Scene};
use serde_json::Value;
#[test]
#[ignore = "requires released region data, recorded crew previews, and an offscreen GPU"]
fn render_crew_previews() {
    pollster::block_on(async {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let out = root.join("out/crew");
        let shots: Vec<Value> =
            serde_json::from_slice(&std::fs::read(out.join("water-gap-shots.json")).unwrap()).unwrap();
        let world = read_region(&root.join("assets/terrain/eastern-pa")).unwrap();
        let mut capture = Capture::new([1280, 800]).await;
        for (phase, label) in [
            ("takeoff", "cockpit-takeoff"),
            ("climb", "wing-climb"),
            ("cruise", "flyby-cruise"),
            ("final", "tower-final"),
        ] {
            let shot = shots
                .iter()
                .find(|s| {
                    if phase == "final" {
                        s["shot"] == "short_final"
                    } else if phase == "cruise" {
                        s["shot"] == "flyby_pass"
                    } else {
                        s["crew"]["phase"] == phase
                    }
                })
                .unwrap();
            let state: flightcraft_sim::State = serde_json::from_value(shot["state"].clone()).unwrap();
            let mut scene = Scene::default();
            scene.set_aircraft("sling2");
            scene.set_world(&world);
            if let Some(arrival) = shot["crew"]["plan"]["arrival"].as_object() {
                let threshold: [f64; 3] = serde_json::from_value(arrival["threshold_m"].clone()).unwrap();
                scene.set_director_runway(Some((threshold, arrival["heading_grid_rad"].as_f64().unwrap())));
            }
            scene.camera = match shot["camera"]["mode"].as_str().unwrap() {
                "cockpit" => CameraMode::Cockpit,
                "wing" => CameraMode::Wing,
                "tower" => CameraMode::Tower,
                "flyby" => CameraMode::Flyby,
                _ => CameraMode::Chase,
            };
            if phase == "cruise" {
                let start = shots.iter().find(|s| s["shot"] == "flyby_start").unwrap();
                let initial: flightcraft_sim::State = serde_json::from_value(start["state"].clone()).unwrap();
                capture.draw(|ui| scene.show(ui, &initial, 1. / 60., crate::theme::scene()));
            }
            for _ in 0..3 {
                capture.draw(|ui| {
                    scene.show(ui, &state, 1. / 60., crate::theme::scene());
                    let rect = ui.max_rect();
                    ui.painter().text(
                        rect.left_top() + eframe::egui::vec2(20., 22.),
                        eframe::egui::Align2::LEFT_TOP,
                        format!("PILOT IN COMMAND · {label}"),
                        eframe::egui::FontId::proportional(24.),
                        crate::theme::TEXT,
                    );
                });
            }
            capture
                .image()
                .save(out.join(format!("director-{label}.png")))
                .unwrap();
        }
        let mut s = Session::new().unwrap();
        s.install_region(world).unwrap();
        assert!(s.load_packs_directory(&root.join(".cache/packs")).is_empty());
        s.execute(
            "aircraft.load",
            serde_json::json!({"source":{"kind":"installed","id":"sling2"}}),
        )
        .unwrap();
        s.enable_app();
        s.execute(
            "sim.reset",
            serde_json::json!({"world":"eastern-pa","start":{"runway":{"airport":"1N7","end":"07"}}}),
        )
        .unwrap();
        s.execute(
            "crew.mode",
            serde_json::json!({"mode":"watching","director":true}),
        )
        .unwrap();
        s.execute("crew.intent",serde_json::json!({"intent":{"kind":"scenic","preset":"delaware_water_gap","airport":"1N7","runway":"07"}})).unwrap();
        capture.resize([540, 840]);
        s.execute("map.set", serde_json::json!({"range_nm":12.})).unwrap();
        let mut panel = CrewUi::scenic_preview();
        let keymap = Keymap::default();
        for _ in 0..3 {
            capture.draw(|ui| {
                eframe::egui::Frame::new()
                    .fill(crate::theme::PANEL)
                    .inner_margin(20.)
                    .show(ui, |ui| {
                        ui.set_min_height(800.);
                        panel.panel(ui, &mut s, &keymap);
                    });
            });
        }
        capture.image().save(out.join("fly-me-panel.png")).unwrap();
        capture.validate().await;
    });
}

#[test]
#[ignore = "requires regional terrain, bundled aircraft, and an offscreen GPU"]
fn render_crew_screen_pages() {
    pollster::block_on(async {
        use flightcraft_control::screens::Layout;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let out = root.join("out/crew");
        std::fs::create_dir_all(&out).unwrap();
        let mut s = Session::new().unwrap();
        assert!(s.load_packs_directory(&root.join(".cache/packs")).is_empty());
        s.install_region(read_region(&root.join("assets/terrain/eastern-pa")).unwrap())
            .unwrap();
        s.execute(
            "aircraft.load",
            serde_json::json!({"source":{"kind":"installed","id":"sling2"}}),
        )
        .unwrap();
        s.execute(
            "sim.reset",
            serde_json::json!({"world":"eastern-pa","start":{"runway":{"airport":"1N7","end":"07"}}}),
        )
        .unwrap();
        s.execute(
            "crew.mode",
            serde_json::json!({"mode":"watching","director":true}),
        )
        .unwrap();
        s.execute("crew.intent", serde_json::json!({"intent":{"kind":"scenic","preset":"delaware_water_gap","airport":"1N7","runway":"07"}})).unwrap();
        s.execute("map.set", serde_json::json!({"range_nm":12.})).unwrap();
        let layout: Layout =
            serde_json::from_str(include_str!("../../../examples/layouts/watching.layout.json")).unwrap();
        s.execute("layout.set", serde_json::json!({"id":0,"layout":layout}))
            .unwrap();
        let screen = s.screens().windows.first().unwrap().clone();
        let mut canvas = crate::screens::Canvas::default();
        let mut capture = Capture::new([1500, 900]).await;
        for _ in 0..3 {
            capture.draw(|ui| canvas.show(ui, &screen, &mut s, true));
        }
        assert!(canvas.error.is_none(), "{:?}", canvas.error);
        capture.image().save(out.join("watching-pages.png")).unwrap();
        capture.validate().await;
    });
}
