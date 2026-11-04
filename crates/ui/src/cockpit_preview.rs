//! Pilot-seat proof: real pack, renderer, panel widgets, and 1N7 FAA runway geometry.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::{
    instruments::{InstrumentInput, InstrumentPanel},
    test_capture::Capture,
    theme,
};
use eframe::egui::{self, Rect};
use flightcraft_control::Session;
use flightcraft_render::{CameraMode, Scene};

#[test]
#[ignore = "requires FLIGHTCRAFT_SLING_PACK and an offscreen GPU; saves out/cockpit/after"]
fn render_sling_cockpit_proof() {
    pollster::block_on(async {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let out = std::env::var_os("FLIGHTCRAFT_COCKPIT_PROOF")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| root.join("out/cockpit/after"));
        std::fs::create_dir_all(&out).unwrap();
        let pack = std::env::var("FLIGHTCRAFT_SLING_PACK").unwrap();
        let mut session = Session::new().unwrap();
        let contents = flightcraft_control::read_pack(std::path::Path::new(&pack)).unwrap();
        let pack_version = contents.manifest.version.clone();
        session.install_pack(&contents).unwrap();
        let receipt: serde_json::Value = serde_json::from_slice(
            &std::fs::read(std::path::Path::new(&pack).join("docs/exterior-blender-validation.json"))
                .unwrap(),
        )
        .unwrap();
        session
            .execute(
                "aircraft.load",
                serde_json::json!({"source":{"kind":"installed","id":"sling2"},"reset":true}),
            )
            .unwrap();
        session
            .install_region(
                flightcraft_control::read_region(&root.join("assets/terrain/eastern-pa")).unwrap(),
            )
            .unwrap();
        session.execute("sim.reset", serde_json::json!({"world":"eastern-pa","start":{"kind":"airport_runway","airport":"1N7","end":"07"}})).unwrap();
        let mut capture = Capture::new([1600, 1000]).await;
        let mut panel = InstrumentPanel::default();
        let asset = session.aircraft_catalog().exterior("sling2").unwrap();
        let mut scene = Scene::default();
        scene.set_aircraft("sling2");
        scene.set_world(session.sim().world());
        scene.set_exterior(asset.clone());
        scene.camera = CameraMode::Cockpit;
        scene.airspace_visible = false;
        let mut manifest = Vec::new();
        let mut images = Vec::new();
        for (label, yaw, pitch, open) in [
            ("forward", 0_f32, 0_f32, false),
            ("left", -1.25, -0.08, false),
            ("right", 1.25, -0.08, false),
            ("panel-down", 0., -0.65, false),
            ("rear-left", -2.1, -0.18, false),
            ("canopy-closed", -0.65, 0.32, false),
            ("canopy-open", -0.65, 0.32, true),
        ] {
            if open {
                session
                    .execute("aircraft.canopy", serde_json::json!({"action":"unlatch"}))
                    .unwrap();
                session
                    .execute(
                        "aircraft.canopy",
                        serde_json::json!({"action":"open","fraction":1.0}),
                    )
                    .unwrap();
                assert_eq!(session.sim().canopy().open_fraction, 1.);
            }
            scene.set_exterior_state(session.sim());
            scene.set_cockpit_look(yaw, pitch);
            let pose = *session.sim().state();
            let glass = session.glass_state();
            for _ in 0..4 {
                capture.draw(|ui| {
                    let full = ui.max_rect();
                    let split = full.top() + full.height() * 0.56;
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(Rect::from_min_max(full.min, egui::pos2(full.right(), split))),
                        |ui| {
                            // Settle the same public head-slew path used by the app.
                            scene.show(ui, &pose, 1., theme::scene());
                        },
                    );
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(Rect::from_min_max(egui::pos2(full.left(), split), full.max)),
                        |ui| {
                            ui.painter().rect_filled(ui.max_rect(), 0., theme::PANEL);
                            let config = session.aircraft_configuration().unwrap();
                            panel
                                .show(
                                    ui,
                                    &config.panel,
                                    InstrumentInput::new(session.sim().systems(), 0.)
                                        .with_aircraft(config)
                                        .with_glass(&glass),
                                    session.flight_generation(),
                                )
                                .unwrap();
                        },
                    );
                });
            }
            let image = capture.image();
            image.save(out.join(format!("{label}.png"))).unwrap();
            images.push(image);
            manifest.push(serde_json::json!({"view":label,"yaw_rad":yaw,"pitch_rad":pitch,"canopy_open_fraction":session.sim().canopy().open_fraction,"size":[1600,1000]}));
        }
        assert!(
            images[5] != images[6],
            "canopy motion must visibly change the same camera view"
        );
        assert!(images[0] != images[1], "left look must change the view");
        assert!(images[1] != images[2], "opposite cabin sides must differ");
        // Look and canopy changes cannot blur or resample the independent instrument surface.
        for image in &images[1..5] {
            assert!(
                image::imageops::crop_imm(image, 0, 600, 1600, 400).to_image()
                    == image::imageops::crop_imm(&images[0], 0, 600, 1600, 400).to_image(),
                "instrument surface changed with look or canopy"
            );
        }
        std::fs::write(out.join("manifest.json"), serde_json::to_vec_pretty(&serde_json::json!({
            "airport":"1N7","runway":"07","pack_version":pack_version,"geometry_revision":receipt["geometry_revision"],"terrain":"Released regional terrain with FAA runway geometry; no imagery or photogrammetry",
            "interior":asset.definition.interior,"state":session.sim().state(),"views":manifest,
            "adapter":format!("{:?}",capture.state.adapter.get_info())
        })).unwrap()).unwrap();
        capture.validate().await;
    });
}
