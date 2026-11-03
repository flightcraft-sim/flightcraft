//! The shared eframe application for desktop and browser.
pub mod actions;
mod audio;
pub mod autopilot;
#[cfg(not(target_arch = "wasm32"))]
mod bridge;
pub mod clock;
pub mod glass;
mod input;
pub mod instruments;
pub mod keymap;
pub mod map;
mod modern;
mod navigation;
mod panel_art;
pub mod panel_preview;
mod procedures;
pub mod radio;
mod radio_speech;
pub mod script;
pub mod theme;
mod weather;

use eframe::egui::{self, Align2, FontId, RichText};
use flightcraft_control::Session;
pub use flightcraft_control::{PackOptions, RegionOptions};
use flightcraft_render::{CameraMode, Scene};
use keymap::{Action, Keymap};

#[cfg(not(target_arch = "wasm32"))]
const PACKS_STORAGE: &str = "flightcraft.packs_directory.v1";

const KEYMAP_STORAGE: &str = "flightcraft.keymap.v1";
const DISPLAY_OPTIONS_STORAGE: &str = "flightcraft.instrument_units.v1";
const COMPACT_PANEL_STORAGE: &str = "flightcraft.compact_panel.v1";

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
    #[cfg(not(target_arch = "wasm32"))]
    pub control: Option<flightcraft_control::bridge::Listener>,
    /// Timed commands to run; they hold the flight controls until they run out.
    pub script: Option<script::Script>,
    pub packs: flightcraft_control::PackOptions,
    pub regions: flightcraft_control::RegionOptions,
}

/// One session, one fixed-step accumulator, and read-only presentation of its state.
pub struct FlightCraftApp {
    session: Session,
    efb_settings: flightcraft_control::EfbSettings,
    efb_applied: flightcraft_control::EfbSettings,
    #[cfg(not(target_arch = "wasm32"))]
    efb_serial: Option<u64>,
    #[cfg(not(target_arch = "wasm32"))]
    packs_directory: String,
    #[cfg(not(target_arch = "wasm32"))]
    bridge: bridge::Bridge,
    scene: Scene,
    clock: clock::FixedStep,
    input: input::Input,
    keymap: Keymap,
    paused: bool,
    last_time: Option<f64>,
    error: Option<String>,
    /// The GPU path drawing the app, for the status bar.
    renderer: &'static str,
    settings: bool,
    compact_panel: bool,
    instrument_panel: instruments::InstrumentPanel,
    glass_controls: glass::GlassControls,
    procedure_controls: procedures::Controls,
    glass_popout_knob: instruments::heading::Knob,
    bindings_json: String,
    airport_search: String,
    planning_map: map::PlanningMap,
    mfd_map: map::PlanningMap,
    navigation: navigation::SearchPanel,
    glance_origin: Option<[f64; 2]>,
    autopilot_controller: autopilot::Controller,
    audio: audio::Audio,
    radio: radio::RadioUi,
    radio_ptt_held: [bool; 2],
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
        options: AppOptions,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        theme::install(&cc.egui_ctx);
        let render_state = cc
            .wgpu_render_state
            .as_ref()
            .ok_or_else(|| std::io::Error::other("FlightCraft needs the wgpu renderer"))?;
        flightcraft_render::initialize(render_state, theme::scene());
        flightcraft_render::initialize_synthetic_vision(render_state, &cc.egui_ctx);
        // Shown in the status bar: which GPU path is drawing matters for every graphics bug report.
        let renderer = match render_state.adapter.get_info().backend {
            eframe::wgpu::Backend::BrowserWebGpu => "WebGPU",
            eframe::wgpu::Backend::Gl if cfg!(target_arch = "wasm32") => "WebGL 2",
            eframe::wgpu::Backend::Gl => "OpenGL",
            eframe::wgpu::Backend::Vulkan => "Vulkan",
            eframe::wgpu::Backend::Metal => "Metal",
            eframe::wgpu::Backend::Dx12 => "Direct3D 12",
            eframe::wgpu::Backend::Noop => "no GPU",
        };
        let stored = cc.storage.and_then(|s| s.get_string(KEYMAP_STORAGE));
        let (keymap, mut error) = match stored {
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
        if let Some(json) = cc.storage.and_then(|s| s.get_string("flightcraft.callsign.v1"))
            && let Ok(value) = serde_json::from_str(&json)
            && let Err(e) = session.execute("comms.callsign", value)
        {
            error = Some(e.to_string());
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(directory) = flightcraft_control::navigation_data_directory()
            && let Err(e) = session.load_navigation_storage(&directory)
        {
            error = Some(e.to_string());
        }
        #[cfg(target_arch = "wasm32")]
        if let Err(e) = session.load_browser_pins() {
            error = Some(e.to_string());
        }
        let mut pack_errors = options.packs.errors;
        let chart_root = options.regions.directory.clone();
        pack_errors.extend(options.regions.errors);
        #[cfg(not(target_arch = "wasm32"))]
        pack_errors.extend(session.load_regions_directory(std::path::Path::new(
            options.regions.directory.as_deref().unwrap_or("assets/terrain"),
        )));
        let initial_region = options.regions.fetched.first().map(|w| w.choice());
        for world in options.regions.fetched {
            if let Err(e) = session.install_region(world) {
                pack_errors.push(e.to_string());
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        let packs_directory = options
            .packs
            .directory
            .or_else(|| cc.storage.and_then(|s| s.get_string(PACKS_STORAGE)))
            .unwrap_or_else(|| "packs".into());
        #[cfg(not(target_arch = "wasm32"))]
        pack_errors.extend(session.load_packs_directory(std::path::Path::new(&packs_directory)));
        for pack in options.packs.fetched {
            if let Err(e) = session.install_pack(&pack) {
                pack_errors.push(e.to_string());
            }
        }
        if !pack_errors.is_empty() {
            if let Some(previous) = error.take() {
                pack_errors.insert(0, previous);
            }
            error = Some(pack_errors.join("\n"));
        }
        if let Some(world) = initial_region {
            session.run(flightcraft_control::Reset {
                world,
                start: flightcraft_control::ResetStart::Runway {},
                weather: None,
                paused: Some(false),
            })?;
        }
        session.enable_app();
        let efb_settings: flightcraft_control::EfbSettings = cc
            .storage
            .and_then(|s| s.get_string("efb.settings"))
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        #[cfg(not(target_arch = "wasm32"))]
        let efb_serial = {
            let serial = cc
                .storage
                .and_then(|s| s.get_string("efb.serial"))
                .and_then(|s| s.parse::<u64>().ok());
            let serial = serial.map(Ok).unwrap_or_else(|| {
                let mut bytes = [0_u8; 8];
                getrandom::fill(&mut bytes)
                    .map(|()| u64::from_be_bytes(bytes))
                    .map_err(|e| e.to_string())
            });
            match serial {
                Ok(serial) => {
                    if let Err(e) = session.initialize_efb(serial) {
                        error = Some(e);
                    } else if let Err(e) = session.run(flightcraft_control::EfbBroadcast {
                        enabled: efb_settings.enabled,
                        destination: efb_settings.destination.clone(),
                        gdl90: efb_settings.gdl90,
                        simulator: efb_settings.simulator,
                    }) {
                        error = Some(e.to_string());
                    }
                    Some(serial)
                }
                Err(e) => {
                    error = Some(format!("Tablet identity could not be created: {e}"));
                    None
                }
            }
        };
        session.run(flightcraft_control::Resume {})?;
        let paused = session.paused();
        let efb_applied = session.efb_status().settings;
        Ok(Self {
            session,
            efb_applied,
            efb_settings,
            #[cfg(not(target_arch = "wasm32"))]
            efb_serial,
            #[cfg(not(target_arch = "wasm32"))]
            packs_directory,
            #[cfg(not(target_arch = "wasm32"))]
            bridge: bridge::Bridge::new(options.control),
            scene: Scene::default(),
            clock: clock::FixedStep::default(),
            input: input::Input::default(),
            keymap,
            paused,
            last_time: None,
            error,
            renderer,
            settings: false,
            compact_panel: cc
                .storage
                .and_then(|s| s.get_string(COMPACT_PANEL_STORAGE))
                .as_deref()
                == Some("true"),
            glass_controls: glass::GlassControls::default(),
            procedure_controls: procedures::Controls::default(),
            glass_popout_knob: Default::default(),
            instrument_panel: instruments::InstrumentPanel::with_options(
                cc.storage
                    .and_then(|s| s.get_string(DISPLAY_OPTIONS_STORAGE))
                    .and_then(|json| serde_json::from_str(&json).ok())
                    .unwrap_or_default(),
            ),
            bindings_json,
            airport_search: String::new(),
            planning_map: map::PlanningMap::with_root(chart_root.clone()),
            mfd_map: map::PlanningMap::with_root(chart_root),
            navigation: navigation::SearchPanel::default(),
            glance_origin: None,
            autopilot_controller: autopilot::Controller::default(),
            radio: radio::RadioUi::default(),
            radio_ptt_held: [false; 2],
            audio: audio::Audio::with_preferences(
                cc.storage
                    .and_then(|s| s.get_string("flightcraft.audio.v1"))
                    .and_then(|json| serde_json::from_str(&json).ok())
                    .unwrap_or_default(),
            ),
            binding_message: None,
            script: options.script.unwrap_or_default(),
            #[cfg(not(target_arch = "wasm32"))]
            screenshot: options.screenshot,
            #[cfg(not(target_arch = "wasm32"))]
            frames: 0,
            #[cfg(not(target_arch = "wasm32"))]
            screenshot_requested: false,
            #[cfg(not(target_arch = "wasm32"))]
            requested_frame: 0,
        })
    }

    /// Register declarative private skin artwork before loading its panel.
    pub fn register_instrument_skin(&mut self, json: &str) -> Result<(), String> {
        self.instrument_panel.registry.register_pack_skin(json)
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
        let applied = self.session.efb_status().settings;
        if applied != self.efb_applied {
            self.efb_settings = applied.clone();
            self.efb_applied = applied;
        }
        egui::Window::new("Preferences and input").open(&mut self.settings).default_width(530.0).max_height(620.0).vscroll(true).show(ctx, |ui| {
            ui.label(RichText::new("INSTRUMENT PANEL").color(theme::ACCENT).size(11.0));
            ui.checkbox(&mut self.compact_panel, "Compact instruments (one row)");
            let mut visible = self.session.airspace_visible();
            if ui.checkbox(&mut visible, "Show airspace curtains and outlines").changed()
                && let Err(e) = self.session.run(flightcraft_control::SetAirspaceView { visible }) {
                self.error = Some(e.to_string());
            }
            ui.checkbox(&mut self.compact_panel, "Compact panel");
            ui.horizontal(|ui| {
                ui.label("Altimeter pressure");
                ui.selectable_value(&mut self.instrument_panel.options.pressure, instruments::PressureUnit::InHg,"inHg");
                ui.selectable_value(&mut self.instrument_panel.options.pressure, instruments::PressureUnit::Hpa,"hPa");
            });
            ui.horizontal(|ui| {
                ui.label("Engine and fuel units");
                ui.selectable_value(&mut self.instrument_panel.options.engine, instruments::EngineUnits::Us,"US customary");
                ui.selectable_value(&mut self.instrument_panel.options.engine, instruments::EngineUnits::Si,"SI");
            });
            ui.separator();
            ui.label(RichText::new("Send to tablet apps").color(theme::ACCENT));
            if cfg!(target_arch = "wasm32") {
                ui.label("Unavailable in the browser: UDP requires the desktop app.");
            } else {
                ui.checkbox(&mut self.efb_settings.enabled, "Enable tablet broadcast");
                ui.horizontal(|ui| {
                    ui.label("Destination IPv4");
                    ui.text_edit_singleline(&mut self.efb_settings.destination);
                });
                ui.checkbox(&mut self.efb_settings.gdl90, "GDL 90 (port 4000)");
                ui.checkbox(&mut self.efb_settings.simulator, "Simulator position and attitude (port 49002)");
                if ui.button("Apply tablet settings").clicked() {
                    match self.session.run(flightcraft_control::EfbBroadcast {
                        enabled: self.efb_settings.enabled, destination: self.efb_settings.destination.clone(),
                        gdl90: self.efb_settings.gdl90, simulator: self.efb_settings.simulator,
                    }) {
                        Ok(status) => self.efb_settings = status.settings,
                        Err(e) => self.error = Some(e.to_string()),
                    }
                }
                let status = self.session.efb_status();
                ui.label(format!("{} packets/s · GDL 90: {} · Simulator: {}", status.packets_per_second, status.gdl90_packets, status.simulator_packets));
                if let Some(reason) = status.reason { ui.label(reason); }
                if let Some(error) = status.last_error { ui.colored_label(theme::AMBER, error); }
            }
            ui.label(RichText::new("SOUND").color(theme::ACCENT).size(11.0));
            ui.checkbox(&mut self.audio.preferences.muted,"Mute sound");
            ui.add(egui::Slider::new(&mut self.audio.preferences.volume,0.0..=1.0).text("Master volume"));
            if let Some(error)=&self.audio.error {ui.colored_label(theme::AMBER,error);}
            ui.separator();
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
            ui.label("Hold a glance, then release to return. Gamepad buttons are editable in gamepad_bindings below.");
            ui.label("Left stick: pitch / roll · Right stick: yaw\nRight trigger: increase throttle · Left trigger: brakes");
            for (action, buttons) in &self.keymap.gamepad_bindings {
                ui.label(format!("{}: {buttons:?}", action.label()));
            }
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
                if let egui::Event::Screenshot { image, user_data, .. } = event
                    && user_data.data.is_none()
                {
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

impl FlightCraftApp {
    /// The GPU path drawing the app: "WebGPU", "WebGL 2", "Vulkan", "Metal", and so on.
    #[must_use]
    pub fn renderer(&self) -> &'static str {
        self.renderer
    }
}

impl eframe::App for FlightCraftApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        #[cfg(not(target_arch = "wasm32"))]
        self.bridge.update(ctx, &mut self.session);
        if ctx.input(|input| {
            input.pointer.any_pressed()
                || input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::Key { pressed: true, .. }))
        }) {
            self.audio.user_gesture();
        }
        self.paused = self.session.paused();
        let now = ctx.input(|i| i.time);
        let elapsed = self
            .last_time
            .map_or(0.0, |previous| (now - previous).max(0.0))
            .min(0.25);
        self.last_time = Some(now);
        let frame = self.input.collect(ctx, &self.keymap);
        self.instrument_panel.starter_key_held = frame.held(Action::Starter);
        for action in [
            Action::ComPtt,
            Action::IntercomPtt,
            Action::RadioPanel,
            Action::Kneeboard,
            Action::RadioTransmit,
        ] {
            if frame.pressed(action) {
                self.radio.shortcut(action);
            }
        }
        let held = [frame.held(Action::ComPtt), frame.held(Action::IntercomPtt)];
        if held != self.radio_ptt_held {
            let setting = flightcraft_control::CommsAudioPanel {
                ptt: (held[0] != self.radio_ptt_held[0]).then_some(held[0]),
                intercom_ptt: (held[1] != self.radio_ptt_held[1]).then_some(held[1]),
                ..Default::default()
            };
            self.radio_ptt_held = held;
            if let Err(error) = self.session.run(setting) {
                self.error = Some(error.to_string());
            }
        }
        if frame.pressed(Action::Pause)
            && let Err(e) = actions::toggle_pause(&mut self.session, &mut self.paused)
        {
            self.error = Some(e.to_string());
        }
        if frame.pressed(Action::PlanningMap) {
            self.planning_map.open = !self.planning_map.open;
        }
        if self.planning_map.open {
            if frame.pressed(Action::MapZoomIn) {
                self.planning_map.zoom(&mut self.session, 0.8);
            }
            if frame.pressed(Action::MapZoomOut) {
                self.planning_map.zoom(&mut self.session, 1.25);
            }
        }
        if frame.pressed(Action::Search) {
            self.navigation.focus = true;
        }
        if frame.pressed(Action::Reset) {
            self.reset();
        }
        if frame.pressed(Action::Camera)
            && let Err(e) = actions::toggle_camera(&mut self.session)
        {
            self.error = Some(e.to_string());
        }
        if frame.pressed(Action::Hood) {
            let enabled = !self.session.sim().hood();
            if let Err(e) = self.session.run(flightcraft_control::SetHood { enabled }) {
                self.error = Some(e.to_string());
            }
        }
        if frame.pressed(Action::Airspace) {
            let visible = !self.session.airspace_visible();
            if let Err(e) = self.session.run(flightcraft_control::SetAirspaceView { visible }) {
                self.error = Some(e.to_string());
            }
        }
        let glass_active = self
            .session
            .aircraft_configuration()
            .is_some_and(|c| c.panel.instruments.iter().any(|i| i.module == "glass_pfd"));
        for action in [
            Action::HeadingBugLeft,
            Action::HeadingBugRight,
            Action::HeadingBugSync,
        ] {
            if frame.pressed(action) {
                let setting = match action {
                    Action::HeadingBugSync => flightcraft_control::AvionicsSetting::HeadingBugSync {},
                    _ => flightcraft_control::AvionicsSetting::HeadingBugStep {
                        detents: if action == Action::HeadingBugLeft { -1 } else { 1 },
                        interval_ms: None,
                    },
                };
                if let Err(e) = self.session.run(flightcraft_control::SetAvionics { setting }) {
                    self.error = Some(e.to_string());
                }
            }
        }
        for action in Action::AVIONICS {
            if matches!(
                action,
                Action::HeadingBugLeft | Action::HeadingBugRight | Action::HeadingBugSync
            ) {
                continue;
            }
            if frame.pressed(action) {
                if action == Action::StandbyDirectTo && self.session.sim().systems().avionics.installed {
                    self.instrument_panel.avionics_controls.open_direct_to();
                }
                if let Some(setting) = instruments::avionics_shortcut_with_options(
                    action,
                    &self.session.sim().systems().avionics,
                    self.instrument_panel.options,
                ) && let Err(e) = self.session.run(flightcraft_control::SetAvionics { setting })
                {
                    self.error = Some(e.to_string());
                }
            }
        }
        if glass_active {
            let state = self.session.glass_state();
            for action in Action::GLASS {
                if matches!(action, Action::HeadingBugLeft | Action::HeadingBugRight) {
                    continue;
                }
                if frame.pressed(action)
                    && let Some(setting) = glass::shortcut(action, &state)
                    && let Err(e) = self.session.run(flightcraft_control::SetAvionics {
                        setting: flightcraft_control::AvionicsSetting::Glass { setting },
                    })
                {
                    self.error = Some(e.to_string());
                }
            }
        }
        for action in Action::SYSTEMS {
            if matches!(action, Action::HeadingBugLeft | Action::HeadingBugRight)
                || (self.session.sim().systems().avionics.installed
                    && matches!(action, Action::AltimeterUp | Action::AltimeterDown))
            {
                continue;
            }
            if frame.pressed(action)
                && let Some(setting) = instruments::shortcut_with_options(
                    action,
                    self.session.sim().systems(),
                    self.instrument_panel.options,
                )
                && let Err(e) = self.session.run(flightcraft_control::SetSystems { setting })
            {
                self.error = Some(e.to_string());
            }
        }
        if let Some(state) = self.session.sim().installed_autopilot().cloned() {
            for action in Action::AUTOPILOT {
                if frame.pressed(action)
                    && let Some(setting) = autopilot::shortcut(action, &state)
                    && let Err(e) = self.session.run(flightcraft_control::SetAvionics { setting })
                {
                    self.error = Some(e.to_string());
                }
            }
        }
        let scripted = self.script.active();
        let sim_time_s = self.session.sim().state().time_s;
        if scripted && let Err(e) = self.script.run_due(&mut self.session, sim_time_s) {
            self.error = Some(e);
        }
        let [yaw, pitch] = frame.cockpit_look();
        if let Some(camera) = self.session.camera() {
            let looking =
                camera.mode == flightcraft_control::CameraMode::Cockpit && (yaw != 0.0 || pitch != 0.0);
            let result = if looking {
                let [base_yaw, base_pitch] = *self
                    .glance_origin
                    .get_or_insert([camera.look_yaw_rad, camera.look_pitch_rad]);
                actions::cockpit_look(&mut self.session, base_yaw + yaw * 1.35, base_pitch + pitch * 0.9)
            } else if let Some([yaw, pitch]) = self.glance_origin.take() {
                actions::cockpit_look(&mut self.session, yaw, pitch)
            } else {
                Ok(())
            };
            if let Err(e) = result {
                self.error = Some(e.to_string());
            }
        }
        self.paused = self.session.paused();
        let external = {
            #[cfg(not(target_arch = "wasm32"))]
            {
                self.bridge.controls_active()
            }
            #[cfg(target_arch = "wasm32")]
            {
                false
            }
        };
        let controlled = if scripted || external || self.session.holds_active() {
            Ok(())
        } else {
            let controls = self
                .input
                .controls(&frame, self.session.sim().controls(), elapsed);
            actions::set_controls(&mut self.session, controls)
        };
        let steps = self.clock.advance(elapsed * self.session.rate(), self.paused);
        if let Err(e) = controlled.and_then(|()| actions::step(&mut self.session, steps)) {
            self.error = Some(e.to_string());
        }
        let disconnect_sequence = self
            .session
            .sim()
            .installed_autopilot()
            .map_or(0, |s| s.disconnect_sequence);
        self.audio
            .observe(self.session.flight_generation(), disconnect_sequence);
        self.radio.show(
            ctx,
            &mut self.session,
            &self.keymap,
            self.audio.preferences.gain(),
        );
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
                    if ui.button(format!("Map [{}]", self.keymap.label(Action::PlanningMap))).clicked() { self.planning_map.open = !self.planning_map.open; }
                    self.navigation.show(ui, &mut self.session);
                    ui.menu_button("Weather", |ui| {
                        if let Err(e) = weather::panel(ui, &mut self.session, &self.keymap) { self.error = Some(e.to_string()); }
                    });
                    self.radio.button(ui, &self.keymap);
                    for setting in self.procedure_controls.show(ui,&self.session){
                        if let Err(error)=self.session.run(flightcraft_control::SetAvionics{setting}){self.error=Some(error.to_string());}
                    }
                    ui.add_space(12.0);
                    let identity = self.session.sim().aircraft().identity.clone();
                    ui.menu_button(
                        RichText::new(&identity.display_name)
                            .monospace()
                            .size(11.0)
                            .color(theme::MUTED),
                        |ui| match actions::aircraft_choices(&mut self.session) {
                            Ok(choices) => {
                                for aircraft in choices.aircraft {
                                    if ui
                                        .selectable_label(identity.id == aircraft.id, &aircraft.display_name)
                                        .on_hover_text(&aircraft.description)
                                        .clicked()
                                    {
                                        match actions::select_aircraft(&mut self.session, &aircraft.id) {
                                            Ok(()) => {
                                                self.paused = self.session.paused();
                                                self.input.trim = 0.0;
                                                self.clock.reset();
                                                self.scene.reset_camera();
                                                self.error = None;
                                            }
                                            Err(e) => self.error = Some(e.to_string()),
                                        }
                                        ui.close();
                                    }
                                }
                            }
                            Err(e) => {
                                self.error = Some(e.to_string());
                            }
                        },
                    );
                    ui.menu_button("World", |ui| {
                        match self.session.run(flightcraft_control::ListWorlds {}) {
                            Ok(choices) => {
                                for world in choices.worlds {
                                    if ui
                                        .selectable_label(
                                            self.session.sim().world().choice() == world.id,
                                            &world.name,
                                        )
                                        .on_hover_text(&world.attribution)
                                        .clicked()
                                    {
                                        match self.session.run(flightcraft_control::Reset {
                                            world: world.id,
                                            start: flightcraft_control::ResetStart::Runway {},
                                            weather: Some(self.session.pilot_weather()),
                                            paused: Some(self.paused),
                                        }) {
                                            Ok(_) => {
                                                self.clock.reset();
                                                self.input.trim = 0.0;
                                                self.scene.reset_camera();
                                                self.error = None;
                                            }
                                            Err(e) => self.error = Some(e.to_string()),
                                        }
                                        ui.close();
                                    }
                                }
                            }
                            Err(e) => self.error = Some(e.to_string()),
                        }
                    });
                    if let Some((label, warning, attribution)) = airspace_status(&self.session) {
                        ui.separator();
                        ui.label(RichText::new(label).size(11.0).color(if warning {
                            theme::AMBER
                        } else {
                            theme::TEXT
                        }))
                        .on_hover_text(attribution);
                    }
                    if self.session.sim().world().region().is_some() {
                        ui.menu_button("Start", |ui| {
                            ui.label("Airport or name");
                            ui.text_edit_singleline(&mut self.airport_search);
                            match self.session.run(flightcraft_control::AirportQuery { nearest_to_m: None }) {
                                Ok(list) => {
                                    let search = self.airport_search.to_lowercase();
                                    egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                                        for airport in list.airports.iter().filter(|a| a.ident.to_lowercase().contains(&search) || a.name.to_lowercase().contains(&search) || a.icao.as_deref().is_some_and(|s|s.to_lowercase().contains(&search))) {
                                            ui.menu_button(format!("{} · {}", airport.ident, airport.name), |ui| {
                                                for runway in &airport.runways {
                                                    for end in &runway.ends {
                                                        if ui.button(format!("Runway {} · {:.0} × {:.0} m", end.ident, runway.length_m, runway.width_m)).clicked() {
                                                            match self.session.run(flightcraft_control::Reset {
                                                                world: self.session.sim().world().choice(),
                                                                start: flightcraft_control::ResetStart::AirportRunway { airport: airport.ident.clone(), end: end.ident.clone() },
                                                                weather: Some(self.session.pilot_weather()), paused: Some(self.paused),
                                                            }) {
                                                                Ok(_) => { self.clock.reset(); self.input.trim = 0.0; self.scene.reset_camera(); self.error = None; }
                                                                Err(e) => self.error = Some(e.to_string()),
                                                            }
                                                            ui.close();
                                                        }
                                                    }
                                                }
                                            });
                                        }
                                    });
                                    ui.separator();
                                    ui.label(&list.attribution);
                                }
                                Err(e) => self.error = Some(e.to_string()),
                            }
                        });
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Preferences").clicked() {
                            self.settings = !self.settings;
                        }
                        let camera = match self.scene.camera {
                            CameraMode::Chase => "Chase",
                            CameraMode::Cockpit => "Cockpit",
                        };
                        if ui
                            .button(format!("{camera}  [{}]", self.keymap.label(Action::Camera)))
                            .clicked()
                            && let Err(e) = actions::toggle_camera(&mut self.session)
                        {
                            self.error = Some(e.to_string());
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
        let readout = self.session.sim().readout();
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
                    ui.separator();
                    ui.label(
                        RichText::new(self.renderer)
                            .monospace()
                            .size(10.0)
                            .color(theme::MUTED),
                    );
                    if let (Some(lat), Some(lon)) = (readout.latitude_deg, readout.longitude_deg) {
                        ui.separator();
                        ui.label(
                            RichText::new(format!("{lat:.5}°, {lon:.5}°"))
                                .monospace()
                                .size(10.0)
                                .color(theme::MUTED),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(self.session.sim().world().attribution())
                                .size(10.0)
                                .color(theme::MUTED),
                        );
                    });
                });
            });
        if self.planning_map.open {
            for setting in self
                .instrument_panel
                .cancel_equipment_holds(self.session.flight_generation())
            {
                if let Err(e) = self.session.run(flightcraft_control::SetEquipment { setting }) {
                    self.error = Some(e.to_string());
                }
            }
        }
        if !self.planning_map.open {
            if self
                .session
                .aircraft_configuration()
                .is_none_or(|c| c.panel.equipment.is_none())
                && let Some(state) = self.session.sim().installed_autopilot().cloned()
            {
                egui::TopBottomPanel::bottom("autopilot-controller").show(ctx, |ui| {
                    for setting in self.autopilot_controller.show(
                        ui,
                        &state,
                        self.session.sim().systems().controls.heading_bug_rad,
                        frame.held(Action::ControlWheelSteering),
                    ) {
                        if let Err(e) = self.session.run(flightcraft_control::SetAvionics { setting }) {
                            self.error = Some(e.to_string());
                        }
                    }
                });
            }
            egui::TopBottomPanel::bottom("instruments")
                .exact_height(self.session.aircraft_configuration().map_or_else(
                    || instruments::panel_height(ctx.content_rect().height(), self.compact_panel),
                    |c| {
                        instruments::view_panel_height(
                            &c.panel,
                            ctx.content_rect().size(),
                            self.compact_panel,
                        )
                    },
                ))
                .frame(
                    egui::Frame::new()
                        .fill(egui::Color32::TRANSPARENT)
                        .inner_margin(egui::Margin::symmetric(0, 5)),
                )
                .show(ctx, |ui| {
                    let state = self.session.glass_state();
                    let mut glass_commands = if glass_active {
                        self.glass_controls.show(ui, &state)
                    } else {
                        Vec::new()
                    };
                    let result = if let Some(configuration) = self.session.aircraft_configuration() {
                        self.instrument_panel.show(
                            ui,
                            &configuration.panel,
                            instruments::InstrumentInput::new(self.session.sim().systems(), readout.time_s)
                                .with_aircraft(configuration)
                                .with_airports(self.session.sim().world().airports())
                                .with_vision(self.session.sim().world(), self.session.sim().state())
                                .with_glass(&state)
                                .with_autopilot(self.session.sim().installed_autopilot()),
                            self.session.flight_generation(),
                        )
                    } else {
                        self.instrument_panel.show_legacy(
                            ui,
                            instruments::InstrumentInput::new(self.session.sim().systems(), readout.time_s),
                            self.session.flight_generation(),
                        )
                    };
                    self.instrument_panel.show_shared_hosts(ui, &mut self.session);
                    if let Some(rect) = self.instrument_panel.map_rect() {
                        self.mfd_map.options = self.instrument_panel.options;
                        self.mfd_map.show_mfd(ui, rect, &mut self.session);
                    }
                    for setting in self.instrument_panel.avionics_controls.take_settings() {
                        if let Err(e) = self.session.run(flightcraft_control::SetAvionics { setting }) {
                            self.error = Some(e.to_string());
                        }
                    }
                    for setting in self.instrument_panel.take_heading_commands() {
                        if let Err(e) = self.session.run(flightcraft_control::SetAvionics { setting }) {
                            self.error = Some(e.to_string());
                        }
                    }
                    for setting in self.instrument_panel.take_equipment_commands() {
                        if let Err(e) = self.session.run(flightcraft_control::SetEquipment { setting }) {
                            self.error = Some(e.to_string());
                        }
                    }
                    glass_commands.extend(self.instrument_panel.take_glass_commands());
                    for setting in glass_commands {
                        if let Err(e) = self.session.run(flightcraft_control::SetAvionics {
                            setting: flightcraft_control::AvionicsSetting::Glass { setting },
                        }) {
                            self.error = Some(e.to_string());
                        }
                    }
                    match result {
                        Ok(settings) => {
                            for setting in settings {
                                if let Err(e) = self.session.run(flightcraft_control::SetSystems { setting })
                                {
                                    self.error = Some(e.to_string());
                                }
                            }
                        }
                        Err(e) => {
                            ui.colored_label(theme::DANGER, &e);
                            self.error = Some(e);
                        }
                    }
                });
        }
        if glass_active && let Some(id) = self.glass_controls.popout.clone() {
            let mut open = true;
            let state = self.session.glass_state();
            let mut knob_commands = Vec::new();
            egui::Window::new(format!("{} display", id))
                .open(&mut open)
                .default_size(egui::vec2(650., 510.))
                .show(ctx, |ui| {
                    if let Ok(mut display) = glass::create(&id) {
                        let mut input =
                            instruments::InstrumentInput::new(self.session.sim().systems(), readout.time_s)
                                .with_vision(self.session.sim().world(), self.session.sim().state())
                                .with_glass(&state)
                                .with_autopilot(self.session.sim().installed_autopilot());
                        input.aircraft = self.session.aircraft_configuration();
                        input.options = self.instrument_panel.options;
                        let draw = display.update(input);
                        let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
                        let [canvas_width, canvas_height] = draw.size;
                        let sx = rect.width() / canvas_width;
                        let sy = rect.height() / canvas_height;
                        glass::paint_fill(ui, rect, draw);
                        if id == "glass_mfd"
                            && !state.mfd_failed
                            && state.settings.page == flightcraft_control::GlassPage::Map
                        {
                            let map_rect = egui::Rect::from_min_max(
                                rect.min + egui::vec2(110. * sx, 28. * sy),
                                rect.max,
                            );
                            self.mfd_map.options = self.instrument_panel.options;
                            self.mfd_map.show_mfd(ui, map_rect, &mut self.session);
                        }
                        if id == "glass_pfd" {
                            let knob_rect = egui::Rect::from_center_size(
                                rect.min + egui::vec2(359. * sx, 404. * sy),
                                egui::vec2(26., 26.) * sx.min(sy),
                            );
                            knob_commands.extend(self.glass_popout_knob.show(
                                ui,
                                ui.id().with("popout_hdg"),
                                knob_rect,
                            ));
                        }
                    }
                });
            for setting in knob_commands {
                if let Err(e) = self.session.run(flightcraft_control::SetAvionics { setting }) {
                    self.error = Some(e.to_string());
                }
            }
            if !open {
                self.glass_controls.popout = None;
            }
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                if self.planning_map.open {
                    self.planning_map.options = self.instrument_panel.options;
                    self.planning_map.show(ui, ui.max_rect(), &mut self.session);
                    return;
                }
                if let Some(camera) = self.session.camera() {
                    let mode = match camera.mode {
                        flightcraft_control::CameraMode::Chase => CameraMode::Chase,
                        flightcraft_control::CameraMode::Cockpit => CameraMode::Cockpit,
                    };
                    if self.scene.camera != mode {
                        self.scene.camera = mode;
                        self.scene.reset_camera();
                    }
                    self.scene
                        .set_chase_offsets(camera.distance_m as f32, camera.height_m as f32);
                    self.scene
                        .set_cockpit_look(camera.look_yaw_rad as f32, camera.look_pitch_rad as f32);
                }
                self.scene.set_world(self.session.sim().world());
                self.scene
                    .set_weather(self.session.sim().conditions(), self.session.sim().hood());
                self.scene.airspace_visible = self.session.airspace_visible();
                self.scene.set_propeller_power(
                    (readout.effective_controls.throttle * readout.failures.engine_power_fraction) as f32,
                );
                self.scene
                    .set_aircraft(&self.session.sim().aircraft().identity.id);
                let layer = egui::LayerId::new(egui::Order::Background, egui::Id::new("outside-world"));
                let mut outside = egui::Ui::new(
                    ctx.clone(),
                    egui::Id::new("outside-world-ui"),
                    egui::UiBuilder::new()
                        .layer_id(layer)
                        .max_rect(ctx.content_rect()),
                );
                self.scene.show(
                    &mut outside,
                    self.session.sim().state(),
                    elapsed as f32,
                    theme::scene(),
                );
                ctx.set_sublayer(layer, egui::LayerId::background());
                let rect = ui.max_rect().shrink(24.0);
                let conditions = ui.painter().layout(
                    weather::note(&self.session),
                    FontId::monospace(12.),
                    theme::TEXT,
                    ui.max_rect().width() * 0.7,
                );
                let origin = ui.max_rect().center_top() + egui::vec2(-conditions.size().x * 0.5, 12.);
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(origin, conditions.size()).expand(5.),
                    3.,
                    theme::PANEL,
                );
                ui.painter().galley(origin, conditions, theme::TEXT);
                let painter = ui.painter();
                painter.text(
                    rect.left_top(),
                    Align2::LEFT_TOP,
                    self.session
                        .sim()
                        .world()
                        .region()
                        .map_or("TRAINING FIELD", |r| r.name.as_str()),
                    FontId::monospace(12.0),
                    theme::BACKGROUND,
                );
                painter.text(
                    rect.left_top() + egui::vec2(0.0, 20.0),
                    Align2::LEFT_TOP,
                    runway_label(self.session.sim().world(), self.session.sim().state().position_m),
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
                let hint = if self.scene.camera == CameraMode::Cockpit {
                    format!(
                        "HOLD TO LOOK   {} LEFT  ·  {} RIGHT  ·  {} UP",
                        self.keymap.label(Action::LookLeft),
                        self.keymap.label(Action::LookRight),
                        self.keymap.label(Action::LookUp)
                    )
                } else {
                    format!(
                        "PITCH {}  ·  ROLL {}  ·  THROTTLE {} / {}",
                        self.keymap.label(Action::ElevatorUp),
                        self.keymap.label(Action::AileronLeft),
                        self.keymap.label(Action::ThrottleUp),
                        self.keymap.label(Action::ThrottleDown)
                    )
                };
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
        #[cfg(not(target_arch = "wasm32"))]
        self.session.update_efb();
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(1.0 / 60.0));
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.session.autosave_flight();
        let callsign = self.session.pilot_callsign();
        storage.set_string(
            "flightcraft.callsign.v1",
            if callsign.overridden {
                serde_json::json!({"registration":callsign.registration,"spoken_type":callsign.spoken_type})
                    .to_string()
            } else {
                "{}".into()
            },
        );
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(serial) = self.efb_serial {
                storage.set_string("efb.serial", serial.to_string());
            }
            if let Ok(json) = serde_json::to_string(&self.session.efb_status().settings) {
                storage.set_string("efb.settings", json);
            }
        }
        if let Ok(json) = serde_json::to_string(&self.audio.preferences) {
            storage.set_string("flightcraft.audio.v1", json);
        }
        storage.set_string(COMPACT_PANEL_STORAGE, self.compact_panel.to_string());
        if let Ok(json) = serde_json::to_string(&self.instrument_panel.options) {
            storage.set_string(DISPLAY_OPTIONS_STORAGE, json);
        }
        #[cfg(not(target_arch = "wasm32"))]
        storage.set_string(PACKS_STORAGE, self.packs_directory.clone());
        if let Ok(json) = self.keymap.json() {
            storage.set_string(KEYMAP_STORAGE, json);
        }
    }
}

/// Open the optional native loopback channel before creating the window.
#[cfg(not(target_arch = "wasm32"))]
pub fn control_listener(port: u16) -> std::io::Result<flightcraft_control::bridge::Listener> {
    flightcraft_control::bridge::Listener::loopback(port)
}

/// The status and remote query use identical geometry and altitude semantics.
fn airspace_status(session: &flightcraft_control::Session) -> Option<(String, bool, String)> {
    use flightcraft_world::{AirspaceKind, AltitudeReference};
    let world = session.sim().world();
    let airspace = world.airspace()?;
    let [n, e, d] = session.sim().state().position_m;
    let query = world.airspace_at(n, e, -d).ok()?;
    let limit = |a: flightcraft_world::AirspaceAltitude| {
        if a.reference == AltitudeReference::SFC && a.value_ft == 0.0 {
            "SFC".to_owned()
        } else if a.reference == AltitudeReference::STD {
            format!("FL{:.0}", a.value_ft / 100.0)
        } else {
            format!("{:.0}", a.value_ft)
        }
    };
    let label = query
        .containing
        .iter()
        .find(|v| v.kind == AirspaceKind::Class)
        .map_or_else(
            || "Class G".to_owned(),
            |v| {
                let name = if matches!(v.class, Some(flightcraft_world::AirspaceClass::D)) {
                    format!(" {}", v.name.strip_suffix(" CLASS D").unwrap_or(&v.name))
                } else {
                    String::new()
                };
                let reference = match v.ceiling.reference {
                    AltitudeReference::MSL => " MSL",
                    AltitudeReference::SFC => " AGL",
                    AltitudeReference::STD => "",
                };
                format!(
                    "Class {:?}{name} {}–{}{reference}",
                    v.class.unwrap_or(flightcraft_world::AirspaceClass::E),
                    limit(v.floor),
                    limit(v.ceiling)
                )
            },
        );
    Some((
        label,
        query.nearest.is_some_and(|b| b.warning),
        airspace.attribution.clone(),
    ))
}

/// Identify the nearest threshold in a region; built-in worlds keep their original readout.
fn runway_label(world: &flightcraft_control::TerrainWorld, [n, e, _]: [f64; 3]) -> String {
    if let Some(catalogue) = world.airports() {
        let nearest = catalogue
            .airports
            .iter()
            .flat_map(|a| a.runways.iter().map(move |r| (a, r)))
            .zip(world.runway_strips())
            .flat_map(|((a, r), strip)| {
                strip.ends_m.iter().zip(&r.ends).map(move |([rn, re], end)| {
                    let distance = (n - rn) * (n - rn) + (e - re) * (e - re);
                    (distance, a, r, end)
                })
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, airport, runway, end)) = nearest {
            return format!(
                "NEAREST {} / {}   ·   {:.0} × {:.0} m",
                airport.ident, end.ident, runway.length_m, runway.width_m
            );
        }
    }
    let runway = world.runway();
    format!(
        "RUNWAY {:03.0}   /   {:.0} × {:.0} m",
        runway.heading_deg, runway.length_m, runway.width_m
    )
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod layout_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod panel_fidelity_tests;
#[cfg(test)]
mod test_capture;
#[cfg(test)]
mod weather_preview;

#[cfg(test)]
mod navigator_audio_tests;
