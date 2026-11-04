//! Touch navigator and display behaviour. Navigation is owned by the session's single GPS.
use crate::touch_efis::{self, EfisConfig, EfisKey, KeyState, Knob, KnobRole, Overlay, PushEffect, Ring};
use crate::{Command, CommandError, GlassSetting, Session, invalid};
use flightcraft_aero::display_model::DisplayModel;
use flightcraft_aero::modern::{DataArea, MfdPage, PanelElement, PilotSeat, TouchLayout};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NavigatorPage {
    #[default]
    Home,
    FlightPlan,
    DirectTo,
    Procedures,
    Nearest,
    Map,
    WaypointInfo,
    Frequencies,
    Transponder,
    Audio,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RadioBand {
    #[default]
    Com,
    Nav,
}
#[derive(Clone, Debug, Serialize, JsonSchema, serde::Deserialize)]
pub struct NavigatorState {
    pub powered: bool,
    pub page: NavigatorPage,
    pub com_active_khz: u32,
    pub com_standby_khz: u32,
    /// The active frequency is the NAV receiver tuning boundary.
    pub nav_active_khz: u32,
    pub nav_standby_khz: u32,
    pub knob_band: RadioBand,
    pub transponder: bool,
    /// Whether the navigator hosts the panel's shared audio installation.
    pub audio_panel: bool,
    pub cdi_source: flightcraft_sim::avionics::radio::CdiSource,
}
impl Default for NavigatorState {
    fn default() -> Self {
        Self {
            powered: false,
            page: NavigatorPage::Home,
            com_active_khz: 122800,
            com_standby_khz: 124000,
            nav_active_khz: 110000,
            nav_standby_khz: 113000,
            knob_band: RadioBand::Com,
            transponder: false,
            audio_panel: false,
            cdi_source: Default::default(),
        }
    }
}
#[derive(Clone, Debug, Serialize, JsonSchema, serde::Deserialize)]
pub struct TouchDisplayState {
    pub id: String,
    /// The page the display shows (or would show, were it not reverted).
    pub layout: TouchLayout,
    /// The pack's installed layout: what the PFD key and menu entry select.
    #[serde(default)]
    pub installed_layout: TouchLayout,
    /// The page this display shows at power-up for the pilot's seat, before any choice of the pilot's own.
    #[serde(default)]
    pub default_page: TouchLayout,
    /// What the MFD page's data column shows.
    #[serde(default)]
    pub data_area: DataArea,
    pub engine_strip: bool,
    pub synthetic_vision: bool,
    pub failed: bool,
    pub powered: bool,
    pub reversion: bool,
    /// The MFD page shown full-screen or in the split pane.
    #[serde(default)]
    pub mfd_page: MfdPage,
    /// A menu or keypad shown over the display.
    #[serde(default)]
    pub overlay: Overlay,
    /// While set and later than the host's clock, the knobs have swapped main and alternate roles.
    #[serde(default)]
    pub knob_toggle_until_s: Option<f64>,
    /// Knob settings: alternate roles, press-to-toggle, split-pane side.
    #[serde(default)]
    pub knobs: EfisConfig,
    /// The physical display model, when the installation declares one.
    #[serde(default)]
    pub model: Option<DisplayModel>,
    /// The skin pack in use; absent means the built-in public skin.
    #[serde(default)]
    pub skin: Option<String>,
    /// The skin pack id the aircraft pack recommends, whether or not it is installed.
    #[serde(default)]
    pub recommended_skin: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize, JsonSchema, serde::Deserialize)]
pub struct TouchState {
    pub navigator: Option<NavigatorState>,
    pub displays: Vec<TouchDisplayState>,
    #[serde(default)]
    pub pilot_seat: PilotSeat,
    /// The pilot pressed display backup: the multi-function display shows the primary flight display.
    #[serde(default)]
    pub manual_reversion: bool,
}
/// One display's choices that survive a flight reset: saved per aircraft variant.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VariantDisplays {
    #[serde(default)]
    pub seat: PilotSeat,
    /// Page by installation id, only where it differs from the default for the seat.
    #[serde(default)]
    pub pages: BTreeMap<String, TouchLayout>,
    #[serde(default)]
    pub data_areas: BTreeMap<String, DataArea>,
    #[serde(default)]
    pub mfd_pages: BTreeMap<String, MfdPage>,
    /// Skin pack id by installation id; `built-in` forces the engine's own skin.
    #[serde(default)]
    pub skins: BTreeMap<String, String>,
    #[serde(default)]
    pub knobs: EfisConfig,
}
fn key_state(d: &TouchDisplayState, runtime: &DisplayRuntime) -> KeyState {
    KeyState {
        layout: d.layout,
        installed_pfd: d.installed_layout,
        page: touch_efis::effective_page(d.layout, d.mfd_page),
        overlay: runtime.overlay,
        previous: runtime.previous,
    }
}
/// Stored by eframe on desktop and in browser storage; keyed by resolved variant id.
pub type DisplayPreferences = BTreeMap<String, VariantDisplays>;
#[derive(Clone, Copy, Debug)]
pub(crate) struct DisplayRuntime {
    pub engine_strip: bool,
    pub failed: bool,
    pub synthetic_vision: bool,
    pub overlay: Overlay,
    pub toggle_until_s: Option<f64>,
    pub previous: Option<(TouchLayout, MfdPage)>,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct TouchAvionics {
    pub generation: u64,
    pub navigator: NavigatorState,
    pub displays: BTreeMap<String, DisplayRuntime>,
    pub manual_reversion: bool,
}
#[derive(Clone, Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NavigatorAction {
    Avionics {
        setting: Box<crate::AvionicsSetting>,
    },
    DirectTo {
        query: String,
    },
    FlightPlan {
        #[schemars(length(max = 64))]
        queries: Vec<String>,
    },
    Page {
        page: NavigatorPage,
    },
    Tune {
        band: RadioBand,
        frequency_khz: u32,
    },
    Flip {
        band: RadioBand,
    },
    KnobBand {
        band: RadioBand,
    },
    Knob {
        outer: bool,
        #[schemars(range(min = -100, max = 100))]
        detents: i32,
    },
    /// Uses the same plan, direct-to, transponder, map, and CDI as every display.
    Shared {
        setting: GlassSetting,
    },
}
#[derive(Clone, Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DisplayAction {
    SyntheticVision {
        enabled: bool,
    },
    /// Select the page. Saved for the aircraft variant.
    Layout {
        layout: TouchLayout,
    },
    /// Select what the MFD page's data column shows. Saved for the aircraft variant.
    DataArea {
        area: DataArea,
    },
    EngineStrip {
        visible: bool,
    },
    Failure {
        failed: bool,
    },
    /// Show an MFD page. From a full PFD this opens the split screen. Saved for the variant.
    MfdPage {
        page: MfdPage,
    },
    /// Press a physical key on a landscape touch EFIS; `hold` is press-and-hold.
    Key {
        key: EfisKey,
        #[serde(default)]
        hold: bool,
    },
    /// Touch the on-screen SPLIT or FULL button.
    ToggleSplit {},
    /// Close the menu or keypad shown over the display.
    CloseOverlay {},
    /// Turn one ring of a knob. `time_s` is the host's clock, so replay reads no clock.
    Knob {
        knob: Knob,
        ring: Ring,
        #[schemars(range(min = -100, max = 100))]
        detents: i32,
        #[serde(default)]
        time_s: f64,
    },
    /// Push a knob's inner ring.
    KnobPush {
        knob: Knob,
        #[serde(default)]
        time_s: f64,
    },
    /// Knob behaviour settings, saved for the variant.
    Knobs {
        config: EfisConfig,
    },
    /// Choose a skin pack for this display; `built-in` forces the engine's own skin and omission
    /// follows the aircraft pack's recommendation. Saved for the variant.
    Skin {
        #[serde(default)]
        skin: Option<String>,
    },
    /// Enter a waypoint for direct-to, as the DIRECT key's keypad does.
    DirectTo {
        query: String,
    },
}
#[derive(Clone, Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EquipmentSetting {
    AudioPanel {
        setting: crate::CommsAudioPanel,
    },
    Systems {
        setting: flightcraft_sim::modern::ModernSetting,
    },
    Navigator {
        action: NavigatorAction,
    },
    Display {
        id: String,
        action: DisplayAction,
    },
}
#[derive(Clone, Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetEquipment {
    pub setting: EquipmentSetting,
}
impl Command for SetEquipment {
    const NAME: &'static str = "equipment.set";
    const DESCRIPTION: &'static str =
        "Operate electronic engine controls, recovery, the touch navigator, or an installed touch display.";
    type Output = TouchState;
    fn run(self, s: &mut Session) -> Result<TouchState, CommandError> {
        s.set_equipment(self.setting)?;
        Ok(s.touch_state())
    }
}
impl Session {
    pub fn touch_state(&self) -> TouchState {
        let Some(p) = self
            .aircraft_configuration()
            .and_then(|c| c.panel.equipment.as_ref())
        else {
            return TouchState::default();
        };
        let current = self.touch.generation == self.flight_generation;
        let powered = |id: &str| {
            self.sim
                .systems()
                .electrical
                .loads
                .iter()
                .any(|l| l.id == id && l.powered)
        };
        let mut result = TouchState::default();
        for e in &p.elements {
            match &e.element {
                PanelElement::Navigator {
                    load,
                    transponder,
                    audio_panel,
                } => {
                    let mut nav = if current {
                        self.touch.navigator.clone()
                    } else {
                        NavigatorState::default()
                    };
                    if let Some(radio) = self.sim.radios().receivers.first() {
                        nav.com_active_khz = self.comms.radios.first().map_or(122800, |r| r.active_khz);
                        nav.com_standby_khz = self.comms.radios.first().map_or(124000, |r| r.standby_khz);
                        nav.nav_active_khz = radio.active_hz / 1000;
                        nav.nav_standby_khz = radio.standby_hz / 1000;
                    }
                    nav.cdi_source = self.sim.radios().source;
                    nav.powered = powered(load);
                    nav.transponder = *transponder;
                    nav.audio_panel = *audio_panel;
                    result.navigator = Some(nav);
                }
                PanelElement::TouchDisplay {
                    load,
                    layout,
                    default_page,
                    engine_strip,
                    ..
                } => {
                    let runtime = if current {
                        self.touch.displays.get(&e.id).copied()
                    } else {
                        None
                    };
                    let default_svt = e.appearance.synthetic_vision || *layout == TouchLayout::IntegratedPfd;
                    let installed = default_page.unwrap_or(*layout);
                    result.displays.push(TouchDisplayState {
                        id: e.id.clone(),
                        layout: installed,
                        installed_layout: *layout,
                        default_page: installed,
                        data_area: DataArea::default(),
                        engine_strip: runtime.map_or(*engine_strip, |r| r.engine_strip),
                        synthetic_vision: runtime.map_or(default_svt, |r| r.synthetic_vision),
                        failed: runtime.is_some_and(|r| r.failed),
                        powered: powered(load),
                        reversion: false,
                        mfd_page: MfdPage::default(),
                        overlay: runtime.map_or(Overlay::None, |r| r.overlay),
                        knob_toggle_until_s: runtime.and_then(|r| r.toggle_until_s),
                        knobs: EfisConfig::default(),
                        model: e.appearance.bezel.as_ref().and_then(|b| b.model),
                        skin: None,
                        recommended_skin: e.appearance.recommended_skin.clone(),
                    });
                }
                _ => {}
            }
        }
        let prefs = self.display_prefs.get(self.callsign_variant());
        result.pilot_seat = prefs.map_or(PilotSeat::Left, |p| p.seat);
        // A right-seat pilot of a two-screen installation gets the two screens' defaults exchanged.
        if result.pilot_seat == PilotSeat::Right
            && let [a, b] = result.displays.as_mut_slice()
        {
            std::mem::swap(&mut a.default_page, &mut b.default_page);
        }
        for display in &mut result.displays {
            display.layout = prefs
                .and_then(|p| p.pages.get(&display.id).copied())
                .unwrap_or(display.default_page);
            display.data_area = prefs
                .and_then(|p| p.data_areas.get(&display.id).copied())
                .unwrap_or_default();
            display.mfd_page = prefs
                .and_then(|p| p.mfd_pages.get(&display.id).copied())
                .unwrap_or_default();
            display.knobs = prefs.map_or_else(EfisConfig::default, |p| p.knobs);
            display.skin = self.resolve_skin(
                display.model,
                prefs.and_then(|p| p.skins.get(&display.id)).map(String::as_str),
                display.recommended_skin.as_deref(),
            );
            if display.layout == TouchLayout::Svt {
                display.synthetic_vision = true;
            }
        }
        result.manual_reversion = current && self.touch.manual_reversion;
        // A lost display reverts the others to a primary flight display with the engine strip, and
        // display backup does the same for a screen that is showing something else.
        let lost = result.displays.iter().any(|d| d.failed || !d.powered);
        let backup = result.manual_reversion;
        for display in &mut result.displays {
            display.reversion =
                display.powered && !display.failed && (lost || (backup && !display.layout.is_pfd()));
        }
        result
    }
    fn save_display_layout(&mut self, id: &str, default_page: TouchLayout, layout: TouchLayout) {
        let variant = self.callsign_variant().to_owned();
        let entry = self.display_prefs.entry(variant.clone()).or_default();
        if layout == default_page {
            entry.pages.remove(id);
        } else {
            entry.pages.insert(id.to_owned(), layout);
        }
        self.prune_display_preferences(&variant);
    }
    /// Persist a key transition's layout and page, and keep its transient parts for this flight.
    fn apply_key_state(
        &mut self,
        id: &str,
        d: &TouchDisplayState,
        runtime: &mut DisplayRuntime,
        next: KeyState,
    ) {
        runtime.overlay = next.overlay;
        runtime.previous = next.previous;
        if next.layout != d.layout {
            self.save_display_layout(id, d.default_page, next.layout);
        }
        if next.page != d.mfd_page {
            let variant = self.callsign_variant().to_owned();
            let entry = self.display_prefs.entry(variant.clone()).or_default();
            if next.page == MfdPage::default() {
                entry.mfd_pages.remove(id);
            } else {
                entry.mfd_pages.insert(id.to_owned(), next.page);
            }
            self.prune_display_preferences(&variant);
        }
    }
    /// Turn a knob ring: the role decides what changes. A page change returns the new key state.
    fn apply_knob_role(
        &mut self,
        role: KnobRole,
        detents: i32,
        d: &TouchDisplayState,
    ) -> Result<Option<KeyState>, CommandError> {
        use crate::GlassSetting as G;
        match role {
            KnobRole::HeadingBug => self.sim.set_avionics(crate::AvionicsSetting::HeadingBugStep {
                detents,
                interval_ms: None,
            })?,
            KnobRole::AltitudeBug => {
                let m = (self.glass.selected_altitude_m + f64::from(detents) * 100. * 0.3048)
                    .clamp(-500., 15000.);
                self.set_glass(G::SelectedAltitude { altitude_m: m })?;
            }
            KnobRole::Baro => {
                let step = if self.glass.pressure_hpa { 100. } else { 33.8639 };
                let pa = (self.sim.systems().controls.altimeter_setting_pa + f64::from(detents) * step)
                    .clamp(80000., 110000.);
                self.set_glass(G::Baro { pressure_pa: pa })?;
            }
            KnobRole::Course => {
                let rad = (self.glass.course_rad + f64::from(detents).to_radians())
                    .rem_euclid(std::f64::consts::TAU);
                self.set_glass(G::Course { course_rad: rad })?;
            }
            KnobRole::MapRange => {
                let range = touch_efis::step_range(self.glass.range_nm, detents, false);
                self.set_glass(G::MapRange { range_nm: range })?;
            }
            KnobRole::MfdPage => {
                let current = touch_efis::effective_page(d.layout, d.mfd_page);
                let page = touch_efis::step_page(current, detents);
                let state = KeyState {
                    layout: d.layout,
                    installed_pfd: d.installed_layout,
                    page: d.mfd_page,
                    overlay: Overlay::None,
                    previous: None,
                };
                let mut next = touch_efis::show_page(state, page);
                next.previous = None;
                return Ok(Some(next));
            }
            // The engine has no flight director pitch bug to move.
            KnobRole::FlightDirectorBug => {}
        }
        Ok(None)
    }
    pub(crate) fn reset_touch_generation(&mut self) {
        if self.touch.generation != self.flight_generation {
            self.touch = TouchAvionics {
                generation: self.flight_generation,
                ..Default::default()
            };
        }
    }
    fn prune_display_preferences(&mut self, variant: &str) {
        if self.display_prefs.get(variant) == Some(&VariantDisplays::default()) {
            self.display_prefs.remove(variant);
        }
    }
    /// Left or right seat. The pilot's own page choices are cleared so the new defaults show.
    pub(crate) fn set_pilot_seat(&mut self, seat: PilotSeat) {
        let variant = self.callsign_variant().to_owned();
        let entry = self.display_prefs.entry(variant.clone()).or_default();
        if entry.seat != seat {
            entry.seat = seat;
            entry.pages.clear();
        }
        self.prune_display_preferences(&variant);
    }
    pub(crate) fn set_display_reversion(&mut self, enabled: bool) {
        self.reset_touch_generation();
        self.touch.manual_reversion = enabled;
    }
    /// Display choices to save, keyed by aircraft variant.
    pub fn display_preferences(&self) -> &DisplayPreferences {
        &self.display_prefs
    }
    /// Validate the complete file before replacing any current preference.
    pub fn restore_display_preferences(&mut self, values: DisplayPreferences) -> Result<(), CommandError> {
        let id_ok = |k: &str| {
            !k.is_empty()
                && k.len() <= 128
                && k.bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'_')
        };
        if values.len() > 1024
            || values.iter().any(|(k, v)| {
                !id_ok(k)
                    || v.pages.len() > 16
                    || v.data_areas.len() > 16
                    || v.mfd_pages.len() > 16
                    || v.skins.len() > 16
                    || v.pages
                        .keys()
                        .chain(v.data_areas.keys())
                        .chain(v.mfd_pages.keys())
                        .chain(v.skins.keys())
                        .any(|d| !id_ok(d))
                    || v.skins.values().any(|s| !id_ok(s))
            })
        {
            return Err(invalid("displays", "invalid saved display preferences"));
        }
        self.display_prefs = values;
        Ok(())
    }
    fn navigator_plan(&mut self, queries: Vec<String>) -> Result<(), CommandError> {
        if queries.len() > 64 {
            return Err(invalid("equipment.set", "at most 64 waypoints"));
        }
        let mut waypoints = Vec::new();
        for query in queries {
            let found = self.run(crate::SearchQuery {
                text: query.clone(),
                reference: None,
                limit: 100,
            })?;
            let result = found
                .results
                .iter()
                .find(|r| r.position.is_some() && r.ident.eq_ignore_ascii_case(query.trim()))
                .or_else(|| {
                    (found.results.len() == 1)
                        .then(|| found.results.first())
                        .flatten()
                })
                .ok_or_else(|| invalid("equipment.set", format!("unknown or ambiguous waypoint {query}")))?;
            let position = result
                .position
                .ok_or_else(|| invalid("equipment.set", "waypoint has no position"))?;
            let kind = match result.kind {
                crate::SearchKind::Airport => crate::WaypointKind::Airport,
                crate::SearchKind::Navaid => crate::WaypointKind::Navaid,
                crate::SearchKind::Fix => crate::WaypointKind::Fix,
                crate::SearchKind::Pin => crate::WaypointKind::Pin,
                _ => crate::WaypointKind::Coordinate,
            };
            waypoints.push(crate::GpsWaypoint {
                ident: result.ident.clone(),
                kind,
                position,
            });
        }
        let mut plan = self.navigation.plan.clone();
        plan.waypoints = waypoints
            .iter()
            .cloned()
            .map(|waypoint| crate::PlanWaypoint {
                waypoint,
                altitude_m: self.glass.selected_altitude_m,
            })
            .collect();
        plan.validate()?;
        self.set_glass(GlassSetting::FlightPlanWaypoints { waypoints })?;
        self.navigation.active_plan = Some(plan.clone());
        self.navigation.plan = plan;
        Ok(())
    }
    pub fn set_equipment(&mut self, setting: EquipmentSetting) -> Result<(), CommandError> {
        const NAME: &str = "equipment.set";
        self.reset_touch_generation();
        match setting {
            EquipmentSetting::AudioPanel { setting } => {
                setting.run(self)?;
            }
            EquipmentSetting::Systems { setting } => self
                .sim
                .set_systems(flightcraft_sim::systems::SystemSetting::Equipment { setting })?,
            EquipmentSetting::Display { id, action } => {
                let d = self
                    .touch_state()
                    .displays
                    .into_iter()
                    .find(|d| d.id == id)
                    .ok_or_else(|| invalid(NAME, "unknown display installation"))?;
                let mut runtime = DisplayRuntime {
                    engine_strip: d.engine_strip,
                    failed: d.failed,
                    synthetic_vision: d.synthetic_vision,
                    overlay: d.overlay,
                    toggle_until_s: d.knob_toggle_until_s,
                    previous: self.touch.displays.get(&id).and_then(|r| r.previous),
                };
                match action {
                    DisplayAction::SyntheticVision { enabled } => runtime.synthetic_vision = enabled,
                    DisplayAction::EngineStrip { visible } => runtime.engine_strip = visible,
                    DisplayAction::Failure { failed } => runtime.failed = failed,
                    DisplayAction::Layout { layout } => {
                        runtime.overlay = Overlay::None;
                        self.save_display_layout(&id, d.default_page, layout);
                    }
                    DisplayAction::MfdPage { page } => {
                        let next = touch_efis::show_page(key_state(&d, &runtime), page);
                        self.apply_key_state(&id, &d, &mut runtime, next);
                    }
                    DisplayAction::Key { key, hold } => {
                        let next = touch_efis::key(key_state(&d, &runtime), key, hold);
                        self.apply_key_state(&id, &d, &mut runtime, next);
                    }
                    DisplayAction::ToggleSplit {} => {
                        let next = touch_efis::toggle_split(key_state(&d, &runtime));
                        self.apply_key_state(&id, &d, &mut runtime, next);
                    }
                    DisplayAction::CloseOverlay {} => runtime.overlay = Overlay::None,
                    DisplayAction::Knob {
                        knob,
                        ring,
                        detents,
                        time_s,
                    } => {
                        if !(-100..=100).contains(&detents) {
                            return Err(invalid(NAME, "at most 100 knob detents"));
                        }
                        let toggled = runtime.toggle_until_s.is_some_and(|t| time_s < t);
                        let mode = if d.reversion {
                            touch_efis::ScreenMode::FullPfd
                        } else {
                            touch_efis::ScreenMode::of(d.layout)
                        };
                        let role = touch_efis::knob_role(mode, d.knobs, knob, ring, toggled);
                        if let Some(next) = self.apply_knob_role(role, detents, &d)? {
                            self.apply_key_state(&id, &d, &mut runtime, next);
                        }
                    }
                    DisplayAction::KnobPush { knob, time_s } => {
                        let toggled = runtime.toggle_until_s.is_some_and(|t| time_s < t);
                        let mode = if d.reversion {
                            touch_efis::ScreenMode::FullPfd
                        } else {
                            touch_efis::ScreenMode::of(d.layout)
                        };
                        match touch_efis::knob_push(mode, d.knobs, knob, toggled) {
                            PushEffect::ToggleFunctions => {
                                runtime.toggle_until_s =
                                    (!toggled).then_some(time_s + touch_efis::TOGGLE_REVERT_S);
                            }
                            PushEffect::SyncHeading => {
                                self.sim.set_avionics(crate::AvionicsSetting::HeadingBugSync {})?;
                            }
                            PushEffect::SyncCourse
                            | PushEffect::SyncFlightDirector
                            | PushEffect::CentreMap
                            | PushEffect::None => {}
                        }
                    }
                    DisplayAction::Knobs { config } => {
                        let variant = self.callsign_variant().to_owned();
                        self.display_prefs.entry(variant.clone()).or_default().knobs = config;
                        self.prune_display_preferences(&variant);
                    }
                    DisplayAction::Skin { skin } => {
                        if let Some(skin) = &skin
                            && skin != crate::skins::BUILT_IN_SKIN
                            && self.resolve_skin(d.model, Some(skin), None).is_none()
                        {
                            return Err(invalid(
                                NAME,
                                "skin is not installed or does not fit this display",
                            ));
                        }
                        let variant = self.callsign_variant().to_owned();
                        let entry = self.display_prefs.entry(variant.clone()).or_default();
                        match skin {
                            Some(skin) => {
                                entry.skins.insert(id.clone(), skin);
                            }
                            None => {
                                entry.skins.remove(&id);
                            }
                        }
                        self.prune_display_preferences(&variant);
                    }
                    DisplayAction::DirectTo { query } => {
                        self.navigator_plan(vec![query])?;
                        runtime.overlay = Overlay::None;
                    }
                    DisplayAction::DataArea { area } => {
                        let variant = self.callsign_variant().to_owned();
                        let entry = self.display_prefs.entry(variant.clone()).or_default();
                        if area == DataArea::default() {
                            entry.data_areas.remove(&id);
                        } else {
                            entry.data_areas.insert(id.clone(), area);
                        }
                        self.prune_display_preferences(&variant);
                    }
                }
                self.touch.displays.insert(id, runtime);
            }
            EquipmentSetting::Navigator { action } => {
                let nav = self
                    .touch_state()
                    .navigator
                    .ok_or_else(|| invalid(NAME, "navigator not installed"))?;
                if !nav.powered {
                    return Err(invalid(NAME, "navigator has no power"));
                }
                if let NavigatorAction::Shared { setting } = action {
                    if matches!(setting, GlassSetting::Transponder { .. }) && !nav.transponder {
                        return Err(invalid(NAME, "transponder control not installed here"));
                    }
                    self.set_glass(setting)?;
                    return Ok(());
                }
                match action {
                    NavigatorAction::Tune {
                        band: RadioBand::Com,
                        frequency_khz,
                    } => {
                        self.run(crate::CommsTune {
                            radio: 1,
                            standby: true,
                            channel_khz: Some(frequency_khz),
                            ..Default::default()
                        })?;
                        return Ok(());
                    }
                    NavigatorAction::Flip { band: RadioBand::Com } => {
                        self.run(crate::CommsTune {
                            radio: 1,
                            swap: true,
                            ..Default::default()
                        })?;
                        return Ok(());
                    }
                    NavigatorAction::Knob { outer, detents }
                        if self.touch.navigator.knob_band == RadioBand::Com =>
                    {
                        if !(-100..=100).contains(&detents) {
                            return Err(invalid(NAME, "at most 100 knob detents"));
                        }
                        let current = self.comms.radios.first().cloned().unwrap_or_default();
                        let eight = current.spacing == crate::ComSpacing::Khz833;
                        let mut channel = current.standby_khz;
                        if outer {
                            channel = (i64::from(channel) + i64::from(detents) * 1000 - 118000)
                                .rem_euclid(19000) as u32
                                + 118000;
                        } else {
                            for _ in 0..detents.unsigned_abs() {
                                loop {
                                    channel = (i64::from(channel) + if detents < 0 { -5 } else { 5 } - 118000)
                                        .rem_euclid(19000)
                                        as u32
                                        + 118000;
                                    if flightcraft_world::comms::valid_channel(channel, eight) {
                                        break;
                                    }
                                }
                            }
                        }
                        self.run(crate::CommsTune {
                            radio: 1,
                            standby: true,
                            channel_khz: Some(channel),
                            ..Default::default()
                        })?;
                        return Ok(());
                    }
                    NavigatorAction::Avionics { setting } => {
                        self.run(crate::SetAvionics { setting: *setting })?;
                        return Ok(());
                    }
                    NavigatorAction::DirectTo { query } => {
                        self.navigator_plan(vec![query])?;
                        return Ok(());
                    }
                    NavigatorAction::FlightPlan { queries } => {
                        self.navigator_plan(queries)?;
                        return Ok(());
                    }
                    NavigatorAction::Tune {
                        band: RadioBand::Nav,
                        frequency_khz,
                    } => {
                        if !(108000..=117950).contains(&frequency_khz) || frequency_khz % 50 != 0 {
                            return Err(invalid(NAME, "frequency outside NAV band or channel grid"));
                        }
                        self.sim.set_avionics(crate::AvionicsSetting::NavRadio {
                            setting: flightcraft_sim::avionics::radio::RadioSetting::Standby {
                                radio: 1,
                                frequency_hz: frequency_khz * 1000,
                            },
                        })?;
                        return Ok(());
                    }
                    NavigatorAction::Flip { band: RadioBand::Nav } => {
                        self.sim.set_avionics(crate::AvionicsSetting::NavRadio {
                            setting: flightcraft_sim::avionics::radio::RadioSetting::Swap { radio: 1 },
                        })?;
                        return Ok(());
                    }
                    NavigatorAction::Knob { outer, detents }
                        if self.touch.navigator.knob_band == RadioBand::Nav =>
                    {
                        if !(-100..=100).contains(&detents) {
                            return Err(invalid(NAME, "at most 100 knob detents"));
                        }
                        let freq = (i64::from(nav.nav_standby_khz)
                            + i64::from(detents) * if outer { 1000 } else { 50 }
                            - 108000)
                            .rem_euclid(10000) as u32
                            + 108000;
                        self.sim.set_avionics(crate::AvionicsSetting::NavRadio {
                            setting: flightcraft_sim::avionics::radio::RadioSetting::Standby {
                                radio: 1,
                                frequency_hz: freq * 1000,
                            },
                        })?;
                        return Ok(());
                    }
                    _ => {}
                }
                if let Some(radio) = self.comms.radios.first() {
                    self.touch.navigator.com_active_khz = radio.active_khz;
                    self.touch.navigator.com_standby_khz = radio.standby_khz;
                }
                let n = &mut self.touch.navigator;
                match action {
                    NavigatorAction::Page { page } => {
                        if page == NavigatorPage::Audio && !nav.audio_panel {
                            return Err(invalid(NAME, "audio control not installed here"));
                        }
                        if page == NavigatorPage::Transponder && !nav.transponder {
                            return Err(invalid(NAME, "transponder control not installed here"));
                        }
                        n.page = page;
                    }
                    NavigatorAction::Tune { band, frequency_khz } => {
                        let (lo, hi, step) = radio_range(band);
                        if !(lo..=hi).contains(&frequency_khz) || frequency_khz % step != 0 {
                            return Err(invalid(NAME, "frequency outside band or channel grid"));
                        }
                        *standby(n, band) = frequency_khz;
                    }
                    NavigatorAction::Flip { band } => match band {
                        RadioBand::Com => std::mem::swap(&mut n.com_active_khz, &mut n.com_standby_khz),
                        RadioBand::Nav => std::mem::swap(&mut n.nav_active_khz, &mut n.nav_standby_khz),
                    },
                    NavigatorAction::KnobBand { band } => n.knob_band = band,
                    NavigatorAction::Knob { outer, detents } => {
                        if !(-100..=100).contains(&detents) {
                            return Err(invalid(NAME, "at most 100 knob detents"));
                        }
                        let (lo, hi, step) = radio_range(n.knob_band);
                        let freq = standby(n, n.knob_band);
                        let delta = detents * if outer { 1000 } else { step as i32 };
                        *freq = (i64::from(*freq) + i64::from(delta) - i64::from(lo))
                            .rem_euclid(i64::from(hi - lo + step)) as u32
                            + lo;
                    }
                    NavigatorAction::Shared { .. }
                    | NavigatorAction::Avionics { .. }
                    | NavigatorAction::DirectTo { .. }
                    | NavigatorAction::FlightPlan { .. } => {}
                }
                if let Some(radio) = self.comms.radios.first_mut() {
                    radio.active_khz = n.com_active_khz;
                    radio.standby_khz = n.com_standby_khz;
                }
                self.comms.next_weather = self.sim.state().time_s;
            }
        }
        Ok(())
    }
}
fn radio_range(band: RadioBand) -> (u32, u32, u32) {
    match band {
        RadioBand::Com => (118000, 136975, 25),
        RadioBand::Nav => (108000, 117950, 50),
    }
}
fn standby(n: &mut NavigatorState, band: RadioBand) -> &mut u32 {
    match band {
        RadioBand::Com => &mut n.com_standby_khz,
        RadioBand::Nav => &mut n.nav_standby_khz,
    }
}
