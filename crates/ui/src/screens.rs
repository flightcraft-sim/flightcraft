//! One canvas and camera path for native windows, browser Workers, and paired panels.
mod drag;
mod picture;
#[cfg(not(target_arch = "wasm32"))]
pub mod remote;
pub mod touch;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod watch;
#[cfg(target_arch = "wasm32")]
pub mod web;
use crate::{glass, instruments, map, navigation, radio, scenario, theme};
use eframe::egui::{self, Align2, FontId, Rect, Sense, Stroke};
use flightcraft_control::{Session, screens::*};
use serde_json::json;
use std::collections::BTreeMap;

/// Seconds for the inset camera to ease to a new framing.
const INSET_EASE_S: f32 = 0.45;

/// The elements that are the small exterior view rather than the main outside view.
fn is_inset(id: &str) -> bool {
    id == flightcraft_control::screens::GROUND_EXTERIOR_ID || id == flightcraft_control::screens::INSET_ID
}

#[derive(Default)]
pub struct PageUi {
    crew: crate::crew::CrewUi,
    picture: picture::Picture,
    scene: flightcraft_render::Scene,
    camera_time: Option<f64>,
    camera_generation: Option<u64>,
    panel: instruments::InstrumentPanel,
    map: map::PlanningMap,
    traffic: crate::traffic_page::TrafficPage,
    instructor: scenario::ScenarioUi,
    instrument: crate::ifr::InstrumentUi,
    navigation: navigation::SearchPanel,
    levers: BTreeMap<String, touch::Lever>,
    knobs: BTreeMap<String, touch::Knob>,
    airport: String,
    tune1: String,
    tune2: String,
    canopy_message: String,
    canopy_drag_origin: f64,
    canopy_drag_target: Option<(egui::Pos2, egui::Vec2)>,
    canopy_reveal: crate::canopy_panel::Reveal,
    checklist: crate::checklist::ChecklistUi,
    inset_frame: Option<flightcraft_aero::exterior_framing::Framing>,
}
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneryPreferences {
    pub quality: flightcraft_render::scenery::Quality,
    #[serde(default)]
    pub lighting: Option<flightcraft_render::lighting::LightingSettings>,
    pub enabled: bool,
    pub local_tileset: String,
    #[serde(default)]
    pub streaming_imagery: bool,
    /// Serialized only by eframe's user-preference store. Never part of a command or recording.
    #[serde(default)]
    pub esri_key: String,
}
#[derive(Default)]
pub struct Canvas {
    pub scenery_preferences: SceneryPreferences,
    scenery_attempt: Option<u64>,
    #[cfg(target_arch = "wasm32")]
    browser_pending: Option<std::sync::mpsc::Receiver<Result<flightcraft_render::scenery::Stream, String>>>,
    scenery: Option<std::sync::Arc<std::sync::Mutex<flightcraft_render::scenery::Stream>>>,
    imagery: Option<std::sync::Arc<std::sync::Mutex<flightcraft_render::scenery::Stream>>>,
    pages: BTreeMap<String, PageUi>,
    drag: Option<Element>,
    drag_delta: egui::Vec2,
    transfer: std::sync::Arc<std::sync::Mutex<drag::Transfer>>,
    chart_root: Option<String>,
    pub edit: bool,
    pub error: Option<String>,
}
fn execute(s: &mut Session, name: &str, input: serde_json::Value, error: &mut Option<String>) {
    if let Err(e) = s.execute(name, input) {
        *error = Some(e.to_string());
    }
}
impl Canvas {
    pub fn reload_scenery(&mut self) {
        self.scenery_attempt = None;
        #[cfg(target_arch = "wasm32")]
        {
            self.browser_pending = None;
        }
    }
    /// Release push-to-hold equipment before a planning map hides the panel.
    pub(crate) fn cancel_equipment_holds(&mut self, session: &mut Session) {
        for page in self.pages.values_mut() {
            for setting in page.panel.cancel_equipment_holds(session.flight_generation()) {
                if let Err(error) = session.run(flightcraft_control::SetEquipment { setting }) {
                    self.error = Some(error.to_string());
                }
            }
        }
    }
    pub fn with_chart_root(chart_root: Option<String>) -> Self {
        Self {
            chart_root,
            ..Default::default()
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui, screen: &Screen, s: &mut Session, control: bool) {
        theme::page(ui);
        if cfg!(target_arch = "wasm32")
            && self.scenery_preferences.quality == flightcraft_render::scenery::Quality::High
        {
            self.scenery_preferences.quality = flightcraft_render::scenery::Quality::Medium;
        }
        if !self.scenery_preferences.enabled {
            self.scenery = None;
        }
        if !self.scenery_preferences.streaming_imagery {
            self.imagery = None;
        }
        if !self.scenery_preferences.enabled && !self.scenery_preferences.streaming_imagery {
            self.scenery = None;
            self.imagery = None;
            self.scenery_attempt = None;
        } else if let Some(region) = s.sim().world().region() {
            let origin = [region.origin.latitude_deg, region.origin.longitude_deg];
            let fingerprint = s.sim().world().fingerprint();
            if self.scenery_attempt != Some(fingerprint) {
                self.scenery_attempt = Some(fingerprint);
                #[cfg(target_arch = "wasm32")]
                {
                    self.browser_pending = None;
                }
                self.scenery = None;
                self.imagery = None;
                if self.scenery_preferences.enabled {
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        let provider = flightcraft_render::scenery::Provider {
                            location: flightcraft_render::scenery::Location::LocalPath(
                                self.scenery_preferences.local_tileset.clone().into(),
                            ),
                            token_config_key: None,
                            enabled: true,
                        };
                        match flightcraft_render::scenery::LocalProvider::open(&provider, origin) {
                            Ok(stream) => {
                                self.scenery = Some(std::sync::Arc::new(std::sync::Mutex::new(stream)))
                            }
                            Err(error) => self.error = Some(format!("Scenery: {error}")),
                        }
                    }
                    #[cfg(target_arch = "wasm32")]
                    {
                        let path =
                            if self.scenery_preferences.local_tileset.is_empty() && region.id == "new-york" {
                                "terrain/new-york/scenery/nyc-lidar/tileset.json".to_owned()
                            } else {
                                self.scenery_preferences.local_tileset.clone()
                            };
                        let provider = flightcraft_render::scenery::Provider {
                            location: flightcraft_render::scenery::Location::Url(path),
                            token_config_key: None,
                            enabled: true,
                        };
                        let (sender, receiver) = std::sync::mpsc::channel();
                        self.browser_pending = Some(receiver);
                        let context = ui.ctx().clone();
                        wasm_bindgen_futures::spawn_local(async move {
                            let result = flightcraft_render::scenery::BrowserProvider::open(
                                &provider,
                                origin,
                                context.clone(),
                            )
                            .await;
                            let _ = sender.send(result);
                            context.request_repaint();
                        });
                    }
                }
                if self.scenery_preferences.streaming_imagery {
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        let provider = flightcraft_render::scenery::Provider {
                            location: flightcraft_render::scenery::Location::Url(
                                flightcraft_render::scenery::EsriProvider::endpoint().into(),
                            ),
                            token_config_key: Some("esri-imagery".into()),
                            enabled: true,
                        };
                        match flightcraft_render::scenery::EsriProvider::open(
                            &provider,
                            self.scenery_preferences.esri_key.clone(),
                            s.sim().world().clone(),
                        ) {
                            Ok(stream) => {
                                self.imagery = Some(std::sync::Arc::new(std::sync::Mutex::new(stream)))
                            }
                            Err(error) => self.error = Some(format!("Scenery: {error}")),
                        }
                    }
                    #[cfg(target_arch = "wasm32")]
                    {
                        self.error =
                            Some("Streaming imagery is currently available in the desktop app".into());
                    }
                }
            }
        } else {
            self.scenery = None;
            self.imagery = None;
            self.scenery_attempt = None;
            #[cfg(target_arch = "wasm32")]
            {
                self.browser_pending = None;
            }
        }

        #[cfg(target_arch = "wasm32")]
        {
            if !self.scenery_preferences.enabled {
                self.browser_pending = None;
            }
            let result = self
                .browser_pending
                .as_ref()
                .and_then(|receiver| receiver.try_recv().ok());
            if let Some(result) = result {
                self.browser_pending = None;
                match result {
                    Ok(stream) => self.scenery = Some(std::sync::Arc::new(std::sync::Mutex::new(stream))),
                    Err(error) => self.error = Some(format!("Scenery: {error}")),
                }
            }
        }

        let available = ui.max_rect();
        let [cw, ch] = screen.layout.canvas;
        let calibration = screen
            .monitor
            .and_then(|index| s.screens().monitors.get(index))
            .map_or(screen.layout.pixels_per_mm, |m| m.pixels_per_mm);
        let [sx, sy] = match screen.layout.units {
            Units::Px => {
                // Logical pixel canvases fill their host. Pages preserve the proportions of
                // instruments internally; monitor-calibrated millimetres remain physical.
                [available.width() / cw, available.height() / ch]
            }
            Units::Mm => {
                let [x, y] = calibration;
                let ppp = ui.ctx().pixels_per_point();
                [x / ppp, y / ppp]
            }
        };
        let origin = available.min;
        drag::track(
            ui,
            screen,
            s,
            available,
            [sx, sy],
            &self.transfer,
            &mut self.error,
        );
        let mut elements = screen.layout.elements.clone();
        elements.sort_by_key(|e| e.z);
        self.pages.retain(|id, _| elements.iter().any(|e| &e.id == id));
        for element in elements {
            let [x, y] = element.position;
            let [w, h] = element.size;
            let rect = element_rect(screen, &element, available, [sx, sy], s);
            #[cfg(test)]
            ui.ctx()
                .data_mut(|data| data.insert_temp(egui::Id::new(("page-rect", element.id.clone())), rect));
            let page = self.pages.entry(element.id.clone()).or_insert_with(|| PageUi {
                map: map::PlanningMap::with_root(self.chart_root.clone()),
                ..Default::default()
            });
            page.scene.set_scenery(self.scenery.clone());
            page.scene.set_imagery(self.imagery.clone());
            page.scene.scenery_quality = self.scenery_preferences.quality;
            if let Some(settings) = self.scenery_preferences.lighting {
                let _ = page.scene.set_lighting(settings);
            } else {
                page.scene.clear_lighting();
            }
            ui.scope_builder(egui::UiBuilder::new().id_salt(&element.id).max_rect(rect), |ui| {
                ui.set_clip_rect(rect.intersect(available));
                if !element.accepts_input || !control || self.edit {
                    ui.disable();
                }
                if let Err(e) = page.show(ui, &element.page, s, &element.id) {
                    self.error = Some(e);
                }
            });
            if element.brightness < 1. {
                ui.painter().rect_filled(
                    rect,
                    0.,
                    theme::BACKGROUND.gamma_multiply(1. - element.brightness),
                );
            }
            if matches!(element.mask, Mask::Circle) {
                mask_circle(ui, rect);
            }
            if element.bezel {
                ui.painter().rect_stroke(
                    rect,
                    3.,
                    Stroke::new(1.0_f32, theme::BORDER),
                    egui::StrokeKind::Inside,
                );
            }
            if element.id == flightcraft_control::screens::INSET_ID && control && !self.edit {
                inset_controls(
                    &mut self.error,
                    ui,
                    s,
                    screen,
                    &element,
                    rect,
                    available,
                    [sx, sy],
                );
            }
            if self.edit && control {
                let response =
                    ui.interact(rect, ui.id().with(("edit", &element.id)), Sense::click_and_drag());
                if response.drag_started() {
                    self.drag = Some(element.clone());
                    self.drag_delta = egui::Vec2::ZERO;
                    drag::begin(ui, screen, &element, rect, &self.transfer);
                }
                let mut edited = element.clone();
                if response.dragged() {
                    self.drag_delta += response.drag_delta();
                }
                if (response.dragged() || response.drag_stopped())
                    && let Some(start) = &self.drag
                {
                    let delta = self.drag_delta;
                    let [a, b] = start.position;
                    edited.position = [
                        snap((a + delta.x / sx).clamp(0., cw - w), cw - w),
                        snap((b + delta.y / sy).clamp(0., ch - h), ch - h),
                    ];
                    align(&mut edited, &screen.layout.elements);
                    let [a, b] = edited.position;
                    let preview = Rect::from_min_size(origin + egui::vec2(a * sx, b * sy), rect.size());
                    ui.painter().rect_stroke(
                        preview,
                        0.,
                        Stroke::new(2.0_f32, theme::ACCENT),
                        egui::StrokeKind::Inside,
                    );
                    ui.painter().line_segment(
                        [
                            egui::pos2(preview.center().x, available.top()),
                            egui::pos2(preview.center().x, available.bottom()),
                        ],
                        Stroke::new(1.0_f32, theme::ACCENT),
                    );
                }
                if response.drag_stopped() && !drag::completed(screen.id, &element.id, &self.transfer) {
                    execute(
                        s,
                        "layout.update",
                        json!({"screen":screen.id,"element":edited}),
                        &mut self.error,
                    );
                    self.drag = None;
                }
                response.context_menu(|ui| {
                    ui.label(&element.id);
                    ui.menu_button("Move to screen…", |ui| {
                        ui.spacing_mut().interact_size.y = 44.;
                        for target in s.screens().windows.clone() {
                            if target.id != screen.id
                                && ui.button(format!("{} · {}", target.id, target.title)).clicked()
                            {
                                execute(
                                    s,
                                    "layout.move",
                                    json!({"from":screen.id,"to":target.id,"element":element.id}),
                                    &mut self.error,
                                );
                                ui.close();
                            }
                        }
                    });
                    let mut changed = element.clone();
                    ui.label("Position / size (canvas units)");
                    let mut update = false;
                    let [mut x, mut y] = changed.position;
                    let [mut w, mut h] = changed.size;
                    ui.horizontal(|ui| {
                        update |= ui.add(egui::DragValue::new(&mut x).prefix("X ")).changed();
                        update |= ui.add(egui::DragValue::new(&mut y).prefix("Y ")).changed();
                    });
                    ui.horizontal(|ui| {
                        update |= ui.add(egui::DragValue::new(&mut w).prefix("W ")).changed();
                        update |= ui.add(egui::DragValue::new(&mut h).prefix("H ")).changed();
                    });
                    egui::ComboBox::from_id_salt("element-page-kind")
                        .selected_text(changed.page.kind.label())
                        .show_ui(ui, |ui| {
                            for kind in PageKind::ALL {
                                update |= ui
                                    .selectable_value(&mut changed.page.kind, kind, kind.label())
                                    .changed();
                            }
                        });
                    let mut item = changed.page.item.clone().unwrap_or_default();
                    ui.label("Equipment / switch / breaker id");
                    update |= ui.text_edit_singleline(&mut item).changed();
                    changed.page.item = (!item.is_empty()).then_some(item);
                    if changed.page.kind == PageKind::Outside {
                        update |= ui
                            .add(
                                egui::Slider::new(&mut changed.page.camera.fov_deg, 15.0..=150.0)
                                    .text("Vertical FOV °"),
                            )
                            .changed();
                        update |= ui
                            .add(
                                egui::Slider::new(
                                    &mut changed.page.camera.heading_offset_deg,
                                    -180.0..=180.0,
                                )
                                .text("Heading offset °"),
                            )
                            .changed();
                    }
                    if changed.page.kind == PageKind::Control {
                        egui::ComboBox::from_id_salt("lever-kind")
                            .selected_text(changed.page.control.kind.label())
                            .show_ui(ui, |ui| {
                                for kind in ControlKind::ALL {
                                    update |= ui
                                        .selectable_value(&mut changed.page.control.kind, kind, kind.label())
                                        .changed();
                                }
                            });
                    }
                    changed.position = [x, y];
                    changed.size = [w, h];
                    update |= ui.checkbox(&mut changed.accepts_input, "Accept input").changed();
                    update |= ui
                        .checkbox(&mut changed.control_mut().friction_locked, "Friction lock")
                        .changed();
                    if update {
                        execute(
                            s,
                            "layout.update",
                            json!({"screen":screen.id,"element":changed}),
                            &mut self.error,
                        );
                    }
                });
                let handle = Rect::from_min_max(rect.max - egui::vec2(24., 24.), rect.max);
                let resize = ui.interact(handle, ui.id().with(("resize", &element.id)), Sense::drag());
                ui.painter().rect_filled(handle, 2., theme::ACCENT);
                ui.painter().text(
                    rect.left_top() + egui::vec2(6., 6.),
                    Align2::LEFT_TOP,
                    &element.id,
                    FontId::monospace(12.),
                    theme::ACCENT,
                );
                if resize.drag_started() {
                    self.drag = Some(element.clone());
                    self.drag_delta = egui::Vec2::ZERO;
                }
                if resize.dragged() {
                    self.drag_delta += resize.drag_delta();
                }
                if resize.drag_stopped()
                    && let Some(mut start) = self.drag.take()
                {
                    let [w, h] = start.size;
                    start.size = [
                        snap(
                            (w + self.drag_delta.x / sx).clamp(20_f32.min(cw - x), cw - x),
                            cw - x,
                        ),
                        snap(
                            (h + self.drag_delta.y / sy).clamp(20_f32.min(ch - y), ch - y),
                            ch - y,
                        ),
                    ];
                    execute(
                        s,
                        "layout.update",
                        json!({"screen":screen.id,"element":start}),
                        &mut self.error,
                    );
                }
            }
        }
        drag::outline(ui, screen.id, &self.transfer);
        if let Some(e) = &self.error {
            ui.painter().text(
                available.left_bottom(),
                Align2::LEFT_BOTTOM,
                e,
                FontId::monospace(12.),
                theme::AMBER,
            );
        }
    }
}
// The built-in outside/panel split follows the actual aircraft and current host size.
// Authored layouts keep their element positions, including calibrated physical panels.
pub(crate) fn element_rect(
    screen: &Screen,
    element: &Element,
    available: Rect,
    scale: [f32; 2],
    s: &Session,
) -> Rect {
    let layout = &screen.layout;
    let default_split =
        matches!(layout.units, Units::Px)
            && layout.canvas == [1280., 800.]
            && layout.elements.len() == 2
            && layout.elements.iter().any(|e| {
                e.page.kind == PageKind::Outside && e.position == [0., 0.] && e.size == [1280., 480.]
            })
            && layout.elements.iter().any(|e| {
                e.page.kind == PageKind::Panel && e.position == [0., 480.] && e.size == [1280., 320.]
            });
    if default_split && let Some(configuration) = s.aircraft_configuration() {
        let height = instruments::view_panel_height(&configuration.panel, available.size(), true);
        return if element.page.kind == PageKind::Panel {
            Rect::from_min_max(
                egui::pos2(available.left(), available.bottom() - height),
                available.max,
            )
        } else {
            Rect::from_min_max(
                available.min,
                egui::pos2(available.right(), available.bottom() - height),
            )
        };
    }
    let [x, y] = element.position;
    let [w, h] = element.size;
    let [sx, sy] = scale;
    Rect::from_min_size(
        available.min + egui::vec2(x * sx, y * sy),
        egui::vec2(w * sx, h * sy),
    )
}
trait ControlMut {
    fn control_mut(&mut self) -> &mut TouchControl;
}
impl ControlMut for Element {
    fn control_mut(&mut self) -> &mut TouchControl {
        &mut self.page.control
    }
}
fn snap(value: f32, max: f32) -> f32 {
    ((value / 8.).round() * 8.).clamp(0., max)
}
fn align(element: &mut Element, others: &[Element]) {
    let [mut x, mut y] = element.position;
    let [w, h] = element.size;
    for e in others.iter().filter(|e| e.id != element.id) {
        let [a, b] = e.position;
        let [ew, eh] = e.size;
        for target in [a, a + ew, a + ew / 2. - w / 2.] {
            if (x - target).abs() < 4. {
                x = target;
            }
        }
        for target in [b, b + eh, b + eh / 2. - h / 2.] {
            if (y - target).abs() < 4. {
                y = target;
            }
        }
    }
    element.position = [x.max(0.), y.max(0.)];
}
fn mask_circle(ui: &egui::Ui, rect: Rect) {
    let c = rect.center();
    let r = rect.width().min(rect.height()) * 0.5;
    for (corner, start) in [
        (rect.left_top(), std::f32::consts::PI),
        (rect.right_top(), 1.5 * std::f32::consts::PI),
        (rect.right_bottom(), 0.),
        (rect.left_bottom(), 0.5 * std::f32::consts::PI),
    ] {
        let mut points = vec![corner];
        for i in 0..=24 {
            let angle = start + i as f32 / 24. * std::f32::consts::FRAC_PI_2;
            points.push(c + egui::vec2(angle.cos(), angle.sin()) * r);
        }
        ui.painter().add(egui::Shape::convex_polygon(
            points,
            theme::BACKGROUND,
            Stroke::NONE,
        ));
    }
}
impl PageUi {
    fn camera_elapsed(&mut self, now: f64, generation: u64) -> f32 {
        if self.camera_generation != Some(generation) {
            self.scene.reset_camera();
            self.camera_time = None;
            self.camera_generation = Some(generation);
        }
        let elapsed = self.camera_time.map_or(0., |previous| (now - previous).max(0.));
        self.camera_time = now.is_finite().then_some(now);
        if elapsed.is_finite() { elapsed as f32 } else { 0. }
    }

    /// Points the scene at the part the pilot just moved (or the walkaround's camera), easing from
    /// the previous framing. The eye is placed in the world from the aircraft's attitude.
    fn frame_inset(&mut self, ui: &egui::Ui, s: &mut Session) {
        use flightcraft_aero::exterior_framing::{Framing, FramingTarget, frame};
        let state = *s.sim().state();
        let Some(asset) = self.scene.exterior_asset().cloned() else {
            return;
        };
        let cg = s
            .aircraft_configuration()
            .map_or([0.; 3], |c| c.weight_balance.cg_m.map(|v| v as f32));
        let walkaround = crate::checklist::walkaround_camera(s);
        let wanted: Framing = walkaround
            .as_ref()
            .and_then(|w| w.body)
            .or_else(|| {
                let target: FramingTarget = s.exterior_inset_state().target;
                frame(&asset, target, cg).or_else(|| frame(&asset, FramingTarget::Default, cg))
            })
            .unwrap_or(Framing {
                eye_m: [-8., 4., -3.],
                target_m: [0.; 3],
                fov_deg: 50.,
            });
        let dt = ui.ctx().input(|i| i.stable_dt).clamp(0., 0.25);
        let current = match self.inset_frame {
            Some(c) => c.lerp(wanted, 1. - (-dt / INSET_EASE_S).exp()),
            None => wanted,
        };
        self.inset_frame = Some(current);
        let attitude = state.attitude.map(|v| v as f32);
        let position = state.position_m.map(|v| v as f32);
        let world = |v: [f32; 3]| -> [f32; 3] {
            let [rx, ry, rz] = flightcraft_aero::exterior_framing::rotate(attitude, v);
            let [px, py, pz] = position;
            [px + rx, py + ry, pz + rz]
        };
        let (eye, target, fov) = match walkaround.as_ref().and_then(|w| w.world) {
            Some((eye, target, fov)) => (eye, target, fov),
            None => (world(current.eye_m), world(current.target_m), current.fov_deg),
        };
        self.scene.set_screen_view(Some(flightcraft_render::ScreenView {
            heading_offset_rad: 0.,
            pitch_rad: 0.,
            fov_rad: fov.to_radians(),
            fixed_eye_ned_m: Some(eye),
        }));
        self.scene.set_look_target(Some(target));
    }

    pub fn show(&mut self, ui: &mut egui::Ui, page: &Page, s: &mut Session, id: &str) -> Result<(), String> {
        theme::page(ui);
        let readout = s.sim().readout();
        match page.kind {
            PageKind::Image => self.picture.show(
                ui,
                page.item.as_deref().ok_or("choose a relative image asset path")?,
            )?,
            PageKind::Atc => {
                let mut error = None;
                egui::ScrollArea::vertical()
                    .show(ui, |ui| radio::atc_panel(ui, s, &mut self.airport, &mut error));
                if let Some(error) = error {
                    return Err(error);
                }
            }
            PageKind::Readout => {
                let name = page.item.as_deref().unwrap_or("readout.indicated_airspeed_mps");
                let mut parts = name.split('.');
                let mut value = match parts.next() {
                    Some("readout") => serde_json::to_value(readout),
                    Some("state") => serde_json::to_value(s.sim().state()),
                    Some("systems") => serde_json::to_value(s.sim().systems()),
                    _ => return Err("readouts start with readout, state, or systems".into()),
                }
                .map_err(|e| e.to_string())?;
                for part in parts {
                    value = value.get(part).cloned().ok_or("unknown readout variable")?;
                }
                ui.label(egui::RichText::new(name).color(theme::MUTED));
                ui.label(egui::RichText::new(value.to_string()).monospace().size(24.));
            }
            PageKind::Switch => {
                let item = page.item.as_deref().unwrap_or("master");
                let controls =
                    serde_json::to_value(&s.sim().systems().controls).map_err(|e| e.to_string())?;
                let mut on = controls
                    .get(item)
                    .and_then(serde_json::Value::as_bool)
                    .ok_or("switch not installed or unknown")?;
                if ui
                    .add_sized(
                        ui.available_size(),
                        egui::Button::new(format!(
                            "{}\n{}",
                            item.to_uppercase(),
                            if on { "ON" } else { "OFF" }
                        )),
                    )
                    .clicked()
                {
                    on = !on;
                    s.execute(
                        "systems.set",
                        json!({"setting":{"kind":"switch","switch":item,"on":on}}),
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
            PageKind::Breaker => {
                let load = page.item.as_deref().ok_or("choose a breaker load id")?;
                let closed = s
                    .sim()
                    .systems()
                    .electrical
                    .loads
                    .iter()
                    .find(|l| l.id == load)
                    .map(|l| l.breaker_closed)
                    .ok_or("breaker is not installed")?;
                if ui
                    .add_sized(
                        ui.available_size(),
                        egui::Button::new(format!("{load}\n{}", if closed { "PULL" } else { "RESET" })),
                    )
                    .clicked()
                {
                    s.execute(
                        "systems.set",
                        json!({"setting":{"kind":"breaker","load":load,"closed":!closed}}),
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
            PageKind::Checklist => {
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| self.checklist.show(ui, s));
            }
            PageKind::FlyMe => {
                let keymap = ui
                    .ctx()
                    .data(|data| data.get_temp::<crate::keymap::Keymap>(egui::Id::new("crew.keymap")))
                    .unwrap_or_default();
                egui::ScrollArea::vertical().show(ui, |ui| self.crew.panel(ui, s, &keymap));
            }
            PageKind::Outside | PageKind::Director => {
                use flightcraft_render::{CameraMode, ScreenView};
                let [n, e, d] = s.sim().state().position_m;
                let main_camera = page.follow_main_camera.then(|| s.camera()).flatten();
                let camera_view = main_camera.map_or(page.camera.view, |c| match c.mode {
                    flightcraft_control::CameraMode::Chase => View::Chase,
                    flightcraft_control::CameraMode::Cockpit => View::Forward,
                    _ => View::Chase,
                });
                let mut yaw = main_camera.map_or(page.camera.heading_offset_deg.to_radians(), |c| {
                    c.look_yaw_rad as f32
                });
                if let Some(camera) = main_camera {
                    self.scene
                        .set_chase_offsets(camera.distance_m as f32, camera.height_m as f32);
                }
                let (mode, fixed) = match camera_view {
                    View::Chase => (CameraMode::Chase, None),
                    View::Tower => (CameraMode::Chase, Some([0., -100., -25.])),
                    View::TopDown => (CameraMode::Chase, Some([n as f32, e as f32, d as f32 - 800.])),
                    View::Left => {
                        yaw -= std::f32::consts::FRAC_PI_2;
                        (CameraMode::Cockpit, None)
                    }
                    View::Right => {
                        yaw += std::f32::consts::FRAC_PI_2;
                        (CameraMode::Cockpit, None)
                    }
                    View::Forward => (CameraMode::Cockpit, None),
                };
                let inset = is_inset(id);
                let mode = if inset {
                    CameraMode::Chase
                } else if page.kind == PageKind::Director {
                    crate::crew::director_camera(s)
                } else {
                    main_camera.map_or(mode, |camera| match camera.mode {
                        flightcraft_control::CameraMode::Cockpit => mode,
                        flightcraft_control::CameraMode::Chase => mode,
                        mode => crate::crew::render_camera(mode),
                    })
                };
                if self.scene.camera != mode {
                    self.scene.camera = mode;
                    self.scene.reset_camera();
                }
                self.scene.set_director_runway(
                    s.crew_state()
                        .plan
                        .as_ref()
                        .and_then(|p| p.arrival.as_ref())
                        .map(|r| (r.threshold_m, r.heading_grid_rad)),
                );
                let screen_view = ScreenView {
                    heading_offset_rad: yaw,
                    pitch_rad: main_camera.map_or(0., |c| c.look_pitch_rad as f32),
                    fov_rad: page.camera.fov_deg.to_radians(),
                    fixed_eye_ned_m: fixed,
                };
                self.scene.set_screen_view(
                    if page.kind == PageKind::Director
                        || matches!(mode, CameraMode::Tower | CameraMode::Flyby | CameraMode::Wing)
                    {
                        None
                    } else {
                        Some(screen_view)
                    },
                );
                self.scene.airspace_visible = s.airspace_visible();
                self.scene
                    .set_light_gun(s.atc_state().light_gun.map(|g| flightcraft_render::LightGun {
                        position_m: g.position_m,
                        elapsed_s: s.sim().state().time_s - g.since_s,
                        pattern: match g.signal {
                            flightcraft_control::LightGunSignal::SteadyGreen => {
                                flightcraft_render::LightGunPattern::SteadyGreen
                            }
                            flightcraft_control::LightGunSignal::FlashingGreen => {
                                flightcraft_render::LightGunPattern::FlashingGreen
                            }
                            flightcraft_control::LightGunSignal::SteadyRed => {
                                flightcraft_render::LightGunPattern::SteadyRed
                            }
                            flightcraft_control::LightGunSignal::FlashingRed => {
                                flightcraft_render::LightGunPattern::FlashingRed
                            }
                            flightcraft_control::LightGunSignal::FlashingWhite => {
                                flightcraft_render::LightGunPattern::FlashingWhite
                            }
                            flightcraft_control::LightGunSignal::AlternatingRedGreen => {
                                flightcraft_render::LightGunPattern::AlternatingRedGreen
                            }
                        },
                    }));
                self.scene.set_world(s.sim().world());
                self.scene.set_weather(s.sim().conditions(), s.sim().hood());
                self.scene.set_aircraft(&s.sim().aircraft().identity.id);
                if let Some(asset) = s.aircraft_catalog().exterior(&s.sim().aircraft().identity.id) {
                    self.scene.set_exterior(asset);
                }
                if inset {
                    self.frame_inset(ui, s);
                } else {
                    self.scene.set_look_target(None);
                }
                let callsign = s.pilot_callsign();
                self.scene
                    .set_registration(callsign.overridden.then_some(callsign.registration.as_str()));
                self.scene.set_exterior_state(s.sim());
                self.scene.set_walkaround(s.preflight_scene());
                self.scene
                    .set_propeller_power(readout.effective_controls.throttle as f32);
                let dt = self.camera_elapsed(ui.ctx().input(|i| i.time), s.flight_generation());
                self.scene.show(ui, s.sim().state(), dt, theme::scene());
                if mode == CameraMode::Cockpit && s.sim().canopy().sliding {
                    self.canopy_controls(ui, s);
                }
                radio::captions(ui, s);
                ui.painter().text(
                    ui.max_rect().center_bottom() - egui::vec2(0., 10.),
                    Align2::CENTER_BOTTOM,
                    crate::weather::note(s),
                    FontId::monospace(11.),
                    theme::TEXT,
                );
            }
            PageKind::Panel | PageKind::PanelItem | PageKind::Instrument => {
                let state = s.glass_state();
                let registration = s.pilot_callsign().registration;
                let mut configuration = s
                    .aircraft_configuration()
                    .cloned()
                    .ok_or("no aircraft panel installed")?;
                if page.kind == PageKind::PanelItem {
                    let equipment = configuration
                        .panel
                        .equipment
                        .as_mut()
                        .ok_or("this aircraft has no equipment panel")?;
                    let selected = page
                        .item
                        .as_deref()
                        .ok_or("select an equipment installation id")?;
                    let mut item = equipment
                        .elements
                        .iter()
                        .find(|e| e.id == selected)
                        .cloned()
                        .ok_or("equipment item not installed")?;
                    equipment.size = item.size;
                    equipment.shell = None;
                    equipment.essential = None;
                    item.position = [0., 0.];
                    equipment.elements = vec![item];
                } else if !page.instruments.is_empty() {
                    configuration.panel.equipment = None;
                    configuration
                        .panel
                        .instruments
                        .retain(|i| page.instruments.contains(&i.id) || page.instruments.contains(&i.module));
                }
                self.panel.standalone = page.kind == PageKind::Instrument;
                self.panel.fit_all = true;
                self.panel.dash_only = page.kind == PageKind::Panel;
                if self.panel.standalone {
                    for instrument in &mut configuration.panel.instruments {
                        instrument.position = [0., 0.];
                        instrument.size = [1., 1.];
                    }
                }
                let skins: Vec<&flightcraft_control::SkinContents> = s.skins().collect();
                let result = self.panel.show(
                    ui,
                    &configuration.panel,
                    instruments::InstrumentInput::new(s.sim().systems(), readout.time_s)
                        .with_pilot(s.sim().controls())
                        .with_skins(&skins)
                        .with_aircraft(&configuration)
                        .with_registration(&registration)
                        .with_airports(s.sim().world().airports())
                        .with_vision(s.sim().world(), s.sim().state())
                        .with_glass(&state)
                        .with_autopilot(s.sim().installed_autopilot()),
                    s.flight_generation(),
                )?;
                self.panel.show_shared_hosts(ui, s);
                if let Some(rect) = self.panel.map_rect() {
                    self.map.show_mfd(ui, rect, s);
                }
                for setting in result {
                    s.run(flightcraft_control::SetSystems { setting })
                        .map_err(|e| e.to_string())?;
                }
                for setting in self.panel.take_equipment_commands() {
                    s.run(flightcraft_control::SetEquipment { setting })
                        .map_err(|e| e.to_string())?;
                }
                for setting in self
                    .panel
                    .take_heading_commands()
                    .into_iter()
                    .chain(self.panel.avionics_controls.take_settings())
                {
                    s.run(flightcraft_control::SetAvionics { setting })
                        .map_err(|e| e.to_string())?;
                }
                for setting in self.panel.take_glass_commands() {
                    s.run(flightcraft_control::SetAvionics {
                        setting: flightcraft_control::AvionicsSetting::Glass { setting },
                    })
                    .map_err(|e| e.to_string())?;
                }
            }
            PageKind::Pfd | PageKind::Mfd => {
                let state = s.glass_state();
                let name = if page.kind == PageKind::Pfd {
                    "glass_pfd"
                } else {
                    "glass_mfd"
                };
                let controls_height = 150_f32.min(ui.max_rect().height() * 0.3);
                let display = Rect::from_min_max(
                    ui.max_rect().min,
                    ui.max_rect().right_bottom() - egui::vec2(0., controls_height),
                );
                let mut module = glass::create(name)?;
                let mut input = instruments::InstrumentInput::new(s.sim().systems(), readout.time_s)
                    .with_vision(s.sim().world(), s.sim().state())
                    .with_glass(&state)
                    .with_autopilot(s.sim().installed_autopilot());
                input.aircraft = s.aircraft_configuration();
                glass::paint_fill(ui, display, module.update(input));
                if page.kind == PageKind::Mfd
                    && state.settings.page == flightcraft_control::GlassPage::Map
                    && !state.mfd_failed
                {
                    self.map.show_mfd_base(
                        ui,
                        Rect::from_min_max(
                            display.min + display.size() * egui::vec2(0.22, 0.074),
                            display.max,
                        ),
                        s,
                        Some(flightcraft_control::BaseMap::Terrain),
                    );
                }
                let bezel = Rect::from_min_max(display.left_bottom(), ui.max_rect().max);
                ui.painter().rect_filled(bezel, 0., theme::RAISED);
                ui.scope_builder(egui::UiBuilder::new().max_rect(bezel.shrink(4.)), |ui| {
                    ui.horizontal(|ui| {
                        for (kind, current) in [
                            (ControlKind::Heading, state.settings.heading_bug_rad),
                            (ControlKind::Altitude, state.settings.selected_altitude_m),
                            (ControlKind::Range, state.settings.range_nm),
                            (ControlKind::Fms, state.settings.active_leg as f64),
                        ] {
                            for (name, input) in self.knobs.entry(format!("{kind:?}")).or_default().show(
                                ui,
                                kind,
                                current,
                                if kind == ControlKind::Fms {
                                    state.settings.flight_plan.len() as f64
                                } else {
                                    readout.altitude_msl_m
                                },
                            ) {
                                s.execute(name, input).map_err(|e| e.to_string())?;
                            }
                        }
                        Ok::<(), String>(())
                    })
                    .inner?;
                    ui.horizontal(|ui| {
                        for (label, name) in [("MAP", "map"), ("NRST", "nearest"), ("FPL", "flight_plan")] {
                            if ui
                                .add_sized([80., 48.], egui::Button::new(label).fill(theme::DIAL))
                                .clicked()
                            {
                                let _ = s.execute(
                                    "avionics.set",
                                    json!({"setting":{"kind":"glass","setting":{"kind":"page","page":name}}}),
                                );
                            }
                        }
                    });
                    Ok::<(), String>(())
                })
                .inner?;
            }
            PageKind::Map => {
                self.map.show(ui, ui.max_rect(), s);
            }
            PageKind::Navigator => {
                self.navigation.show(ui, s);
            }
            PageKind::Traffic => {
                self.traffic.show(ui, s);
            }
            PageKind::Charts => {
                ui.heading("EFB / charts");
                ui.label("Regional sectional and TAC charts • offline availability shown on the map");
                self.map.show(ui, ui.available_rect_before_wrap(), s);
            }
            PageKind::Instructor => {
                let mut error = None;
                egui::ScrollArea::vertical().show(ui, |ui| {
                    egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(16, 12))
                        .show(ui, |ui| {
                            ui.heading("Instructor station");
                            let elapsed = readout.time_s.max(0.0) as u64;
                            ui.label(format!(
                                "Live · {}:{:02}:{:02} · {:.0} ft · {:.0} kt",
                                elapsed / 3600,
                                elapsed / 60 % 60,
                                elapsed % 60,
                                readout.altitude_msl_m / 0.3048,
                                readout.indicated_airspeed_mps / 0.514444
                            ));
                            ui.horizontal(|ui| {
                                if ui
                                    .add_sized(
                                        [110., 44.],
                                        egui::Button::new(if s.paused() { "Resume" } else { "Freeze" }),
                                    )
                                    .clicked()
                                {
                                    let _ = s.execute(
                                        if s.paused() { "sim.resume" } else { "sim.pause" },
                                        json!({}),
                                    );
                                }
                                for (preset, label) in [("vfr", "VFR"), ("ifr", "IFR"), ("night", "Night")] {
                                    if ui.add_sized([80., 44.], egui::Button::new(label)).clicked() {
                                        let _ = s.execute("weather.set", json!({"preset":preset}));
                                    }
                                }
                            });
                            if ui.button("Reposition to runway").clicked() {
                                let _ = crate::actions::reset(s);
                            }
                            self.instrument.show(ui, s);
                            ui.separator();
                            if let Err(e) = self.instructor.panel(ui, s) {
                                ui.colored_label(theme::AMBER, e.to_string());
                            }
                            ui.separator();
                            ui.heading("ATC / radio monitor");
                            radio::atc_panel(ui, s, &mut self.airport, &mut error);
                            radio::transcript(ui, s);
                            let grades = &s.flight_record().events;
                            ui.label(format!("Live flight events / grades: {}", grades.len()));
                            for event in grades.iter().rev().filter(|e| e.grade.is_some()).take(20) {
                                if let Some(grade) = &event.grade {
                                    ui.colored_label(
                                        if grade.passed { theme::GREEN } else { theme::AMBER },
                                        format!(
                                            "{:.1}s · {} · {}",
                                            event.time_s,
                                            if grade.passed { "PASS" } else { "REVIEW" },
                                            grade.feedback
                                        ),
                                    );
                                }
                            }
                        });
                });
            }
            PageKind::Radio => {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let state = s.comms_state();
                    let mut error = None;
                    radio::panel(ui, s, &state, &mut self.tune1, &mut self.tune2, &mut error);
                    radio::atc_panel(ui, s, &mut self.airport, &mut error);
                    radio::transcript(ui, s);
                    if let Some(e) = error {
                        ui.colored_label(theme::AMBER, e);
                    }
                });
            }
            PageKind::Debrief => {
                let mut error = None;
                egui::ScrollArea::vertical().show(ui, |ui| {
                    crate::ifr::debrief(ui, s.lesson_debrief());
                    if s.lesson_debrief().title.is_none() {
                        ui.heading("Debrief");
                    }
                    if let Some(d) = s.scenario_state().emergency.clone() {
                        let _ = scenario::debrief(ui, s, &d);
                    } else if s.lesson_debrief().title.is_none() {
                        ui.label("No emergency debrief yet.");
                    }
                    let archive = s.flight_archive_status();
                    if let Some(saved) = &archive.loaded
                        && let Some(lesson) = &saved.record.lesson
                    {
                        crate::ifr::debrief(ui, lesson);
                    }
                    ui.label(format!("{} saved flights", archive.flights.len()));
                    radio::atc_panel(ui, s, &mut self.airport, &mut error);
                    radio::transcript(ui, s);
                });
            }
            PageKind::Control => {
                let kind = page.control.kind;
                let value = control_value(s, kind);
                let id_value = serde_json::to_value(kind)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default();
                let mut definition = s
                    .aircraft_configuration()
                    .and_then(|c| c.panel.touch_controls.iter().find(|d| d.id == id_value))
                    .cloned();
                if definition.is_none()
                    && kind == ControlKind::Throttle
                    && s.aircraft_configuration().is_some_and(|c| {
                        matches!(
                            c.engine.control,
                            flightcraft_aero::modern::EngineControl::ElectronicDualLane { .. }
                        )
                    })
                {
                    definition = Some(flightcraft_aero::TouchControlDefinition {
                        id: id_value,
                        label: "Throttle".into(),
                        style: flightcraft_aero::TouchControlStyle::Quadrant,
                        detents: Vec::new(),
                    });
                }
                if matches!(
                    kind,
                    ControlKind::Heading
                        | ControlKind::Altitude
                        | ControlKind::Range
                        | ControlKind::Fms
                        | ControlKind::Baro
                        | ControlKind::Course
                ) {
                    for (name, input) in self.knobs.entry(id.into()).or_default().show(
                        ui,
                        kind,
                        value,
                        if kind == ControlKind::Fms {
                            s.glass_state().settings.flight_plan.len() as f64
                        } else {
                            readout.altitude_msl_m
                        },
                    ) {
                        s.execute(name, input).map_err(|e| e.to_string())?;
                    }
                } else if let Some(value) = self.levers.entry(id.into()).or_default().show_defined(
                    ui,
                    ui.max_rect(),
                    value,
                    &page.control,
                    definition.as_ref(),
                ) {
                    let (name, input) = touch::lever_command(kind, value, page.item.as_deref());
                    s.execute(name, input).map_err(|e| e.to_string())?;
                }
            }
        }
        Ok(())
    }
}
fn control_value(s: &Session, kind: ControlKind) -> f64 {
    let systems = s.sim().systems();
    let c = s.sim().controls();
    match kind {
        ControlKind::Throttle => c.throttle,
        ControlKind::Mixture => systems.controls.mixture,
        ControlKind::Primer => systems.primer_charge,
        ControlKind::Magnetos => match systems.controls.magnetos {
            flightcraft_sim::systems::Magnetos::Off => 0.,
            flightcraft_sim::systems::Magnetos::Left => 0.33,
            flightcraft_sim::systems::Magnetos::Right => 0.6,
            flightcraft_sim::systems::Magnetos::Both => 1.,
        },
        ControlKind::Trim => (systems.controls.trim + 0.3) / 0.6,
        ControlKind::CarbHeat => f64::from(systems.controls.carb_heat),
        ControlKind::Flaps => c.flaps,
        ControlKind::ParkingBrake => f64::from(systems.controls.park_brake || c.brake >= 0.5),
        ControlKind::Heading => s.glass_state().settings.heading_bug_rad,
        ControlKind::Altitude => s.glass_state().settings.selected_altitude_m,
        ControlKind::Range => s.glass_state().settings.range_nm,
        ControlKind::Fms => s.glass_state().settings.active_leg as f64,
        ControlKind::Baro => s.glass_state().baro_pa,
        ControlKind::Course => s.glass_state().settings.course_rad,
        ControlKind::FuelSelector => match systems.controls.fuel_selector {
            flightcraft_sim::systems::FuelSelector::Off => 0.,
            flightcraft_sim::systems::FuelSelector::Left => 0.33,
            flightcraft_sim::systems::FuelSelector::Right => 0.6,
            flightcraft_sim::systems::FuelSelector::Both => 1.,
        },
        _ => 0.5,
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, PartialEq)]
struct WindowGeometry {
    size: [f32; 2],
    position: Option<[f32; 2]>,
    fullscreen: bool,
    borderless: bool,
}
#[derive(Default)]
pub struct ScreensUi {
    pub main: Canvas,
    #[cfg(not(target_arch = "wasm32"))]
    root_geometry: Option<WindowGeometry>,
    #[cfg(not(target_arch = "wasm32"))]
    windows: BTreeMap<u64, std::sync::Arc<std::sync::Mutex<Canvas>>>,
    #[cfg(not(target_arch = "wasm32"))]
    pub remote: Option<remote::Server>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) watcher: Option<watch::Folder>,
}
impl ScreensUi {
    pub fn with_chart_root(root: Option<String>) -> Self {
        #[cfg(target_arch = "wasm32")]
        {
            Self {
                main: Canvas::with_chart_root(root),
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            Self {
                main: Canvas::with_chart_root(root),
                ..Default::default()
            }
        }
    }
    pub fn menu(&mut self, ui: &mut egui::Ui, s: &mut Session) {
        ui.menu_button("Screens", |ui| {
            ui.checkbox(&mut self.main.edit, "Edit layouts (all windows)");
            for kind in PageKind::ALL {
                if ui.button(format!("Open {}", kind.label())).clicked() {
                    match s.execute("screen.open", json!({"page":{"kind":kind}})) {
                        Ok(screen) => {
                            #[cfg(target_arch = "wasm32")]
                            if let Some(id) = screen.get("id").and_then(serde_json::Value::as_u64)
                                && let Some(window) = web_sys::window()
                                && let Err(error) = window.open_with_url(&format!("?screen={id}"))
                            {
                                self.main.error = Some(format!("Could not open screen: {error:?}"));
                            }
                            #[cfg(not(target_arch = "wasm32"))]
                            let _ = screen;
                        }
                        Err(error) => self.main.error = Some(error.to_string()),
                    }
                    ui.close();
                }
            }
            ui.separator();
            for window in s.screens().windows.clone() {
                ui.menu_button(format!("{} · {}", window.id, window.title), |ui| {
                    for kind in PageKind::ALL {
                        if ui.button(kind.label()).clicked() {
                            execute(
                                s,
                                "screen.set_page",
                                json!({"id":window.id,"page":{"kind":kind}}),
                                &mut self.main.error,
                            );
                        }
                    }
                    if ui.button("Split aircraft panel into pieces").clicked()
                        && let Some(config) = s.aircraft_configuration()
                    {
                        let layout = split_panel(&config.panel, &s.sim().systems().electrical.loads);
                        execute(
                            s,
                            "layout.set",
                            json!({"id":window.id,"layout":layout}),
                            &mut self.main.error,
                        );
                    }
                    let [mut width, mut height] = window.size;
                    let mut resized = false;
                    ui.horizontal(|ui| {
                        resized |= ui
                            .add(
                                egui::DragValue::new(&mut width)
                                    .range(160.0..=16384.0)
                                    .prefix("W "),
                            )
                            .changed();
                        resized |= ui
                            .add(
                                egui::DragValue::new(&mut height)
                                    .range(160.0..=16384.0)
                                    .prefix("H "),
                            )
                            .changed();
                    });
                    if resized {
                        execute(
                            s,
                            "screen.configure",
                            json!({"id":window.id,"size":[width,height]}),
                            &mut self.main.error,
                        );
                    }
                    let mut full = window.fullscreen;
                    let mut borderless = window.borderless;
                    if ui.checkbox(&mut full, "Fullscreen").changed()
                        || ui.checkbox(&mut borderless, "Borderless").changed()
                    {
                        execute(
                            s,
                            "screen.configure",
                            json!({"id":window.id,"fullscreen":full,"borderless":borderless}),
                            &mut self.main.error,
                        );
                    }
                    for (index, monitor) in s.screens().monitors.clone().iter().enumerate() {
                        if ui.button(format!("Monitor: {}", monitor.name)).clicked() {
                            execute(
                                s,
                                "screen.configure",
                                json!({"id":window.id,"monitor":index}),
                                &mut self.main.error,
                            );
                        }
                    }
                    if window.id != 0 && ui.button("Close screen").clicked() {
                        execute(s, "screen.close", json!({"id":window.id}), &mut self.main.error);
                    }
                });
            }
            ui.horizontal(|ui| {
                if ui.button("Undo layout").clicked() {
                    execute(s, "layout.undo", json!({}), &mut self.main.error);
                }
                if ui.button("Redo").clicked() {
                    execute(s, "layout.redo", json!({}), &mut self.main.error);
                }
            });
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(server) = &self.remote {
                ui.separator();
                ui.label(format!("LAN panels: {}", server.address()));
                ui.label(format!("Pairing token: {}", server.token()));
                for screen in server.clients() {
                    ui.horizontal(|ui| {
                        ui.label(format!("{} · {:?}", screen.name, screen.permission));
                        let mut control = screen.permission == remote::Permission::Control;
                        if ui.checkbox(&mut control, "Control").changed() {
                            server.grant(
                                screen.id,
                                if control {
                                    remote::Permission::Control
                                } else {
                                    remote::Permission::ReadOnly
                                },
                            );
                        }
                        if ui.button("Disconnect").clicked() {
                            server.disconnect(screen.id);
                        }
                    });
                }
            }
        });
    }
    pub fn root(&mut self, ui: &mut egui::Ui, s: &mut Session) {
        if let Some(mut screen) = s.screens().windows.iter().find(|s| s.id == 0).cloned() {
            screen.layout = main_layout(ui.ctx(), s, &screen.layout, ui.max_rect().width() < 700.);
            #[cfg(not(target_arch = "wasm32"))]
            {
                let geometry = WindowGeometry {
                    size: screen.size,
                    position: screen.position,
                    fullscreen: screen.fullscreen,
                    borderless: screen.borderless,
                };
                if self.root_geometry != Some(geometry) {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::InnerSize(screen.size.into()));
                    if let Some(position) = screen.position {
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::OuterPosition(position.into()));
                    }
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Fullscreen(screen.fullscreen));
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Decorations(!screen.borderless));
                    self.root_geometry = Some(geometry);
                } else {
                    capture_geometry(ui.ctx(), s, &screen);
                }
            }
            self.main.show(ui, &screen, s, true);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn viewports(
        &mut self,
        ctx: &egui::Context,
        runtime: &crate::runtime::Runtime,
        s: &mut Session,
    ) {
        if let Some(watcher) = &mut self.watcher {
            watcher.poll(s, &mut self.main.error);
        }
        let set = s.screens().clone();
        self.windows
            .retain(|id, _| set.windows.iter().any(|s| s.id == *id));
        for window in set.windows.into_iter().filter(|w| w.id != 0) {
            let canvas = self.windows.entry(window.id).or_default().clone();
            if let Ok(mut c) = canvas.lock() {
                c.edit = self.main.edit;
                c.transfer = self.main.transfer.clone();
                c.chart_root = self.main.chart_root.clone();
                c.scenery_preferences = self.main.scenery_preferences.clone();
            }
            let source = runtime.source();
            let id = window.id;
            let mut builder = egui::ViewportBuilder::default()
                .with_title(&window.title)
                .with_inner_size(window.size)
                .with_decorations(!window.borderless)
                .with_fullscreen(window.fullscreen);
            if let Some(position) = window.position {
                builder = builder.with_position(position);
            }
            if window.position.is_none()
                && let Some(monitor) = window.monitor.and_then(|index| s.screens().monitors.get(index))
            {
                builder = builder.with_position(monitor.origin);
            }
            ctx.show_viewport_deferred(
                egui::ViewportId::from_hash_of(("flightcraft-screen", id)),
                builder,
                move |ctx, _class| {
                    if let Ok(mut session) = source.snapshot() {
                        if ctx.input(|i| i.viewport().close_requested()) {
                            let _ = session.execute("screen.close", json!({"id":id}));
                            return;
                        }
                        if let Some(screen) = session.screens().windows.iter().find(|w| w.id == id).cloned()
                            && let Ok(mut canvas) = canvas.lock()
                        {
                            capture_geometry(ctx, &mut session, &screen);
                            egui::CentralPanel::default()
                                .frame(egui::Frame::new().fill(theme::BACKGROUND))
                                .show(ctx, |ui| canvas.show(ui, &screen, &mut session, true));
                        }
                    }
                    ctx.request_repaint_after(std::time::Duration::from_secs_f64(1. / 60.));
                },
            );
        }
    }
}
/// Drag the inset by its body and resize it by the corner; it stays inside the outside view.
#[allow(clippy::too_many_arguments)]
fn inset_controls(
    error: &mut Option<String>,
    ui: &mut egui::Ui,
    s: &mut Session,
    screen: &Screen,
    element: &Element,
    rect: Rect,
    available: Rect,
    scale: [f32; 2],
) {
    let Some(band_element) = screen
        .layout
        .elements
        .iter()
        .find(|e| e.page.kind == PageKind::Outside && e.id != element.id)
    else {
        return;
    };
    let band = element_rect(screen, band_element, available, scale, s);
    if band.width() < 1. || band.height() < 1. {
        return;
    }
    let handle = Rect::from_min_size(rect.right_bottom() - egui::vec2(18., 18.), egui::vec2(18., 18.));
    let move_response = ui.interact(rect, ui.id().with("inset-move"), Sense::drag());
    let size_response = ui.interact(handle, ui.id().with("inset-size"), Sense::drag());
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            handle.right_bottom(),
            handle.right_top() + egui::vec2(0., 6.),
            handle.left_bottom() + egui::vec2(6., 0.),
        ],
        theme::ACCENT.gamma_multiply(0.8),
        Stroke::NONE,
    ));
    let mut p = s.view_preferences().placement;
    let (dx, dy) = (
        move_response.drag_delta().x / band.width(),
        move_response.drag_delta().y / band.height(),
    );
    let (rx, ry) = (
        size_response.drag_delta().x / band.width(),
        size_response.drag_delta().y / band.height(),
    );
    if size_response.dragged() {
        p.w += rx;
        p.h += ry;
    } else if move_response.dragged() {
        p.x += dx;
        p.y += dy;
    } else {
        return;
    }
    if let Err(e) = s.execute("view.exterior_inset", json!({"placement": p})) {
        *error = Some(e.to_string());
    }
}

/// The main window's layout for this frame: the ground arrangement while a checklist is open on
/// the ground, otherwise the stored layout with the exterior inset over the outside view when the
/// setting calls for it. The stored layout is never changed.
pub(crate) fn main_layout(ctx: &egui::Context, s: &Session, stored: &Layout, stacked: bool) -> Layout {
    let state = s.view_layout_state();
    if !state.ground {
        // No checklist is on screen, so no dash control is being pointed at.
        ctx.data_mut(|d| d.remove::<String>(egui::Id::new(crate::checklist::HIGHLIGHT_KEY)));
    }
    if state.ground {
        stored.ground(
            state.exterior_inset.mode != flightcraft_control::InsetMode::Off,
            stacked,
        )
    } else if state.exterior_inset.visible {
        stored.with_inset(state.exterior_inset.placement)
    } else {
        stored.clone()
    }
}
pub fn split_panel(panel: &flightcraft_aero::Panel, loads: &[flightcraft_sim::systems::LoadState]) -> Layout {
    let mut layout = Layout::single(Page::new(PageKind::Panel));
    layout.elements.clear();
    layout.name = "Split aircraft panel".into();
    if let Some(equipment) = &panel.equipment {
        layout.units = Units::Mm;
        layout.canvas = [equipment.size[0] as f32, equipment.size[1] as f32];
        for item in &equipment.elements {
            let mut page = Page::new(PageKind::PanelItem);
            page.item = Some(item.id.clone());
            let [x, y] = item.position;
            let [w, h] = item.size;
            layout.elements.push(Element::page(
                &item.id,
                page,
                [x as f32, y as f32],
                [w as f32, h as f32],
            ));
        }
        let console_y = equipment.size[1] as f32 + 8.;
        let mut throttle = Page::new(PageKind::Control);
        throttle.control.kind = ControlKind::Throttle;
        layout.elements.push(Element::page(
            "engine-throttle",
            throttle,
            [8., console_y],
            [60., 140.],
        ));
        layout.canvas[1] = console_y + 148.;
    } else {
        for (i, instrument) in panel.instruments.iter().enumerate() {
            let mut page = Page::new(match instrument.module.as_str() {
                "glass_pfd" => PageKind::Pfd,
                "glass_mfd" => PageKind::Mfd,
                _ => PageKind::Instrument,
            });
            page.instruments = vec![instrument.id.clone()];
            layout.elements.push(Element::page(
                &instrument.id,
                page,
                [(i % 4) as f32 * 310., (i / 4) as f32 * 250.],
                [300., 240.],
            ));
        }
        let bottom = ((panel.instruments.len().div_ceil(4)) as f32 * 250.).max(800.);
        for (i, kind) in [
            ControlKind::Throttle,
            ControlKind::Mixture,
            ControlKind::CarbHeat,
            ControlKind::Flaps,
            ControlKind::Trim,
            ControlKind::FuelSelector,
            ControlKind::ParkingBrake,
            ControlKind::Magnetos,
            ControlKind::Primer,
        ]
        .into_iter()
        .enumerate()
        {
            let mut page = Page::new(PageKind::Control);
            page.control.kind = kind;
            layout.elements.push(Element::page(
                &format!("control-{kind:?}"),
                page,
                [i as f32 * 140., bottom],
                [130., 280.],
            ));
        }
        for (i, item) in [
            "master",
            "avionics",
            "alternator",
            "lights",
            "pitot_heat",
            "fuel_pump",
            "carb_heat",
            "alternate_static",
            "starter",
        ]
        .into_iter()
        .enumerate()
        {
            let mut page = Page::new(PageKind::Switch);
            page.item = Some(item.into());
            layout.elements.push(Element::page(
                item,
                page,
                [i as f32 * 140., bottom + 290.],
                [132., 70.],
            ));
        }
        for (i, kind) in [
            ControlKind::Heading,
            ControlKind::Baro,
            ControlKind::Altitude,
            ControlKind::Range,
            ControlKind::Course,
            ControlKind::Fms,
        ]
        .into_iter()
        .enumerate()
        {
            let mut page = Page::new(PageKind::Control);
            page.control.kind = kind;
            layout.elements.push(Element::page(
                &format!("knob-{kind:?}"),
                page,
                [i as f32 * 150., bottom + 370.],
                [140., 85.],
            ));
        }
        for (i, load) in loads.iter().enumerate() {
            let mut page = Page::new(PageKind::Breaker);
            page.item = Some(load.id.clone());
            layout.elements.push(Element::page(
                &format!("breaker-{}", load.id),
                page,
                [(i % 8) as f32 * 156., bottom + 470. + (i / 8) as f32 * 80.],
                [148., 70.],
            ));
        }
        layout.canvas = [1280., bottom + 480. + loads.len().div_ceil(8) as f32 * 80.];
    }
    layout
}

#[cfg(not(target_arch = "wasm32"))]
fn capture_geometry(ctx: &egui::Context, s: &mut Session, screen: &Screen) {
    let viewport = ctx.input(|i| i.viewport().clone());
    if viewport.minimized == Some(true) || viewport.fullscreen == Some(true) {
        return;
    }
    if let Some(rect) = viewport.inner_rect {
        let size = rect.size();
        let [w, h] = screen.size;
        let position = viewport.outer_rect.map(|r| [r.min.x, r.min.y]);
        if size.x >= 160.
            && size.y >= 160.
            && ((size.x - w).abs() > 2.
                || (size.y - h).abs() > 2.
                || position.is_some_and(|p| {
                    screen
                        .position
                        .is_none_or(|old| old.into_iter().zip(p).any(|(a, b)| (a - b).abs() > 2.))
                }))
        {
            let _ = s.execute(
                "screen.configure",
                json!({"id":screen.id,"size":[size.x,size.y],"position":position}),
            );
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod preview;

impl PageUi {
    fn canopy_controls(&mut self, ui: &mut egui::Ui, s: &mut Session) {
        use crate::canopy_panel::{CanopyPanel, position_text, visibility};
        let canopy = s.sim().canopy();
        let now = ui.ctx().input(|i| i.time);
        self.canopy_reveal
            .observe(now, canopy.latched, canopy.open_fraction);
        let mut action = None;
        if let Some(point) = self.scene.canopy_latch_screen() {
            let response = ui.interact(
                Rect::from_center_size(point, egui::vec2(36., 32.)),
                ui.id().with("inside-canopy-latch"),
                Sense::click(),
            );
            if response.clicked() {
                action = Some(if canopy.latched { "unlatch" } else { "latch" });
            }
            if response.hovered() || response.clicked() {
                self.canopy_reveal.touch(now);
            }
            response.on_hover_text("Inside canopy latch · top centre of roll-over bar");
        }
        // Retain pointer capture even after the moving bow passes behind the pilot.
        if !ui.input(|i| i.pointer.primary_down()) || self.canopy_drag_target.is_none() {
            self.canopy_drag_target = self.scene.canopy_drag_screen();
        }
        if let Some((point, aft)) = self.canopy_drag_target {
            let response = ui.interact(
                Rect::from_center_size(point, egui::vec2(56., 38.)),
                ui.id().with("canopy-front-bow"),
                Sense::drag(),
            );
            if response.hovered() || response.dragged() {
                self.canopy_reveal.touch(now);
            }
            if response.drag_started() {
                self.canopy_drag_origin = canopy.open_fraction;
            }
            if response.dragged() {
                let fraction = (self.canopy_drag_origin + f64::from(response.drag_delta().dot(aft) / 200.))
                    .clamp(0., 1.);
                self.canopy_message =
                    match s.execute("aircraft.canopy", json!({"action":"open","fraction":fraction})) {
                        Ok(_) => String::new(),
                        Err(e) => e.to_string(),
                    };
            }
            response.on_hover_text("Drag the canopy frame along its rails; release the latch first");
        }
        // The overhead control is only drawn when it is useful: on the ground, while the latch or
        // frame is hovered or clicked, or briefly after a canopy change.
        let rect = Rect::from_min_size(ui.max_rect().min + egui::vec2(12., 12.), egui::vec2(340., 118.));
        let panel_hovered = ui
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|p| rect.contains(p));
        let on_ground = s.sim().readout().on_ground;
        let mut shown = visibility(on_ground, canopy.latched, self.canopy_reveal.remaining_s(now));
        if panel_hovered && !on_ground && matches!(shown, CanopyPanel::Controls(_)) {
            self.canopy_reveal.touch(now);
            shown = CanopyPanel::Controls(1.);
        }
        match shown {
            CanopyPanel::Hidden => {}
            CanopyPanel::Unlatched => {
                ui.painter().text(
                    ui.max_rect().min + egui::vec2(14., 14.),
                    Align2::LEFT_TOP,
                    "CANOPY UNLATCHED",
                    FontId::monospace(13.),
                    theme::AMBER,
                );
            }
            CanopyPanel::Controls(opacity) => {
                if opacity < 1. {
                    ui.ctx().request_repaint();
                }
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    ui.set_opacity(opacity);
                    egui::Frame::new()
                        .fill(theme::PANEL)
                        .inner_margin(8.)
                        .show(ui, |ui| {
                            ui.label("Roll-over bar · canopy");
                            ui.horizontal(|ui| {
                                if ui
                                    .button(if canopy.latched { "Unlatch" } else { "Latch" })
                                    .clicked()
                                {
                                    action = Some(if canopy.latched { "unlatch" } else { "latch" });
                                }
                                if ui.button("Close").clicked() {
                                    action = Some("close");
                                }
                                ui.label(if canopy.latched { "Latched" } else { "Unlatched" });
                                ui.label(position_text(canopy.open_fraction));
                            });
                            let mut fraction = canopy.open_fraction;
                            if ui
                                .add_enabled(
                                    !canopy.latched,
                                    egui::Slider::new(&mut fraction, 0. ..=1.)
                                        .show_value(false)
                                        .text("Slide aft"),
                                )
                                .changed()
                            {
                                self.canopy_message = match s
                                    .execute("aircraft.canopy", json!({"action":"open","fraction":fraction}))
                                {
                                    Ok(_) => String::new(),
                                    Err(e) => e.to_string(),
                                };
                            }
                            ui.colored_label(
                                if canopy.secure() {
                                    theme::TEXT
                                } else {
                                    theme::AMBER
                                },
                                if canopy.secure() {
                                    "Canopy closed and latched"
                                } else {
                                    "Close and latch the canopy before takeoff"
                                },
                            );
                            if !self.canopy_message.is_empty() {
                                ui.colored_label(theme::AMBER, &self.canopy_message);
                            }
                        });
                });
            }
        }
        if let Some(action) = action {
            self.canopy_message = match s.execute("aircraft.canopy", json!({"action":action})) {
                Ok(_) => String::new(),
                Err(e) => e.to_string(),
            };
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod interaction_tests {
    use super::*;
    #[test]
    fn aircraft_defined_plungers_have_slop_and_move_with_two_contacts() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut session = Session::new().unwrap();
        let mut canvas = Canvas::default();
        let mut layout = Layout::single(Page::new(PageKind::Control));
        layout.canvas = [400., 600.];
        layout.elements.clear();
        for (id, x, kind) in [
            ("throttle", 0., ControlKind::Throttle),
            ("mixture", 200., ControlKind::Mixture),
        ] {
            let mut page = Page::new(PageKind::Control);
            page.control.kind = kind;
            layout
                .elements
                .push(Element::page(id, page, [x, 0.], [200., 600.]));
        }
        let screen = Screen {
            id: 0,
            title: "Touch test".into(),
            monitor: None,
            position: None,
            size: [400., 600.],
            fullscreen: false,
            borderless: false,
            layout,
        };
        let touch = |id, phase, x, y| egui::Event::Touch {
            device_id: egui::TouchDeviceId(1),
            id: egui::TouchId(id),
            phase,
            pos: egui::pos2(x, y),
            force: None,
        };
        let mut draw = |events| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400., 600.))),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ctx, |ui| canvas.show(ui, &screen, &mut session, true));
                },
            );
        };
        draw(vec![]);
        draw(vec![
            touch(1, egui::TouchPhase::Start, 100., 524.),
            touch(2, egui::TouchPhase::Start, 300., 76.),
        ]);
        draw(vec![
            touch(1, egui::TouchPhase::Move, 100., 522.),
            touch(2, egui::TouchPhase::Move, 300., 78.),
        ]);
        draw(vec![
            touch(1, egui::TouchPhase::Move, 100., 324.),
            touch(2, egui::TouchPhase::Move, 300., 276.),
        ]);
        assert!((session.sim().controls().throttle - 200. / 448.).abs() < 0.001);
        assert!((session.sim().systems().controls.mixture - (1. - 200. / 448.)).abs() < 0.001);
        assert!(canvas.error.is_none(), "{:?}", canvas.error);
    }
    #[test]
    fn every_page_owns_a_dark_fill_even_under_a_light_parent() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut session = Session::new().unwrap();
        for kind in PageKind::ALL {
            ctx.set_visuals(egui::Visuals::light());
            let mut page = PageUi::default();
            let out = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 800.))),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let _ = page.show(ui, &Page::new(kind), &mut session, "contrast");
                        assert_eq!(ui.visuals().panel_fill, theme::PANEL, "{kind:?}");
                        assert_eq!(ui.visuals().override_text_color, Some(theme::TEXT), "{kind:?}");
                    });
                },
            );
            assert!(
                out.shapes
                    .iter()
                    .any(|shape| matches!(&shape.shape,egui::Shape::Rect(r) if r.fill==theme::PANEL)),
                "{kind:?}"
            );
        }
        use flightcraft_aero::panel_fidelity::SrgbHex;
        let hex = |c: egui::Color32| SrgbHex(format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b()));
        // Exercise populated IFR content too: empty pages cannot catch unstyled headings/notes.
        ctx.set_visuals(egui::Visuals::light());
        let out = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                crate::ifr::debrief(
                    ui,
                    &flightcraft_control::LessonDebrief {
                        title: Some("Instrument approach debrief".into()),
                        ..Default::default()
                    },
                );
                assert_eq!(ui.visuals().panel_fill, theme::PANEL);
                assert_eq!(ui.visuals().override_text_color, Some(theme::TEXT));
            });
        });
        assert!(
            out.shapes
                .iter()
                .any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.fill == theme::PANEL))
        );
        for shape in &out.shapes {
            if let egui::Shape::Text(text) = &shape.shape {
                for section in &text.galley.job.sections {
                    assert!(hex(section.format.color).contrast(&hex(theme::PANEL)).unwrap() >= 4.5);
                }
            }
        }
        for background in [theme::PANEL, theme::RAISED, theme::DIAL] {
            for foreground in [
                theme::TEXT,
                theme::MUTED,
                theme::ACCENT,
                theme::GREEN,
                theme::AMBER,
                theme::DANGER,
            ] {
                assert!(
                    hex(foreground).contrast(&hex(background)).unwrap() >= 4.5,
                    "{foreground:?} on {background:?}"
                );
            }
        }
        for background in [theme::SKY, theme::EARTH, theme::BORDER] {
            assert!(hex(theme::TEXT).contrast(&hex(background)).unwrap() >= 4.5);
        }
        for background in [
            theme::MAP_DAY_LAND,
            theme::MAP_DAY_HIGH,
            theme::MAP_DAY_WATER,
            theme::MAP_DAY_OUTSIDE,
        ] {
            assert!(hex(theme::MAP_DAY_TEXT).contrast(&hex(background)).unwrap() >= 4.5);
        }
    }
    #[test]
    fn editor_drag_moves_and_resizes_once_with_undo() {
        let mut s = Session::new().unwrap();
        let mut layout = Layout::single(Page::new(PageKind::Control));
        layout.canvas = [600., 400.];
        if let Some(e) = layout.elements.first_mut() {
            e.position = [40., 40.];
            e.size = [160., 240.];
        }
        s.execute("layout.set", json!({"id":0,"layout":layout})).unwrap();
        let ctx = egui::Context::default();
        let mut canvas = Canvas {
            edit: true,
            ..Default::default()
        };
        let mut draw = |events: Vec<egui::Event>| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600., 400.))),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ctx, |ui| {
                            let screen = s.screens().windows.first().unwrap().clone();
                            canvas.show(ui, &screen, &mut s, true);
                        });
                },
            );
        };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        draw(vec![]);
        let start = egui::pos2(80., 70.);
        draw(vec![egui::Event::PointerMoved(start), button(start, true)]);
        let next = egui::pos2(120., 102.);
        draw(vec![egui::Event::PointerMoved(next)]);
        let end = egui::pos2(144., 118.);
        draw(vec![egui::Event::PointerMoved(end)]);
        draw(vec![button(end, false)]);
        let moved = s
            .screens()
            .windows
            .first()
            .unwrap()
            .layout
            .elements
            .first()
            .unwrap()
            .position;
        assert_ne!(moved, [40., 40.]);
        let [x, y] = moved;
        assert!(x >= 80. && y >= 64., "{moved:?}");
        s.execute("layout.undo", json!({})).unwrap();
        assert_eq!(
            s.screens()
                .windows
                .first()
                .unwrap()
                .layout
                .elements
                .first()
                .unwrap()
                .position,
            [40., 40.]
        );
    }
    #[test]
    fn glass_softkey_tap_selects_nearest_and_knob_push_selects_altitude() {
        let mut session = Session::new().unwrap();
        let mut page = PageUi::default();
        let ctx = egui::Context::default();
        let button = |pressed| egui::Event::PointerButton {
            pos: egui::pos2(130., 758.),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        for events in [
            vec![],
            vec![egui::Event::PointerMoved(egui::pos2(130., 758.)), button(true)],
            vec![button(false)],
        ] {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640., 800.))),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        page.show(ui, &Page::new(PageKind::Pfd), &mut session, "pfd")
                            .unwrap();
                    });
                },
            );
        }
        assert_eq!(
            session.glass_state().settings.page,
            flightcraft_control::GlassPage::Nearest
        );
        let (name, input) = touch::knob_push(ControlKind::Altitude, 1000.);
        session.execute(name, input).unwrap();
        assert_eq!(session.glass_state().settings.selected_altitude_m, 1000.);
    }
    #[test]
    fn disabled_touch_page_cannot_mutate_even_its_local_copy() {
        let mut s = Session::new().unwrap();
        let mut page = PageUi::default();
        let ctx = egui::Context::default();
        let initial = s.sim().controls();
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200., 400.))),
                events: vec![
                    egui::Event::Touch {
                        device_id: egui::TouchDeviceId(1),
                        id: egui::TouchId(1),
                        phase: egui::TouchPhase::Start,
                        pos: egui::pos2(70., 300.),
                        force: None,
                    },
                    egui::Event::Touch {
                        device_id: egui::TouchDeviceId(1),
                        id: egui::TouchId(1),
                        phase: egui::TouchPhase::Move,
                        pos: egui::pos2(70., 100.),
                        force: None,
                    },
                ],
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.disable();
                    page.show(ui, &Page::new(PageKind::Control), &mut s, "throttle")
                        .unwrap();
                });
            },
        );
        assert_eq!(initial, s.sim().controls());
    }
    #[test]
    fn splitting_the_legacy_panel_keeps_each_control_and_breaker_separate() {
        let session = Session::new().unwrap();
        let panel = &session.aircraft_configuration().unwrap().panel;
        let loads = &session.sim().systems().electrical.loads;
        let layout = split_panel(panel, loads);
        layout.validate().unwrap();
        for load in loads {
            assert!(
                layout
                    .elements
                    .iter()
                    .any(|e| { e.page.kind == PageKind::Breaker && e.page.item.as_ref() == Some(&load.id) })
            );
        }
        for kind in [
            ControlKind::Throttle,
            ControlKind::Mixture,
            ControlKind::Magnetos,
            ControlKind::Primer,
            ControlKind::Trim,
        ] {
            assert!(
                layout
                    .elements
                    .iter()
                    .any(|e| e.page.kind == PageKind::Control && e.page.control.kind == kind)
            );
        }
    }
}

#[cfg(test)]
mod camera_tests;
