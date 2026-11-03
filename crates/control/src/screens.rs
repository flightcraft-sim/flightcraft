//! Screen and layout contracts shared by desktop, browser panels, scripts, and MCP.
use crate::{Command, CommandError, Session, invalid};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PageKind {
    #[default]
    Outside,
    FlyMe,
    Director,
    Panel,
    Instrument,
    Pfd,
    Mfd,
    Navigator,
    Map,
    Charts,
    Instructor,
    Atc,
    Radio,
    Debrief,
    Control,
    PanelItem,
    Switch,
    Breaker,
    Readout,
    Image,
}
impl PageKind {
    pub const ALL: [Self; 20] = [
        Self::Outside,
        Self::FlyMe,
        Self::Director,
        Self::Panel,
        Self::Instrument,
        Self::Pfd,
        Self::Mfd,
        Self::Navigator,
        Self::Map,
        Self::Charts,
        Self::Instructor,
        Self::Atc,
        Self::Radio,
        Self::Debrief,
        Self::Control,
        Self::PanelItem,
        Self::Switch,
        Self::Breaker,
        Self::Readout,
        Self::Image,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Outside => "Outside view",
            Self::FlyMe => "Fly me",
            Self::Director => "Director camera",
            Self::Panel => "Cockpit panel",
            Self::Instrument => "Instrument",
            Self::Pfd => "PFD",
            Self::Mfd => "MFD",
            Self::Navigator => "Navigator",
            Self::Map => "Planning map",
            Self::Charts => "EFB / charts",
            Self::Atc => "ATC / tower",
            Self::Instructor => "Instructor station",
            Self::Radio => "Radio / kneeboard",
            Self::Debrief => "Debrief",
            Self::Control => "Touch control",
            Self::PanelItem => "Panel item",
            Self::Switch => "Switch",
            Self::Breaker => "Breaker",
            Self::Readout => "Variable readout",
            Self::Image => "Image / placard",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum View {
    #[default]
    Forward,
    Left,
    Right,
    Chase,
    Tower,
    TopDown,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Camera {
    #[serde(default)]
    pub view: View,
    #[serde(default = "default_fov")]
    #[schemars(range(min = 15, max = 150))]
    pub fov_deg: f32,
    #[serde(default)]
    #[schemars(range(min=-180,max=180))]
    pub heading_offset_deg: f32,
}
fn default_fov() -> f32 {
    60.0
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            view: View::Forward,
            fov_deg: default_fov(),
            heading_offset_deg: 0.0,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Page {
    #[serde(default)]
    pub follow_main_camera: bool,
    #[serde(default)]
    pub control: TouchControl,
    /// An equipment installation id for an independently movable switch, knob, or breaker.
    #[serde(default)]
    pub item: Option<String>,
    pub kind: PageKind,
    #[serde(default)]
    pub camera: Camera,
    /// Empty selects the aircraft's full panel. Instrument pages select one module id.
    #[serde(default)]
    #[schemars(length(max = 64))]
    pub instruments: Vec<String>,
}
impl Page {
    pub fn new(kind: PageKind) -> Self {
        Self {
            kind,
            ..Default::default()
        }
    }
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Units {
    #[default]
    Px,
    Mm,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Mask {
    #[default]
    Rectangle,
    Circle,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Element {
    pub id: String,
    pub page: Page,
    pub position: [f32; 2],
    pub size: [f32; 2],
    #[serde(default)]
    pub z: i32,
    #[serde(default)]
    pub rotation_deg: u16,
    #[serde(default)]
    pub mask: Mask,
    #[serde(default = "yes")]
    pub bezel: bool,
    #[serde(default = "one")]
    #[schemars(range(min = 0, max = 1))]
    pub brightness: f32,
    #[serde(default)]
    pub lighting_group: String,
    #[serde(default = "yes")]
    pub accepts_input: bool,
}
fn yes() -> bool {
    true
}
fn one() -> f32 {
    1.0
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    #[schemars(range(min = 1, max = 1))]
    pub version: u32,
    pub name: String,
    #[serde(default)]
    pub units: Units,
    pub canvas: [f32; 2],
    /// Independent X and Y monitor calibration; CSS mm are never assumed physical.
    #[serde(default = "calibration")]
    pub pixels_per_mm: [f32; 2],
    #[schemars(length(max = 128))]
    pub elements: Vec<Element>,
}
fn calibration() -> [f32; 2] {
    [96.0 / 25.4; 2]
}
fn text_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control)
}
impl Layout {
    pub fn single(page: Page) -> Self {
        Self {
            version: 1,
            name: page.kind.label().into(),
            units: Units::Px,
            canvas: [1280., 800.],
            pixels_per_mm: calibration(),
            elements: vec![Element {
                id: "page".into(),
                page,
                position: [0., 0.],
                size: [1280., 800.],
                z: 0,
                rotation_deg: 0,
                mask: Mask::Rectangle,
                bezel: true,
                brightness: 1.,
                lighting_group: String::new(),
                accepts_input: true,
            }],
        }
    }
    pub fn from_json(text: &str) -> Result<Self, String> {
        if text.len() > 1024 * 1024 {
            return Err("layout exceeds 1 MiB".into());
        }
        let layout: Self = serde_json::from_str(text).map_err(|e| e.to_string())?;
        layout.validate()?;
        Ok(layout)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1
            || !text_ok(&self.name)
            || self.elements.len() > 128
            || self
                .canvas
                .iter()
                .any(|n| !n.is_finite() || *n <= 0. || *n > 32768.)
            || self
                .pixels_per_mm
                .iter()
                .any(|n| !n.is_finite() || *n < 0.1 || *n > 100.)
        {
            return Err("invalid layout version, name, canvas, calibration, or element count".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        for e in &self.elements {
            let [x, y] = e.position;
            let [w, h] = e.size;
            let [cw, ch] = self.canvas;
            if !text_ok(&e.id)
                || !ids.insert(&e.id)
                || [x, y, w, h].iter().any(|n| !n.is_finite())
                || x < 0.
                || y < 0.
                || w <= 0.
                || h <= 0.
                || x + w > cw
                || y + h > ch
                || ![0, 90, 180, 270].contains(&e.rotation_deg)
                || !e.brightness.is_finite()
                || !(0.0..=1.0).contains(&e.brightness)
                || e.lighting_group.len() > 128
            {
                return Err(format!("invalid element {}", e.id));
            }
            e.page.validate()?;
        }
        Ok(())
    }
}
impl Page {
    pub fn validate(&self) -> Result<(), String> {
        if !self.camera.fov_deg.is_finite()
            || !(15.0..=150.0).contains(&self.camera.fov_deg)
            || !self.camera.heading_offset_deg.is_finite()
            || !(-180.0..=180.0).contains(&self.camera.heading_offset_deg)
            || self.instruments.len() > 64
            || self.instruments.iter().any(|s| !text_ok(s))
            || self.item.as_ref().is_some_and(|s| !text_ok(s))
            || self.control.detents.len() > 32
            || self
                .control
                .detents
                .iter()
                .any(|n| !n.is_finite() || !(0.0..=1.0).contains(n))
        {
            return Err("invalid page camera or instrument selection".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Screen {
    pub id: u64,
    pub title: String,
    #[serde(default)]
    pub monitor: Option<usize>,
    #[serde(default)]
    pub position: Option<[f32; 2]>,
    pub size: [f32; 2],
    #[serde(default)]
    pub fullscreen: bool,
    #[serde(default)]
    pub borderless: bool,
    pub layout: Layout,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScreenSet {
    #[serde(default)]
    pub monitors: Vec<Monitor>,
    pub windows: Vec<Screen>,
    pub next_id: u64,
}
impl Default for ScreenSet {
    fn default() -> Self {
        let mut layout = Layout::single(Page::new(PageKind::Outside));
        if let Some(outside) = layout.elements.first_mut() {
            outside.size = [1280., 480.];
            outside.page.follow_main_camera = true;
        }
        let mut panel = Element::page("panel", Page::new(PageKind::Panel), [0., 480.], [1280., 320.]);
        panel.id = "panel".into();
        panel.position = [0., 480.];
        panel.size = [1280., 320.];
        layout.elements.push(panel);
        layout.name = "Single screen".into();
        Self {
            monitors: Vec::new(),
            windows: vec![Screen {
                id: 0,
                title: "FlightCraft".into(),
                monitor: None,
                position: None,
                size: [1280., 800.],
                fullscreen: false,
                borderless: false,
                layout,
            }],
            next_id: 1,
        }
    }
}
impl ScreenSet {
    pub fn validate(&self) -> Result<(), String> {
        if self.windows.is_empty() || self.windows.len() > 64 || !self.windows.iter().any(|w| w.id == 0) {
            return Err("screen set needs a main window and at most 64 windows".into());
        }
        if self.monitors.len() > 16
            || self.monitors.iter().any(|m| {
                !text_ok(&m.name)
                    || m.origin.iter().any(|v| !v.is_finite() || v.abs() > 100000.)
                    || m.size.iter().any(|v| !v.is_finite() || *v < 160. || *v > 32768.)
                    || m.pixels_per_mm
                        .iter()
                        .any(|v| !v.is_finite() || *v < 0.1 || *v > 100.)
            })
        {
            return Err("invalid monitor profile".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        for w in &self.windows {
            if !ids.insert(w.id)
                || w.id >= self.next_id
                || !text_ok(&w.title)
                || w.size.iter().any(|n| !n.is_finite() || *n < 160. || *n > 16384.)
                || w.position
                    .is_some_and(|p| p.iter().any(|n| !n.is_finite() || n.abs() > 100000.))
                || w.monitor.is_some_and(|m| m >= self.monitors.len())
            {
                return Err("invalid window identity, title, position, monitor, or size".into());
            }
            w.layout.validate()?;
        }
        Ok(())
    }
}
impl Session {
    pub fn screens(&self) -> &ScreenSet {
        &self.screens
    }
    pub fn restore_screens(&mut self, set: ScreenSet) -> Result<(), String> {
        set.validate()?;
        self.screens = set;
        Ok(())
    }
}
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpenScreen {
    pub page: Page,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub monitor: Option<usize>,
    #[serde(default)]
    pub position: Option<[f32; 2]>,
    #[serde(default = "window_size")]
    pub size: [f32; 2],
    #[serde(default)]
    pub fullscreen: bool,
    #[serde(default)]
    pub borderless: bool,
}
fn window_size() -> [f32; 2] {
    [1000., 700.]
}
impl Command for OpenScreen {
    const NAME: &'static str = "screen.open";
    const DESCRIPTION: &'static str = "Open a native screen with an independent page and camera.";
    type Output = Screen;
    fn run(self, s: &mut Session) -> Result<Screen, CommandError> {
        let mut set = s.screens.clone();
        let id = set.next_id;
        set.next_id = id
            .checked_add(1)
            .ok_or_else(|| invalid(Self::NAME, "screen id exhausted"))?;
        let screen = Screen {
            id,
            title: self.title.unwrap_or_else(|| self.page.kind.label().into()),
            monitor: self.monitor,
            position: self.position,
            size: self.size,
            fullscreen: self.fullscreen,
            borderless: self.borderless,
            layout: Layout::single(self.page),
        };
        set.windows.push(screen.clone());
        set.validate().map_err(|e| invalid(Self::NAME, e))?;
        s.screens = set;
        Ok(screen)
    }
}
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct CloseScreen {
    pub id: u64,
}
impl Command for CloseScreen {
    const NAME: &'static str = "screen.close";
    const DESCRIPTION: &'static str =
        "Close an extra screen; the main window cannot be closed by this command.";
    type Output = ScreenSet;
    fn run(self, s: &mut Session) -> Result<ScreenSet, CommandError> {
        if self.id == 0 || !s.screens.windows.iter().any(|w| w.id == self.id) {
            return Err(invalid(Self::NAME, "unknown extra screen"));
        }
        s.screens.windows.retain(|w| w.id != self.id);
        Ok(s.screens.clone())
    }
}
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ListScreens {}
impl Command for ListScreens {
    const NAME: &'static str = "screen.list";
    const DESCRIPTION: &'static str = "List the saved window set, layouts, and independent cameras.";
    type Output = ScreenSet;
    fn run(self, s: &mut Session) -> Result<ScreenSet, CommandError> {
        Ok(s.screens.clone())
    }
}
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetScreenPage {
    pub id: u64,
    pub page: Page,
}
impl Command for SetScreenPage {
    const NAME: &'static str = "screen.set_page";
    const DESCRIPTION: &'static str = "Replace a screen's layout with a single page.";
    type Output = Screen;
    fn run(self, s: &mut Session) -> Result<Screen, CommandError> {
        self.page.validate().map_err(|e| invalid(Self::NAME, e))?;
        let w = s
            .screens
            .windows
            .iter_mut()
            .find(|w| w.id == self.id)
            .ok_or_else(|| invalid(Self::NAME, "unknown screen"))?;
        w.layout = Layout::single(self.page);
        Ok(w.clone())
    }
}
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetLayout {
    pub id: u64,
    pub layout: Layout,
}
impl Command for SetLayout {
    const NAME: &'static str = "layout.set";
    const DESCRIPTION: &'static str =
        "Validate and replace a screen layout atomically, retaining the last good layout on errors.";
    type Output = Screen;
    fn run(self, s: &mut Session) -> Result<Screen, CommandError> {
        self.layout.validate().map_err(|e| invalid(Self::NAME, e))?;
        let mut set = s.screens.clone();
        let w = window_mut(&mut set, self.id)?;
        w.layout = self.layout;
        let out = w.clone();
        s.commit_layout(set)?;
        Ok(out)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ControlKind {
    #[default]
    Throttle,
    Mixture,
    Prop,
    CarbHeat,
    FuelSelector,
    Flaps,
    Trim,
    ParkingBrake,
    Magnetos,
    Primer,
    Heading,
    Altitude,
    Range,
    Fms,
    Baro,
    Course,
}
impl ControlKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Throttle => "Throttle",
            Self::Mixture => "Mixture",
            Self::Prop => "Propeller",
            Self::CarbHeat => "Carb heat",
            Self::FuelSelector => "Fuel selector",
            Self::Flaps => "Flaps",
            Self::Trim => "Trim",
            Self::ParkingBrake => "Parking brake",
            Self::Magnetos => "Magnetos",
            Self::Primer => "Primer",
            Self::Heading => "Heading",
            Self::Altitude => "Altitude",
            Self::Range => "Range",
            Self::Fms => "FMS",
            Self::Baro => "Baro",
            Self::Course => "Course",
        }
    }
    pub const ALL: [Self; 16] = [
        Self::Throttle,
        Self::Mixture,
        Self::Prop,
        Self::CarbHeat,
        Self::FuelSelector,
        Self::Flaps,
        Self::Trim,
        Self::ParkingBrake,
        Self::Magnetos,
        Self::Primer,
        Self::Heading,
        Self::Altitude,
        Self::Range,
        Self::Fms,
        Self::Baro,
        Self::Course,
    ];
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TouchControl {
    #[serde(default)]
    pub kind: ControlKind,
    #[serde(default)]
    pub detents: Vec<f64>,
    #[serde(default)]
    pub friction_locked: bool,
}
impl Element {
    pub fn page(id: &str, page: Page, position: [f32; 2], size: [f32; 2]) -> Self {
        Self {
            id: id.into(),
            page,
            position,
            size,
            z: 0,
            rotation_deg: 0,
            mask: Mask::Rectangle,
            bezel: true,
            brightness: 1.,
            lighting_group: String::new(),
            accepts_input: true,
        }
    }
}
impl Session {
    fn commit_layout(&mut self, set: ScreenSet) -> Result<ScreenSet, CommandError> {
        set.validate().map_err(|e| invalid("layout.edit", e))?;
        if self.layout_undo.len() >= 64 {
            self.layout_undo.remove(0);
        }
        self.layout_undo.push(self.screens.clone());
        self.layout_redo.clear();
        self.screens = set;
        Ok(self.screens.clone())
    }
}
fn window_mut(set: &mut ScreenSet, id: u64) -> Result<&mut Screen, CommandError> {
    set.windows
        .iter_mut()
        .find(|w| w.id == id)
        .ok_or_else(|| invalid("layout.edit", "unknown screen"))
}
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct EditElement {
    pub screen: u64,
    pub element: Element,
}
impl Command for EditElement {
    const NAME: &'static str = "layout.update";
    const DESCRIPTION: &'static str = "Replace an element atomically; records one undo entry.";
    type Output = ScreenSet;
    fn run(self, s: &mut Session) -> Result<ScreenSet, CommandError> {
        let mut set = s.screens.clone();
        let w = window_mut(&mut set, self.screen)?;
        let e = w
            .layout
            .elements
            .iter_mut()
            .find(|e| e.id == self.element.id)
            .ok_or_else(|| invalid(Self::NAME, "unknown element"))?;
        *e = self.element;
        s.commit_layout(set)
    }
}
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct AddElement {
    pub screen: u64,
    pub element: Element,
}
impl Command for AddElement {
    const NAME: &'static str = "layout.add";
    const DESCRIPTION: &'static str =
        "Add an independent instrument, display, switch, knob, breaker, or lever.";
    type Output = ScreenSet;
    fn run(self, s: &mut Session) -> Result<ScreenSet, CommandError> {
        let mut set = s.screens.clone();
        window_mut(&mut set, self.screen)?
            .layout
            .elements
            .push(self.element);
        s.commit_layout(set)
    }
}
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct RemoveElement {
    pub screen: u64,
    pub element: String,
}
impl Command for RemoveElement {
    const NAME: &'static str = "layout.remove";
    const DESCRIPTION: &'static str = "Remove a layout element with undo.";
    type Output = ScreenSet;
    fn run(self, s: &mut Session) -> Result<ScreenSet, CommandError> {
        let mut set = s.screens.clone();
        let w = window_mut(&mut set, self.screen)?;
        let before = w.layout.elements.len();
        w.layout.elements.retain(|e| e.id != self.element);
        if before == w.layout.elements.len() {
            return Err(invalid(Self::NAME, "unknown element"));
        }
        s.commit_layout(set)
    }
}
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct MoveElement {
    pub from: u64,
    pub to: u64,
    pub element: String,
    #[serde(default)]
    pub position: Option<[f32; 2]>,
}
impl Command for MoveElement {
    const NAME: &'static str = "layout.move";
    const DESCRIPTION: &'static str =
        "Move an element between screens, converting calibrated physical units; atomic and undoable.";
    type Output = ScreenSet;
    fn run(self, s: &mut Session) -> Result<ScreenSet, CommandError> {
        if self.from == self.to {
            return Err(invalid(Self::NAME, "choose another screen"));
        }
        let mut set = s.screens.clone();
        let source_scale = units_per_mm(&set, self.from)?;
        let target_scale = units_per_mm(&set, self.to)?;
        let source = window_mut(&mut set, self.from)?;
        let index = source
            .layout
            .elements
            .iter()
            .position(|e| e.id == self.element)
            .ok_or_else(|| invalid(Self::NAME, "unknown element"))?;
        let mut element = source.layout.elements.remove(index);
        let dest = window_mut(&mut set, self.to)?;
        let [w, h] = element.size;
        let [sx, sy] = source_scale;
        let [tx, ty] = target_scale;
        element.size = [w / sx * tx, h / sy * ty];
        element.position = self.position.unwrap_or([0., 0.]);
        dest.layout.elements.push(element);
        s.commit_layout(set)
    }
}
fn units_per_mm(set: &ScreenSet, id: u64) -> Result<[f32; 2], CommandError> {
    let screen = set
        .windows
        .iter()
        .find(|w| w.id == id)
        .ok_or_else(|| invalid("layout.move", "unknown screen"))?;
    Ok(match screen.layout.units {
        Units::Mm => [1., 1.],
        Units::Px => screen
            .monitor
            .and_then(|i| set.monitors.get(i))
            .map_or(screen.layout.pixels_per_mm, |m| m.pixels_per_mm),
    })
}
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct UndoLayout {}
impl Command for UndoLayout {
    const NAME: &'static str = "layout.undo";
    const DESCRIPTION: &'static str = "Undo the last layout edit across all screens.";
    type Output = ScreenSet;
    fn run(self, s: &mut Session) -> Result<ScreenSet, CommandError> {
        let previous = s
            .layout_undo
            .pop()
            .ok_or_else(|| invalid(Self::NAME, "nothing to undo"))?;
        s.layout_redo.push(std::mem::replace(&mut s.screens, previous));
        Ok(s.screens.clone())
    }
}
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct RedoLayout {}
impl Command for RedoLayout {
    const NAME: &'static str = "layout.redo";
    const DESCRIPTION: &'static str = "Redo a layout edit.";
    type Output = ScreenSet;
    fn run(self, s: &mut Session) -> Result<ScreenSet, CommandError> {
        let next = s
            .layout_redo
            .pop()
            .ok_or_else(|| invalid(Self::NAME, "nothing to redo"))?;
        s.layout_undo.push(std::mem::replace(&mut s.screens, next));
        Ok(s.screens.clone())
    }
}
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SaveLayout {
    pub screen: u64,
    pub path: String,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct LayoutSaved {
    pub path: String,
}
impl Command for SaveLayout {
    const NAME: &'static str = "layout.save";
    const DESCRIPTION: &'static str = "Save a layout to an explicit JSON path, without a dialog.";
    type Output = LayoutSaved;
    fn run(self, s: &mut Session) -> Result<LayoutSaved, CommandError> {
        let w = s
            .screens
            .windows
            .iter()
            .find(|w| w.id == self.screen)
            .ok_or_else(|| invalid(Self::NAME, "unknown screen"))?;
        if self.path.is_empty() || self.path.len() > 4096 || !self.path.ends_with(".json") {
            return Err(invalid(Self::NAME, "provide a JSON path"));
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let path = std::path::Path::new(&self.path);
            let json =
                serde_json::to_vec_pretty(&w.layout).map_err(|e| invalid(Self::NAME, e.to_string()))?;
            let temp = path.with_extension("json.tmp");
            std::fs::write(&temp, json)
                .and_then(|()| std::fs::rename(&temp, path))
                .map_err(|e| invalid(Self::NAME, e.to_string()))?;
            Ok(LayoutSaved { path: self.path })
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = w;
            Err(invalid(
                Self::NAME,
                "browser layouts use export or host-side saving",
            ))
        }
    }
}

/// Read-only queries admitted by the panel transport. No prefix matching or mutating queries.
pub fn read_only(command: &str) -> bool {
    if matches!(command, "lesson.list" | "lesson.debrief") {
        return true;
    }
    matches!(
        command,
        "crew.state"
            | "atc.state"
            | "sim.state"
            | "weather.state"
            | "avionics.state"
            | "scenario.state"
            | "map.state"
            | "screen.list"
            | "commands.list"
            | "aircraft.list"
            | "autopilot.state"
            | "efb.status"
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn screen_commands_are_atomic_and_schema_checked() {
        let mut s = Session::new().unwrap();
        let opened = s
            .execute(
                "screen.open",
                json!({"page":{"kind":"instructor"},"size":[900,700],"borderless":true}),
            )
            .unwrap();
        let id = opened.get("id").and_then(|v| v.as_u64()).unwrap();
        assert_eq!(s.screens.windows.len(), 2);
        s.execute("screen.set_page",json!({"id":id,"page":{"kind":"outside","camera":{"view":"left","fov_deg":65,"heading_offset_deg":-8}}})).unwrap();
        let before = serde_json::to_value(s.screens()).unwrap();
        for input in [
            json!({"page":{"kind":"outside","camera":{"fov_deg":0}}}),
            json!({"page":{"kind":"panel"},"size":[-1,700]}),
            json!({"page":{"kind":"unknown"}}),
        ] {
            assert!(s.execute("screen.open", input).is_err());
            assert_eq!(before, serde_json::to_value(s.screens()).unwrap());
        }
        assert!(s.execute("screen.close", json!({"id":0})).is_err());
        s.execute("screen.close", json!({"id":id})).unwrap();
        assert_eq!(s.screens.windows.len(), 1);
        for command in Session::commands()
            .into_iter()
            .filter(|c| c.name.starts_with("screen.") || c.name.starts_with("layout."))
        {
            assert!(
                jsonschema::validator_for(&command.input_schema).is_ok(),
                "{}",
                command.name
            );
            assert!(
                jsonschema::validator_for(&command.output_schema).is_ok(),
                "{}",
                command.name
            );
        }
    }
    #[test]
    fn calibrated_move_undo_and_restart() {
        let mut s = Session::new().unwrap();
        s.execute("screen.open", json!({"page":{"kind":"panel"}}))
            .unwrap();
        let mut layout = Layout::single(Page::new(PageKind::Instrument));
        layout.pixels_per_mm = [4., 5.];
        layout.elements.clear();
        layout.elements.push(Element::page(
            "airspeed",
            Page::new(PageKind::Instrument),
            [0., 0.],
            [100., 100.],
        ));
        layout.elements.push(Element::page(
            "attitude",
            Page::new(PageKind::Instrument),
            [120., 0.],
            [100., 100.],
        ));
        s.execute("layout.set", json!({"id":0,"layout":layout})).unwrap();
        let mut dest = Layout::single(Page::new(PageKind::Control));
        dest.units = Units::Mm;
        dest.pixels_per_mm = [4., 5.];
        dest.canvas = [320., 160.];
        if let Some(e) = dest.elements.first_mut() {
            e.size = [20., 20.];
            e.position = [200., 0.];
        }
        s.execute("layout.set", json!({"id":1,"layout":dest})).unwrap();
        s.execute(
            "layout.move",
            json!({"from":0,"to":1,"element":"airspeed","position":[10.,20.]}),
        )
        .unwrap();
        let target = s.screens.windows.last().unwrap().layout.elements.last().unwrap();
        assert_eq!(target.size, [25., 20.]);
        let saved = serde_json::to_string(s.screens()).unwrap();
        let mut restored = Session::new().unwrap();
        restored
            .restore_screens(serde_json::from_str(&saved).unwrap())
            .unwrap();
        assert_eq!(saved, serde_json::to_string(restored.screens()).unwrap());
        s.execute("layout.undo", json!({})).unwrap();
        assert_eq!(s.screens.windows.first().unwrap().layout.elements.len(), 2);
        s.execute("layout.redo", json!({})).unwrap();
        assert_eq!(s.screens.windows.first().unwrap().layout.elements.len(), 1);
        let mut source = s.screens.windows.first().unwrap().layout.clone();
        source.units = Units::Mm;
        source.pixels_per_mm = [7., 8.];
        s.execute("layout.set", json!({"id":0,"layout":source})).unwrap();
        s.execute("layout.move", json!({"from":0,"to":1,"element":"attitude"}))
            .unwrap();
        assert_eq!(
            s.screens
                .windows
                .last()
                .unwrap()
                .layout
                .elements
                .last()
                .unwrap()
                .size,
            [100., 100.]
        );
    }
    #[test]
    fn layout_validation_and_last_good_fallback() {
        let mut s = Session::new().unwrap();
        let layout = Layout::single(Page::new(PageKind::Pfd));
        let schema = schemars::schema_for!(Layout);
        let validator = jsonschema::validator_for(&serde_json::to_value(schema).unwrap()).unwrap();
        assert!(validator.is_valid(&serde_json::to_value(&layout).unwrap()));
        let before = serde_json::to_value(s.screens()).unwrap();
        let mut bad = layout.clone();
        bad.elements.push(bad.elements.first().unwrap().clone());
        assert!(bad.validate().is_err());
        assert!(s.execute("layout.set", json!({"id":0,"layout":bad})).is_err());
        assert_eq!(before, serde_json::to_value(s.screens()).unwrap());
        assert!(Layout::from_json(r#"{"version":999}"#).is_err());
        assert!(Layout::from_json("not json").is_err());
    }
    #[test]
    fn wire_round_trip_does_not_advance_a_presentation() {
        let mut owner = Session::new().unwrap();
        owner.run(crate::Step { steps: 120 }).unwrap();
        let snapshot = owner.screen_snapshot();
        let wire = serde_json::to_string(&snapshot).unwrap();
        let mut view = Session::new().unwrap();
        view.apply_screen_snapshot(serde_json::from_str(&wire).unwrap())
            .unwrap();
        assert_eq!(view.sim().state(), owner.sim().state());
        view.execute("sim.set_controls", json!({"throttle":0.8})).unwrap();
        let commands = view.take_screen_commands();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands.first().unwrap().0, "sim.set_controls");
        assert_ne!(owner.sim().controls().throttle, 0.8);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Monitor {
    pub name: String,
    pub origin: [f32; 2],
    pub size: [f32; 2],
    pub pixels_per_mm: [f32; 2],
}
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfigureMonitor {
    pub index: usize,
    pub monitor: Monitor,
}
impl Command for ConfigureMonitor {
    const NAME: &'static str = "screen.monitor";
    const DESCRIPTION: &'static str =
        "Define a monitor origin, size in desktop points, and independent physical X/Y calibration.";
    type Output = ScreenSet;
    fn run(self, s: &mut Session) -> Result<ScreenSet, CommandError> {
        let mut set = s.screens.clone();
        if self.index > set.monitors.len() {
            return Err(invalid(Self::NAME, "monitor indices must be consecutive"));
        }
        if let Some(monitor) = set.monitors.get_mut(self.index) {
            *monitor = self.monitor;
        } else {
            set.monitors.push(self.monitor);
        }
        set.validate().map_err(|e| invalid(Self::NAME, e))?;
        s.screens = set;
        Ok(s.screens.clone())
    }
}
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfigureScreen {
    pub id: u64,
    #[serde(default)]
    pub monitor: Option<usize>,
    #[serde(default)]
    pub position: Option<[f32; 2]>,
    #[serde(default)]
    pub size: Option<[f32; 2]>,
    #[serde(default)]
    pub fullscreen: Option<bool>,
    #[serde(default)]
    pub borderless: Option<bool>,
}
impl Command for ConfigureScreen {
    const NAME: &'static str = "screen.configure";
    const DESCRIPTION: &'static str =
        "Change a window's monitor, position, size, fullscreen, or borderless setting.";
    type Output = ScreenSet;
    fn run(self, s: &mut Session) -> Result<ScreenSet, CommandError> {
        let mut set = s.screens.clone();
        let monitor_origin = self
            .monitor
            .and_then(|index| set.monitors.get(index))
            .map(|m| m.origin);
        let w = window_mut(&mut set, self.id)?;
        if let Some(m) = self.monitor {
            w.monitor = Some(m);
            w.position = monitor_origin;
        }
        if let Some(p) = self.position {
            w.position = Some(p);
        }
        if let Some(size) = self.size {
            w.size = size;
        }
        if let Some(v) = self.fullscreen {
            w.fullscreen = v;
        }
        if let Some(v) = self.borderless {
            w.borderless = v;
        }
        set.validate().map_err(|e| invalid(Self::NAME, e))?;
        s.screens = set;
        Ok(s.screens.clone())
    }
}
