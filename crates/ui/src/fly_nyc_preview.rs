//! Offscreen proof of the Fly NYC showcase: the Hudson corridor tileset built from NAIP, the 1 m
//! DEM, NHD water and USGS 3DEP lidar buildings. Real data only; never an invented city.
//!
//! ```sh
//! FLIGHTCRAFT_FLYNYC_TILESET=<built or installed>/tileset.json \
//! FLIGHTCRAFT_CITY_REGION=<assets/terrain/new-york with the fetched .fcht tiles> \
//! cargo test -p flightcraft-ui render_fly_nyc_proof_shots -- --ignored --nocapture --test-threads=1
//! ```
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::{test_capture::Capture, theme};
use flightcraft_render::{
    CameraMode, Scene, ScreenView,
    scenery::{LocalProvider, Location, Provider, Quality},
};
use flightcraft_sim::State;
use std::sync::{Arc, Mutex};

fn pose(n: f64, e: f64, height: f64, heading: f64) -> State {
    State {
        position_m: [n, e, -height],
        attitude: [(heading / 2.).cos(), 0., 0., (heading / 2.).sin()],
        ..State::default()
    }
}

/// (file name, latitude, longitude, feet MSL, direction of view in degrees true, pitch degrees)
#[rustfmt::skip]
const SHOTS: [(&str, f64, f64, f64, f64, f64); 12] = [
    // Close-ups for checking facades and roofs while building (development only).
    ("dev-roofs", 40.7480, -73.9930, 700., 20., -35.),
    ("dev-close-a", 40.7430, -74.0000, 450., 50., -12.),
    ("dev-close-b", 40.7600, -73.9760, 600., 250., -22.),
    // Midtown from over the Hudson, northeast across the river.
    ("midtown-from-the-river", 40.7440, -74.0060, 900., 50., -6.),
    // Southbound over the river at 1,000 ft, looking ahead at downtown and out to the left at midtown.
    ("hudson-southbound-abeam-midtown-ahead", 40.7645, -74.0115, 1000., 185., -3.),
    ("hudson-southbound-abeam-midtown-left", 40.7645, -74.0115, 1000., 100., -4.),
    // Statue of Liberty (bearing 236, 2.3 km) and the Battery from the harbor side.
    ("statue-of-liberty-and-battery", 40.7010, -74.0215, 1000., 236., -8.),
    ("battery-and-lower-manhattan", 40.6880, -74.0330, 1000., 35., -6.),
    // George Washington Bridge from just north, then from the river to its south.
    ("george-washington-bridge-from-north", 40.8660, -73.9560, 1000., 180., -6.),
    ("george-washington-bridge-from-south", 40.8300, -73.9640, 1000., 8., -3.),
    // Teterboro: rolling on runway 06, then climbing out toward the skyline.
    ("kteb-runway-06-departure", 40.8478, -74.0680, 140., 48.2, -2.),
    ("kteb-climbout-toward-manhattan", 40.8590, -74.0450, 1100., 150., -4.),
];

#[test]
#[ignore = "requires the built hudson-corridor tileset, the New York region tiles, and an offscreen GPU"]
fn render_fly_nyc_proof_shots() {
    pollster::block_on(async {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let region_root = std::env::var_os("FLIGHTCRAFT_CITY_REGION")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| root.join("assets/terrain/new-york"));
        let world = flightcraft_control::read_region(&region_root).unwrap();
        let region = world.region().unwrap();
        let origin = [region.origin.latitude_deg, region.origin.longitude_deg];
        let projection = flightcraft_world::geo::LocalProjection::new(origin[0], origin[1]).unwrap();
        let path = std::env::var_os("FLIGHTCRAFT_FLYNYC_TILESET")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| root.join("assets/terrain/new-york/scenery/hudson-corridor/tileset.json"));
        let provider = Provider {
            location: Location::LocalPath(path),
            enabled: true,
            token_config_key: None,
        };
        let stream = Arc::new(Mutex::new(LocalProvider::open(&provider, origin).unwrap()));
        let mut capture = Capture::new([1600, 900]).await;
        let mut scene = Scene::default();
        scene.set_world(&world);
        scene.set_scenery(Some(stream.clone()));
        scene.scenery_quality = Quality::Medium;
        scene.airspace_visible = false;
        let out = std::env::var_os("FLIGHTCRAFT_FLYNYC_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| root.join("out/scenery/fly-nyc"));
        std::fs::create_dir_all(&out).unwrap();
        let only = std::env::var("FLIGHTCRAFT_FLYNYC_ONLY").ok();
        for (name, lat, lon, feet, heading, pitch) in SHOTS {
            if only.as_deref().is_some_and(|o| !name.contains(o)) {
                continue;
            }
            let [n, e] = projection.project(lat, lon).unwrap();
            let height = feet * 0.3048;
            let (heading, pitch) = (heading.to_radians(), pitch.to_radians());
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
            let mut quiet = 0;
            for frame in 0..20_000 {
                capture.draw(|ui| scene.show(ui, &target, 1. / 60., theme::scene()));
                let stats = stream.lock().unwrap().stats();
                assert_eq!(stats.failed_tiles, 0, "scenery content failed to decode");
                quiet = if frame > 32 && stats.resident_tiles > 0 && stats.pending_tiles == 0 {
                    quiet + 1
                } else {
                    0
                };
                // Refinement loads in waves; wait until nothing is pending for a while.
                if quiet > 90 || start.elapsed() > std::time::Duration::from_secs(240) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(4));
            }
            for _ in 0..16 {
                capture.draw(|ui| scene.show(ui, &target, 1. / 60., theme::scene()));
            }
            let stats = stream.lock().unwrap().stats();
            eprintln!(
                "{name}: {:?} resident tiles {} ({} MB), selected {}",
                start.elapsed(),
                stats.resident_tiles,
                stats.resident_bytes / (1024 * 1024),
                stats.selected_tiles
            );
            capture.image().save(out.join(format!("{name}.png"))).unwrap();
        }
        capture.validate().await;
    });
}
