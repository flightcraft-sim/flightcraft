//! Reproducible offscreen proof for the local 3D Tiles provider. No window or network.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::{test_capture::Capture, theme};
use flightcraft_render::{
    CameraMode, Scene, ScreenView,
    scenery::{LocalProvider, Location, Provider, Quality},
};
use flightcraft_sim::State;
use flightcraft_world::{HeightTile, Region, Surface, World};
use std::sync::{Arc, Mutex};

fn reference_world() -> World {
    let region=Region::from_json(r#"{"id":"scenery-preview","name":"Blairstown imagery preview","origin":{"latitude_deg":40.96905613156273,"longitude_deg":-75.00242278301268,"field_elevation_m":112,"runway_heading_deg":60.76659070034455,"runway_length_m":948.5376,"runway_width_m":21.336},"southwest_m":[-1000,-1000],"tiles_north":1,"tiles_east":1,"spacing_m":125,"tile_size":33,"runway_blend_m":200,"source":"Original level preview surface","licence":"Apache-2.0","attribution":"Original preview surface; elevation is not measured","tiles":["000.fcht"]}"#).unwrap();
    World::from_region(
        region,
        vec![
            HeightTile::new(
                [-1000., -1000.],
                125.,
                33,
                vec![112.; 33 * 33],
                vec![Surface::Grass; 32 * 32],
            )
            .unwrap(),
        ],
    )
    .unwrap()
}
fn provider() -> Provider {
    Provider {location:Location::LocalPath(std::env::var_os("FLIGHTCRAFT_SCENERY_TILESET").map(std::path::PathBuf::from).unwrap_or_else(||"/var/mnt/inland/builds/codex-jobs/scenery/eastern-pa/1n7-test/reference-built/tileset.json".into())),token_config_key:None,enabled:true}
}
fn origin() -> [f64; 2] {
    [40.96905613156273, -75.00242278301268]
}
fn pose(n: f64, e: f64, height: f64, heading: f64) -> State {
    State {
        position_m: [n, e, -height],
        attitude: [(heading / 2.).cos(), 0., 0., (heading / 2.).sin()],
        ..State::default()
    }
}

#[test]
#[ignore = "requires an offscreen GPU and scenery build --sample; see docs/scenery.md"]
fn render_1n7_scenery_reference_shots() {
    pollster::block_on(async {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let out = root.join("out/scenery");
        std::fs::create_dir_all(&out).unwrap();
        let stream = Arc::new(Mutex::new(LocalProvider::open(&provider(), origin()).unwrap()));
        let mut capture = Capture::new([1280, 720]).await;
        let mut scene = Scene::default();
        scene.set_world(&reference_world());
        scene.set_scenery(Some(stream.clone()));
        scene.scenery_quality = Quality::Medium;
        scene.airspace_visible = false;
        let heading = 60.76659070034455_f64.to_radians();
        let (sn, ce) = (heading.cos(), heading.sin());
        let reference = image::open("/home/ray/builds/codex-jobs/1n7-naip-sample.jpg")
            .unwrap()
            .to_rgba8();
        let texture = eframe::egui::ColorImage::from_rgba_unmultiplied(
            [reference.width() as usize, reference.height() as usize],
            reference.as_raw(),
        );
        let mut reference_texture = None;
        for (name, label, position, camera, eye) in [
            (
                "runway-07",
                "Runway 07 — imagery preview",
                pose(25. * sn, 25. * ce, 113., heading),
                CameraMode::Cockpit,
                None,
            ),
            (
                "short-final-25",
                "Short final, Runway 25 — imagery preview",
                pose(1050. * sn, 1050. * ce, 135., heading + std::f64::consts::PI),
                CameraMode::Cockpit,
                None,
            ),
            (
                "hangars",
                "Hangar flyby — imagery preview",
                pose(230., 320., 145., heading),
                CameraMode::Flyby,
                Some([150., 500., -195.]),
            ),
        ] {
            scene.reset_camera();
            scene.camera = camera;
            scene.set_screen_view(eye.map(|fixed| ScreenView {
                heading_offset_rad: 0.,
                pitch_rad: 0.,
                fov_rad: 0.9,
                fixed_eye_ned_m: Some(fixed),
            }));
            // Wait for actual worker results through frame updates; a cold scene must first
            // obtain root coverage. Keep the last warm render for each known viewpoint.
            let mut loaded = false;
            let loading_started = std::time::Instant::now();
            for frame in 0..3000 {
                if loading_started.elapsed() > std::time::Duration::from_secs(30) {
                    break;
                }
                capture.draw(|ui| {
                    let full=ui.max_rect();
                    let left=eframe::egui::Rect::from_min_max(full.min,eframe::egui::pos2(full.left()+full.width()*0.68,full.bottom()));
                    ui.scope_builder(eframe::egui::UiBuilder::new().max_rect(left),|ui| {
                        scene.show(ui,&position,1./60.,theme::scene());
                        ui.painter().text(left.left_top()+eframe::egui::vec2(12.,12.),eframe::egui::Align2::LEFT_TOP,label,eframe::egui::FontId::proportional(15.),theme::TEXT);
                    });
                    let right=eframe::egui::Rect::from_min_max(eframe::egui::pos2(left.right()+8.,full.top()),full.max);
                    ui.scope_builder(eframe::egui::UiBuilder::new().max_rect(right),|ui| {
                        ui.label("NAIP reference (overhead)");
                        let texture=reference_texture.get_or_insert_with(||ui.ctx().load_texture("1N7 reference",texture.clone(),Default::default()));
                        ui.image((texture.id(),eframe::egui::vec2(right.width(),right.width()*860./1600.)));
                        ui.label("Reference imagery resampled for pipeline proof. Terrain is level. Measured trees, roofs, and water are pending.");
                    });
                });
                let stats = stream.lock().unwrap().stats();
                assert_eq!(stats.failed_tiles, 0, "local content decode failed");
                if stats.resident_tiles > 8 && stats.pending_tiles == 0 && frame > 20 {
                    loaded = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(
                loaded,
                "local tile worker must produce rendered coverage: {:?}",
                stream.lock().unwrap().stats()
            );
            for _ in 0..32 {
                capture.draw(|ui| scene.show(ui, &position, 1. / 60., theme::scene()));
            }
            let image = capture.image();
            scene.set_scenery(None);
            capture.draw(|ui| scene.show(ui, &position, 1. / 60., theme::scene()));
            let base = capture.image();
            assert!(
                image
                    .pixels()
                    .zip(base.pixels())
                    .filter(|(a, b)| a.0.iter().zip(b.0).take(3).any(|(a, b)| a.abs_diff(b) > 12))
                    .count()
                    > 2000,
                "loaded orthoimagery must change the actual view pixels"
            );
            scene.set_scenery(Some(stream.clone()));
            assert!(
                image.pixels().filter(|p| p[1] > p[0] && p[1] > p[2]).count() > 1000,
                "imagery must contribute visible green pixels"
            );
            image.save(out.join(format!("{name}.png"))).unwrap();
            // Store the comparison with the reference on the right, preserving scene pixels.
            let mut comparison = image::RgbaImage::new(1920, 720);
            image::imageops::replace(&mut comparison, &image, 0, 0);
            let reference =
                image::imageops::resize(&reference, 640, 344, image::imageops::FilterType::Triangle);
            image::imageops::replace(&mut comparison, &reference, 1280, 0);
            comparison.save(out.join(format!("{name}-naip.png"))).unwrap();
        }
        capture.validate().await;
    });
}

#[test]
#[ignore = "requires target integrated GPU, a warm local tileset, and an unloaded machine"]
fn scenery_1080p_frame_budget() {
    pollster::block_on(async {
        let mut capture = Capture::new([1920, 1080]).await;
        let mut scene = Scene::default();
        scene.set_world(&reference_world());
        scene.airspace_visible = false;
        let stream = Arc::new(Mutex::new(LocalProvider::open(&provider(), origin()).unwrap()));
        scene.set_scenery(Some(stream.clone()));
        scene.scenery_quality = Quality::Low;
        scene.camera = CameraMode::Cockpit;
        let pose = pose(25., 40., 114., 60.76659070034455_f64.to_radians());
        let mut loaded = false;
        let loading_started = std::time::Instant::now();
        for frame in 0..3000 {
            if loading_started.elapsed() > std::time::Duration::from_secs(30) {
                break;
            }
            capture.draw(|ui| scene.show(ui, &pose, 1. / 60., theme::scene()));
            let stats = stream.lock().unwrap().stats();
            assert_eq!(stats.failed_tiles, 0);
            if stats.selected_tiles > 0 && stats.pending_tiles == 0 && frame > 30 {
                loaded = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            loaded,
            "benchmark must render loaded scenery: {:?}",
            stream.lock().unwrap().stats()
        );
        for _ in 0..32 {
            capture.draw(|ui| scene.show(ui, &pose, 1. / 60., theme::scene()));
        }
        let mut times = Vec::new();
        for _ in 0..120 {
            let now = std::time::Instant::now();
            capture.draw(|ui| scene.show(ui, &pose, 1. / 60., theme::scene()));
            times.push(now.elapsed().as_secs_f64() * 1000.);
        }
        times.sort_by(f64::total_cmp);
        let p95 = times[114];
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/scenery");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("frame-budget.json"),serde_json::to_vec_pretty(&serde_json::json!({"width":1920,"height":1080,"quality":"low","adapter":capture.state.adapter.get_info().name,"p95_ms":p95,"budget_ms":1000./60.})).unwrap()).unwrap();
        assert!(
            p95 <= 1000. / 60.,
            "1080p p95 {p95:.2} ms exceeds the 60 fps frame budget"
        );
        capture.validate().await;
    });
}
