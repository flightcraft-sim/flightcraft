//! Touch navigator and display behaviour. Navigation is owned by the session's single GPS.
use crate::{Command, CommandError, GlassSetting, Session, invalid};
use flightcraft_aero::modern::{PanelElement, TouchLayout};
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
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RadioBand {
    #[default]
    Com,
    Nav,
}
#[derive(Clone, Debug, Serialize, JsonSchema)]
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
            cdi_source: Default::default(),
        }
    }
}
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct TouchDisplayState {
    pub id: String,
    pub layout: TouchLayout,
    pub engine_strip: bool,
    pub synthetic_vision: bool,
    pub failed: bool,
    pub powered: bool,
    pub reversion: bool,
}
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
pub struct TouchState {
    pub navigator: Option<NavigatorState>,
    pub displays: Vec<TouchDisplayState>,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct TouchAvionics {
    pub generation: u64,
    pub navigator: NavigatorState,
    pub displays: BTreeMap<String, (TouchLayout, bool, bool, bool)>,
}
#[derive(Clone, Debug, Deserialize, JsonSchema)]
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
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DisplayAction {
    SyntheticVision { enabled: bool },
    Layout { layout: TouchLayout },
    EngineStrip { visible: bool },
    Failure { failed: bool },
}
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EquipmentSetting {
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
#[derive(Clone, Debug, Deserialize, JsonSchema)]
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
                PanelElement::Navigator { load, transponder } => {
                    let mut nav = if current {
                        self.touch.navigator.clone()
                    } else {
                        NavigatorState::default()
                    };
                    if let Some(radio) = self.sim.radios().receivers.first() {
                        nav.nav_active_khz = radio.active_hz / 1000;
                        nav.nav_standby_khz = radio.standby_hz / 1000;
                    }
                    nav.cdi_source = self.sim.radios().source;
                    nav.powered = powered(load);
                    nav.transponder = *transponder;
                    result.navigator = Some(nav);
                }
                PanelElement::TouchDisplay {
                    load,
                    layout,
                    engine_strip,
                    ..
                } => {
                    let (layout, engine_strip, failed, synthetic_vision) = if current {
                        self.touch.displays.get(&e.id).copied()
                    } else {
                        None
                    }
                    .unwrap_or((*layout, *engine_strip, false, false));
                    result.displays.push(TouchDisplayState {
                        id: e.id.clone(),
                        layout,
                        engine_strip,
                        synthetic_vision,
                        failed,
                        powered: powered(load),
                        reversion: false,
                    });
                }
                _ => {}
            }
        }
        let lost = result.displays.iter().any(|d| d.failed || !d.powered);
        for display in &mut result.displays {
            display.reversion = lost && display.powered && !display.failed;
        }
        result
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
        if self.touch.generation != self.flight_generation {
            self.touch = TouchAvionics {
                generation: self.flight_generation,
                ..Default::default()
            };
        }
        match setting {
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
                let mut values = (d.layout, d.engine_strip, d.failed, d.synthetic_vision);
                match action {
                    DisplayAction::SyntheticVision { enabled } => values.3 = enabled,
                    DisplayAction::Layout { layout } => values.0 = layout,
                    DisplayAction::EngineStrip { visible } => values.1 = visible,
                    DisplayAction::Failure { failed } => values.2 = failed,
                }
                self.touch.displays.insert(id, values);
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
                let n = &mut self.touch.navigator;
                match action {
                    NavigatorAction::Page { page } => {
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
