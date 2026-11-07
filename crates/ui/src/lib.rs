//! The shared eframe application for desktop and browser.
pub mod actions;
mod audio;
pub mod autopilot;
#[cfg(not(target_arch = "wasm32"))]
mod bridge;
mod callsign_preferences;
mod canopy_panel;
mod checklist;
pub mod clock;
pub mod crew;
#[cfg(test)]
mod crew_preview;
mod display_preferences;
mod efis;
pub mod fly_nyc;
pub use fly_nyc::FLY_NYC_TILESET;
pub mod glass;
mod ifr;
#[cfg(test)]
mod ifr_preview;
mod input;
pub mod instruments;
mod key_actions;
pub mod keymap;
pub mod map;
mod modern;
mod navigation;
mod panel_art;
pub mod panel_preview;
mod procedures;
pub mod radio;
mod radio_speech;
#[cfg(not(target_arch = "wasm32"))]
mod runtime;
mod scenario;
pub mod screens;
pub mod script;
pub mod speech_input;
pub mod theme;
pub mod traffic_page;
mod view_preferences;
mod weather;
mod weather_live;

use eframe::egui::{self, RichText};
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
    pub layout: Option<flightcraft_control::screens::Layout>,
    #[cfg(not(target_arch = "wasm32"))]
    pub screen_set: Option<std::path::PathBuf>,
    #[cfg(not(target_arch = "wasm32"))]
    pub layouts_dir: Option<std::path::PathBuf>,
    #[cfg(not(target_arch = "wasm32"))]
    pub panels: Option<(std::net::SocketAddr, std::path::PathBuf)>,
    #[cfg(not(target_arch = "wasm32"))]
    pub screenshot: Option<Screenshot>,
    #[cfg(not(target_arch = "wasm32"))]
    pub control: Option<flightcraft_control::bridge::Listener>,
    /// Timed commands to run; they hold the flight controls until they run out.
    pub script: Option<script::Script>,
    pub packs: flightcraft_control::PackOptions,
    pub regions: flightcraft_control::RegionOptions,
}

/// The main-window UI, presenting the session owner’s snapshots and sending it commands.
pub struct FlightCraftApp {
    session: Session,
    efb_settings: flightcraft_control::EfbSettings,
    efb_applied: flightcraft_control::EfbSettings,
    #[cfg(not(target_arch = "wasm32"))]
    efb_serial: Option<u64>,
    #[cfg(not(target_arch = "wasm32"))]
    packs_directory: String,
    #[cfg(not(target_arch = "wasm32"))]
    runtime: Option<runtime::Runtime>,
    screens: screens::ScreensUi,
    scene: Scene,
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
    procedure_controls: procedures::Controls,
    bindings_json: String,
    airport_search: String,
    planning_map: map::PlanningMap,
    navigation: navigation::SearchPanel,
    glance_origin: Option<[f64; 2]>,
    audio: audio::Audio,
    radio: radio::RadioUi,
    live_weather: weather_live::LiveUi,
    scenarios: scenario::ScenarioUi,
    fly_me: crew::FlyMeWindow,
    radio_ptt_held: [bool; 2],
    binding_message: Option<String>,
    scripted: bool,
    external: bool,
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
    #[cfg(not(target_arch = "wasm32"))]
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
        let mut error = None;
        // The app opens flying in real time; headless sessions start paused.
        let mut session = Session::new()?;
        if let Err(e) = callsign_preferences::restore(&mut session, cc.storage) {
            error = Some(e);
        }
        if let Err(e) = display_preferences::restore(&mut session, cc.storage) {
            error = Some(e);
        }
        if let Err(e) = view_preferences::restore(&mut session, cc.storage) {
            error = Some(e);
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
        // Skin packs sit beside the aircraft packs. FLIGHTCRAFT_SKINS_DIR adds a local directory
        // (for example private artwork outside any repository), loaded last so its ids win.
        #[cfg(not(target_arch = "wasm32"))]
        {
            pack_errors.extend(
                session.load_skins_directory(&std::path::Path::new(&packs_directory).with_file_name("skins")),
            );
            if let Some(extra) = std::env::var_os("FLIGHTCRAFT_SKINS_DIR") {
                pack_errors.extend(session.load_skins_directory(std::path::Path::new(&extra)));
            }
        }
        for skin in options.packs.skins {
            if let Err(e) = session.install_skin(skin) {
                pack_errors.push(format!("skin: {e}"));
            }
        }
        if !pack_errors.is_empty() {
            if let Some(previous) = error.take() {
                pack_errors.insert(0, previous);
            }
            error = Some(pack_errors.join("\n"));
        }
        #[cfg(not(target_arch = "wasm32"))]
        if session
            .run(flightcraft_control::ListAircraft {})
            .is_ok_and(|list| {
                list.aircraft
                    .iter()
                    .any(|a| a.id == "sling2" && a.panel == "sling2-ifr")
            })
        {
            session.execute(
                "aircraft.load",
                serde_json::json!({"source":{"kind":"installed","id":"sling2"},"reset":true}),
            )?;
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
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(saved) = cc.storage.and_then(|s| s.get_string("flightcraft.screens.v1")) {
            match serde_json::from_str(&saved)
                .map_err(|e| e.to_string())
                .and_then(|set| session.restore_screens(set))
            {
                Ok(()) => (),
                Err(e) => error = Some(e),
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(path) = options.screen_set {
            if std::fs::metadata(&path)?.len() > 1024 * 1024 {
                return Err(std::io::Error::other("screen set exceeds 1 MiB").into());
            }
            let data = std::fs::read_to_string(path)?;
            if data.len() > 1024 * 1024 {
                return Err(std::io::Error::other("screen set exceeds 1 MiB").into());
            }
            session
                .restore_screens(serde_json::from_str(&data)?)
                .map_err(std::io::Error::other)?;
        }
        if let Some(layout) = options.layout {
            session.execute("layout.set", serde_json::json!({"id":0,"layout":layout}))?;
        }
        #[cfg(not(target_arch = "wasm32"))]
        let runtime = runtime::Runtime::start(
            session,
            options.script.unwrap_or_default(),
            bridge::Bridge::new(options.control),
            cc.egui_ctx.clone(),
        )?;
        #[cfg(not(target_arch = "wasm32"))]
        let session = runtime.snapshot().map_err(std::io::Error::other)?;
        #[cfg(target_arch = "wasm32")]
        let screens = screens::ScreensUi::with_chart_root(chart_root.clone());
        #[cfg(not(target_arch = "wasm32"))]
        let mut screens = screens::ScreensUi::with_chart_root(chart_root.clone());
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some((address, root)) = options.panels {
                screens.remote = Some(
                    screens::remote::Server::start(address, root, runtime.source())
                        .map_err(std::io::Error::other)?,
                );
            }
            if let Some(root) = options
                .layouts_dir
                .or_else(|| flightcraft_control::navigation_data_directory().map(|root| root.join("layouts")))
            {
                screens.watcher = Some(screens::watch::Folder::new(root).map_err(std::io::Error::other)?);
            }
        }
        let mut app = Self::from_presentation(session, cc.storage, renderer, chart_root)?;
        app.error = error.or(app.error);
        app.efb_serial = efb_serial;
        app.packs_directory = packs_directory;
        app.runtime = Some(runtime);
        screens.main.scenery_preferences = app.screens.main.scenery_preferences.clone();
        app.screens = screens;
        app.screenshot = options.screenshot;
        Ok(app)
    }

    /// Shared main-window presentation. Its session routes commands to an external owner.
    pub(crate) fn from_presentation(
        session: Session,
        storage: Option<&dyn eframe::Storage>,
        renderer: &'static str,
        chart_root: Option<String>,
    ) -> Result<Self, String> {
        let stored = storage.and_then(|s| s.get_string(KEYMAP_STORAGE));
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
        let bindings_json = keymap.json().map_err(|e| e.to_string())?;
        let paused = session.paused();
        let efb_applied = session.efb_status().settings;
        let efb_settings = storage
            .and_then(|s| s.get_string("efb.settings"))
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Ok(Self {
            session,
            efb_applied,
            efb_settings,
            #[cfg(not(target_arch = "wasm32"))]
            efb_serial: None,
            #[cfg(not(target_arch = "wasm32"))]
            packs_directory: "packs".into(),
            #[cfg(not(target_arch = "wasm32"))]
            runtime: None,
            screens: {
                let mut screens = screens::ScreensUi::with_chart_root(chart_root.clone());
                screens.main.scenery_preferences = storage
                    .and_then(|s| s.get_string("flightcraft.scenery.v1"))
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default();
                screens
            },
            scene: Scene::default(),
            input: input::Input::default(),
            keymap,
            paused,
            last_time: None,
            error,
            renderer,
            settings: false,
            compact_panel: storage
                .and_then(|s| s.get_string(COMPACT_PANEL_STORAGE))
                .as_deref()
                == Some("true"),
            procedure_controls: procedures::Controls::default(),
            instrument_panel: instruments::InstrumentPanel::with_options(
                storage
                    .and_then(|s| s.get_string(DISPLAY_OPTIONS_STORAGE))
                    .and_then(|json| serde_json::from_str(&json).ok())
                    .unwrap_or_default(),
            ),
            bindings_json,
            airport_search: String::new(),
            planning_map: map::PlanningMap::with_root(chart_root.clone()),
            navigation: navigation::SearchPanel::default(),
            glance_origin: None,
            radio: radio::RadioUi::with_speech_preferences(
                storage
                    .and_then(|s| s.get_string("flightcraft.speech.v1"))
                    .and_then(|json| serde_json::from_str(&json).ok())
                    .unwrap_or_default(),
            ),
            live_weather: weather_live::LiveUi::default(),
            scenarios: scenario::ScenarioUi::default(),
            fly_me: crew::FlyMeWindow::default(),
            radio_ptt_held: [false; 2],
            audio: audio::Audio::with_preferences(
                storage
                    .and_then(|s| s.get_string("flightcraft.audio.v1"))
                    .and_then(|json| serde_json::from_str(&json).ok())
                    .unwrap_or_default(),
            ),
            binding_message: None,
            scripted: false,
            external: false,
            #[cfg(not(target_arch = "wasm32"))]
            screenshot: None,
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
                self.input.trim = 0.0;
                self.scene.reset_camera();
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// Each installed touch display's page, the pilot's seat, and display backup. Choices are saved
    /// for the aircraft variant; they use the same `avionics.set` settings a script would.
    fn display_settings(session: &mut Session, error: &mut Option<String>, ui: &mut egui::Ui) {
        use flightcraft_aero::modern::{DataArea, PilotSeat, TouchLayout};
        use flightcraft_control::GlassSetting;
        let touch = session.touch_state();
        if touch.displays.is_empty() {
            return;
        }
        let mut changes = Vec::new();
        let mut equipment = Vec::new();
        ui.separator();
        ui.label(RichText::new("COCKPIT DISPLAYS").color(theme::ACCENT).size(11.0));
        if touch.displays.len() == 2 {
            ui.horizontal(|ui| {
                ui.label("Pilot seat");
                for (seat, label) in [(PilotSeat::Left, "Left"), (PilotSeat::Right, "Right")] {
                    if ui.selectable_label(touch.pilot_seat == seat, label).clicked()
                        && touch.pilot_seat != seat
                    {
                        changes.push(GlassSetting::PilotSeat { seat });
                    }
                }
            });
        }
        for display in &touch.displays {
            ui.horizontal(|ui| {
                let name = display.id.replace('_', " ");
                ui.label(RichText::new(name[..1].to_uppercase() + &name[1..]));
                egui::ComboBox::from_id_salt(("display page", &display.id))
                    .selected_text(display.layout.label())
                    .show_ui(ui, |ui| {
                        for page in TouchLayout::pages(display.installed_layout) {
                            if ui
                                .selectable_label(display.layout == page, page.label())
                                .clicked()
                            {
                                changes.push(GlassSetting::DisplayPage {
                                    display: display.id.clone(),
                                    page,
                                });
                            }
                        }
                    });
                if let Some(model) = display.model {
                    let current = display.skin.clone();
                    egui::ComboBox::from_id_salt(("display skin", &display.id))
                        .selected_text(
                            current
                                .as_deref()
                                .and_then(|id| session.skin(id))
                                .map_or("Built-in skin", |s| s.manifest.display_name.as_str()),
                        )
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(current.is_none(), "Built-in skin").clicked() {
                                equipment.push(flightcraft_control::EquipmentSetting::Display {
                                    id: display.id.clone(),
                                    action: flightcraft_control::DisplayAction::Skin {
                                        skin: Some(flightcraft_control::BUILT_IN_SKIN.into()),
                                    },
                                });
                            }
                            for skin in session.skins_for(model) {
                                let id = &skin.manifest.id;
                                if ui
                                    .selectable_label(
                                        current.as_deref() == Some(id),
                                        &skin.manifest.display_name,
                                    )
                                    .clicked()
                                {
                                    equipment.push(flightcraft_control::EquipmentSetting::Display {
                                        id: display.id.clone(),
                                        action: flightcraft_control::DisplayAction::Skin {
                                            skin: Some(id.clone()),
                                        },
                                    });
                                }
                            }
                            if display.recommended_skin.is_some()
                                && ui
                                    .selectable_label(false, "Follow the aircraft's recommendation")
                                    .clicked()
                            {
                                equipment.push(flightcraft_control::EquipmentSetting::Display {
                                    id: display.id.clone(),
                                    action: flightcraft_control::DisplayAction::Skin { skin: None },
                                });
                            }
                        });
                    let mut knobs = display.knobs;
                    let mut changed = ui
                        .checkbox(&mut knobs.press_to_toggle, "Push knob toggles functions")
                        .changed();
                    egui::ComboBox::from_id_salt(("display knobs", &display.id))
                        .selected_text(match knobs.alternate {
                            flightcraft_control::touch_efis::AlternateKnobs::CourseBaro => {
                                "Right knob: course / baro"
                            }
                            flightcraft_control::touch_efis::AlternateKnobs::FdBugBaro => {
                                "Right knob: FD bug / baro"
                            }
                            flightcraft_control::touch_efis::AlternateKnobs::HeadingAltitude => {
                                "Right knob: heading / altitude"
                            }
                        })
                        .show_ui(ui, |ui| {
                            use flightcraft_control::touch_efis::AlternateKnobs as A;
                            for (value, label) in [
                                (A::CourseBaro, "Course / baro"),
                                (A::FdBugBaro, "FD bug / baro"),
                                (A::HeadingAltitude, "Heading / altitude"),
                            ] {
                                if ui.selectable_label(knobs.alternate == value, label).clicked() {
                                    knobs.alternate = value;
                                    changed = true;
                                }
                            }
                        });
                    if changed {
                        equipment.push(flightcraft_control::EquipmentSetting::Display {
                            id: display.id.clone(),
                            action: flightcraft_control::DisplayAction::Knobs { config: knobs },
                        });
                    }
                }
                if display.layout == TouchLayout::Mfd {
                    egui::ComboBox::from_id_salt(("display data", &display.id))
                        .selected_text(display.data_area.label())
                        .show_ui(ui, |ui| {
                            for area in DataArea::ALL {
                                if ui
                                    .selectable_label(display.data_area == area, area.label())
                                    .clicked()
                                {
                                    changes.push(GlassSetting::DisplayData {
                                        display: display.id.clone(),
                                        area,
                                    });
                                }
                            }
                        });
                }
            });
        }
        let mut backup = touch.manual_reversion;
        if touch.displays.len() > 1
            && ui
                .checkbox(
                    &mut backup,
                    "Display backup (multi-function display shows the PFD with engine strip)",
                )
                .changed()
        {
            changes.push(GlassSetting::DisplayReversion { enabled: backup });
        }
        for setting in equipment {
            if let Err(e) = session.run(flightcraft_control::SetEquipment { setting }) {
                *error = Some(e.to_string());
            }
        }
        for setting in changes {
            if let Err(e) = session.run(flightcraft_control::SetAvionics {
                setting: flightcraft_sim::avionics::AvionicsSetting::Glass { setting },
            }) {
                *error = Some(e.to_string());
            }
        }
    }

    fn bindings_window(&mut self, ctx: &egui::Context) {
        let applied = self.session.efb_status().settings;
        if applied != self.efb_applied {
            self.efb_settings = applied.clone();
            self.efb_applied = applied;
        }
        egui::Window::new("Preferences and input").constrain_to(theme::window_bounds(ctx)).open(&mut self.settings).default_width(530.0).max_height(620.0).vscroll(true).show(ctx, |ui| {
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
            Self::display_settings(&mut self.session, &mut self.error, ui);
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
            ui.separator();
            ui.label(RichText::new("SCENERY").color(theme::ACCENT).size(11.0));
            let preferences = &mut self.screens.main.scenery_preferences;
            ui.horizontal(|ui| {
                ui.label("Quality");
                ui.selectable_value(&mut preferences.quality, flightcraft_render::scenery::Quality::Low, "Low");
                ui.selectable_value(&mut preferences.quality, flightcraft_render::scenery::Quality::Medium, "Medium");
                if !cfg!(target_arch = "wasm32") { ui.selectable_value(&mut preferences.quality, flightcraft_render::scenery::Quality::High, "High"); }
            });
            let mut dated=preferences.lighting.is_some();
            if ui.checkbox(&mut dated,"Set sun date and UTC time").changed() {preferences.lighting=if dated {Some(Default::default())}else{None};}
            if let Some(settings)=&mut preferences.lighting {
                ui.horizontal(|ui| {
                    ui.label("Date (year, month, day)");
                    ui.add(egui::DragValue::new(&mut settings.year).range(1900..=2100));
                    ui.add(egui::DragValue::new(&mut settings.month).range(1..=12));
                    ui.add(egui::DragValue::new(&mut settings.day).range(1..=31));
                });
                ui.horizontal(|ui| {
                    ui.label("UTC at flight start");
                    let mut hour=settings.utc_hour.floor() as u8;let mut minute=((settings.utc_hour-f64::from(hour))*60.).round() as u8;
                    ui.add(egui::DragValue::new(&mut hour).range(0..=23));ui.label(":");ui.add(egui::DragValue::new(&mut minute).range(0..=59));
                    settings.utc_hour=f64::from(hour)+f64::from(minute)/60.;
                });
                ui.add(egui::Slider::new(&mut settings.exposure_ev,-3.0..=3.0).text("Exposure (EV)"));
                if settings.validate().is_err() {ui.colored_label(theme::AMBER,"Choose a valid calendar date.");}
            }
            ui.checkbox(&mut preferences.enabled, "Show 3D scenery");
            if !cfg!(target_arch = "wasm32") {
                ui.label("Tileset file (empty: use the scenery installed for this region)");
                ui.text_edit_singleline(&mut preferences.local_tileset);
                ui.separator();
                ui.checkbox(&mut preferences.streaming_imagery, "Streaming imagery (your own key)");
                ui.label("ArcGIS API key");
                ui.add(egui::TextEdit::singleline(&mut preferences.esri_key).password(true));
                ui.label("Stored only in your user preferences. Streaming uses your ArcGIS allowance.");
                ui.label("Memory cache only. Disable pay-as-you-go in your account to avoid charges.");
                ui.hyperlink_to("Esri attribution and terms", "https://www.esri.com/en-us/legal/terms/full-master-agreement");
            }
            if ui.button("Apply scenery").clicked() { self.screens.main.reload_scenery(); }
            ui.separator();
            ui.label(RichText::new("SOUND").color(theme::ACCENT).size(11.0));
            ui.checkbox(&mut self.audio.preferences.muted,"Mute sound");
            ui.add(egui::Slider::new(&mut self.audio.preferences.volume,0.0..=1.0).text("Master volume"));
            self.radio.input.preferences_ui(ui);
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

impl FlightCraftApp {
    /// Checklists, procedure lessons, the ground layout, and the exterior inset.
    fn procedures_menu(&mut self, ui: &mut egui::Ui) {
        use flightcraft_control::{InsetMode, LayoutMode};
        ui.menu_button("Procedures and view", |ui| {
            let lists = self
                .session
                .execute("checklist.list", serde_json::json!({}))
                .ok()
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default();
            ui.menu_button("Checklists", |ui| {
                if lists.is_empty() {
                    ui.label("This aircraft has no checklists.");
                }
                for list in &lists {
                    let id = list["id"].as_str().unwrap_or_default();
                    if ui
                        .button(list["title"].as_str().unwrap_or(id))
                        .on_hover_text(list["poh_ref"].as_str().unwrap_or_default())
                        .clicked()
                    {
                        let result = self
                            .session
                            .execute("checklist.select", serde_json::json!({"id": id}))
                            .and_then(|_| {
                                self.session
                                    .execute("view.layout", serde_json::json!({"mode":"ground"}))
                            });
                        if let Err(e) = result {
                            self.error = Some(e.to_string());
                        }
                        ui.close();
                    }
                }
            });
            ui.menu_button("Procedure lessons", |ui| {
                let lessons = self
                    .session
                    .execute("procedure_lesson.list", serde_json::json!({}))
                    .ok()
                    .and_then(|v| v.as_array().cloned())
                    .unwrap_or_default();
                for lesson in &lessons {
                    let id = lesson["id"].as_str().unwrap_or_default();
                    if ui
                        .button(lesson["title"].as_str().unwrap_or(id))
                        .on_hover_text(lesson["briefing"].as_str().unwrap_or_default())
                        .clicked()
                    {
                        if let Err(e) = self
                            .session
                            .execute("procedure_lesson.start", serde_json::json!({"id": id}))
                        {
                            self.error = Some(e.to_string());
                        }
                        ui.close();
                    }
                }
            });
            ui.separator();
            let prefs = *self.session.view_preferences();
            for (mode, label) in [
                (LayoutMode::Auto, "Layout: ground view with a checklist"),
                (LayoutMode::Ground, "Layout: ground view now"),
                (LayoutMode::Normal, "Layout: normal"),
            ] {
                if ui.selectable_label(prefs.layout == mode, label).clicked() {
                    if let Err(e) = self
                        .session
                        .execute("view.layout", serde_json::json!({"mode": mode}))
                    {
                        self.error = Some(e.to_string());
                    }
                    ui.close();
                }
            }
            ui.separator();
            for (mode, label) in [
                (InsetMode::Off, "Exterior inset: off"),
                (InsetMode::Ground, "Exterior inset: on the ground"),
                (InsetMode::Always, "Exterior inset: always"),
            ] {
                if ui.selectable_label(prefs.exterior_inset == mode, label).clicked() {
                    if let Err(e) = self
                        .session
                        .execute("view.exterior_inset", serde_json::json!({"mode": mode}))
                    {
                        self.error = Some(e.to_string());
                    }
                    ui.close();
                }
            }
        });
    }

    fn flight_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Flight", |ui| {
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
            if self.session.sim().world().region().is_some() {
                ui.menu_button("Start", |ui| {
                    ui.label("Airport or name");
                    ui.text_edit_singleline(&mut self.airport_search);
                    match self
                        .session
                        .run(flightcraft_control::AirportQuery { nearest_to_m: None })
                    {
                        Ok(list) => {
                            let search = self.airport_search.to_lowercase();
                            egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                                for airport in list.airports.iter().filter(|a| {
                                    a.ident.to_lowercase().contains(&search)
                                        || a.name.to_lowercase().contains(&search)
                                        || a.icao
                                            .as_deref()
                                            .is_some_and(|s| s.to_lowercase().contains(&search))
                                }) {
                                    ui.menu_button(format!("{} · {}", airport.ident, airport.name), |ui| {
                                        for runway in &airport.runways {
                                            for end in &runway.ends {
                                                if ui
                                                    .button(format!(
                                                        "Runway {} · {:.0} × {:.0} m",
                                                        end.ident, runway.length_m, runway.width_m
                                                    ))
                                                    .clicked()
                                                {
                                                    match self.session.run(flightcraft_control::Reset {
                                                        world: self.session.sim().world().choice(),
                                                        start:
                                                            flightcraft_control::ResetStart::AirportRunway {
                                                                airport: airport.ident.clone(),
                                                                end: end.ident.clone(),
                                                            },
                                                        weather: Some(self.session.pilot_weather()),
                                                        paused: Some(self.paused),
                                                    }) {
                                                        Ok(_) => {
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
            {
                let camera = match self.scene.camera {
                    CameraMode::Chase => "Chase",
                    CameraMode::Cockpit => "Cockpit",
                    CameraMode::Wing => "Wing",
                    CameraMode::Tower => "Tower",
                    CameraMode::Flyby => "Flyby",
                    CameraMode::Walkaround => "Walkaround",
                };
                if ui
                    .button(format!("{camera}  [{}]", self.keymap.label(Action::Camera)))
                    .clicked()
                    && let Err(e) = actions::toggle_camera(&mut self.session)
                {
                    self.error = Some(e.to_string());
                }
                self.procedures_menu(ui);
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
            }
        });
    }

    /// One shell for both hosts. Automatic axes retain keyboard provenance in the Worker transport,
    /// so scripts and external levers keep ownership while menus use the regular command queue.
    pub(crate) fn show_main(
        &mut self,
        ctx: &egui::Context,
        mut keyboard_controls: impl FnMut(
            &mut Session,
            flightcraft_sim::Controls,
        ) -> Result<(), flightcraft_control::CommandError>,
    ) {
        if ctx.input(|input| {
            input.pointer.any_pressed()
                || input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::Key { pressed: true, .. }))
        }) {
            self.audio.user_gesture();
            self.radio.input.user_gesture();
        }
        self.paused = self.session.paused();
        if let Some(camera) = self.session.camera() {
            self.scene.camera = crew::render_camera(camera.mode);
        }
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
        if frame.pressed(Action::CrewHandover) {
            crew::exchange(&mut self.session, &mut self.error);
        }
        let held = [
            frame.held(Action::ComPtt) || self.radio.touch_ptt[0],
            frame.held(Action::IntercomPtt) || self.radio.touch_ptt[1],
        ];
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
        if frame.pressed(Action::StandbyDirectTo) && self.session.sim().systems().avionics.installed {
            self.instrument_panel.avionics_controls.open_direct_to();
        }
        key_actions::apply_session_actions(
            &mut self.session,
            &frame,
            self.instrument_panel.options,
            &mut self.error,
        );
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
        let crew_controls = self.session.crew_owns_controls();
        let controlled = if self.scripted || self.external || crew_controls || self.session.holds_active() {
            Ok(())
        } else {
            self.input.trim = self.session.sim().systems().controls.trim;
            self.input.flap_stages = self.session.aircraft_configuration().map_or_else(Vec::new, |c| {
                c.variant.flap_stages.iter().map(|s| s.position).collect()
            });
            self.input.panel_brake = ctx.data(|d| {
                d.get_temp::<bool>(egui::Id::new(modern::PANEL_BRAKE_HELD))
                    .unwrap_or(false)
            });
            if frame.pressed(Action::ParkBrake) {
                key_actions::toggle_park_brake(&mut self.session, &mut self.error);
            }
            let controls = self
                .input
                .controls(&frame, self.session.sim().controls(), elapsed);
            let trimmed = if self.input.trim != self.session.sim().systems().controls.trim {
                self.session
                    .run(flightcraft_control::SetSystems {
                        setting: flightcraft_sim::systems::SystemSetting::Trim {
                            value: self.input.trim,
                        },
                    })
                    .map(|_| ())
            } else {
                Ok(())
            };
            trimmed.and_then(|()| keyboard_controls(&mut self.session, controls))
        };
        if let Err(e) = controlled {
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
        let readout = self.session.sim().readout();
        self.audio.observe_door_noise(
            readout.failures.door_open,
            readout.true_airspeed_mps,
            self.session.paused(),
        );
        self.scenarios.show(ctx, &mut self.session);
        self.fly_me.show(ctx, &mut self.session, &self.keymap);
        self.live_weather.show(ctx, &mut self.session);
        let f = scenario::emergency_failures(&self.session);
        if f.engine_fire
            || f.electrical_fire
            || f.cabin_smoke
            || f.door_open
            || f.carb_icing
            || f.airframe_icing
            || f.flap_asymmetry
            || f.trim_runaway
            || f.trim_stuck
            || f.radio_failure
            || f.pitot_blocked
            || f.static_blocked
            || f.vacuum_pump
            || f.alternator
        {
            egui::TopBottomPanel::bottom("emergency-annunciations")
                .frame(egui::Frame::new().fill(theme::PANEL))
                .show(ctx, |ui| {
                    ui.horizontal_wrapped(|ui| scenario::indications(ui, &self.session));
                });
        }
        let header = egui::TopBottomPanel::top("header")
            .min_height(50.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::BACKGROUND)
                    .inner_margin(egui::Margin::symmetric(22, 10)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
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
                    if ui
                        .button(format!("Map [{}]", self.keymap.label(Action::PlanningMap)))
                        .clicked()
                    {
                        self.planning_map.open = !self.planning_map.open;
                    }
                    self.navigation.show(ui, &mut self.session);
                    ui.menu_button("Weather", |ui| {
                        if ui.button("Live…").clicked() {
                            self.live_weather.open = true;
                            ui.close();
                        }
                        if let Err(e) = weather::panel(ui, &mut self.session, &self.keymap) {
                            self.error = Some(e.to_string());
                        }
                    });
                    self.radio.button(ui, &self.keymap);
                    crate::crew::page_button(
                        ui,
                        &mut self.session,
                        &self.keymap,
                        &mut self.error,
                        &mut self.fly_me,
                    );
                    if ui.button("Scenarios").clicked() {
                        self.scenarios.open = !self.scenarios.open;
                    }
                    if scenario::emergency_active(&self.session) {
                        ui.menu_button(egui::RichText::new("Emergency").color(theme::DANGER), |ui| {
                            scenario::indications(ui, &self.session);
                            if checklist::active_emergency(&self.session).is_some() {
                                let shown = checklist::emergency_panel_shown(ui.ctx(), &self.session);
                                if ui
                                    .button(if shown {
                                        "Hide emergency procedure"
                                    } else {
                                        "Show emergency procedure"
                                    })
                                    .clicked()
                                {
                                    checklist::set_emergency_panel(ui.ctx(), !shown);
                                    ui.close();
                                }
                            }
                        });
                    }
                    for setting in self.procedure_controls.show(ui, &self.session) {
                        if let Err(error) = self.session.run(flightcraft_control::SetAvionics { setting }) {
                            self.error = Some(error.to_string());
                        }
                    }
                    self.flight_menu(ui);
                    ui.menu_button("Settings", |ui| {
                        self.screens.menu(ui, &mut self.session);
                        if ui.button("Preferences").clicked() {
                            self.settings = !self.settings;
                            ui.close();
                        }
                    });
                    if let Some((label, warning, attribution)) = airspace_status(&self.session) {
                        ui.separator();
                        ui.add(
                            egui::Label::new(RichText::new(&label).size(11.0).color(if warning {
                                theme::AMBER
                            } else {
                                theme::TEXT
                            }))
                            .truncate(),
                        )
                        .on_hover_text(format!("{label}\n{attribution}"));
                    }
                });
            });
        theme::record_header_bottom(ctx, header.response.rect.bottom());
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
            self.screens.main.cancel_equipment_holds(&mut self.session);
            for setting in self
                .instrument_panel
                .cancel_equipment_holds(self.session.flight_generation())
            {
                if let Err(e) = self.session.run(flightcraft_control::SetEquipment { setting }) {
                    self.error = Some(e.to_string());
                }
            }
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BACKGROUND))
            .show(ctx, |ui| {
                if self.planning_map.open {
                    self.planning_map.show(ui, ui.max_rect(), &mut self.session);
                } else {
                    self.screens.root(ui, &mut self.session);
                }
            });
        if let Some(error) = &self.error {
            egui::Window::new("FlightCraft message")
                .constrain_to(theme::window_bounds(ctx))
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label(RichText::new(error).color(theme::AMBER));
                });
        }
        self.bindings_window(ctx);
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(1.0 / 60.0));
    }
}

impl eframe::App for FlightCraftApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(runtime) = &self.runtime {
            match runtime.snapshot() {
                Ok(s) => self.session = s,
                Err(e) => self.error = Some(e),
            }
            if let Some(e) = runtime.error() {
                self.error = Some(e);
            }
            self.scripted = runtime.script_active();
            self.external = runtime.external_active();
        }
        self.show_main(ctx, actions::set_controls);
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(runtime) = &self.runtime {
                self.screens.viewports(ctx, runtime, &mut self.session);
            }
            self.screenshot(ctx);
            self.session.update_efb();
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        #[cfg(not(target_arch = "wasm32"))]
        self.session.autosave_flight();
        if let Ok(json) = serde_json::to_string(self.session.screens()) {
            storage.set_string("flightcraft.screens.v1", json);
        }
        callsign_preferences::save(&self.session, storage);
        display_preferences::save(&self.session, storage);
        view_preferences::save(&self.session, storage);
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
        if let Ok(json) = serde_json::to_string(&self.screens.main.scenery_preferences) {
            storage.set_string("flightcraft.scenery.v1", json);
        }
        if let Ok(json) = serde_json::to_string(&self.radio.input.preferences) {
            storage.set_string("flightcraft.speech.v1", json);
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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod cockpit_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod console_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod ground_layout_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod layout_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod panel_fidelity_tests;
#[cfg(test)]
mod test_capture;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod weather_live_preview;
#[cfg(test)]
mod weather_preview;

#[cfg(test)]
mod efis_g3x_tests;
#[cfg(test)]
mod efis_pfd_tests;
#[cfg(test)]
mod efis_tests;
#[cfg(test)]
mod navigator_audio_tests;
#[cfg(test)]
mod scenario_preview;

#[cfg(test)]
mod main_ui_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod cockpit_preview;
#[cfg(test)]
mod exterior_preview;
#[cfg(test)]
mod walkaround_clearance;
#[cfg(test)]
mod walkaround_preview;

/// Schema-only re-export for the repository schema generator.
pub type SceneryProviderSchema = flightcraft_render::scenery::Provider;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod fly_nyc_preview;
#[cfg(not(target_arch = "wasm32"))]
pub mod installed_scenery;
#[cfg(test)]
mod scenery_preview;
