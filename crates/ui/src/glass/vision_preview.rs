//! GPU captures exercise the real offscreen terrain pass and the instrument overlay together.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use super::*;
use eframe::egui::{self, Rect};
use flightcraft_control::Session;
use flightcraft_render::{SyntheticVision, synthetic_vision_stats};
use flightcraft_sim::State;
use std::sync::{Arc, mpsc};

struct Capture {
    state: egui_wgpu::RenderState,
    context: egui::Context,
    texture: wgpu::Texture,
    size: [u32; 2],
    frame: u32,
}
impl Capture {
    async fn new(size: [u32; 2]) -> Self {
        let backends = if std::env::var("FLIGHTCRAFT_TEST_BACKEND").as_deref() == Ok("gl") {
            wgpu::Backends::GL
        } else {
            wgpu::Backends::VULKAN
        };
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends,
            ..Default::default()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_limits: wgpu::Limits::downlevel_webgl2_defaults(),
                ..Default::default()
            })
            .await
            .unwrap();
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let renderer = egui_wgpu::Renderer::new(&device, format, Default::default());
        let state = egui_wgpu::RenderState {
            adapter,
            available_adapters: Vec::new(),
            device,
            queue,
            target_format: format,
            renderer: Arc::new(egui::mutex::RwLock::new(renderer)),
        };
        flightcraft_render::initialize(&state, crate::theme::scene());
        let context = egui::Context::default();
        crate::theme::install(&context);
        flightcraft_render::initialize_synthetic_vision(&state, &context);
        let [width, height] = size;
        let texture = state.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("PFD preview"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        Self {
            state,
            context,
            texture,
            size,
            frame: 0,
        }
    }
    fn resize(&mut self, size: [u32; 2]) {
        self.size = size;
        let [width, height] = size;
        self.texture = self.state.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("PFD preview"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.state.target_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
    }
    fn draw(&mut self, mut paint: impl FnMut(&mut egui::Ui)) {
        let [width, height] = self.size;
        let output = self.context.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width as f32, height as f32),
                )),
                time: Some(f64::from(self.frame) / 120.),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::new())
                    .show(ctx, |ui| paint(ui));
            },
        );
        self.frame += 1;
        let jobs = self.context.tessellate(output.shapes, output.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: self.size,
            pixels_per_point: output.pixels_per_point,
        };
        let view = self.texture.create_view(&Default::default());
        let mut renderer = self.state.renderer.write();
        for (id, delta) in &output.textures_delta.set {
            renderer.update_texture(&self.state.device, &self.state.queue, *id, delta);
        }
        let mut encoder = self.state.device.create_command_encoder(&Default::default());
        let commands = renderer.update_buffers(
            &self.state.device,
            &self.state.queue,
            &mut encoder,
            &jobs,
            &screen,
        );
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            renderer.render(&mut pass, &jobs, &screen);
        }
        self.state
            .queue
            .submit(commands.into_iter().chain([encoder.finish()]));
        for id in &output.textures_delta.free {
            renderer.free_texture(id);
        }
        self.state
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
    }
    fn image(&self) -> image::RgbaImage {
        let [width, height] = self.size;
        let stride = (width * 4).div_ceil(256) * 256;
        let buffer = self.state.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("PFD readback"),
            size: u64::from(stride * height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.state.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.state.queue.submit([encoder.finish()]);
        let (tx, rx) = mpsc::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            tx.send(r).unwrap();
        });
        self.state
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        rx.recv().unwrap().unwrap();
        let bytes = buffer.slice(..).get_mapped_range();
        let pixels = bytes
            .chunks_exact(stride as usize)
            .flat_map(|row| row[..(width * 4) as usize].iter().copied())
            .collect();
        let image = image::RgbaImage::from_raw(width, height, pixels).unwrap();
        drop(bytes);
        buffer.unmap();
        image
    }
    async fn validate(&self) {
        assert!(self.state.device.pop_error_scope().await.is_none());
    }
}
fn pfd(ui: &mut egui::Ui, session: &Session, pose: &State, glass: &GlassState) {
    pfd_area(ui, ui.max_rect(), session, pose, glass);
}
fn pfd_area(ui: &mut egui::Ui, area: Rect, session: &Session, pose: &State, glass: &GlassState) {
    paint_fill(
        ui,
        area,
        super::pfd::draw(
            glass,
            InstrumentInput::new(session.sim().systems(), pose.time_s)
                .with_glass(glass)
                .with_vision(session.sim().world(), pose)
                .with_aircraft(session.aircraft_configuration().unwrap())
                .with_autopilot(session.sim().installed_autopilot()),
            false,
        ),
    );
}
fn camera_pose(position: [f64; 3], heading: f64, pitch: f64) -> State {
    let (sh, ch) = (heading * 0.5).sin_cos();
    let (sp, cp) = (pitch * 0.5).sin_cos();
    State {
        position_m: position,
        attitude: [ch * cp, -sh * sp, ch * sp, sh * cp],
        velocity_mps: [50. * heading.cos(), 50. * heading.sin(), 0.],
        ..State::default()
    }
}
fn match_snapshot(glass: &mut GlassState, world: &flightcraft_world::World, pose: &State, grid_heading: f64) {
    let reference =
        flightcraft_sim::magnetic::reference_at(world, pose.position_m, Default::default()).unwrap();
    glass.air_data.heading_rad = reference.grid_to_magnetic(grid_heading);
    glass.ground_track_rad = glass.air_data.heading_rad;
    glass.ground_speed_mps = 50.;
    glass.air_data.indicated_airspeed_mps = 50.;
    glass.air_data.true_airspeed_mps = 50.;
    glass.air_data.vertical_speed_mps = -pose.velocity_mps[2];
}
#[test]
#[ignore = "requires a graphics adapter; actual PFD terrain previews and pass-count checks"]
fn render_synthetic_vision_previews() {
    pollster::block_on(async {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut session = Session::new().unwrap();
        session
            .execute(
                "aircraft.load",
                serde_json::json!({"source":{"kind":"installed","id":"corvid-glass"},"reset":true}),
            )
            .unwrap();
        let world = flightcraft_control::read_region(&root.join("assets/terrain/new-york")).unwrap();
        session.install_region(world.clone()).unwrap();
        session.execute("sim.reset", serde_json::json!({"world":"new-york","start":{"kind":"air","height_m":1000.,"airspeed_mps":50.,"heading_rad":0.}})).unwrap();
        let mut glass = session.glass_state();
        glass.air_data.attitude_valid = true;
        glass.air_data.air_data_valid = true;
        glass.air_data.powered = true;
        glass.air_data.roll_rad = 0.;
        glass.air_data.pitch_rad = 0.;
        let directory = root.join("out/synthetic-vision");
        std::fs::create_dir_all(&directory).unwrap();
        let mut capture = Capture::new([1000, 880]).await;
        let ridge = camera_pose([3000., -1800., -650.], -40_f64.to_radians(), 0.);
        // No enabled display means no callback, hence no offscreen pass, even with a world loaded.
        pfd_disabled(&mut capture, &session, &ridge, &glass);
        glass.settings.synthetic_vision = true;
        glass.air_data.altitude_msl_m = 650.;
        match_snapshot(&mut glass, &world, &ridge, -40_f64.to_radians());
        for _ in 0..2 {
            capture.draw(|ui| pfd(ui, &session, &ridge, &glass));
        }
        capture
            .image()
            .save(directory.join("1n7-kittatinny-pfd.png"))
            .unwrap();
        let airport = world
            .airports()
            .unwrap()
            .airports
            .iter()
            .find(|a| a.icao.as_deref() == Some("KMMU") || a.ident == "KMMU")
            .unwrap();
        let runway = airport
            .runways
            .iter()
            .find(|r| r.ends.iter().any(|e| e.ident == "23"))
            .unwrap();
        let strip = flightcraft_world::RunwayStrip::project(
            runway,
            &flightcraft_world::geo::LocalProjection::new(
                world.region().unwrap().origin.latitude_deg,
                world.region().unwrap().origin.longitude_deg,
            )
            .unwrap(),
        )
        .unwrap();
        let index = usize::from(runway.ends[0].ident != "23");
        let [n, e] = strip.ends_m[index];
        let [nn, ee] = strip.ends_m[1 - index];
        let heading = (ee - e).atan2(nn - n);
        let distance = 1852.;
        let altitude = strip.elevations_m[index] + distance * 3_f64.to_radians().tan();
        let mut final_pose = camera_pose(
            [
                n - distance * heading.cos(),
                e - distance * heading.sin(),
                -altitude,
            ],
            heading,
            0.,
        );
        final_pose.velocity_mps[2] = 50. * 3_f64.to_radians().tan();
        glass.air_data.altitude_msl_m = altitude;
        match_snapshot(&mut glass, &world, &final_pose, heading);
        capture.frame += 4;
        capture.draw(|ui| pfd(ui, &session, &final_pose, &glass));
        capture
            .image()
            .save(directory.join("kmmu-23-final-pfd.png"))
            .unwrap();
        // A 50 ft AGL aircraft at the KMMU 23 threshold must see the warning band.
        let ground = world.ground(n, e).height_msl_m;
        let low = camera_pose([n, e, -ground - 50. * 0.3048], heading, 0.);
        let vision = SyntheticVision::new(&world, &low, 0., 0., ground + 50. * 0.3048).unwrap();
        capture.frame += 4;
        capture.draw(|ui| vision.paint(ui.painter(), ui.max_rect()));
        let image = capture.image();
        let red = image
            .pixels()
            .filter(|p| {
                p.0[0] > 120
                    && f32::from(p.0[0]) > f32::from(p.0[1]) * 1.6
                    && f32::from(p.0[0]) > f32::from(p.0[2]) * 1.6
            })
            .count();
        assert!(red > 1000, "50 ft AGL terrain warning missing: {red}");
        image.save(directory.join("kmmu-50ft-warning.png")).unwrap();
        // Both physical displays request exactly the same view; 120 paint frames yield 30 passes.
        let before = synthetic_vision_stats(&capture.state).terrain_passes;
        capture.frame = 120;
        let mut frame_times = Vec::new();
        for _ in 0..120 {
            let start = std::time::Instant::now();
            session.run(flightcraft_control::Step { steps: 1 }).unwrap();
            capture.draw(|ui| {
                let (left, right) = ui.max_rect().split_left_right_at_fraction(0.5);
                pfd_area(ui, left, &session, &final_pose, &glass);
                pfd_area(ui, right, &session, &final_pose, &glass);
            });
            frame_times.push(start.elapsed().as_secs_f64() * 1000.);
        }
        frame_times.sort_by(f64::total_cmp);
        eprintln!(
            "120 Hz simulation and two shared terrain composites: median {:.3} ms, p95 {:.3} ms",
            frame_times[60], frame_times[114]
        );
        assert_eq!(synthetic_vision_stats(&capture.state).terrain_passes - before, 30);
        let enabled_passes = synthetic_vision_stats(&capture.state).terrain_passes;
        glass.settings.synthetic_vision = false;
        let mut baseline = Vec::new();
        for _ in 0..120 {
            let start = std::time::Instant::now();
            session.run(flightcraft_control::Step { steps: 1 }).unwrap();
            capture.draw(|ui| {
                let (left, right) = ui.max_rect().split_left_right_at_fraction(0.5);
                pfd_area(ui, left, &session, &final_pose, &glass);
                pfd_area(ui, right, &session, &final_pose, &glass);
            });
            baseline.push(start.elapsed().as_secs_f64() * 1000.);
        }
        baseline.sort_by(f64::total_cmp);
        eprintln!(
            "Two PFDs with SVT disabled: median {:.3} ms, p95 {:.3} ms; enabled median delta {:.3} ms",
            baseline[60],
            baseline[114],
            frame_times[60] - baseline[60]
        );
        assert_eq!(
            synthetic_vision_stats(&capture.state).terrain_passes,
            enabled_passes
        );
        // Exercise the installed touch composition, including both insets and the engine strips.
        let mut demo = Session::new().unwrap();
        demo.install_pack(
            &flightcraft_control::read_pack(&root.join("examples/packs/equipment-demo")).unwrap(),
        )
        .unwrap();
        demo.install_region(world.clone()).unwrap();
        demo.execute(
            "aircraft.load",
            serde_json::json!({"source":{"kind":"installed","id":"equipment-demonstrator"},"reset":true}),
        )
        .unwrap();
        let [pn, pe, pd] = final_pose.position_m;
        let [latitude_deg, longitude_deg] = world.geodetic(pn, pe).unwrap();
        demo.execute("sim.reset",serde_json::json!({"world":"new-york","paused":true,"start":{
            "kind":"geographic_air","position":{"latitude_deg":latitude_deg,"longitude_deg":longitude_deg},
            "height_agl_m":-pd-world.ground(pn,pe).height_msl_m,"airspeed_mps":50.,"heading_rad":glass.air_data.heading_rad}})).unwrap();
        for id in ["left_display", "right_display"] {
            demo.execute("equipment.set",serde_json::json!({"setting":{"kind":"display","id":id,"action":{"kind":"synthetic_vision","enabled":true}}})).unwrap();
        }
        let mut panel_state = demo.glass_state();
        panel_state.air_data = glass.air_data.clone();
        panel_state.standby = panel_state.air_data.clone();
        assert!(
            panel_state
                .touch
                .displays
                .iter()
                .all(|d| d.synthetic_vision && d.powered)
        );
        let configuration = demo.aircraft_configuration().unwrap().clone();
        let mut panel = crate::instruments::InstrumentPanel::default();
        capture.resize([1600, 900]);
        capture.frame += 4;
        let before_panel = synthetic_vision_stats(&capture.state).terrain_passes;
        capture.draw(|ui| {
            panel
                .show(
                    ui,
                    &configuration.panel,
                    InstrumentInput::new(demo.sim().systems(), 0.)
                        .with_glass(&panel_state)
                        .with_aircraft(&configuration)
                        .with_vision(&world, &final_pose)
                        .with_autopilot(demo.sim().installed_autopilot()),
                    demo.flight_generation(),
                )
                .unwrap();
            panel.show_shared_hosts(ui, &mut demo);
        });
        assert_eq!(
            synthetic_vision_stats(&capture.state).terrain_passes - before_panel,
            1
        );
        capture
            .image()
            .save(directory.join("equipment-panel.png"))
            .unwrap();
        capture.resize([1000, 880]);
        // Pixel-level shader checks on level terrain, on both sides of each MFD threshold.
        let flat = flightcraft_world::World::default();
        for (height_ft, token) in [
            (50., Some(crate::theme::DANGER)),
            (100., Some(crate::theme::DANGER)),
            (101., Some(crate::theme::AMBER)),
            (1000., Some(crate::theme::AMBER)),
            (1001., None),
        ] {
            let height = height_ft * 0.3048;
            let pose = camera_pose([-1000., 500., -height], 0., 0.);
            let view = SyntheticVision::new(&flat, &pose, 0., 0., height).unwrap();
            capture.frame += 4;
            capture.draw(|ui| view.paint(ui.painter(), ui.max_rect()));
            let image = capture.image();
            let p = image.get_pixel(200, 750).0;
            if let Some(token) = token {
                let expected = token.to_array();
                let matches_hue = p[..3].iter().zip(expected).all(|(a, b)| {
                    (f32::from(*a) / f32::from(p[0].max(1)) - f32::from(b) / f32::from(expected[0])).abs()
                        < 0.03
                });
                assert!(
                    matches_hue && f32::from(p[0]) >= f32::from(expected[0]) * 0.7,
                    "band at {height_ft} ft: {p:?}"
                );
            } else {
                assert!(
                    p[1] > p[0] && p[1] > p[2],
                    "clear terrain must remain green: {p:?}"
                );
            }
        }
        capture.validate().await;
    });
}
fn pfd_disabled(capture: &mut Capture, session: &Session, pose: &State, glass: &GlassState) {
    capture.draw(|ui| pfd(ui, session, pose, glass));
    assert_eq!(synthetic_vision_stats(&capture.state).terrain_passes, 0);
}

#[test]
#[ignore = "requires a graphics adapter; full HD cockpit layout previews"]
fn render_layout_cockpit_previews() {
    pollster::block_on(async {
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/layout/after");
        std::fs::create_dir_all(&out).unwrap();
        let external = std::env::var_os("FLIGHTCRAFT_LAYOUT_PACK");
        for (name, id) in [
            ("equipment-demo", "equipment-demonstrator"),
            ("panel-fidelity-demo", "equipment-fidelity-demonstrator"),
            ("sling", "sling2"),
        ] {
            let mut session = crate::panel_fidelity_tests::geographic_session();
            if name == "sling" {
                let Some(path) = external.as_ref() else { continue };
                session
                    .install_pack(&flightcraft_control::read_pack(std::path::Path::new(path)).unwrap())
                    .unwrap();
            } else if name == "equipment-demo" {
                let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../examples/packs/equipment-demo");
                session
                    .install_pack(&flightcraft_control::read_pack(&path).unwrap())
                    .unwrap();
            }
            session
                .execute(
                    "aircraft.load",
                    serde_json::json!({"source":{"kind":"installed","id":id},"reset":true}),
                )
                .unwrap();
            session.execute("sim.reset",serde_json::json!({"world":"panel-preview","start":{"kind":"air","height_m":1200.,"airspeed_mps":45.,"heading_rad":0.5}})).unwrap();
            session
                .execute(
                    "assist.set",
                    serde_json::json!({"altitude_m":1200.,"airspeed_mps":45.,"heading_rad":0.5}),
                )
                .unwrap();
            session
                .execute("sim.step", serde_json::json!({"steps":7200}))
                .unwrap();
            let config = session.aircraft_configuration().unwrap().clone();
            let state = session.glass_state();
            let mut scene = flightcraft_render::Scene::default();
            scene.camera = flightcraft_render::CameraMode::Cockpit;
            scene.set_world(session.sim().world());
            let mut panel = crate::instruments::InstrumentPanel::default();
            let mut capture = Capture::new([1920, 1080]).await;
            for _ in 0..3 {
                capture.draw(|ui| {
                    scene.show(ui, session.sim().state(), 0., crate::theme::scene());
                    panel
                        .show(
                            ui,
                            &config.panel,
                            InstrumentInput::new(session.sim().systems(), session.sim().state().time_s)
                                .with_aircraft(&config)
                                .with_glass(&state)
                                .with_vision(session.sim().world(), session.sim().state())
                                .with_autopilot(session.sim().installed_autopilot()),
                            session.flight_generation(),
                        )
                        .unwrap();
                    panel.show_shared_hosts(ui, &mut session);
                });
            }
            let image = capture.image();
            // The open area above the glare shield must show the outside scene.
            let sky = image.get_pixel(960, 20).0;
            assert_ne!(sky, crate::theme::BACKGROUND.to_array());
            assert_ne!(sky, crate::theme::PANEL.to_array());
            image.save(out.join(format!("{name}-cockpit.png"))).unwrap();
            capture.validate().await;
        }
    });
}
