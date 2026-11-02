//! Actual scene pixels, panel composition, and frame costs using WebGL 2 limits.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::{
    instruments::{InstrumentInput, InstrumentPanel},
    test_capture::Capture,
    theme,
};
use eframe::egui::{self, Rect};
use flightcraft_control::Session;
use flightcraft_render::{CameraMode, Scene};
use flightcraft_sim::State;
use flightcraft_world::{CloudLayer, Conditions, Coverage, Obscuration, TimeOfDay};

fn paint(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    session: &Session,
    pose: &State,
    panel: &mut InstrumentPanel,
    label: &str,
) {
    let full = ui.max_rect();
    let split = full.top() + full.height() * 0.62;
    ui.scope_builder(
        egui::UiBuilder::new().max_rect(Rect::from_min_max(full.min, egui::pos2(full.right(), split))),
        |ui| {
            scene.show(ui, pose, 1. / 60., theme::scene());
            let caption =
                ui.painter()
                    .layout_no_wrap(label.to_owned(), egui::FontId::monospace(16.), theme::TEXT);
            let origin = ui.max_rect().left_top() + egui::vec2(16., 16.);
            ui.painter().rect_filled(
                Rect::from_min_size(origin, caption.size()).expand(5.),
                3.,
                theme::PANEL,
            );
            ui.painter().galley(origin, caption, theme::TEXT);
        },
    );
    ui.scope_builder(
        egui::UiBuilder::new().max_rect(Rect::from_min_max(egui::pos2(full.left(), split), full.max)),
        |ui| {
            ui.painter().rect_filled(ui.max_rect(), 0., theme::PANEL);
            let config = session.aircraft_configuration().unwrap();
            panel
                .show(
                    ui,
                    &config.panel,
                    InstrumentInput::new(session.sim().systems(), 0.).with_aircraft(config),
                    session.flight_generation(),
                )
                .unwrap();
        },
    );
}
fn frame_cost(capture: &mut Capture, mut paint: impl FnMut(&mut egui::Ui)) -> (f64, f64) {
    for _ in 0..10 {
        capture.draw(&mut paint);
    }
    let mut times = Vec::new();
    for _ in 0..120 {
        let now = std::time::Instant::now();
        capture.draw(&mut paint);
        times.push(now.elapsed().as_secs_f64() * 1000.);
    }
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    times.sort_by(f64::total_cmp);
    (mean, times[114])
}
#[test]
#[ignore = "requires an offscreen GPU and fetched eastern-pa region"]
fn render_weather_previews() {
    pollster::block_on(async {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let directory = root.join("out/weather");
        std::fs::create_dir_all(&directory).unwrap();
        let world = flightcraft_control::read_region(&root.join("assets/terrain/eastern-pa")).unwrap();
        let airport = world
            .airports()
            .unwrap()
            .airports
            .iter()
            .find(|a| a.icao.as_deref() == Some("KABE") || a.ident == "KABE")
            .unwrap();
        let runway = airport
            .runways
            .iter()
            .find(|r| r.ends.iter().any(|e| e.ident == "06"))
            .unwrap();
        let projection = flightcraft_world::geo::LocalProjection::new(
            world.region().unwrap().origin.latitude_deg,
            world.region().unwrap().origin.longitude_deg,
        )
        .unwrap();
        let strip = flightcraft_world::RunwayStrip::project(runway, &projection).unwrap();
        let index = usize::from(runway.ends[0].ident != "06");
        let [n, e] = strip.ends_m[index];
        let [nn, ee] = strip.ends_m[1 - index];
        let heading = (ee - e).atan2(nn - n);
        let elevation = strip.elevations_m[index];
        let pose = |agl_ft: f64| {
            let distance = agl_ft * 0.3048 / 3_f64.to_radians().tan();
            State {
                position_m: [
                    n - distance * heading.cos(),
                    e - distance * heading.sin(),
                    -(elevation + agl_ft * 0.3048),
                ],
                attitude: [(heading / 2.).cos(), 0., 0., (heading / 2.).sin()],
                ..State::default()
            }
        };
        let conditions = Conditions {
            layers: vec![CloudLayer {
                base_ft_msl: elevation / 0.3048 + 600.,
                top_ft_msl: elevation / 0.3048 + 3600.,
                coverage: Coverage::Ovc,
            }],
            visibility_sm: 2.,
            obscuration: Obscuration::Mist,
            ..Default::default()
        };
        let mut session = Session::new().unwrap();
        session.enable_app();
        let mut scene = Scene::default();
        scene.set_world(&world);
        scene.airspace_visible = false;
        scene.camera = CameraMode::Cockpit;
        let mut capture = Capture::new([1280, 800]).await;
        let scenarios = [
            (
                "kabe-06-700ft-cloud.png",
                700.,
                conditions.clone(),
                false,
                "KABE ILS 06 · 700 ft AGL · OVC 600 AGL · 2 SM BR · IN CLOUD",
            ),
            (
                "kabe-06-500ft-breakout.png",
                500.,
                conditions.clone(),
                false,
                "KABE ILS 06 · 500 ft AGL · OVC 600 AGL · 2 SM BR · BREAKOUT",
            ),
            (
                "kabe-06-500ft-night.png",
                500.,
                Conditions {
                    time_of_day: TimeOfDay::Night,
                    ..conditions.clone()
                },
                false,
                "KABE ILS 06 · 500 ft AGL · OVC 600 AGL · 2 SM BR · NIGHT",
            ),
            (
                "kabe-06-hood.png",
                500.,
                Conditions::default(),
                true,
                "KABE ILS 06 · HOOD · SIMULATED INSTRUMENT PRACTICE",
            ),
            (
                "kabe-06-vfr.png",
                500.,
                Conditions::default(),
                false,
                "KABE ILS 06 · 500 ft AGL · CLEAR VFR REFERENCE",
            ),
        ];
        let mut images = Vec::new();
        let mut report = String::from("Backend,scene,mean_ms,p95_ms,mean_fps\n");
        for (name, altitude, weather, hood, label) in scenarios {
            let mut panel = InstrumentPanel::default();
            let mut pose = pose(altitude);
            pose.velocity_mps = [
                50. * heading.cos(),
                50. * heading.sin(),
                50. * 3_f64.to_radians().tan(),
            ];
            // Feed the real panel matching flight truth, rather than a parked instrument snapshot.
            *session.sim_mut() = flightcraft_sim::Sim::from_state_in_world(
                pose,
                flightcraft_sim::Controls::default(),
                world.clone(),
            )
            .unwrap();
            session.sim_mut().set_conditions(weather.clone()).unwrap();
            session.sim_mut().set_hood(hood);
            scene.set_weather(&weather, hood);
            for _ in 0..3 {
                capture.draw(|ui| paint(ui, &mut scene, &session, &pose, &mut panel, label));
            }
            let image = capture.image();
            image.save(directory.join(name)).unwrap();
            images.push(image);
            let (mean, p95) = frame_cost(&mut capture, |ui| {
                paint(ui, &mut scene, &session, &pose, &mut panel, label)
            });
            let backend = format!("{:?}", capture.state.adapter.get_info().backend);
            report.push_str(&format!(
                "{backend},{name},{mean:.3},{p95:.3},{:.1}\n",
                1000. / mean
            ));
        }
        let backend = format!("{:?}", capture.state.adapter.get_info().backend).to_lowercase();
        std::fs::write(directory.join(format!("frame-cost-{backend}.csv")), &report).unwrap();
        std::fs::write(
            directory.join(format!("adapter-{backend}.txt")),
            format!("{:?}", capture.state.adapter.get_info()),
        )
        .unwrap();
        let mut panel = InstrumentPanel::default();
        // Cloud removes all terrain/horizon contrast in the central outside area.
        let cloud = &images[0];
        assert_eq!(cloud.get_pixel(600, 180), cloud.get_pixel(680, 220));
        let breakout = &images[1];
        assert_ne!(cloud.get_pixel(640, 300), breakout.get_pixel(640, 300));
        // At just below the base, the approach fixtures remain visible within 2 SM.
        scene.set_weather(&conditions, false);
        scene.lights_visible = false;
        capture.draw(|ui| {
            paint(
                ui,
                &mut scene,
                &session,
                &pose(500.),
                &mut panel,
                "LIGHTS COMPARISON",
            )
        });
        let unlit = capture.image();
        let lit = breakout
            .enumerate_pixels()
            .filter(|(x, y, p)| {
                let dark = unlit.get_pixel(*x, *y);
                *x > 400
                    && *x < 880
                    && *y > 220
                    && *y < 420
                    && (u16::from(p[0]) + u16::from(p[1]) + u16::from(p[2]))
                        > u16::from(dark[0]) + u16::from(dark[1]) + u16::from(dark[2]) + 20
            })
            .count();
        assert!(lit > 8, "approach light contrast pixels: {lit}");
        scene.lights_visible = true;
        // The camera, including its pilot-eye offset, is only 5 cm below the layer base.
        let mut boundary = pose(500.);
        boundary.position_m[2] = -(elevation + 600. * 0.3048 - 0.43 - 0.05);
        capture.draw(|ui| paint(ui, &mut scene, &session, &boundary, &mut panel, "JUST BELOW BASE"));
        let below = capture.image();
        assert_ne!(below.get_pixel(640, 300), cloud.get_pixel(640, 300));
        // Hood removes external pixels, including after a head pan; the actual panel stays intact.
        assert_eq!(images[3].get_pixel(600, 180), images[3].get_pixel(680, 220));
        assert_eq!(images[3].get_pixel(600, 650), images[4].get_pixel(600, 650));
        scene.set_weather(&Conditions::default(), true);
        scene.set_cockpit_look(1.2, 0.5);
        for _ in 0..80 {
            capture.draw(|ui| paint(ui, &mut scene, &session, &pose(500.), &mut panel, "HOOD HEAD PAN"));
        }
        let panned = capture.image();
        let mask = images[3].get_pixel(600, 180);
        let hidden = panned
            .enumerate_pixels()
            .filter(|(x, y, p)| *x >= 200 && *x < 1000 && *y >= 100 && *y < 300 && *p == mask)
            .count();
        assert!(
            hidden > 100_000,
            "hood must cover the panned outside viewport, got {hidden} pixels"
        );
        let panel_changes = panned
            .enumerate_pixels()
            .filter(|(x, y, p)| *y > 520 && *p != images[4].get_pixel(*x, *y))
            .count();
        assert_eq!(panel_changes, 0, "hood must leave the instrument panel intact");
        let final_pose = pose(700.);
        let vision =
            flightcraft_render::SyntheticVision::new(&world, &final_pose, 0., 0., elevation + 700. * 0.3048)
                .unwrap();
        capture.draw(|ui| vision.paint(ui.painter(), ui.max_rect()));
        let synthetic = capture.image();
        assert_ne!(synthetic.get_pixel(640, 100), synthetic.get_pixel(640, 600));
        // Include the original smoke field for a comparable bounded browser scene workload.
        scene = Scene::default();
        let smoke = State {
            position_m: [0., 0., -100.],
            ..State::default()
        };
        for (label, weather) in [
            ("smoke-vfr", Conditions::default()),
            (
                "smoke-ifr",
                Conditions::preset(flightcraft_world::WeatherPreset::Ifr),
            ),
        ] {
            scene.set_weather(&weather, false);
            let (mean, p95) = frame_cost(&mut capture, |ui| {
                scene.show(ui, &smoke, 1. / 60., theme::scene())
            });
            report.push_str(&format!(
                "{:?},{label},{mean:.3},{p95:.3},{:.1}\n",
                capture.state.adapter.get_info().backend,
                1000. / mean
            ));
        }
        let backend = format!("{:?}", capture.state.adapter.get_info().backend).to_lowercase();
        std::fs::write(directory.join(format!("frame-cost-{backend}.csv")), &report).unwrap();
        eprintln!("{report}");
        // Also capture literal 700/500 ft MSL with a 600 ft MSL base, near ILS decision height.
        // The main images above express the reported 600 ft AGL ceiling explicitly.
        let msl_conditions = Conditions::preset(flightcraft_world::WeatherPreset::Ifr);
        for (name, altitude_msl_ft, weather, hood, label) in [
            (
                "kabe-06-700ft-msl-cloud.png",
                700.,
                msl_conditions.clone(),
                false,
                "KABE ILS 06 · 700 ft MSL · OVC BASE 600 MSL · 2 SM BR · IN CLOUD",
            ),
            (
                "kabe-06-500ft-msl-breakout.png",
                500.,
                msl_conditions.clone(),
                false,
                "KABE ILS 06 · 500 ft MSL · OVC BASE 600 MSL · 2 SM BR · BREAKOUT",
            ),
            (
                "kabe-06-500ft-msl-night.png",
                500.,
                Conditions {
                    time_of_day: TimeOfDay::Night,
                    ..msl_conditions
                },
                false,
                "KABE ILS 06 · 500 ft MSL · OVC BASE 600 MSL · 2 SM BR · NIGHT",
            ),
            (
                "kabe-06-msl-hood.png",
                500.,
                Conditions::default(),
                true,
                "KABE ILS 06 · 500 ft MSL · HOOD · SIMULATED INSTRUMENT PRACTICE",
            ),
            (
                "kabe-06-msl-vfr.png",
                500.,
                Conditions::default(),
                false,
                "KABE ILS 06 · 500 ft MSL · CLEAR VFR REFERENCE",
            ),
        ] {
            let mut state = pose(altitude_msl_ft - elevation / 0.3048);
            state.velocity_mps = [
                50. * heading.cos(),
                50. * heading.sin(),
                50. * 3_f64.to_radians().tan(),
            ];
            *session.sim_mut() = flightcraft_sim::Sim::from_state_in_world(
                state,
                flightcraft_sim::Controls::default(),
                world.clone(),
            )
            .unwrap();
            session.sim_mut().set_conditions(weather.clone()).unwrap();
            session.sim_mut().set_hood(hood);
            scene = Scene::default();
            scene.set_world(&world);
            scene.camera = CameraMode::Cockpit;
            scene.airspace_visible = false;
            scene.set_weather(&weather, hood);
            let mut panel = InstrumentPanel::default();
            for _ in 0..3 {
                capture.draw(|ui| paint(ui, &mut scene, &session, &state, &mut panel, label));
            }
            capture.image().save(directory.join(name)).unwrap();
        }
        capture.validate().await;
    });
}
