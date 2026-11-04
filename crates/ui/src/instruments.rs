//! Instrument modules emit draw lists in local coordinates, with explicit canvas dimensions.
//! Skins consume only that list and theme tokens; panel actions return command inputs.
mod avionics_controls;
mod behaviour;
mod electronic;
pub(crate) mod formatting;
pub(crate) mod heading;
mod options;
mod registration;
pub use options::{DisplayOptions, EngineUnits, PressureUnit};
mod switches;

use crate::theme as t;
use eframe::egui::{self, Align2, FontId, Painter, Pos2, Rect, Sense, Shape, Stroke};
use flightcraft_aero::{InstrumentInstallation, Panel};
use flightcraft_sim::systems::{SystemSetting, SystemsState};
use serde::Deserialize;
use std::collections::BTreeMap;

pub use avionics_controls::{
    shortcut as avionics_shortcut, shortcut_with_options as avionics_shortcut_with_options,
};
pub use switches::{shortcut, shortcut_with_options};

#[derive(Clone, Copy)]
pub struct InstrumentInput<'a> {
    pub vision: Option<(&'a flightcraft_world::World, &'a flightcraft_sim::State)>,
    pub systems: &'a SystemsState,
    pub glass: Option<&'a flightcraft_control::GlassState>,
    pub autopilot: Option<&'a flightcraft_sim::equipment::EquipmentState>,
    /// Simulation time only: paused redraws consume no needle time.
    pub time_s: f64,
    pub options: DisplayOptions,
    pub airports: Option<&'a flightcraft_world::Airports>,
    pub aircraft: Option<&'a flightcraft_aero::ResolvedAircraft>,
    /// Current registration (the pilot's override, else the pack's) for `{registration}` placards.
    pub registration: Option<&'a str>,
}
impl<'a> InstrumentInput<'a> {
    pub fn new(systems: &'a SystemsState, time_s: f64) -> Self {
        Self {
            vision: None,
            systems,
            glass: None,
            autopilot: None,
            time_s,
            options: DisplayOptions::default(),
            aircraft: None,
            airports: None,
            registration: None,
        }
    }
    pub fn with_registration(mut self, registration: &'a str) -> Self {
        self.registration = Some(registration);
        self
    }
    pub fn with_vision(
        mut self,
        world: &'a flightcraft_world::World,
        state: &'a flightcraft_sim::State,
    ) -> Self {
        self.vision = Some((world, state));
        self
    }
    pub fn with_airports(mut self, airports: Option<&'a flightcraft_world::Airports>) -> Self {
        self.airports = airports;
        self
    }
    pub fn with_glass(mut self, glass: &'a flightcraft_control::GlassState) -> Self {
        self.glass = Some(glass);
        self
    }
    pub fn with_autopilot(mut self, state: Option<&'a flightcraft_sim::equipment::EquipmentState>) -> Self {
        self.autopilot = state;
        self
    }
    pub fn with_aircraft(mut self, aircraft: &'a flightcraft_aero::ResolvedAircraft) -> Self {
        self.aircraft = Some(aircraft);
        self
    }
}

pub trait Instrument: Send {
    fn update(&mut self, input: InstrumentInput<'_>) -> DrawList;
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Ink {
    Face,
    Text,
    Muted,
    Needle,
    Warning,
    Alert,
    Sky,
    Earth,
    Normal,
    Inactive,
    Overlay,
    Gps,
    AirspaceBlue,
    TerrainHigh,
    TerrainLow,
}

/// Original theme tokens are also the vocabulary available to data-defined private skins.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Token {
    Dial,
    Text,
    Muted,
    Accent,
    Amber,
    Danger,
    Sky,
    Earth,
    Raised,
    Border,
    Green,
}
impl Token {
    fn color(self) -> egui::Color32 {
        match self {
            Self::Dial => t::DIAL,
            Self::Text => t::TEXT,
            Self::Muted => t::MUTED,
            Self::Accent => t::ACCENT,
            Self::Amber => t::AMBER,
            Self::Danger => t::DANGER,
            Self::Sky => t::SKY,
            Self::Earth => t::EARTH,
            Self::Raised => t::RAISED,
            Self::Border => t::BORDER,
            Self::Green => t::GREEN,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DrawItem {
    /// A uniformly scaled symbol cluster anchored within a resizable display.
    #[serde(skip)]
    Group {
        center: [f32; 2],
        items: Vec<DrawItem>,
    },
    #[serde(skip)]
    SyntheticVision {
        min: [f32; 2],
        max: [f32; 2],
        view: Box<flightcraft_render::SyntheticVision>,
    },
    #[serde(skip)]
    Mesh {
        vertices: Vec<([f32; 2], Ink, Ink, f32)>,
        indices: Vec<u32>,
    },
    Circle {
        center: [f32; 2],
        radius: f32,
        ink: Ink,
    },
    Line {
        a: [f32; 2],
        b: [f32; 2],
        width: f32,
        ink: Ink,
    },
    Polygon {
        points: Vec<[f32; 2]>,
        ink: Ink,
    },
    Label {
        at: [f32; 2],
        text: String,
        size: f32,
        ink: Ink,
    },
}
#[derive(Clone, Debug)]
pub struct DrawList {
    /// Local canvas dimensions; compact system gauges use a wide, shallow canvas.
    pub size: [f32; 2],
    pub items: Vec<DrawItem>,
    pub indicated: f64,
    pub valid: bool,
}
impl DrawList {
    #[cfg(test)]
    pub(crate) fn flat_items(&self) -> Vec<&DrawItem> {
        fn collect<'a>(items: &'a [DrawItem], out: &mut Vec<&'a DrawItem>) {
            for item in items {
                if let DrawItem::Group { items, .. } = item {
                    collect(items, out);
                } else {
                    out.push(item);
                }
            }
        }
        let mut out = Vec::new();
        collect(&self.items, &mut out);
        out
    }

    fn circle(&mut self, center: [f32; 2], radius: f32, ink: Ink) {
        self.items.push(DrawItem::Circle { center, radius, ink });
    }
    fn line(&mut self, a: [f32; 2], b: [f32; 2], ink: Ink, width: f32) {
        self.items.push(DrawItem::Line { a, b, ink, width });
    }
    fn label(&mut self, at: [f32; 2], text: impl ToString, size: f32, ink: Ink) {
        self.items.push(DrawItem::Label {
            at,
            text: formatting::clean_zero(&text.to_string()),
            size,
            ink,
        });
    }
}

pub trait Skin: Send {
    fn paint(&self, painter: &Painter, draw: &DrawList);
}

/// A pack host can register this bounded, declarative artwork without engine code changes.
/// Coordinates use the instrument's local space; colours are always theme tokens.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkinPack {
    pub id: String,
    #[serde(default)]
    pub colors: Vec<(Ink, Token)>,
    /// Optional original or privately supplied face art, beneath dynamic items.
    #[serde(default)]
    pub face: Vec<DrawItem>,
}
impl Skin for SkinPack {
    fn paint(&self, p: &Painter, draw: &DrawList) {
        let color = |ink| {
            self.colors.iter().find(|(role, _)| *role == ink).map_or_else(
                || match ink {
                    Ink::Face => t::DIAL,
                    Ink::Text => t::TEXT,
                    Ink::Muted => t::MUTED,
                    Ink::Needle => t::ACCENT,
                    Ink::Warning => t::AMBER,
                    Ink::Alert => t::DANGER,
                    Ink::Sky => t::SKY,
                    Ink::Earth => t::EARTH,
                    Ink::Normal => t::GREEN,
                    Ink::Inactive => t::BORDER,
                    Ink::Overlay => t::DIAL.gamma_multiply(0.72),
                    Ink::Gps => t::GLASS_GPS,
                    Ink::AirspaceBlue => t::AIRSPACE_BLUE,
                    Ink::TerrainHigh => t::TERRAIN_HIGH,
                    Ink::TerrainLow => t::TERRAIN_LOW,
                },
                |(_, token)| token.color(),
            )
        };
        fn paint_item(p: &Painter, item: &DrawItem, color: &impl Fn(Ink) -> egui::Color32) {
            match item {
                DrawItem::Group { items, .. } => {
                    for child in items {
                        paint_item(p, child, color);
                    }
                }
                DrawItem::SyntheticVision { min, max, view } => {
                    view.paint(p, Rect::from_min_max((*min).into(), (*max).into()))
                }
                DrawItem::Mesh { vertices, indices } => {
                    let mut mesh = egui::Mesh::default();
                    for (at, a, b, blend) in vertices {
                        mesh.colored_vertex(Pos2::from(*at), color(*a).lerp_to_gamma(color(*b), *blend));
                    }
                    mesh.indices = indices.clone();
                    p.add(Shape::mesh(mesh));
                }
                DrawItem::Circle { center, radius, ink } => {
                    p.circle_filled(Pos2::from(*center), *radius, color(*ink));
                }
                DrawItem::Line { a, b, width, ink } => {
                    p.line_segment([Pos2::from(*a), Pos2::from(*b)], Stroke::new(*width, color(*ink)));
                }
                DrawItem::Polygon { points, ink } => {
                    p.add(Shape::convex_polygon(
                        points.iter().copied().map(Pos2::from).collect(),
                        color(*ink),
                        Stroke::NONE,
                    ));
                }
                DrawItem::Label { at, text, size, ink } => {
                    p.text(
                        Pos2::from(*at),
                        Align2::CENTER_CENTER,
                        text,
                        FontId::monospace(*size),
                        color(if matches!(ink, Ink::Inactive) {
                            Ink::Muted
                        } else {
                            *ink
                        }),
                    );
                }
            }
        }
        for item in
            self.face.iter().chain(draw.items.iter().filter(|item| {
                self.face.is_empty() || !matches!(item, DrawItem::Circle { ink: Ink::Face, .. })
            }))
        {
            paint_item(p, item, &color);
        }
    }
}

type Factory = fn(&str) -> Result<Box<dyn Instrument>, String>;
/// Read-only equipment and sensed flight state passed to artwork painters.
#[derive(Clone, Copy)]
pub struct EquipmentSnapshot<'a> {
    pub installation: &'a flightcraft_aero::modern::EquipmentInstallation,
    pub input: InstrumentInput<'a>,
}
/// Paint artwork only. The host retains all command hit regions and hold ownership.
pub trait EquipmentSkin: Send {
    fn paint(&self, painter: &Painter, rect: Rect, snapshot: EquipmentSnapshot<'_>);
}
pub struct Registry {
    equipment_skins: BTreeMap<String, Box<dyn EquipmentSkin>>,
    equipment_installations: BTreeMap<String, String>,
    modules: BTreeMap<String, Factory>,
    skins: BTreeMap<String, Box<dyn Skin>>,
}
impl Default for Registry {
    fn default() -> Self {
        let mut r = Self {
            equipment_skins: BTreeMap::new(),
            equipment_installations: BTreeMap::new(),
            modules: BTreeMap::new(),
            skins: BTreeMap::new(),
        };
        for id in flightcraft_aero::INSTRUMENT_MODULES {
            r.modules.insert(
                (*id).into(),
                if id.starts_with("electronic_") {
                    electronic::create
                } else if id.starts_with("glass_") {
                    crate::glass::create
                } else if *id == "registration" {
                    registration::create
                } else {
                    behaviour::create
                },
            );
        }
        r.skins.insert(
            "original".into(),
            Box::new(SkinPack {
                id: "original".into(),
                colors: Vec::new(),
                face: Vec::new(),
            }),
        );
        r
    }
}
impl Registry {
    pub fn register_equipment_skin(
        &mut self,
        id: String,
        skin: Box<dyn EquipmentSkin>,
    ) -> Result<(), String> {
        if !valid_id(&id) || id == "original" || self.equipment_skins.contains_key(&id) {
            return Err("Invalid or duplicate equipment skin id".into());
        }
        self.equipment_skins.insert(id, skin);
        Ok(())
    }
    /// Resolve artwork by installation id; commands always remain in the host.
    pub fn set_equipment_skin(&mut self, installation: String, skin: String) -> Result<(), String> {
        if !valid_id(&installation) || (skin != "original" && !self.equipment_skins.contains_key(&skin)) {
            return Err("Invalid installation or unknown equipment skin".into());
        }
        self.equipment_installations.insert(installation, skin);
        Ok(())
    }
    pub(crate) fn equipment_skin(
        &self,
        installation: &flightcraft_aero::modern::EquipmentInstallation,
    ) -> Option<&dyn EquipmentSkin> {
        self.equipment_installations
            .get(&installation.id)
            .or(installation.appearance.skin.as_ref())
            .and_then(|id| self.equipment_skins.get(id))
            .map(Box::as_ref)
    }
    pub fn instrument(&self, id: &str) -> Result<Box<dyn Instrument>, String> {
        self.modules
            .get(id)
            .ok_or_else(|| format!("Unknown instrument module: {id}"))?(id)
    }
    pub fn register_skin(&mut self, id: String, skin: Box<dyn Skin>) -> Result<(), String> {
        if !valid_id(&id) || self.skins.contains_key(&id) {
            return Err("Invalid or duplicate skin id".into());
        }
        self.skins.insert(id, skin);
        Ok(())
    }
    pub fn register_pack_skin(&mut self, json: &str) -> Result<(), String> {
        if json.len() > 65536 {
            return Err("Skin exceeds 64 KiB".into());
        }
        let skin: SkinPack = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if skin.colors.len() > 10 || skin.face.len() > 256 || !skin.face.iter().all(valid_item) {
            return Err("Skin artwork exceeds local bounds or limits".into());
        }
        self.register_skin(skin.id.clone(), Box::new(skin))
    }
    /// Validate all ids before replacing a loaded panel, so a failed load is atomic.
    pub fn validate(&self, panel: &Panel) -> Result<(), String> {
        flightcraft_aero::LayerFile::Panel(panel.clone())
            .validate()
            .map_err(|e| e.to_string())?;
        if let Some(equipment) = &panel.equipment {
            for e in &equipment.elements {
                let selected = self
                    .equipment_installations
                    .get(&e.id)
                    .or(e.appearance.skin.as_ref());
                if selected.is_some_and(|id| id != "original") && self.equipment_skin(e).is_none() {
                    return Err(format!("Unknown equipment skin for {}", e.id));
                }
            }
        }
        for i in &panel.instruments {
            if !self.modules.contains_key(&i.module) {
                return Err(format!("Unknown instrument module: {}", i.module));
            }
            if !self.skins.contains_key(&i.skin) {
                return Err(format!("Unknown instrument skin: {}", i.skin));
            }
        }
        Ok(())
    }
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}
fn valid_item(item: &DrawItem) -> bool {
    let point = |p: &[f32; 2]| {
        let [x, y] = *p;
        x.is_finite() && y.is_finite() && (0. ..=320.).contains(&x) && (0. ..=180.).contains(&y)
    };
    match item {
        DrawItem::Group { .. } | DrawItem::Mesh { .. } | DrawItem::SyntheticVision { .. } => false,
        DrawItem::Circle { center, radius, .. } => {
            let [x, y] = *center;
            point(center)
                && radius.is_finite()
                && *radius > 0.
                && x - radius >= 0.
                && x + radius <= 320.
                && y - radius >= 0.
                && y + radius <= 180.
        }
        DrawItem::Line { a, b, width, .. } => {
            point(a) && point(b) && width.is_finite() && (0.1..=10.).contains(width)
        }
        DrawItem::Polygon { points, .. } => (3..=128).contains(&points.len()) && points.iter().all(point),
        DrawItem::Label { at, text, size, .. } => {
            point(at) && text.len() <= 128 && size.is_finite() && (4. ..=24.).contains(size)
        }
    }
}

#[derive(Default)]
pub struct InstrumentPanel {
    pub standalone: bool,
    pub fit_all: bool,
    pub(crate) dash_only: bool,
    modern: crate::modern::PanelUi,
    pub registry: Registry,
    pub options: DisplayOptions,
    pub starter_key_held: bool,
    starter_hold: switches::StarterHold,
    pub avionics_controls: avionics_controls::Controls,
    loaded: Option<Panel>,
    modules: Vec<Box<dyn Instrument>>,
    generation: Option<u64>,
    heading_knobs: BTreeMap<String, heading::Knob>,
    heading_commands: Vec<flightcraft_sim::avionics::AvionicsSetting>,
    glass_commands: Vec<flightcraft_control::GlassSetting>,
    map_rect: Option<Rect>,
}
impl InstrumentPanel {
    pub(crate) fn cancel_equipment_holds(
        &mut self,
        generation: u64,
    ) -> Vec<flightcraft_control::EquipmentSetting> {
        let commands = self.modern.cancel_holds();
        if self.generation == Some(generation) {
            commands
        } else {
            Vec::new()
        }
    }
    pub fn map_rect(&self) -> Option<Rect> {
        self.map_rect
    }
    pub fn show_shared_hosts(&mut self, ui: &mut egui::Ui, session: &mut flightcraft_control::Session) {
        for (id, viewport) in &self.modern.maps {
            let map = self.modern.map_widgets.entry(id.clone()).or_default();
            map.options = self.options;
            ui.push_id(id, |ui| {
                map.show_mfd_base(ui, viewport.rect, session, viewport.base)
            });
        }
        if let Some(rect) = self.modern.procedure_rect {
            self.heading_commands
                .extend(self.modern.procedures.show_inline(ui, rect, session));
        }
    }
    pub fn take_equipment_commands(&mut self) -> Vec<flightcraft_control::EquipmentSetting> {
        std::mem::take(&mut self.modern.commands)
    }
    pub fn take_heading_commands(&mut self) -> Vec<flightcraft_sim::avionics::AvionicsSetting> {
        std::mem::take(&mut self.heading_commands)
    }
    pub fn take_glass_commands(&mut self) -> Vec<flightcraft_control::GlassSetting> {
        std::mem::take(&mut self.glass_commands)
    }
    pub fn with_options(options: DisplayOptions) -> Self {
        Self {
            options,
            ..Default::default()
        }
    }
    /// Fixed-mass files have no panel layer; keep the standard six sensed instruments.
    pub fn show_legacy(
        &mut self,
        ui: &mut egui::Ui,
        input: InstrumentInput<'_>,
        generation: u64,
    ) -> Result<Vec<SystemSetting>, String> {
        let panel = Panel {
            touch_controls: Vec::new(),
            audio_panel: Default::default(),
            equipment: None,
            autopilot: None,
            format_version: 2,
            identity: flightcraft_aero::Identity {
                id: "legacy-panel".into(),
                display_name: "Legacy panel".into(),
                description: "Standard sensed instruments for fixed-mass aircraft.".into(),
                author: "FlightCraft contributors".into(),
                registration: String::new(),
                spoken_type: String::new(),
            },
            instruments: flightcraft_aero::INSTRUMENT_MODULES
                .iter()
                .take(6)
                .zip(0_u32..)
                .map(|(id, index)| InstrumentInstallation {
                    id: (*id).into(),
                    module: (*id).into(),
                    skin: "original".into(),
                    position: [f64::from(index % 3), f64::from(index / 3)],
                    size: [1., 1.],
                })
                .collect(),
        };
        self.show(ui, &panel, input, generation)
    }
    pub fn load(&mut self, panel: &Panel) -> Result<(), String> {
        self.registry.validate(panel)?;
        let modules = panel
            .instruments
            .iter()
            .map(|i| self.registry.instrument(&i.module))
            .collect::<Result<Vec<_>, _>>()?;
        self.modules = modules;
        self.loaded = Some(panel.clone());
        Ok(())
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        panel: &Panel,
        mut input: InstrumentInput<'_>,
        generation: u64,
    ) -> Result<Vec<SystemSetting>, String> {
        input.options = self.options;
        self.map_rect = None;
        if self.loaded.as_ref() != Some(panel) || self.generation != Some(generation) {
            self.load(panel)?;
            self.modern = crate::modern::PanelUi::default();
            self.generation = Some(generation);
        }
        if let Some(equipment) = &panel.equipment {
            self.modern.dash_only = self.dash_only;
            let settings = self.modern.show(ui, equipment, input, &self.registry);
            self.heading_commands.append(&mut self.modern.avionics);
            return Ok(settings);
        }
        let mut settings = if input.systems.enabled && !self.standalone {
            let (mut settings, pointer_held) = switches::show(ui, input.systems);
            settings.extend(
                self.starter_hold
                    .update(input.systems, pointer_held || self.starter_key_held),
            );
            settings
        } else {
            Vec::new()
        };
        let (rect, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
        let rect = if self.standalone || self.fit_all {
            let [width, height] = panel_dimensions(panel);
            fit_size(rect, [width as f32, height as f32])
        } else {
            rect
        };
        let cells = layout(panel, rect);
        for ((installation, module), cell) in panel.instruments.iter().zip(&mut self.modules).zip(cells) {
            let mut draw = module.update(input);
            if self.standalone
                && matches!(
                    installation.module.as_str(),
                    "airspeed"
                        | "attitude"
                        | "altimeter"
                        | "turn_coordinator"
                        | "heading"
                        | "vertical_speed"
                        | "tachometer"
                )
            {
                draw.items.retain(|item| !matches!(item, DrawItem::Label { at, .. } if at.get(1).is_some_and(|y| *y > 158. || *y < 24.)));
                // Crop the explanatory caption margin; artwork and live knobs retain their coordinates.
                let shift = |point: &mut [f32; 2]| {
                    let [x, y] = *point;
                    *point = [x - 26., y - 14.];
                };
                for item in &mut draw.items {
                    match item {
                        DrawItem::Circle { center, .. } => shift(center),
                        DrawItem::Label { at, .. } => shift(at),
                        DrawItem::Line { a, b, .. } => {
                            shift(a);
                            shift(b);
                        }
                        DrawItem::Polygon { points, .. } => {
                            for p in points {
                                shift(p);
                            }
                        }
                        _ => {}
                    }
                }
                draw.size = [148., 148.];
            }
            if installation.module.starts_with("electronic_")
                && ui
                    .interact(cell, ui.id().with(&installation.id), Sense::click())
                    .on_hover_text("Click for heading, course, baro, altitude, and direct-to controls")
                    .clicked()
            {
                self.avionics_controls.open = true;
            }
            let skin = self
                .registry
                .skins
                .get(&installation.skin)
                .ok_or_else(|| format!("Unknown skin: {}", installation.skin))?;
            let local = fit_size(cell, draw.size);
            scaled(ui.painter(), local, draw.size, |p| skin.paint(p, &draw));
            if installation.module == "glass_mfd"
                && input
                    .glass
                    .is_some_and(|g| !g.mfd_failed && g.settings.page == flightcraft_control::GlassPage::Map)
            {
                let factor = local.width() / 500.;
                self.map_rect = Some(Rect::from_min_size(
                    local.min + egui::vec2(110., 28.) * factor,
                    egui::vec2(390., 352.) * factor,
                ));
            }
            let knob = match installation.module.as_str() {
                "heading" => Some(([155., 143.], 12.)),
                "electronic_hsi" => Some(([174., 110.], 12.)),
                "glass_pfd" => Some(([359., 404.], 13.)),
                _ => None,
            };
            if let Some(([x, y], radius)) = knob {
                let [canvas_w, _] = draw.size;
                let factor = local.width() / canvas_w;
                let rect = Rect::from_center_size(
                    local.min + egui::vec2(x, y) * factor,
                    egui::vec2(radius * 2., radius * 2.) * factor,
                );
                self.heading_commands.extend(
                    self.heading_knobs
                        .entry(installation.id.clone())
                        .or_default()
                        .show(ui, ui.id().with(("hdg", &installation.id)), rect),
                );
            }

            if installation.module == "altimeter" {
                let scale = local.width() / 200.;
                let knob = Rect::from_center_size(
                    local.min + egui::vec2(155., 143.) * scale,
                    egui::vec2(24., 24.) * scale,
                );
                let response = ui.interact(knob, ui.id().with(&installation.id), Sense::click_and_drag());
                if installation.module == "heading" && response.clicked() {
                    settings.push(SystemSetting::HeadingBug {
                        heading_rad: input.systems.sensed.heading_rad,
                    });
                }
                if response.hovered() {
                    let wheel = ui.input_mut(|i| {
                        let delta = i.raw_scroll_delta.y;
                        i.raw_scroll_delta.y = 0.;
                        delta
                    });
                    if wheel != 0. {
                        settings.push(if installation.module == "altimeter" {
                            SystemSetting::Altimeter {
                                pressure_pa: self.options.adjust_pressure(
                                    input.systems.controls.altimeter_setting_pa,
                                    f64::from(wheel.signum()),
                                ),
                            }
                        } else {
                            SystemSetting::HeadingBug {
                                heading_rad: (input.systems.controls.heading_bug_rad
                                    + f64::from(wheel.signum()).to_radians())
                                .rem_euclid(std::f64::consts::TAU),
                            }
                        });
                    }
                    response.on_hover_text(if installation.module == "altimeter" {
                        match self.options.pressure {
                            PressureUnit::InHg => "Wheel: 0.01 inHg; range 28.10–31.00",
                            PressureUnit::Hpa => "Wheel: 1 hPa; range 950–1050",
                        }
                    } else {
                        "Wheel: heading bug, 1 degree; push: sync to heading"
                    });
                }
            }
        }
        self.avionics_controls.show(ui.ctx(), input);
        Ok(settings)
    }
}

fn panel_dimensions(panel: &Panel) -> [f64; 2] {
    panel.instruments.iter().fold([1_f64, 1_f64], |[w, h], i| {
        let [x, y] = i.position;
        let [iw, ih] = i.size;
        [w.max(x + iw), h.max(y + ih)]
    })
}
pub(crate) fn view_panel_height(panel: &Panel, window: egui::Vec2, compact: bool) -> f32 {
    let [w, h] = panel.equipment.as_ref().map_or_else(
        || panel_dimensions(panel),
        |p| {
            crate::panel_art::dash_bounds(p).map_or(p.size, |r| [f64::from(r.width()), f64::from(r.height())])
        },
    );
    let controls = if panel.equipment.is_some() { 10. } else { 65. };
    (window.x * (h / w) as f32 + controls).min(window.y * if compact { 0.55 } else { 0.80 })
}
pub(crate) fn layout(panel: &Panel, rect: Rect) -> Vec<Rect> {
    let [width, height] = panel_dimensions(panel);
    let rect = panel_fit(rect, [width as f32, height as f32]);
    panel
        .instruments
        .iter()
        .map(
            |InstrumentInstallation {
                 position: [x, y],
                 size: [w, h],
                 ..
             }| {
                Rect::from_min_size(
                    rect.min
                        + egui::vec2(
                            (x / width) as f32 * rect.width(),
                            (y / height) as f32 * rect.height(),
                        ),
                    egui::vec2(
                        (w / width) as f32 * rect.width(),
                        (h / height) as f32 * rect.height(),
                    ),
                )
            },
        )
        .collect()
}
#[cfg(test)]
fn fit(cell: Rect) -> Rect {
    fit_size(cell, [200., 180.])
}
pub(crate) fn fit_size(cell: Rect, [width, height]: [f32; 2]) -> Rect {
    let scale = (cell.width() / width).min(cell.height() / height);
    Rect::from_center_size(cell.center(), egui::vec2(width, height) * scale)
}
/// Fit the entire panel, including the lowest controls, inside its assigned host.
pub(crate) fn panel_fit(available: Rect, size: [f32; 2]) -> Rect {
    fit_size(available, size)
}
pub(crate) fn scaled(p: &Painter, cell: Rect, size: [f32; 2], draw: impl FnOnce(&Painter)) {
    let [width, _] = size;
    let scale = cell.width() / width;
    if !scale.is_finite() || scale <= 0. {
        return;
    }
    let transform = egui::emath::TSTransform::new(cell.min.to_vec2(), scale);
    let clip = transform.inverse().mul_rect(cell.intersect(p.clip_rect()));
    let local = Painter::new(p.ctx().clone(), p.layer_id(), clip);
    let start = p.ctx().graphics_mut(|g| g.entry(p.layer_id()).next_idx());
    draw(&local);
    p.ctx().graphics_mut(|g| {
        let list = g.entry(p.layer_id());
        list.transform_range(start, list.next_idx(), transform);
    });
}

#[cfg(test)]
pub(crate) fn panel_height(window_height: f32, compact: bool) -> f32 {
    if compact {
        260.
    } else {
        (window_height * 0.46).clamp(300., 380.)
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) mod preview;
