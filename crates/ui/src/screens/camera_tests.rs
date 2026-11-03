//! Headless checks of camera commands through the actual shared window/page path.
#![allow(clippy::unwrap_used)]
use super::*;
use flightcraft_render::CameraMode;

#[test]
fn camera_elapsed_tracks_each_pages_paints_and_resets_with_the_flight() {
    let mut page = PageUi::default();
    assert_eq!(page.camera_elapsed(10., 1), 0.);
    assert_eq!(page.camera_elapsed(10., 1), 0.);
    assert!((page.camera_elapsed(11.3, 1) - 1.3).abs() < 1e-6);
    assert_eq!(page.camera_elapsed(20., 2), 0.);
    assert_eq!(page.camera_elapsed(19., 2), 0.);
    assert_eq!(page.camera_elapsed(f64::NAN, 2), 0.);
    assert_eq!(page.camera_elapsed(21., 2), 0.);
}

#[test]
fn main_page_obeys_camera_commands_during_crew_takeoff_without_director() {
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let mut owner = takeoff_session();
    let mut canvas = Canvas::default();
    let mut independent = PageUi::default();
    let mut page = Page::new(PageKind::Outside);
    page.camera.view = View::Right;
    let mut climbed = false;
    for (frame, (command_mode, expected)) in [
        ("chase", CameraMode::Chase),
        ("wing", CameraMode::Wing),
        ("flyby", CameraMode::Flyby),
        ("tower", CameraMode::Tower),
        ("cockpit", CameraMode::Cockpit),
        ("chase", CameraMode::Chase),
    ]
    .into_iter()
    .enumerate()
    {
        owner
            .execute(
                "view.set_camera",
                json!({"mode":command_mode,"distance_m":7.,"height_m":1.2}),
            )
            .unwrap();
        owner.execute("sim.step", json!({"steps":1200})).unwrap();
        assert!(!owner.crew_state().director);
        assert!(!owner.sim().readout().crashed);
        climbed |= owner.sim().readout().height_agl_m > 10.;
        // The window receives a new presentation copy, while its page/scene persists.
        let mut snapshot = owner.presentation_copy();
        let screen = snapshot.screens().windows.first().unwrap().clone();
        let _ = ctx.run(
            egui::RawInput {
                time: Some(frame as f64 * 10.),
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280., 800.))),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    canvas.show(ui, &screen, &mut snapshot, true);
                    independent.show(ui, &page, &mut snapshot, "independent").unwrap();
                });
            },
        );
        assert!(canvas.error.is_none(), "{:?}", canvas.error);
        let outside = screen
            .layout
            .elements
            .iter()
            .find(|e| e.page.kind == PageKind::Outside)
            .unwrap();
        assert!(outside.page.follow_main_camera);
        let page_ui = canvas.pages.get(&outside.id).unwrap();
        assert_eq!(page_ui.scene.camera, expected);
        assert_eq!(page_ui.camera_time, Some(frame as f64 * 10.));
        assert_eq!(independent.scene.camera, CameraMode::Cockpit);
    }
    assert!(climbed, "exercise the crew's takeoff-to-climb camera transitions");
}

fn takeoff_session() -> Session {
    let mut owner = Session::new().unwrap();
    owner.enable_app();
    owner
        .execute("sim.reset", json!({"start":{"kind":"runway"},"paused":true}))
        .unwrap();
    owner
        .execute("crew.mode", json!({"mode":"watching","director":false}))
        .unwrap();
    owner
        .execute(
            "crew.intent",
            json!({"intent":{"kind":"fly_to_airport","airport":"default","runway":"36","altitude_m":315.}}),
        )
        .unwrap();
    owner
        .execute("crew.handover", json!({"to":"crew","actor":"crew"}))
        .unwrap();
    owner
        .execute(
            "crew.handover",
            json!({"to":"crew","actor":"pilot","confirm":true}),
        )
        .unwrap();
    owner
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "requires an offscreen GPU; writes moving camera captures to out/camera-follow"]
fn camera_follow_gpu_preview() {
    pollster::block_on(async {
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/camera-follow");
        std::fs::create_dir_all(&out).unwrap();
        let mut capture = crate::test_capture::Capture::new([1280, 800]).await;
        for (sequence, mode) in (0..).zip(["chase", "wing", "flyby", "tower"]) {
            let mut owner = takeoff_session();
            owner
                .execute(
                    "view.set_camera",
                    json!({"mode":mode,"distance_m":7.,"height_m":1.2}),
                )
                .unwrap();
            let mut canvas = Canvas::default();
            let mut previous_steps = 0;
            for steps in [0, 156, 1560, 4680, 7800] {
                if steps > previous_steps {
                    owner
                        .execute("sim.step", json!({"steps":steps - previous_steps}))
                        .unwrap();
                }
                assert!(!owner.sim().readout().crashed);
                let mut snapshot = owner.presentation_copy();
                let screen = snapshot.screens().windows.first().unwrap().clone();
                capture.frame = sequence * 10000 + steps;
                capture.draw(|ui| canvas.show(ui, &screen, &mut snapshot, true));
                assert!(canvas.error.is_none(), "{:?}", canvas.error);
                capture
                    .image()
                    .save(out.join(format!("{mode}-{steps:05}.png")))
                    .unwrap();
                previous_steps = steps;
            }
        }
        capture.validate().await;
    });
}
