//! The shared eframe application for desktop and browser.
pub mod actions;
pub mod clock;
mod input;
mod instruments;
pub mod keymap;
pub mod script;
pub mod theme;

use eframe::egui::{self, Align2, FontId, RichText};
use flightcraft_control::Session;
use flightcraft_render::{CameraMode, Scene};
use keymap::{Action, Keymap};

const KEYMAP_STORAGE: &str = "flightcraft.keymap.v1";

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
pub struct Screenshot {
    path: std::path::PathBuf,
    frames: u64,
    /// When set, capture once simulated time reaches this many seconds instead of after `frames`.
    at_sim_s: Option<f64>,
    succeeded: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Screenshot {
    pub fn new(path: std::path::PathBuf, frames: u64) -> Self {
        Self {
            path,
            frames: frames.max(1),
            at_sim_s: None,
            succeeded: std::sync::Arc::default(),
        }
    }

    /// Capture once simulated time reaches `seconds` (a finite value), instead of after a frame count.
    #[must_use]
    pub fn at_sim_time(mut self, seconds: f64) -> Self {
        self.at_sim_s = seconds.is_finite().then_some(seconds.max(0.0));
        self
    }

    /// Only true once a fresh screenshot has been successfully written.
    pub fn succeeded(&self) -> bool {
        self.succeeded.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[derive(Default)]
pub struct AppOptions {
    #[cfg(not(target_arch = "wasm32"))]
    pub screenshot: Option<Screenshot>,
    /// Timed commands to run; they hold the flight controls until they run out.
    pub script: Option<script::Script>,
}

/// One session, one fixed-step accumulator, and read-only presentation of its state.
pub struct FlightCraftApp {
    session: Session,
    scene: Scene,
    clock: clock::FixedStep,
    input: input::Input,
    keymap: Keymap,
    paused: bool,
    last_time: Option<f64>,
    error: Option<String>,
    settings: bool,
    bindings_json: String,
    binding_message: Option<String>,
    script: script::Script,
    #[cfg(not(target_arch = "wasm32"))]
    screenshot: Option<Screenshot>,
    #[cfg(not(target_arch = "wasm32"))]
    frames: u64,
    #[cfg(not(target_arch = "wasm32"))]
    screenshot_requested: bool,
    #[cfg(not(target_arch = "wasm32"))]
    requested_frame: u64,
}

impl FlightCraftApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        _options: AppOptions,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        theme::install(&cc.egui_ctx);
        let render_state = cc
            .wgpu_render_state
            .as_ref()
            .ok_or_else(|| std::io::Error::other("FlightCraft needs the wgpu renderer"))?;
        flightcraft_render::initialize(render_state, theme::scene());
        let stored = cc.storage.and_then(|s| s.get_string(KEYMAP_STORAGE));
        let (keymap, error) = match stored {
            Some(json) => match Keymap::from_json(&json) {
                Ok(map) => (map, None),
                Err(e) => (
                    Keymap::default(),
                    Some(format!("Saved keymap could not be loaded: {e}")),
                ),
            },
            None => (Keymap::default(), None),
        };
        let bindings_json = keymap.json()?;
        // The app opens flying in real time; headless sessions start paused.
        let mut session = Session::new()?;
        session.run(flightcraft_control::Resume {})?;
        let paused = session.paused();
        Ok(Self {
            session,
            scene: Scene::default(),
            clock: clock::FixedStep::default(),
            input: input::Input::default(),
            keymap,
            paused,
            last_time: None,
            error,
            settings: false,
            bindings_json,
            binding_message: None,
            #[cfg(not(target_arch = "wasm32"))]
            script: _options.script.unwrap_or_default(),
            #[cfg(not(target_arch = "wasm32"))]
            screenshot: _options.screenshot,
            #[cfg(not(target_arch = "wasm32"))]
            frames: 0,
            #[cfg(not(target_arch = "wasm32"))]
            screenshot_requested: false,
            #[cfg(not(target_arch = "wasm32"))]
            requested_frame: 0,
        })
    }

    fn reset(&mut self) {
        match actions::reset(&mut self.session) {
            Ok(()) => {
                self.clock.reset();
                self.input.trim = 0.0;
                self.scene.reset_camera();
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn bindings_window(&mut self, ctx: &egui::Context) {
        egui::Window::new("Input bindings").open(&mut self.settings).default_width(530.0).show(ctx, |ui| {
            ui.label(RichText::new("KEYBOARD").color(theme::ACCENT).size(11.0));
            egui::Grid::new("binding list").num_columns(2).striped(true).show(ui, |ui| {
                for action in Action::ALL {
                    ui.label(action.label());
                    ui.monospace(self.keymap.label(action));
                    ui.end_row();
                }
            });
            ui.add_space(8.0);
            ui.label(RichText::new("GAMEPAD").color(theme::ACCENT).size(11.0));
            ui.label(&self.input.gamepad_status);
            ui.label("Left stick: pitch / roll · Right stick: yaw\nRight trigger: increase throttle · Left trigger: brakes\nD-pad: throttle · South: flaps · North: camera\nStart: pause · Select: reset · East: brakes");
            ui.separator();
            ui.label("Edit or paste JSON below. Apply validates it; settings save automatically.");
            egui::ScrollArea::vertical().max_height(170.0).show(ui, |ui| {
                ui.add(egui::TextEdit::multiline(&mut self.bindings_json).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY));
            });
            ui.horizontal(|ui| {
                if ui.button("Apply JSON").clicked() {
                    match Keymap::from_json(&self.bindings_json) {
                        Ok(map) => { self.keymap = map; self.binding_message = Some("Bindings applied".to_owned()); }
                        Err(e) => self.binding_message = Some(e),
                    }
                }
                if ui.button("Restore defaults").clicked() {
                    self.keymap = Keymap::default();
                    match self.keymap.json() {
                        Ok(json) => self.bindings_json = json,
                        Err(e) => self.binding_message = Some(e.to_string()),
                    }
                }
                if ui.button("Copy JSON").clicked() { ctx.copy_text(self.bindings_json.clone()); }
            });
            if let Some(message) = &self.binding_message { ui.label(RichText::new(message).color(theme::AMBER)); }
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn screenshot(&mut self, ctx: &egui::Context) {
        let Some(capture) = &self.screenshot else { return };
        self.frames = ctx.cumulative_frame_nr();
        let image = ctx.input(|i| {
            i.events.iter().find_map(|event| {
                if let egui::Event::Screenshot { image, .. } = event {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if self.screenshot_requested
            && let Some(image) = image
        {
            let pixels: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
            let result = image::save_buffer(
                &capture.path,
                &pixels,
                image.width() as u32,
                image.height() as u32,
                image::ColorType::Rgba8,
            );
            match result {
                Ok(()) => {
                    capture
                        .succeeded
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                    eprintln!("Saved screenshot: {}", capture.path.display());
                }
                Err(e) => eprintln!("Screenshot failed: {e}"),
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            self.screenshot = None;
        } else if !self.screenshot_requested
            && match capture.at_sim_s {
                Some(at) => self.session.sim().state().time_s >= at,
                None => self.frames >= capture.frames,
            }
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            self.screenshot_requested = true;
            self.requested_frame = self.frames;
        } else if self.screenshot_requested && self.frames > self.requested_frame.saturating_add(240) {
            eprintln!("Screenshot failed: the renderer did not return an image");
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            self.screenshot = None;
        }
    }
}

impl eframe::App for FlightCraftApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = ctx.input(|i| i.time);
        let elapsed = self
            .last_time
            .map_or(0.0, |previous| (now - previous).max(0.0))
            .min(0.25);
        self.last_time = Some(now);
        let frame = self.input.collect(ctx, &self.keymap);
        if frame.pressed(Action::Pause)
            && let Err(e) = actions::toggle_pause(&mut self.session, &mut self.paused)
        {
            self.error = Some(e.to_string());
        }
        if frame.pressed(Action::Reset) {
            self.reset();
        }
        if frame.pressed(Action::Camera) {
            actions::toggle_camera(&mut self.scene);
        }
        let controls = self
            .input
            .controls(&frame, self.session.sim().controls(), elapsed);
        let steps = self.clock.advance(elapsed, self.paused);
        let scripted = self.script.active();
        let sim_time_s = self.session.sim().state().time_s;
        if scripted && let Err(e) = self.script.run_due(&mut self.session, sim_time_s) {
            self.error = Some(e);
        }
        let controlled = if scripted {
            Ok(())
        } else {
            actions::set_controls(&mut self.session, controls)
        };
        if let Err(e) = controlled.and_then(|()| actions::step(&mut self.session, steps)) {
            self.error = Some(e.to_string());
        }
        let readout = self.session.sim().readout();

        egui::TopBottomPanel::top("header")
            .exact_height(64.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::BACKGROUND)
                    .inner_margin(egui::Margin::symmetric(22, 10)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    let (logo, _) = ui.allocate_exact_size(egui::vec2(28.0, 30.0), egui::Sense::hover());
                    let c = logo.center();
                    ui.painter().line_segment(
                        [c + egui::vec2(0.0, -11.0), c + egui::vec2(0.0, 11.0)],
                        egui::Stroke::new(2.5_f32, theme::ACCENT),
                    );
                    ui.painter().line_segment(
                        [c + egui::vec2(-13.0, -2.0), c + egui::vec2(13.0, -2.0)],
                        egui::Stroke::new(2.5_f32, theme::ACCENT),
                    );
                    ui.label(RichText::new("FlightCraft").size(23.0).strong());
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new("CORVID  /  01")
                            .monospace()
                            .size(11.0)
                            .color(theme::MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Input bindings").clicked() {
                            self.settings = !self.settings;
                        }
                        let camera = match self.scene.camera {
                            CameraMode::Chase => "Chase",
                            CameraMode::Cockpit => "Cockpit",
                        };
                        if ui
                            .button(format!("{camera}  [{}]", self.keymap.label(Action::Camera)))
                            .clicked()
                        {
                            actions::toggle_camera(&mut self.scene);
                        }
                        if ui
                            .button(format!("Reset  [{}]", self.keymap.label(Action::Reset)))
                            .clicked()
                        {
                            self.reset();
                        }
                        let pause = if self.paused { "Resume" } else { "Pause" };
                        if ui
                            .button(format!("{pause}  [{}]", self.keymap.label(Action::Pause)))
                            .clicked()
                            && let Err(e) = actions::toggle_pause(&mut self.session, &mut self.paused)
                        {
                            self.error = Some(e.to_string());
                        }
                    });
                });
            });
        egui::TopBottomPanel::bottom("footer")
            .exact_height(30.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::BACKGROUND)
                    .inner_margin(egui::Margin::symmetric(22, 6)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (status, color) = if readout.crashed {
                        ("CRASHED · RESET TO FLY", theme::DANGER)
                    } else if readout.on_ground {
                        ("ON GROUND", theme::ACCENT)
                    } else {
                        ("AIRBORNE", theme::ACCENT)
                    };
                    ui.label(RichText::new(status).monospace().size(10.0).color(color));
                    ui.separator();
                    ui.label(
                        RichText::new(if self.paused {
                            "PAUSED"
                        } else {
                            "RUNNING · 120 Hz"
                        })
                        .monospace()
                        .size(10.0)
                        .color(if self.paused {
                            theme::AMBER
                        } else {
                            theme::MUTED
                        }),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new("LOCAL NED  ·  STANDARD ATMOSPHERE  ·  NO WIND")
                                .size(10.0)
                                .color(theme::MUTED),
                        );
                    });
                });
            });
        egui::TopBottomPanel::bottom("instruments")
            .exact_height(220.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .inner_margin(egui::Margin::symmetric(12, 5)),
            )
            .show(ctx, |ui| {
                instruments::panel(ui, &readout, self.input.trim);
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BACKGROUND))
            .show(ctx, |ui| {
                self.scene
                    .show(ui, self.session.sim().state(), elapsed as f32, theme::scene());
                let rect = ui.max_rect().shrink(24.0);
                let painter = ui.painter();
                painter.text(
                    rect.left_top(),
                    Align2::LEFT_TOP,
                    "TRAINING FIELD",
                    FontId::monospace(12.0),
                    theme::BACKGROUND,
                );
                painter.text(
                    rect.left_top() + egui::vec2(0.0, 20.0),
                    Align2::LEFT_TOP,
                    "RUNWAY 36   /   900 × 23 m",
                    FontId::monospace(10.0),
                    theme::BACKGROUND,
                );
                painter.text(
                    rect.right_top(),
                    Align2::RIGHT_TOP,
                    format!(
                        "SESSION   {:02}:{:05.2}",
                        (readout.time_s / 60.0) as u64,
                        readout.time_s % 60.0
                    ),
                    FontId::monospace(12.0),
                    theme::BACKGROUND,
                );
                let hint = format!(
                    "PITCH {}  ·  ROLL {}  ·  THROTTLE {} / {}",
                    self.keymap.label(Action::ElevatorUp),
                    self.keymap.label(Action::AileronLeft),
                    self.keymap.label(Action::ThrottleUp),
                    self.keymap.label(Action::ThrottleDown)
                );
                let hint_rect = egui::Rect::from_min_size(
                    rect.left_bottom() - egui::vec2(0.0, 29.0),
                    egui::vec2((rect.width() - 4.0).min(660.0), 29.0),
                );
                painter.rect_filled(hint_rect, 4.0, theme::BACKGROUND.linear_multiply(0.82));
                painter.text(
                    hint_rect.left_center() + egui::vec2(12.0, 0.0),
                    Align2::LEFT_CENTER,
                    hint,
                    FontId::monospace(10.0),
                    theme::TEXT,
                );
                if self.paused {
                    painter.text(
                        rect.center_top() + egui::vec2(0.0, 20.0),
                        Align2::CENTER_TOP,
                        "P A U S E D",
                        FontId::monospace(18.0),
                        theme::BACKGROUND,
                    );
                }
            });
        if let Some(error) = &self.error {
            egui::Window::new("FlightCraft message")
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label(RichText::new(error).color(theme::AMBER));
                });
        }
        self.bindings_window(ctx);
        #[cfg(not(target_arch = "wasm32"))]
        self.screenshot(ctx);
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(1.0 / 60.0));
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Ok(json) = self.keymap.json() {
            storage.set_string(KEYMAP_STORAGE, json);
        }
    }
}
