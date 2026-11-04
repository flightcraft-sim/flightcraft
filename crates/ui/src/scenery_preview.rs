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
    reference_world_at(112.)
}
fn reference_world_at(height: f64) -> World {
    let mut region=Region::from_json(r#"{"id":"scenery-preview","name":"Blairstown imagery preview","origin":{"latitude_deg":40.96905613156273,"longitude_deg":-75.00242278301268,"field_elevation_m":112,"runway_heading_deg":60.76659070034455,"runway_length_m":948.5376,"runway_width_m":21.336},"southwest_m":[-1000,-1000],"tiles_north":1,"tiles_east":1,"spacing_m":125,"tile_size":33,"runway_blend_m":200,"source":"Original level preview surface","licence":"Apache-2.0","attribution":"Original preview surface; elevation is not measured","tiles":["000.fcht"]}"#).unwrap();
    region.origin.field_elevation_m = height;
    World::from_region(
        region,
        vec![
            HeightTile::new(
                [-1000., -1000.],
                125.,
                33,
                vec![height as f32; 33 * 33],
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
fn scenery_sling(scene: &mut Scene) {
    let path = std::env::var_os("FLIGHTCRAFT_SLING_PACK")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/home/ray/builds/codex-jobs/scenery/sling-v9-proof".into());
    let definition: flightcraft_aero::exterior::Exterior =
        serde_json::from_slice(&std::fs::read(path.join("exterior.json")).unwrap()).unwrap();
    let model = |name: &str| {
        Arc::new(
            flightcraft_aero::exterior::load_glb(&std::fs::read(path.join(name)).unwrap(), &definition)
                .unwrap(),
        )
    };
    let asset = flightcraft_aero::exterior::ExteriorAsset {
        lod0: model("exterior.glb"),
        lod1: model("exterior-lod1.glb"),
        lod2: Some(model("exterior-lod2.glb")),
        definition,
    };
    let variant: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path.join("variant.json")).unwrap()).unwrap();
    let mut pose = flightcraft_render::exterior::Pose::default();
    pose.cg_m = serde_json::from_value(variant["empty_mass"]["cg_m"].clone()).unwrap();
    pose.height_agl_m = 1.1;
    scene.set_exterior_pose(pose).unwrap();
    scene.set_aircraft("sling2");
    scene.set_exterior(Arc::new(asset));
    scene
        .set_lighting(flightcraft_render::lighting::LightingSettings {
            utc_hour: 18.,
            ..Default::default()
        })
        .unwrap();
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
        let out = std::env::var_os("FLIGHTCRAFT_SCENERY_PROOF_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| root.join("out/scenery"));
        std::fs::create_dir_all(&out).unwrap();
        let stream = Arc::new(Mutex::new(LocalProvider::open(&provider(), origin()).unwrap()));
        let mut capture = Capture::new([1280, 720]).await;
        let mut scene = Scene::default();
        scene.set_world(&reference_world());
        scenery_sling(&mut scene);
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
                pose(180., 215., 114.1, heading),
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
                if loading_started.elapsed() > std::time::Duration::from_secs(90) {
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
                        ui.label("Measured lidar terrain, crowns, and roofs. Procedural facades. Water and painted-runway acceptance remain pending.");
                    });
                });
                if frame == 0 {
                    capture.validate().await;
                    eprintln!("cold Sling preparation: {:?}", loading_started.elapsed());
                }
                let stats = stream.lock().unwrap().stats();
                assert_eq!(stats.failed_tiles, 0, "local content decode failed");
                if stats.resident_tiles > 8 && stats.pending_tiles == 0 && frame > 20 {
                    loaded = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            eprintln!(
                "loaded {name}: {:?}, {:?}",
                loading_started.elapsed(),
                stream.lock().unwrap().stats()
            );
            assert!(
                loaded,
                "local tile worker must produce rendered coverage: {:?}",
                stream.lock().unwrap().stats()
            );
            let minimum = std::env::var("FLIGHTCRAFT_SCENERY_MIN_INSTANCES")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(0);
            assert!(
                stream.lock().unwrap().stats().selected_instances >= minimum,
                "the selected coverage must contain the measured GPU tree instances"
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
        scenery_sling(&mut scene);
        scene.airspace_visible = false;
        let stream = Arc::new(Mutex::new(LocalProvider::open(&provider(), origin()).unwrap()));
        scene.set_scenery(Some(stream.clone()));
        scene.scenery_quality = Quality::Low;
        scene.camera = CameraMode::Cockpit;
        let pose = pose(25., 40., 114., 60.76659070034455_f64.to_radians());
        let mut loaded = false;
        let loading_started = std::time::Instant::now();
        for frame in 0..3000 {
            if loading_started.elapsed() > std::time::Duration::from_secs(90) {
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
        let stats = stream.lock().unwrap().stats();
        let minimum = std::env::var("FLIGHTCRAFT_SCENERY_MIN_INSTANCES")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(0);
        assert!(
            stats.selected_instances >= minimum,
            "frame budget must include the measured tree instances"
        );
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
        std::fs::write(out.join("frame-budget.json"),serde_json::to_vec_pretty(&serde_json::json!({"width":1920,"height":1080,"quality":"low","adapter":capture.state.adapter.get_info().name,"p95_ms":p95,"budget_ms":6.,"selected_instances":stats.selected_instances,"selected_tiles":stats.selected_tiles,"resident_bytes":stats.resident_bytes})).unwrap()).unwrap();
        assert!(
            p95 <= 6.,
            "1080p p95 {p95:.2} ms exceeds the Low 6 ms frame budget"
        );
        capture.validate().await;
    });
}

/// Real-data proof only: this test never substitutes an invented city for the TUM dataset.
#[test]
#[ignore = "requires fetched NYC city tiles, an installed New York region, and an offscreen GPU"]
fn render_nyc_city_reference_shots() {
    pollster::block_on(async {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let region_root = std::env::var_os("FLIGHTCRAFT_CITY_REGION")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| root.join("assets/terrain/new-york"));
        let world = flightcraft_control::read_region(&region_root).unwrap();
        let region = world.region().unwrap();
        let origin = [region.origin.latitude_deg, region.origin.longitude_deg];
        let projection = flightcraft_world::geo::LocalProjection::new(origin[0], origin[1]).unwrap();
        let path = std::env::var_os("FLIGHTCRAFT_CITY_TILESET")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                "/var/mnt/inland/builds/codex-jobs/scenery/new-york/nyc-lidar/built/tileset.json".into()
            });
        let provider = Provider {
            location: Location::LocalPath(path),
            enabled: true,
            token_config_key: None,
        };
        let stream = Arc::new(Mutex::new(LocalProvider::open(&provider, origin).unwrap()));
        let mut capture = Capture::new([1280, 720]).await;
        let mut scene = Scene::default();
        scene.set_world(&world);
        scene.set_scenery(Some(stream.clone()));
        scene.scenery_quality = Quality::Medium;
        scene.airspace_visible = false;
        let out = root.join("out/scenery/nyc");
        std::fs::create_dir_all(&out).unwrap();
        for (name, lat, lon, heading, pitch) in [
            ("hudson-1000ft", 40.7100, -74.0350, 0_f64, 0_f64),
            ("manhattan-skyline-1000ft", 40.6750, -74.0450, 10., 0.),
            ("hudson-shoreline-1000ft", 40.7600, -74.0150, 180., -15.),
        ] {
            let [n, e] = projection.project(lat, lon).unwrap();
            let height = 1000. * 0.3048;
            let heading = heading.to_radians();
            let pitch = pitch.to_radians();
            let target = pose(
                n + 1000. * heading.cos() * pitch.cos(),
                e + 1000. * heading.sin() * pitch.cos(),
                height + 1000. * pitch.sin(),
                heading,
            );
            scene.reset_camera();
            scene.camera = CameraMode::Cockpit;
            scene.set_screen_view(Some(ScreenView {
                heading_offset_rad: 0.,
                pitch_rad: 0.,
                fov_rad: 60_f32.to_radians(),
                fixed_eye_ned_m: Some([n as f32, e as f32, -height as f32]),
            }));
            let start = std::time::Instant::now();
            let mut loaded = false;
            for frame in 0..6000 {
                capture.draw(|ui| scene.show(ui, &target, 1. / 60., theme::scene()));
                let stats = stream.lock().unwrap().stats();
                assert_eq!(stats.failed_tiles, 0, "City content failed to decode");
                if frame > 32 && stats.resident_tiles > 0 && stats.pending_tiles == 0 {
                    loaded = true;
                    break;
                }
                if start.elapsed() > std::time::Duration::from_secs(60) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(loaded, "City coverage did not load");
            for _ in 0..16 {
                capture.draw(|ui| scene.show(ui, &target, 1. / 60., theme::scene()));
            }
            let image = capture.image();
            scene.set_scenery(None);
            capture.draw(|ui| scene.show(ui, &target, 1. / 60., theme::scene()));
            let base = capture.image();
            assert!(
                image
                    .pixels()
                    .zip(base.pixels())
                    .filter(|(a, b)| a.0.iter().zip(b.0).take(3).any(|(a, b)| a.abs_diff(b) > 12))
                    .count()
                    > 1000,
                "City geometry must contribute visible pixels"
            );
            image.save(out.join(format!("{name}.png"))).unwrap();
            scene.set_scenery(Some(stream.clone()));
        }
        capture.validate().await;
    });
}

#[test]
#[ignore = "requires an offscreen GPU and a built 1N7 test-area tileset"]
fn imagery_replaces_higher_coarse_ground_only_in_the_scenery_footprint() {
    pollster::block_on(async {
        let mut capture = Capture::new([960, 640]).await;
        let mut scene = Scene::default();
        scene.set_world(&reference_world_at(1000.));
        scene.airspace_visible = false;
        scene.camera = CameraMode::Flyby;
        scene.set_screen_view(Some(ScreenView {
            heading_offset_rad: 0.,
            pitch_rad: 0.,
            fov_rad: 1.,
            fixed_eye_ned_m: Some([150., 500., -3000.]),
        }));
        let position = pose(230., 320., 112., 0.);
        // Diagnostic colors identify base terrain independently of NAIP. These are test-only.
        let mut palette = theme::scene();
        for color in [
            &mut palette.ground,
            &mut palette.earth,
            &mut palette.dry_grass,
            &mut palette.rock,
            &mut palette.forest,
            &mut palette.shoreline,
            &mut palette.water,
            &mut palette.grid,
            &mut palette.runway,
            &mut palette.shadow,
        ] {
            *color = [1., 0., 1., 1.];
        }
        palette.sky_horizon = [0.8, 0.8, 0.8, 1.];
        palette.marking = [0.95, 0.95, 0.95, 1.];
        for _ in 0..32 {
            capture.draw(|ui| scene.show(ui, &position, 1. / 60., palette));
        }
        let base = capture.image();
        let stream = Arc::new(Mutex::new(LocalProvider::open(&provider(), origin()).unwrap()));
        scene.set_scenery(Some(stream.clone()));
        scene.scenery_quality = Quality::Low;
        let started = std::time::Instant::now();
        let mut loaded = false;
        for frame in 0..3000 {
            if started.elapsed() > std::time::Duration::from_secs(90) {
                break;
            }
            capture.draw(|ui| scene.show(ui, &position, 1. / 60., palette));
            let stats = stream.lock().unwrap().stats();
            assert_eq!(stats.failed_tiles, 0);
            if stats.selected_tiles > 0 && stats.pending_tiles == 0 && frame > 32 {
                loaded = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(loaded, "scenery must be loaded before checking occlusion");
        for _ in 0..32 {
            capture.draw(|ui| scene.show(ui, &position, 1. / 60., palette));
        }
        let imagery = capture.image();
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/scenery");
        std::fs::create_dir_all(&out).unwrap();
        base.save(out.join("higher-coarse-ground-baseline.png")).unwrap();
        imagery
            .save(out.join("higher-coarse-ground-imagery.png"))
            .unwrap();
        // NAIP can show dormant vegetation. Detect its actual, non-magenta texture pixels
        // against the independent baseline rather than assuming a particular seasonal color.
        let exposed = imagery
            .pixels()
            .zip(base.pixels())
            .filter(|(p, b)| {
                p.0.iter().zip(b.0).take(3).any(|(a, b)| a.abs_diff(b) > 20)
                    && i16::from(p[0]) - i16::from(p[1]) < 20
                    && i16::from(p[2]) - i16::from(p[1]) < 20
            })
            .count();
        assert!(
            exposed > 2000,
            "loaded imagery must remain visible when fallback terrain is higher; exposed pixels: {exposed}"
        );
        // Outside the test-area footprint, coarse ground must still cover the scene.
        let magenta = |image: &image::RgbaImage| {
            image
                .pixels()
                .filter(|p| p[0] > p[1].saturating_add(30) && p[2] > p[1].saturating_add(30))
                .count()
        };
        assert!(
            magenta(&imagery) > 1000,
            "coarse terrain must remain outside the replacement footprint"
        );
        capture.validate().await;
    });
}

#[test]
#[ignore = "requires an offscreen GPU and a built local test-area tileset"]
fn tile_replacement_keeps_gpu_coverage_while_uploads_are_spread_out() {
    pollster::block_on(async {
        let mut capture = Capture::new([640, 480]).await;
        let mut scene = Scene::default();
        scene.set_world(&reference_world_at(1000.));
        scene.camera = CameraMode::Flyby;
        scene.airspace_visible = false;
        scene.scenery_quality = Quality::High;
        let eye = [150., 500., -3000.];
        scene.set_screen_view(Some(ScreenView {
            heading_offset_rad: 0.,
            pitch_rad: 0.,
            fov_rad: 1.,
            fixed_eye_ned_m: Some(eye),
        }));
        let position = pose(230., 320., 112., 0.);
        let mut palette = theme::scene();
        for color in [
            &mut palette.ground,
            &mut palette.earth,
            &mut palette.dry_grass,
            &mut palette.rock,
            &mut palette.forest,
            &mut palette.shoreline,
            &mut palette.water,
            &mut palette.grid,
        ] {
            *color = [1., 0., 1., 1.];
        }
        let stream = Arc::new(Mutex::new(LocalProvider::open(&provider(), origin()).unwrap()));
        scene.set_scenery(Some(stream.clone()));
        let started = std::time::Instant::now();
        let mut loaded = false;
        for frame in 0..3000 {
            assert!(
                started.elapsed() < std::time::Duration::from_secs(90),
                "replacement regression needs warm coverage"
            );
            capture.draw(|ui| scene.show(ui, &position, 1. / 60., palette));
            let stats = stream.lock().unwrap().stats();
            assert_eq!(stats.failed_tiles, 0);
            if stats.selected_tiles >= 4 && stats.pending_tiles == 0 && frame > 32 {
                loaded = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(loaded);
        for _ in 0..32 {
            capture.draw(|ui| scene.show(ui, &position, 1. / 60., palette));
        }
        let before = capture.image();
        let selected = stream
            .lock()
            .unwrap()
            .select(eye.map(f64::from), 480., 1., Quality::High)
            .tiles;
        assert!(selected.len() > Quality::High.uploads_per_frame());
        // Independent CPU payload identities model eviction/reload or a refreshed provider.
        // Geometry and imagery are identical, so replacing them cannot expose coarse ground.
        for (id, payload) in selected {
            stream
                .lock()
                .unwrap()
                .insert(id, Arc::new((*payload).clone()), Quality::High)
                .unwrap();
        }
        for frame in 0..3 {
            capture.draw(|ui| scene.show(ui, &position, 1. / 60., palette));
            let after = capture.image();
            let changed = before
                .pixels()
                .zip(after.pixels())
                .filter(|(a, b)| a.0.iter().zip(b.0).take(3).any(|(a, b)| a.abs_diff(b) > 12))
                .count();
            assert!(
                changed < 500,
                "budgeted replacement exposed a scenery hole in frame {frame}: {changed} pixels"
            );
        }
        capture.validate().await;
    });
}

#[test]
#[ignore = "requires receipt-backed scenery, the installed Sling, and an offscreen GPU"]
fn physical_aircraft_shadow_and_imc_proof() {
    pollster::block_on(async {
        let mut capture = Capture::new([640, 480]).await;
        let mut scene = Scene::default();
        scene.set_world(&reference_world());
        scenery_sling(&mut scene);
        scene.airspace_visible = false;
        let stream = Arc::new(Mutex::new(LocalProvider::open(&provider(), origin()).unwrap()));
        scene.set_scenery(Some(stream.clone()));
        scene.scenery_quality = Quality::Low;
        scene.camera = CameraMode::Flyby;
        scene.set_screen_view(Some(ScreenView {
            fixed_eye_ned_m: Some([150., 285., -135.]),
            heading_offset_rad: 0.,
            pitch_rad: 0.,
            fov_rad: 0.8,
        }));
        let state = pose(180., 215., 114.1, 60.76659_f64.to_radians());
        let start = std::time::Instant::now();
        let mut loaded = false;
        for i in 0..3000 {
            capture.draw(|ui| scene.show(ui, &state, 1. / 60., theme::scene()));
            let stats = stream.lock().unwrap().stats();
            assert_eq!(stats.failed_tiles, 0);
            if i > 32 && stats.resident_tiles > 8 && stats.pending_tiles == 0 {
                loaded = true;
                break;
            }
            assert!(start.elapsed().as_secs() < 90);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(loaded);
        for _ in 0..32 {
            capture.draw(|ui| scene.show(ui, &state, 1. / 60., theme::scene()));
        }
        let shadow = capture.image();
        scene.aircraft_shadow = false;
        for _ in 0..4 {
            capture.draw(|ui| scene.show(ui, &state, 1. / 60., theme::scene()));
        }
        let without = capture.image();
        let changed = shadow
            .pixels()
            .zip(without.pixels())
            .filter(|(a, b)| a.0.iter().zip(b.0).take(3).any(|(a, b)| a.abs_diff(b) > 8))
            .count();
        capture.validate().await;
        assert!(
            changed > 30,
            "actual Sling geometry must cast a visible depth shadow; changed pixels: {changed}"
        );
        let out = std::env::var_os("FLIGHTCRAFT_SCENERY_PROOF_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/scenery/lighting")
            });
        std::fs::create_dir_all(&out).unwrap();
        shadow.save(out.join("sling-shadow.png")).unwrap();
        without.save(out.join("sling-without-shadow.png")).unwrap();
        let conditions = flightcraft_world::Conditions {
            layers: vec![flightcraft_world::CloudLayer {
                base_ft_msl: 0.,
                top_ft_msl: 3000.,
                coverage: flightcraft_world::Coverage::Ovc,
            }],
            visibility_sm: 0.125,
            ..Default::default()
        };
        scene.set_weather(&conditions, false);
        capture.draw(|ui| scene.show(ui, &state, 1. / 60., theme::scene()));
        let cloud = capture.image();
        cloud.save(out.join("sling-imc.png")).unwrap();
        let reference = cloud.get_pixel(320, 240);
        let uniform = (40..440)
            .flat_map(|y| (40..600).map(move |x| (x, y)))
            .filter(|(x, y)| {
                cloud
                    .get_pixel(*x, *y)
                    .0
                    .iter()
                    .zip(reference.0)
                    .take(3)
                    .all(|(a, b)| a.abs_diff(b) <= 3)
            })
            .count();
        assert!(
            uniform > 220_000,
            "IMC must hide aircraft and terrain, preserving the cloud endpoint: {uniform}"
        );
        capture.validate().await;
    });
}
