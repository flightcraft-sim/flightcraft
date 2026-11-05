//! Original, declaratively placed equipment faces. Interactions emit typed commands only.
use crate::panel_art::{color, fit_text};
use crate::{instruments::InstrumentInput, theme as t};
use eframe::egui::{self, Align2, FontId, Rect, Sense, Stroke};
use flightcraft_aero::modern::*;
use flightcraft_aero::panel_fidelity::{BackupCircuit, ControlStyle, NavigatorLayout};
use flightcraft_control::{
    AvionicsSetting, DisplayAction, EquipmentSetting, GlassSetting, NavigatorAction, NavigatorPage, RadioBand,
};
use flightcraft_sim::{
    modern::{KeyPosition, Lane, ModernSetting, Pump},
    systems::{FuelSelector, SystemSetting, SystemSwitch},
};

pub(crate) mod cockpit;

/// egui data key: the dash hand brake lever is held, so the app's keyboard path leaves the brakes on.
pub(crate) const PANEL_BRAKE_HELD: &str = "panel-brake-held";

#[derive(Default)]
pub(crate) struct PanelUi {
    pub dash_only: bool,
    console_open: bool,
    console: Option<Box<PanelUi>>,
    pub commands: Vec<EquipmentSetting>,
    pub avionics: Vec<AvionicsSetting>,
    controller: crate::autopilot::Controller,
    pub maps: Vec<(String, crate::map::MapViewport)>,
    pub map_widgets: std::collections::BTreeMap<String, crate::map::PlanningMap>,
    pub procedure_rect: Option<Rect>,
    pub procedures: crate::procedures::Controls,
    pull_owned: bool,
    key_owned: bool,
    /// The hand brake lever is held by the pointer on this panel.
    brake_owned: bool,
    entry: String,
}
fn text(ui: &egui::Ui, rect: Rect, label: &str, size: f32, color: egui::Color32) {
    let bounded = ui
        .data(|d| d.get_temp::<bool>(ui.id().with("bounded-caption")))
        .unwrap_or(false);
    if bounded {
        let desired = size.min((rect.width() - 4.).max(1.) / (label.len().max(1) as f32 * 0.61));
        fit_text(ui, rect, label, desired, color);
        return;
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::monospace(size.min((rect.width() - 4.).max(1.) / (label.len().max(1) as f32 * 0.61))),
        color,
    );
}
fn button(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash,
    rect: Rect,
    label: &str,
    lit: bool,
    font: f32,
) -> egui::Response {
    ui.painter().rect(
        rect,
        3.,
        if lit { t::BORDER } else { t::DIAL },
        Stroke::new(1.0_f32, if lit { t::ACCENT } else { t::BORDER }),
        egui::StrokeKind::Inside,
    );
    text(ui, rect, label, font, if lit { t::ACCENT } else { t::TEXT });
    ui.interact(rect, ui.id().with(id), Sense::click_and_drag())
}
const NAV_TILES: [(&str, NavigatorPage); 9] = [
    ("Flight plan", NavigatorPage::FlightPlan),
    ("Direct-to", NavigatorPage::DirectTo),
    ("Procedures", NavigatorPage::Procedures),
    ("Nearest", NavigatorPage::Nearest),
    ("Map", NavigatorPage::Map),
    ("Waypoint info", NavigatorPage::WaypointInfo),
    ("Frequencies", NavigatorPage::Frequencies),
    ("Transponder", NavigatorPage::Transponder),
    ("Audio", NavigatorPage::Audio),
];
fn caption_galley(
    ui: &egui::Ui,
    rect: Rect,
    label: &str,
    size: f32,
    color: egui::Color32,
) -> std::sync::Arc<egui::Galley> {
    let layout = |s: String| ui.painter().layout_no_wrap(s, FontId::monospace(size), color);
    let single = layout(label.into());
    if single.size().x <= rect.width() - 2. {
        return single;
    }
    label
        .match_indices(' ')
        .map(|(i, _)| {
            layout(format!(
                "{}\n{}",
                label.get(..i).unwrap_or(label),
                label.get(i + 1..).unwrap_or("")
            ))
        })
        .min_by(|a, b| a.size().x.total_cmp(&b.size().x))
        .unwrap_or(single)
}
fn shared_caption_size(ui: &egui::Ui, captions: &[(Rect, String)], maximum: f32) -> f32 {
    let (mut low, mut high) = (0.01, maximum.max(0.01));
    for _ in 0..20 {
        let mid = (low + high) * 0.5;
        if captions.iter().all(|(rect, label)| {
            let g = caption_galley(ui, *rect, label, mid, t::TEXT);
            g.size().x <= rect.width() - 2. && g.size().y <= rect.height() - 1.
        }) {
            low = mid;
        } else {
            high = mid;
        }
    }
    low
}
fn uniform_text(ui: &egui::Ui, rect: Rect, label: &str, size: f32, color: egui::Color32) {
    let galley = caption_galley(ui, rect, label, size, color);
    ui.painter()
        .with_clip_rect(ui.clip_rect().intersect(rect))
        .galley(rect.center() - galley.size() * 0.5, galley, color);
}
fn uniform_button(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash,
    rect: Rect,
    label: &str,
    lit: bool,
    font: f32,
) -> egui::Response {
    ui.painter().rect(
        rect,
        3.,
        if lit { t::BORDER } else { t::DIAL },
        Stroke::new(1.0_f32, if lit { t::ACCENT } else { t::BORDER }),
        egui::StrokeKind::Inside,
    );
    uniform_text(ui, rect, label, font, if lit { t::ACCENT } else { t::TEXT });
    ui.interact(rect, ui.id().with(id), Sense::click_and_drag())
}
fn part(rect: Rect, x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::from_min_size(
        rect.min + egui::vec2(x * rect.width(), y * rect.height()),
        egui::vec2(w * rect.width(), h * rect.height()),
    )
}
impl PanelUi {
    pub(crate) fn cancel_holds(&mut self) -> Vec<EquipmentSetting> {
        let mut commands = Vec::new();
        if self.pull_owned {
            commands.push(EquipmentSetting::Systems {
                setting: ModernSetting::ParachuteHandle { held: false },
            });
            self.pull_owned = false;
        }
        if self.key_owned {
            commands.push(EquipmentSetting::Systems {
                setting: ModernSetting::Key {
                    position: KeyPosition::On,
                },
            });
            self.key_owned = false;
        }
        if let Some(console) = &mut self.console {
            commands.extend(console.cancel_holds());
        }
        commands
    }
    fn system(&mut self, setting: ModernSetting) {
        self.commands.push(EquipmentSetting::Systems { setting });
    }
    fn nav(&mut self, action: NavigatorAction) {
        self.commands.push(EquipmentSetting::Navigator { action });
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        panel: &EquipmentPanel,
        input: InstrumentInput<'_>,
        registry: &crate::instruments::Registry,
    ) -> Vec<SystemSetting> {
        self.maps.clear();
        self.procedure_rect = None;
        let (available, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
        let [width, _] = panel.size;
        let dash = self
            .dash_only
            .then(|| crate::panel_art::dash_bounds(panel))
            .flatten();
        let rect = if dash.is_some() {
            crate::panel_preview::dash_rect(panel, available)
        } else {
            crate::panel_preview::equipment_rect(panel, available)
        };
        let scale = rect.width() / width as f32;
        let registration = input
            .registration
            .or_else(|| input.aircraft.map(|a| a.variant.identity.registration.as_str()))
            .unwrap_or_default()
            .to_owned();
        #[cfg(test)]
        {
            let visible = dash.map_or(rect, |r| {
                Rect::from_min_size(rect.min + r.min.to_vec2() * scale, r.size() * scale)
            });
            ui.ctx()
                .data_mut(|data| data.insert_temp(egui::Id::new("painted-equipment-bounds"), visible));
        }
        if panel.shell.is_none() {
            ui.painter().rect(
                rect,
                if panel.shape == PanelShape::Rounded {
                    14.
                } else {
                    0.
                },
                match panel.color {
                    PanelColor::Panel => t::PANEL,
                    PanelColor::Raised => t::RAISED,
                    PanelColor::Dial => t::DIAL,
                },
                Stroke::new(1.0_f32, t::BORDER),
                egui::StrokeKind::Inside,
            );
        }
        if dash.is_some() {
            let mut artwork = panel.clone();
            if let Some(shell) = &mut artwork.shell {
                shell.centre_console = None;
                shell.floor = None;
            }
            crate::panel_art::shell(ui, rect, &artwork);
        } else {
            crate::panel_art::shell(ui, rect, panel);
        }
        let Some(glass) = input.glass else {
            return Vec::new();
        };
        let modern = input.systems.modern.as_ref();
        let font = (12. * scale).max(6.);
        let palette = panel.shell.as_ref().map(|s| &s.palette);
        let placard = color(palette.and_then(|p| p.placard_text.as_ref()), t::TEXT);
        let bezel = color(palette.and_then(|p| p.bezel.as_ref()), t::DIAL);
        let mut settings = Vec::new();
        let mut held = false;
        let mut start_held = false;
        let hosted_audio = panel.elements.iter().any(|e| {
            matches!(
                e.element,
                PanelElement::Navigator {
                    audio_panel: true,
                    ..
                }
            )
        });
        for e in &panel.elements {
            if dash.is_some_and(|r| {
                !r.expand(0.01)
                    .contains_rect(crate::panel_art::installation_rect(e))
            }) {
                continue;
            }
            if hosted_audio && matches!(e.element, PanelElement::AudioPanel) {
                continue;
            }
            let [x, y] = e.position;
            let [w, h] = e.size;
            let cell = Rect::from_min_size(
                rect.min + egui::vec2(x as f32, y as f32) * scale,
                egui::vec2(w as f32, h as f32) * scale,
            );
            ui.push_id(&e.id, |ui| {
                let bounded = e.appearance != Default::default()
                    || (e.size[0] < 34. && matches!(e.element, PanelElement::MasterLight { .. }))
                    || (e.size[0] < 90. && matches!(e.element, PanelElement::Parachute))
                    || (e.size[1] < 120. && matches!(e.element, PanelElement::Navigator { .. }));
                ui.data_mut(|d| d.insert_temp(ui.id().with("bounded-caption"), bounded));
                let skin = registry.equipment_skin(e);
                let opacity = ui.opacity();
                let map_count = self.maps.len();
                if skin.is_some() {
                    ui.set_opacity(0.);
                }
                match &e.element {
                    PanelElement::AudioPanel => self.audio_panel(ui, cell, &glass.comms, font),
                    PanelElement::TouchDisplay {
                        composition,
                        layout: default_layout,
                        ..
                    } => {
                        if let Some(d) = glass.touch.displays.iter().find(|d| d.id == e.id) {
                            if let (Some(model), true) = (d.model, e.appearance.bezel.is_some()) {
                                let style = d
                                    .skin
                                    .as_deref()
                                    .and_then(|id| input.skin(id))
                                    .map_or_else(crate::efis::Style::built_in, |skin| {
                                        crate::efis::Style::from_skin(ui.ctx(), skin)
                                    });
                                let bezel_rect = crate::efis::bezel_rect(cell, model);
                                crate::efis::paint_bezel(ui, bezel_rect, model, &style);
                                let screen = crate::efis::screen_rect(bezel_rect, model);
                                self.maps.extend(
                                    crate::glass::paint_touch(ui, screen, glass, input, d, composition)
                                        .into_iter()
                                        .enumerate()
                                        .map(|(i, rect)| (format!("{}-{i}", e.id), rect)),
                                );
                                let mut out = crate::efis::Output::default();
                                crate::efis::controls(ui, bezel_rect, model, &e.id, &mut out);
                                crate::efis::chrome(
                                    ui,
                                    screen,
                                    &e.id,
                                    d,
                                    &style,
                                    &crate::efis::Shared {
                                        backup: glass.touch.manual_reversion,
                                        traffic_shown: glass.settings.traffic_shown,
                                        track_up: glass.settings.orientation
                                            != flightcraft_sim::avionics::glass::MapOrientation::NorthUp,
                                        pressure_hpa: glass.settings.pressure_hpa,
                                        range_nm: glass.settings.range_nm,
                                        com_active_khz: glass.comms.com1.active_khz,
                                        com_standby_khz: glass.comms.com1.standby_khz,
                                        nav_active_khz: glass
                                            .touch
                                            .navigator
                                            .as_ref()
                                            .map(|n| n.nav_active_khz),
                                        nav_standby_khz: glass
                                            .touch
                                            .navigator
                                            .as_ref()
                                            .map(|n| n.nav_standby_khz),
                                        xpdr_code: glass.settings.transponder_code.clone(),
                                        xpdr_mode: glass.settings.transponder_mode,
                                        cas_active: !glass.cas.messages.is_empty(),
                                        cas_overflow: glass.cas.overflow(),
                                        cas: glass.cas.clone(),
                                        ident_remaining_s: glass.settings.ident_remaining_s,
                                        user_field: glass.settings.data_bar_field,
                                        vspeeds: glass.settings.vspeeds,
                                        pack_vspeeds_kt: input.aircraft.map_or([None; 3], |a| {
                                            let v = &a.variant.v_speeds;
                                            [v.vr_mps, v.vx_mps, v.vy_mps]
                                                .map(|m| Some(m * 1.943_844_492_440_6))
                                        }),
                                        fpl: crate::efis::FplView {
                                            rows: glass
                                                .gps
                                                .legs
                                                .iter()
                                                .map(|leg| crate::efis::FplRow {
                                                    ident: leg.airport.clone(),
                                                    bearing_deg: leg.bearing_rad.to_degrees() as f32,
                                                    distance_nm: leg.distance_nm as f32,
                                                    constraint: leg.constraint.clone(),
                                                })
                                                .collect(),
                                            cursor: glass.settings.fpl_cursor,
                                            active: glass.gps.active_leg,
                                        },
                                    },
                                    &mut out,
                                );
                                self.commands.extend(out.equipment);
                                self.avionics.extend(
                                    out.glass
                                        .into_iter()
                                        .map(|setting| AvionicsSetting::Glass { setting }),
                                );
                            } else {
                                let b = e.appearance.bezel.as_ref();
                                let screen = b.map_or_else(
                                    || part(cell, 0.02, 0.03, 0.96, 0.84),
                                    |b| {
                                        let [l, t, r, bottom] = b.margins;
                                        Rect::from_min_max(
                                            cell.min + egui::vec2(l as f32, t as f32) * scale,
                                            cell.max - egui::vec2(r as f32, bottom as f32) * scale,
                                        )
                                    },
                                );
                                ui.painter().rect_filled(
                                    cell,
                                    6.,
                                    color(b.and_then(|b| b.color.as_ref()), bezel),
                                );
                                if let Some(trim) = b.and_then(|b| b.edge_trim.as_ref()) {
                                    ui.painter().rect_stroke(
                                        cell,
                                        6.,
                                        Stroke::new(scale.max(0.5), color(Some(trim), t::MUTED)),
                                        egui::StrokeKind::Inside,
                                    );
                                }
                                if b.is_some_and(|b| b.card_slot) {
                                    ui.painter().rect_filled(
                                        part(cell, 0.43, 0.955, 0.14, 0.012),
                                        1.,
                                        t::BORDER,
                                    );
                                }
                                let key_count = b.map_or(5, |b| {
                                    if b.softkeys {
                                        usize::from(b.softkey_count)
                                    } else {
                                        0
                                    }
                                });
                                self.maps.extend(
                                    crate::glass::paint_touch(ui, screen, glass, input, d, composition)
                                        .into_iter()
                                        .enumerate()
                                        .map(|(i, rect)| (format!("{}-{i}", e.id), rect)),
                                );
                                if b.is_none() {
                                    for (i, (label, layout)) in [
                                        (
                                            "PFD",
                                            if *default_layout == TouchLayout::IntegratedPfd {
                                                TouchLayout::IntegratedPfd
                                            } else {
                                                TouchLayout::FullPfd
                                            },
                                        ),
                                        ("PFD / MAP", TouchLayout::PfdMap),
                                        ("MAP / ENG", TouchLayout::MapEngine),
                                    ]
                                    .into_iter()
                                    .enumerate()
                                    {
                                        if button(
                                            ui,
                                            i,
                                            part(cell, 0.075 + i as f32 * 0.175, 0.89, 0.168, 0.09),
                                            label,
                                            d.layout == layout,
                                            font,
                                        )
                                        .clicked()
                                        {
                                            self.commands.push(EquipmentSetting::Display {
                                                id: e.id.clone(),
                                                action: DisplayAction::Layout { layout },
                                            });
                                        }
                                    }
                                    if button(
                                        ui,
                                        "strip",
                                        part(cell, 0.775, 0.89, 0.15, 0.09),
                                        "ENG",
                                        d.engine_strip,
                                        font,
                                    )
                                    .clicked()
                                    {
                                        self.commands.push(EquipmentSetting::Display {
                                            id: e.id.clone(),
                                            action: DisplayAction::EngineStrip {
                                                visible: !d.engine_strip,
                                            },
                                        });
                                    }
                                    if button(
                                        ui,
                                        "vision",
                                        part(cell, 0.60, 0.89, 0.168, 0.09),
                                        "SVT",
                                        d.synthetic_vision,
                                        font,
                                    )
                                    .clicked()
                                    {
                                        self.commands.push(EquipmentSetting::Display {
                                            id: e.id.clone(),
                                            action: DisplayAction::SyntheticVision {
                                                enabled: !d.synthetic_vision,
                                            },
                                        });
                                    }
                                } else {
                                    for i in 0..key_count {
                                        let (label, action, lit) = match i % 5 {
                                            0 => (
                                                "PFD",
                                                DisplayAction::Layout {
                                                    layout: if *default_layout == TouchLayout::IntegratedPfd {
                                                        TouchLayout::IntegratedPfd
                                                    } else {
                                                        TouchLayout::FullPfd
                                                    },
                                                },
                                                matches!(
                                                    d.layout,
                                                    TouchLayout::FullPfd | TouchLayout::IntegratedPfd
                                                ),
                                            ),
                                            1 => (
                                                "MAP",
                                                // The MFD page; pressed again, it steps the data column.
                                                if d.layout == TouchLayout::Mfd {
                                                    DisplayAction::DataArea {
                                                        area: match d.data_area {
                                                            DataArea::Engine => DataArea::FlightPlan,
                                                            DataArea::FlightPlan => DataArea::Nearest,
                                                            DataArea::Nearest => DataArea::Traffic,
                                                            DataArea::Traffic => DataArea::Engine,
                                                        },
                                                    }
                                                } else {
                                                    DisplayAction::Layout {
                                                        layout: TouchLayout::Mfd,
                                                    }
                                                },
                                                d.layout == TouchLayout::Mfd,
                                            ),
                                            2 if !d.layout.is_pfd() && d.layout != TouchLayout::MapEngine => {
                                                (
                                                    "ENG",
                                                    DisplayAction::Layout {
                                                        layout: if d.layout == TouchLayout::Engine {
                                                            TouchLayout::Mfd
                                                        } else {
                                                            TouchLayout::Engine
                                                        },
                                                    },
                                                    d.layout == TouchLayout::Engine,
                                                )
                                            }
                                            2 => (
                                                "ENG",
                                                DisplayAction::EngineStrip {
                                                    visible: !d.engine_strip,
                                                },
                                                d.engine_strip,
                                            ),
                                            3 => (
                                                "SVT",
                                                DisplayAction::SyntheticVision {
                                                    enabled: !d.synthetic_vision,
                                                },
                                                d.synthetic_vision,
                                            ),
                                            _ => (
                                                "LIST",
                                                DisplayAction::Layout {
                                                    layout: match d.layout {
                                                        TouchLayout::FlightPlan => TouchLayout::Nearest,
                                                        TouchLayout::Nearest => TouchLayout::Traffic,
                                                        _ => TouchLayout::FlightPlan,
                                                    },
                                                },
                                                matches!(
                                                    d.layout,
                                                    TouchLayout::FlightPlan
                                                        | TouchLayout::Nearest
                                                        | TouchLayout::Traffic
                                                ),
                                            ),
                                        };
                                        let key = part(
                                            cell,
                                            0.14 + i as f32 * 0.72 / key_count as f32,
                                            0.89,
                                            0.64 / key_count as f32,
                                            0.055,
                                        );
                                        if button(ui, ("softkey", i), key, label, lit, font).clicked() {
                                            self.commands.push(EquipmentSetting::Display {
                                                id: e.id.clone(),
                                                action,
                                            });
                                        }
                                    }
                                }
                                let knobs: Vec<_> = b.map_or_else(
                                    || {
                                        vec![
                                            part(cell, 0.015, 0.89, 0.045, 0.09),
                                            part(cell, 0.94, 0.89, 0.045, 0.09),
                                        ]
                                    },
                                    |b| {
                                        b.corner_knobs
                                            .iter()
                                            .map(|[x, y]| {
                                                Rect::from_center_size(
                                                    cell.min + egui::vec2(*x as f32, *y as f32) * scale,
                                                    egui::Vec2::splat(cell.width() * 0.045),
                                                )
                                            })
                                            .collect()
                                    },
                                );
                                for (i, knob) in knobs.into_iter().enumerate() {
                                    let side = i == 1;
                                    ui.painter().circle_stroke(
                                        knob.center(),
                                        knob.width() * 0.45,
                                        Stroke::new(1.5_f32, t::MUTED),
                                    );
                                    let response = ui.interact(
                                        knob,
                                        ui.id().with(("corner", side)),
                                        Sense::click_and_drag(),
                                    );
                                    if response.hovered() {
                                        let wheel = ui.input(|i| i.raw_scroll_delta.y);
                                        if wheel != 0. {
                                            self.avionics.push(AvionicsSetting::Glass {
                                                setting: if side {
                                                    GlassSetting::MapRange {
                                                        range_nm: (glass.settings.range_nm
                                                            * if wheel > 0. { 0.8 } else { 1.25 })
                                                        .clamp(1., 200.),
                                                    }
                                                } else {
                                                    GlassSetting::HeadingBug {
                                                        heading_rad: (glass.settings.heading_bug_rad
                                                            + f64::from(wheel.signum()).to_radians())
                                                        .rem_euclid(std::f64::consts::TAU),
                                                    }
                                                },
                                            });
                                        }
                                    }
                                    if response.clicked() && !side {
                                        self.avionics.push(AvionicsSetting::HeadingBugSync {});
                                    }
                                }
                                if d.reversion {
                                    text(ui, part(cell, 0.2, 0., 0.6, 0.06), "REVERSION", font, t::AMBER);
                                }
                            }
                        }
                    }
                    PanelElement::Navigator { .. } => {
                        self.navigator(ui, cell, input, font, e.appearance.navigator_layout, bezel)
                    }
                    PanelElement::Standby => crate::glass::paint_standby(ui, cell, glass, input),
                    PanelElement::Autopilot => {
                        if let Some(layout) = &e.appearance.controller {
                            self.avionics.extend(
                                self.controller
                                    .show_declared(ui, cell, e.size, layout, input, bezel),
                            );
                        } else if let Some(state) = input.autopilot {
                            self.avionics.extend(self.controller.show_in(
                                ui,
                                cell,
                                state,
                                input.systems.controls.heading_bug_rad,
                                false,
                            ));
                        }
                    }
                    PanelElement::Breaker { load, label } => {
                        if let Some(l) = input.systems.electrical.loads.iter().find(|l| &l.id == load) {
                            let center = part(cell, 0., 0.26, 1., 0.64).center()
                                + egui::vec2(0., if l.breaker_closed { 0. } else { 3. * scale });
                            let radius = cell.width().min(cell.height()) * 0.20;
                            let amps = input
                                .aircraft
                                .and_then(|c| c.installed_systems.definition.as_ref())
                                .and_then(|d| d.electrical.loads.iter().find(|d| &d.id == load))
                                .map_or(0., |d| d.breaker_amps);
                            ui.painter().circle_filled(
                                center,
                                radius * 1.30,
                                if l.breaker_closed { t::BORDER } else { t::TEXT },
                            );
                            ui.painter().circle_filled(center, radius, t::DIAL);
                            ui.painter().circle_stroke(
                                center,
                                radius * 1.25,
                                Stroke::new(
                                    if l.tripped { 2. * scale } else { 0.7 },
                                    if l.tripped { t::DANGER } else { t::MUTED },
                                ),
                            );
                            text(
                                ui,
                                Rect::from_center_size(center, egui::vec2(radius * 2., radius * 2.)),
                                &format!("{amps:.0}"),
                                radius * 0.95,
                                t::TEXT,
                            );
                            text(ui, part(cell, 0., 0.02, 1., 0.20), label, font * 0.72, t::TEXT);
                            if ui
                                .interact(cell, ui.id().with("pull"), Sense::click())
                                .on_hover_text(if l.breaker_closed {
                                    "Pull breaker to open"
                                } else {
                                    "Push breaker to reset"
                                })
                                .clicked()
                            {
                                settings.push(SystemSetting::Breaker {
                                    load: load.clone(),
                                    closed: !l.breaker_closed,
                                });
                            }
                        }
                    }
                    PanelElement::Rocker {
                        switch,
                        label,
                        backlit,
                        sublabel,
                    } => {
                        let c = &input.systems.controls;
                        let (on, setting) = match switch {
                            Rocker::Beacon
                            | Rocker::Nav
                            | Rocker::Strobe
                            | Rocker::Taxi
                            | Rocker::Landing => (
                                modern.is_some_and(|m| {
                                    m.light_switches
                                        .iter()
                                        .any(|(light, on)| Some(*light) == rocker_light(*switch) && *on)
                                }),
                                None,
                            ),
                            Rocker::LandingWigwag => (
                                modern.is_some_and(|m| {
                                    m.light_switches
                                        .iter()
                                        .any(|(s, on)| *s == flightcraft_aero::LoadSwitch::Landing && *on)
                                }),
                                None,
                            ),
                            Rocker::EmsBackup | Rocker::EfisBackup => (
                                modern.is_some_and(|m| {
                                    m.backup_buses.iter().any(|b| {
                                        b.circuit
                                            == if *switch == Rocker::EmsBackup {
                                                BackupCircuit::Ems
                                            } else {
                                                BackupCircuit::Efis
                                            }
                                            && b.on
                                    })
                                }),
                                None,
                            ),
                            Rocker::LaneA => (modern.is_some_and(|m| m.lane_a.on), None),
                            Rocker::LaneB => (modern.is_some_and(|m| m.lane_b.on), None),
                            Rocker::Pump1 => (modern.is_some_and(|m| m.pump_1.on), None),
                            Rocker::Pump2 => (modern.is_some_and(|m| m.pump_2.on), None),
                            Rocker::Master => (c.master, Some(SystemSwitch::Master)),
                            Rocker::Avionics => (c.avionics, Some(SystemSwitch::Avionics)),
                            Rocker::Alternator => (c.alternator, Some(SystemSwitch::Alternator)),
                            Rocker::Lights => (c.lights, Some(SystemSwitch::Lights)),
                            Rocker::PitotHeat => (c.pitot_heat, Some(SystemSwitch::PitotHeat)),
                        };
                        let above = e.appearance.style == Some(ControlStyle::BlackRocker);
                        let face = part(
                            cell,
                            0.2,
                            if above { 0.28 } else { 0.05 },
                            0.6,
                            if above { 0.5 } else { 0.66 },
                        );
                        ui.painter().rect_filled(face, 3., t::DIAL);
                        ui.painter().rect_filled(
                            part(face, 0.15, if on { 0.10 } else { 0.65 }, 0.7, 0.15),
                            1.,
                            if on { t::GREEN } else { t::BORDER },
                        );
                        text(
                            ui,
                            part(
                                cell,
                                0.,
                                if above { 0.02 } else { 0.72 },
                                1.,
                                if above { 0.22 } else { 0.18 },
                            ),
                            label,
                            font * 0.85,
                            if *backlit {
                                t::TEXT.linear_multiply(if input.systems.electrical.main_bus_voltage > 9. {
                                    1.
                                } else {
                                    0.22
                                })
                            } else {
                                placard
                            },
                        );
                        if let Some(sublabel) = sublabel {
                            text(ui, part(cell, 0., 0.90, 1., 0.1), sublabel, font * 0.6, t::MUTED);
                        }
                        let fault = modern.is_some_and(|m| match switch {
                            Rocker::LaneA => m.lane_a.light,
                            Rocker::LaneB => m.lane_b.light,
                            _ => false,
                        });
                        if fault {
                            ui.painter().circle_filled(face.right_top(), 3. * scale, t::AMBER);
                        }
                        if ui
                            .interact(cell, ui.id().with("rocker"), Sense::click())
                            .clicked()
                        {
                            if *switch == Rocker::LandingWigwag {
                                let wigwag = modern.is_some_and(|m| m.wigwag);
                                if on && !wigwag {
                                    self.system(ModernSetting::Wigwag { on: true });
                                } else {
                                    if wigwag {
                                        self.system(ModernSetting::Wigwag { on: false });
                                    }
                                    self.system(ModernSetting::Light {
                                        light: flightcraft_aero::LoadSwitch::Landing,
                                        on: !on,
                                    });
                                }
                            } else if matches!(switch, Rocker::EmsBackup | Rocker::EfisBackup) {
                                self.system(ModernSetting::BackupPower {
                                    circuit: if *switch == Rocker::EmsBackup {
                                        BackupCircuit::Ems
                                    } else {
                                        BackupCircuit::Efis
                                    },
                                    on: !on,
                                });
                            } else if let Some(switch) = setting {
                                settings.push(SystemSetting::Switch { switch, on: !on });
                            } else {
                                self.system(match switch {
                                    Rocker::Beacon
                                    | Rocker::Nav
                                    | Rocker::Strobe
                                    | Rocker::Taxi
                                    | Rocker::Landing => ModernSetting::Light {
                                        light: rocker_light(*switch)
                                            .unwrap_or(flightcraft_aero::LoadSwitch::Lights),
                                        on: !on,
                                    },
                                    Rocker::LaneA => ModernSetting::Lane {
                                        lane: Lane::A,
                                        on: !on,
                                    },
                                    Rocker::LaneB => ModernSetting::Lane {
                                        lane: Lane::B,
                                        on: !on,
                                    },
                                    Rocker::Pump1 => ModernSetting::Pump {
                                        pump: Pump::One,
                                        on: !on,
                                    },
                                    _ => ModernSetting::Pump {
                                        pump: Pump::Two,
                                        on: !on,
                                    },
                                });
                            }
                        }
                    }
                    PanelElement::MasterLight { severity } => {
                        let lit = modern.is_some_and(|m| match severity {
                            Severity::Caution => m.master_caution,
                            Severity::Warning => m.master_warning,
                        });
                        let color = match severity {
                            Severity::Caution => t::AMBER,
                            Severity::Warning => t::DANGER,
                        };
                        let center = part(cell, 0., 0.25, 1., 0.75).center();
                        let radius = cell.width().min(cell.height()) * 0.30;
                        ui.painter().circle_filled(center, radius * 1.18, t::DIAL);
                        ui.painter()
                            .circle_stroke(center, radius * 1.12, Stroke::new(1.0_f32, t::BORDER));
                        if lit {
                            ui.painter()
                                .circle_filled(center, radius * 1.02, color.linear_multiply(0.35));
                        }
                        ui.painter().circle_filled(
                            center,
                            radius * 0.85,
                            color.linear_multiply(if lit { 1. } else { 0.14 }),
                        );
                        if lit {
                            ui.painter().circle_filled(
                                center - egui::vec2(radius * 0.22, radius * 0.22),
                                radius * 0.24,
                                t::TEXT.linear_multiply(0.55),
                            );
                        }
                        text(
                            ui,
                            part(cell, 0., 0., 1., 0.20),
                            match severity {
                                Severity::Caution => "CAUTION",
                                Severity::Warning => "WARNING",
                            },
                            font * 0.85,
                            t::TEXT,
                        );
                        if ui
                            .interact(cell, ui.id().with("ack"), Sense::click())
                            .on_hover_text(
                                modern
                                    .map(|m| {
                                        m.annunciations
                                            .iter()
                                            .filter(|a| a.active && a.severity == *severity)
                                            .map(|a| a.label.as_str())
                                            .collect::<Vec<_>>()
                                            .join("\n")
                                    })
                                    .unwrap_or_default(),
                            )
                            .clicked()
                        {
                            self.system(ModernSetting::Acknowledge { severity: *severity });
                        }
                    }
                    PanelElement::Key => {
                        let pos = modern.map_or(KeyPosition::Off, |m| m.key);
                        let pivot = cell.center();
                        let radius = cell.width().min(cell.height()) * 0.23;
                        ui.painter().circle_filled(pivot, radius * 1.2, t::MUTED);
                        ui.painter().circle_filled(pivot, radius, t::DIAL);
                        let angle = match pos {
                            KeyPosition::Off => -150_f32,
                            KeyPosition::On => -60.,
                            KeyPosition::Start => 0.,
                        }
                        .to_radians();
                        ui.painter().line_segment(
                            [
                                pivot - egui::vec2(angle.cos(), angle.sin()) * radius * 0.3,
                                pivot + egui::vec2(angle.cos(), angle.sin()) * radius * 0.85,
                            ],
                            Stroke::new(radius * 0.30, t::TEXT),
                        );
                        for (label, position, x, y) in [
                            ("OFF", KeyPosition::Off, 0.02, 0.08),
                            ("ON", KeyPosition::On, 0.65, 0.08),
                            ("START", KeyPosition::Start, 0.72, 0.48),
                        ] {
                            let area = part(cell, x, y, 0.28, 0.24);
                            text(ui, area, label, font * 0.65, t::TEXT);
                            let response = ui.interact(area, ui.id().with(label), Sense::click_and_drag());
                            if position == KeyPosition::Start {
                                start_held = response.is_pointer_button_down_on();
                                if start_held && !self.key_owned {
                                    self.system(ModernSetting::Key { position });
                                    self.key_owned = true;
                                }
                            } else if response.clicked() {
                                self.system(ModernSetting::Key { position });
                                self.key_owned = false;
                            }
                        }
                        text(
                            ui,
                            part(cell, 0., 0.88, 1., 0.12),
                            "IGNITION",
                            font * 0.65,
                            t::MUTED,
                        );
                    }
                    PanelElement::FuelSelector => {
                        let pivot = cell.center();
                        let radius = cell.height() * 0.32;
                        text(
                            ui,
                            part(
                                cell,
                                0.,
                                0.,
                                1.,
                                if e.appearance.style == Some(ControlStyle::RedTHandle) {
                                    0.10
                                } else {
                                    0.13
                                },
                            ),
                            "FUEL SELECTOR",
                            font,
                            t::MUTED,
                        );
                        ui.painter().circle_filled(pivot, radius, t::DIAL);
                        ui.painter()
                            .circle_stroke(pivot, radius, Stroke::new(1.0_f32, t::MUTED));
                        let open = modern
                            .and_then(|m| m.panel_guards.get(&e.id))
                            .copied()
                            .unwrap_or(false);
                        for (label, position, degrees) in [
                            ("LEFT", FuelSelector::Left, 225_f32),
                            ("RIGHT", FuelSelector::Right, 315.),
                            ("OFF", FuelSelector::Off, 90.),
                        ] {
                            let angle = degrees.to_radians();
                            let direction = egui::vec2(angle.cos(), angle.sin());
                            let target = pivot + direction * radius * 1.35;
                            let target_rect = Rect::from_center_size(
                                target,
                                egui::vec2(cell.width() * 0.25, cell.height() * 0.15),
                            );
                            text(
                                ui,
                                target_rect,
                                label,
                                font,
                                if position == FuelSelector::Off {
                                    t::DANGER
                                } else {
                                    t::TEXT
                                },
                            );
                            if ui
                                .interact(target_rect, ui.id().with(label), Sense::click())
                                .clicked()
                                && (position != FuelSelector::Off || open)
                            {
                                settings.push(SystemSetting::FuelSelector { position });
                            }
                            if input.systems.controls.fuel_selector == position {
                                ui.painter().line_segment(
                                    [
                                        pivot - direction * radius * 0.3,
                                        pivot + direction * radius * 0.85,
                                    ],
                                    Stroke::new(radius * 0.20, t::DANGER),
                                );
                                let tip = pivot + direction * radius * 0.75;
                                let side = egui::vec2(-direction.y, direction.x) * radius * 0.4;
                                ui.painter().line_segment(
                                    [tip - side, tip + side],
                                    Stroke::new(radius * 0.17, t::DANGER),
                                );
                            }
                        }
                        let guard = part(cell, 0.68, 0.76, 0.30, 0.20);
                        if button(
                            ui,
                            "off_guard",
                            guard,
                            if open { "GUARD OPEN" } else { "LIFT GUARD" },
                            open,
                            font * 0.72,
                        )
                        .clicked()
                        {
                            self.system(ModernSetting::PanelGuard {
                                id: e.id.clone(),
                                open: !open,
                            });
                        }
                    }
                    PanelElement::Parachute => {
                        if let Some(p) = modern.and_then(|m| m.recovery.as_ref()) {
                            let rounded = e.appearance.style == Some(ControlStyle::RoundedPullHandle);
                            if rounded {
                                ui.painter().rect_filled(
                                    part(cell, 0., 0., 1., 0.9),
                                    cell.height() * 0.40,
                                    t::DANGER,
                                );
                            }
                            let pin = part(cell, 0., 0., if rounded { 0.28 } else { 0.35 }, 0.9);
                            let pin_label = if p.pin_inserted { "PIN IN" } else { "PIN OUT" };
                            let pin_clicked = if rounded {
                                fit_text(ui, pin, pin_label, font * 0.7, t::DIAL);
                                ui.interact(pin, ui.id().with("pin"), Sense::click()).clicked()
                            } else {
                                button(ui, "pin", pin, pin_label, !p.pin_inserted, font).clicked()
                            };
                            if pin_clicked {
                                self.system(ModernSetting::ParachutePin {
                                    inserted: !p.pin_inserted,
                                });
                            }
                            let handle = if rounded {
                                part(cell, 0.30, 0., 0.7, 0.9)
                            } else {
                                part(cell, 0.4, 0., 0.6, 0.9)
                            };
                            let response = if rounded {
                                fit_text(ui, handle, "HOLD TO PULL", font, t::DIAL);
                                ui.interact(handle, ui.id().with("handle"), Sense::click_and_drag())
                            } else {
                                let response =
                                    button(ui, "handle", handle, "HOLD TO PULL", !p.pin_inserted, font);
                                ui.painter().rect_stroke(
                                    handle.shrink(2. * scale),
                                    3.,
                                    Stroke::new(2. * scale, t::DANGER),
                                    egui::StrokeKind::Inside,
                                );
                                response
                            };
                            held = response.is_pointer_button_down_on() && !p.pin_inserted;
                            if held && !self.pull_owned {
                                self.system(ModernSetting::ParachuteHandle { held: true });
                                self.pull_owned = true;
                            }
                            ui.painter().rect_filled(
                                part(handle, 0., 0.92, p.hold_progress as f32, 0.08),
                                0.,
                                t::DANGER,
                            );
                            if held {
                                ui.ctx().request_repaint();
                            }
                        }
                    }
                    PanelElement::Throttle { .. }
                    | PanelElement::BrakeLever { .. }
                    | PanelElement::ParkBrake { .. }
                    | PanelElement::FlapSelector { .. }
                    | PanelElement::ControlStick { .. } => {
                        self.cockpit_control(ui, cell, &e.element, input, font, scale, &mut settings);
                    }
                    _ => {
                        if let Some(setting) = crate::panel_art::primitive(
                            ui,
                            cell,
                            e,
                            modern,
                            font,
                            scale,
                            placard,
                            &registration,
                        ) {
                            self.system(setting);
                        }
                    }
                }
                if let Some(skin) = skin {
                    ui.set_opacity(opacity);
                    self.maps.truncate(map_count);
                    if matches!(e.element, PanelElement::Navigator { .. }) {
                        self.procedure_rect = None;
                    }
                    skin.paint(
                        ui.painter(),
                        cell,
                        crate::instruments::EquipmentSnapshot {
                            installation: e,
                            input,
                        },
                    );
                }
            });
        }
        if self.pull_owned && !held {
            self.system(ModernSetting::ParachuteHandle { held: false });
            self.pull_owned = false;
        }
        if self.key_owned && !start_held {
            self.system(ModernSetting::Key {
                position: KeyPosition::On,
            });
            self.key_owned = false;
        }
        let highlighted = ui
            .ctx()
            .data(|d| d.get_temp::<String>(egui::Id::new(crate::checklist::HIGHLIGHT_KEY)))
            .and_then(|id| panel.elements.iter().find(|e| e.id == id));
        let below_dash = |e: &EquipmentInstallation| {
            dash.is_some_and(|r| {
                !r.expand(0.01)
                    .contains_rect(crate::panel_art::installation_rect(e))
            })
        };
        if let Some(e) = highlighted
            && !below_dash(e)
        {
            let [x, y] = e.position;
            let [w, h] = e.size;
            let cell = Rect::from_min_size(
                rect.min + egui::vec2(x as f32, y as f32) * scale,
                egui::vec2(w as f32, h as f32) * scale,
            );
            crate::checklist::pulse(ui, cell.expand(3. * scale));
        }
        if let Some(dash) = dash {
            let button = Rect::from_min_size(
                available.right_bottom() - egui::vec2(94., 31.),
                egui::vec2(88., 25.),
            );
            if ui.put(button, egui::Button::new("Console")).clicked() {
                self.console_open = true;
            }
            // The current item's control is on the lower console: point at the way to it.
            if !self.console_open && highlighted.is_some_and(below_dash) {
                crate::checklist::pulse(ui, button.expand(3.));
            }
            if self.console_open {
                let console = crate::panel_art::console_panel(panel, dash);
                let mut runtime = self.console.take().unwrap_or_default();
                egui::Window::new("Lower console")
                    .enabled(ui.is_enabled())
                    .id(ui.id().with("lower-console"))
                    .constrain_to(t::window_bounds(ui.ctx()))
                    .open(&mut self.console_open)
                    .fixed_size(console_window_size(ui.ctx(), &console))
                    .show(ui.ctx(), |ui| {
                        settings.extend(runtime.show(ui, &console, input, registry));
                    });
                if !self.console_open {
                    self.commands.extend(runtime.cancel_holds());
                }
                self.commands.append(&mut runtime.commands);
                self.avionics.append(&mut runtime.avionics);
                self.maps.append(&mut runtime.maps);
                self.console = Some(runtime);
            }
        }
        settings
    }
    fn navigator(
        &mut self,
        ui: &mut egui::Ui,
        cell: Rect,
        input: InstrumentInput<'_>,
        font: f32,
        layout: NavigatorLayout,
        bezel: egui::Color32,
    ) {
        let Some(glass) = input.glass else {
            return;
        };
        let Some(n) = &glass.touch.navigator else {
            return;
        };
        ui.painter().rect(
            cell,
            5.,
            bezel,
            Stroke::new(1.5_f32, t::BORDER),
            egui::StrokeKind::Inside,
        );
        if !n.powered {
            text(ui, cell, "NO POWER", font, t::DANGER);
            return;
        }
        let columns = layout == NavigatorLayout::HomeColumns;
        let font = font.min(cell.height() * 0.09);
        let cell_part = |x, y, w, h| {
            let r = part(cell, x, y, w, h);
            if columns {
                navigator_columns_rect(cell, r)
            } else {
                r
            }
        };
        let mut frequency_captions = Vec::new();
        for row in 0..if columns { 1 } else { 2 } {
            let y = 0.02 + row as f32 * 0.13;
            let (active, standby) = if row == 1 || (columns && n.knob_band == RadioBand::Nav) {
                (n.nav_active_khz, n.nav_standby_khz)
            } else {
                (n.com_active_khz, n.com_standby_khz)
            };
            frequency_captions.extend([
                (cell_part(0.02, y, 0.15, 0.11), "COM".into()),
                (
                    cell_part(0.18, y, 0.28, 0.11),
                    format!("{}.{:03}", active / 1000, active % 1000),
                ),
                (
                    cell_part(0.62, y, 0.36, 0.11),
                    format!("{}.{:03}", standby / 1000, standby % 1000),
                ),
                (cell_part(0.47, y, 0.13, 0.11), "SWAP".into()),
            ]);
        }
        let xpdr = format!(
            "{} {:?}",
            glass.settings.transponder_code, glass.settings.transponder_mode
        );
        if columns {
            frequency_captions.extend([
                (part(cell, 0.79, 0.55, 0.09, 0.08), "COM".into()),
                (part(cell, 0.89, 0.55, 0.09, 0.08), "NAV".into()),
            ]);
            if n.transponder {
                frequency_captions.push((part(cell, 0.79, 0.43, 0.19, 0.10), xpdr.clone()));
            }
        }
        if columns {
            for (i, label) in ["M−", "M+", "k−", "k+"].into_iter().enumerate() {
                frequency_captions.push((cell_part(0.26 + i as f32 * 0.12, 0.86, 0.11, 0.11), label.into()));
            }
        }
        let frequency_font = shared_caption_size(ui, &frequency_captions, cell.height() * 0.12);
        if columns && n.transponder {
            uniform_text(
                ui,
                part(cell, 0.79, 0.43, 0.19, 0.10),
                &xpdr,
                frequency_font,
                t::GREEN,
            );
        }
        if columns {
            for (i, band) in [RadioBand::Com, RadioBand::Nav].into_iter().enumerate() {
                if uniform_button(
                    ui,
                    ("column_band", i),
                    part(cell, 0.79 + i as f32 * 0.10, 0.55, 0.09, 0.08),
                    if i == 0 { "COM" } else { "NAV" },
                    n.knob_band == band,
                    frequency_font,
                )
                .clicked()
                {
                    self.nav(NavigatorAction::KnobBand { band });
                }
            }
        }
        for (row, mut band, mut active, mut standby) in [
            (0, RadioBand::Com, n.com_active_khz, n.com_standby_khz),
            (1, RadioBand::Nav, n.nav_active_khz, n.nav_standby_khz),
        ] {
            if columns && row == 1 {
                continue;
            }
            if columns && n.knob_band == RadioBand::Nav {
                band = RadioBand::Nav;
                active = n.nav_active_khz;
                standby = n.nav_standby_khz;
            }
            let band_name = if band == RadioBand::Com { "COM" } else { "NAV" };
            let y = 0.02 + row as f32 * 0.13;
            if uniform_button(
                ui,
                ("band", row),
                cell_part(0.02, y, 0.15, 0.11),
                band_name,
                n.knob_band == band,
                frequency_font,
            )
            .clicked()
            {
                self.nav(NavigatorAction::KnobBand { band });
            }
            uniform_text(
                ui,
                cell_part(0.18, y, 0.28, 0.11),
                &format!("{}.{:03}", active / 1000, active % 1000),
                frequency_font,
                t::GREEN,
            );
            if uniform_button(
                ui,
                ("flip", row),
                cell_part(0.47, y, 0.13, 0.11),
                "SWAP",
                false,
                frequency_font,
            )
            .clicked()
            {
                self.nav(NavigatorAction::Flip { band });
            }
            uniform_text(
                ui,
                cell_part(0.62, y, 0.36, 0.11),
                &format!("{}.{:03}", standby / 1000, standby % 1000),
                frequency_font,
                t::ACCENT,
            );
        }
        for (index, (label, source)) in [
            ("GPS", flightcraft_sim::avionics::radio::CdiSource::Gps),
            ("NAV1", flightcraft_sim::avionics::radio::CdiSource::Nav1),
        ]
        .into_iter()
        .enumerate()
        {
            if button(
                ui,
                ("cdi", index),
                cell_part(0.55 + index as f32 * 0.22, 0.265, 0.20, 0.07),
                label,
                n.cdi_source == source,
                font * 0.8,
            )
            .clicked()
            {
                self.nav(NavigatorAction::Avionics {
                    setting: Box::new(AvionicsSetting::CdiSource { source }),
                });
            }
        }
        let screen = cell_part(0.02, 0.35, 0.96, 0.47);
        match n.page {
            NavigatorPage::Home => {
                let tiles: Vec<_> = NAV_TILES
                    .into_iter()
                    .filter(|(_, page)| {
                        (*page != NavigatorPage::Transponder || n.transponder)
                            && (*page != NavigatorPage::Audio || n.audio_panel)
                    })
                    .collect();
                let cols = if columns { 4 } else { 2 };
                let rows = tiles.len().div_ceil(cols);
                let tile = |i: usize| {
                    part(
                        screen,
                        (i % cols) as f32 / cols as f32,
                        (i / cols) as f32 / rows as f32,
                        1. / cols as f32 - if columns { 0.01 } else { 0.02 },
                        1. / rows as f32 - if columns { 0.03 } else { 0.02 },
                    )
                };
                let captions: Vec<_> = tiles
                    .iter()
                    .enumerate()
                    .map(|(i, (label, _))| (tile(i), (*label).into()))
                    .collect();
                let size = shared_caption_size(ui, &captions, screen.height() * 0.45);
                for (i, (label, page)) in tiles.into_iter().enumerate() {
                    if uniform_button(ui, i, tile(i), label, false, size).clicked() {
                        self.nav(NavigatorAction::Page { page });
                    }
                }
            }
            NavigatorPage::Audio => {
                self.audio_controls(ui, screen, &glass.comms, font, 3);
            }
            NavigatorPage::FlightPlan | NavigatorPage::DirectTo => {
                let title = if n.page == NavigatorPage::FlightPlan {
                    "FLIGHT PLAN · SHARED GPS"
                } else {
                    "DIRECT-TO · SHARED GPS"
                };
                text(ui, part(screen, 0., 0., 1., 0.18), title, font, t::ACCENT);
                text(
                    ui,
                    part(screen, 0., 0.2, 1., 0.2),
                    &glass.settings.flight_plan.join(" > "),
                    font,
                    t::TEXT,
                );
                ui.scope_builder(
                    egui::UiBuilder::new().max_rect(part(screen, 0., 0.45, 0.68, 0.3)),
                    |ui| {
                        ui.add_sized(
                            ui.available_size(),
                            egui::TextEdit::singleline(&mut self.entry)
                                .hint_text("Airport, fix, navaid, or pin")
                                .font(FontId::monospace(font)),
                        );
                    },
                );
                if button(
                    ui,
                    "activate",
                    part(screen, 0.7, 0.45, 0.3, 0.3),
                    "ACTIVATE",
                    false,
                    font,
                )
                .clicked()
                {
                    self.nav(if n.page == NavigatorPage::DirectTo {
                        NavigatorAction::DirectTo {
                            query: self.entry.trim().into(),
                        }
                    } else {
                        NavigatorAction::FlightPlan {
                            queries: self.entry.split_whitespace().map(str::to_owned).collect(),
                        }
                    });
                }
            }
            NavigatorPage::Nearest => {
                for (i, leg) in glass.nearest.iter().take(4).enumerate() {
                    if button(
                        ui,
                        i,
                        part(screen, 0., i as f32 * 0.25, 1., 0.23),
                        &format!("{}  {:.1} NM  DIRECT", leg.airport, leg.distance_nm),
                        false,
                        font,
                    )
                    .clicked()
                    {
                        self.nav(NavigatorAction::Shared {
                            setting: GlassSetting::DirectTo {
                                airport: leg.airport.clone(),
                            },
                        });
                    }
                }
            }
            NavigatorPage::Map => {
                self.maps
                    .push(("navigator-map".into(), crate::map::MapViewport::shared(screen)));
            }
            NavigatorPage::Procedures => {
                self.procedure_rect = Some(screen);
            }
            NavigatorPage::WaypointInfo => text(
                ui,
                screen,
                &glass
                    .gps
                    .destination
                    .as_ref()
                    .map_or("NO ACTIVE WAYPOINT".into(), |id| {
                        format!(
                            "{} · {:.1} NM · {:03.0}°",
                            id,
                            glass.gps.distance_nm,
                            glass.gps.bearing_rad.to_degrees()
                        )
                    }),
                font,
                t::TEXT,
            ),
            NavigatorPage::Frequencies => {
                text(
                    ui,
                    part(screen, 0., 0., 1., 0.3),
                    "TURN KNOBS TO TUNE STANDBY",
                    font,
                    t::MUTED,
                );
                text(
                    ui,
                    part(screen, 0., 0.35, 1., 0.3),
                    "COM 25 kHz · NAV 50 kHz",
                    font,
                    t::TEXT,
                );
            }
            NavigatorPage::Transponder => {
                text(
                    ui,
                    part(screen, 0., 0., 1., 0.25),
                    &format!(
                        "XPDR {} {:?}{}",
                        glass.settings.transponder_code,
                        glass.settings.transponder_mode,
                        if glass.settings.ident_remaining_s > 0. {
                            " IDENT"
                        } else {
                            ""
                        }
                    ),
                    font,
                    t::GREEN,
                );
                ui.scope_builder(
                    egui::UiBuilder::new().max_rect(part(screen, 0., 0.35, 0.65, 0.3)),
                    |ui| {
                        ui.add_sized(
                            ui.available_size(),
                            egui::TextEdit::singleline(&mut self.entry)
                                .hint_text("Squawk")
                                .char_limit(4)
                                .font(FontId::monospace(font)),
                        );
                    },
                );
                if button(
                    ui,
                    "squawk",
                    part(screen, 0.7, 0.35, 0.3, 0.3),
                    "SET",
                    false,
                    font,
                )
                .clicked()
                {
                    self.nav(NavigatorAction::Shared {
                        setting: GlassSetting::Transponder {
                            code: self.entry.clone(),
                            mode: glass.settings.transponder_mode,
                        },
                    });
                }
                for (i, (label, mode)) in [
                    ("STBY", flightcraft_control::TransponderMode::Standby),
                    ("ON", flightcraft_control::TransponderMode::On),
                    ("ALT", flightcraft_control::TransponderMode::Alt),
                ]
                .into_iter()
                .enumerate()
                {
                    if button(
                        ui,
                        i,
                        part(screen, i as f32 / 4., 0.72, 0.24, 0.25),
                        label,
                        false,
                        font,
                    )
                    .clicked()
                    {
                        self.nav(NavigatorAction::Shared {
                            setting: GlassSetting::Transponder {
                                code: glass.settings.transponder_code.clone(),
                                mode,
                            },
                        });
                    }
                }
                if button(
                    ui,
                    "ident",
                    part(screen, 0.75, 0.72, 0.24, 0.25),
                    "IDENT",
                    glass.settings.ident_remaining_s > 0.,
                    font,
                )
                .clicked()
                {
                    self.nav(NavigatorAction::Shared {
                        setting: GlassSetting::Ident {},
                    });
                }
            }
        }
        if button(ui, "home", cell_part(0.02, 0.86, 0.20, 0.11), "HOME", false, font).clicked() {
            self.nav(NavigatorAction::Page {
                page: NavigatorPage::Home,
            });
        }
        for (i, (label, outer, detents)) in [
            ("M−", true, -1),
            ("M+", true, 1),
            ("k−", false, -1),
            ("k+", false, 1),
        ]
        .into_iter()
        .enumerate()
        {
            let area = cell_part(0.26 + i as f32 * 0.12, 0.86, 0.11, 0.11);
            let response = if columns {
                uniform_button(ui, ("knob_step", i), area, label, false, frequency_font)
            } else {
                button(ui, ("knob_step", i), area, label, false, font * 0.85)
            };
            if response.clicked() {
                self.nav(NavigatorAction::Knob { outer, detents });
            }
        }
        let knob = cell_part(0.81, 0.84, 0.16, 0.14);
        let radius = knob.height().min(knob.width()) * 0.48;
        ui.painter().circle_filled(knob.center(), radius, t::RAISED);
        ui.painter()
            .circle_stroke(knob.center(), radius, Stroke::new(1.5_f32, t::MUTED));
        ui.painter()
            .circle_stroke(knob.center(), radius * 0.55, Stroke::new(1.5_f32, t::ACCENT));
        let response = ui
            .interact(knob, ui.id().with("dual_knob"), Sense::click_and_drag())
            .on_hover_text("Outer: MHz; inner: channel. Wheel or drag to tune standby.");
        let pointer = response
            .interact_pointer_pos()
            .or_else(|| ui.input(|i| i.pointer.hover_pos()));
        let outer = pointer.is_some_and(|p| p.distance(knob.center()) > radius * 0.55);
        let wheel = if response.hovered() {
            ui.input(|i| i.raw_scroll_delta.y)
        } else {
            0.
        };
        let drag = if response.dragged() {
            -ui.input(|i| i.pointer.delta().y)
        } else {
            0.
        };
        let delta = if wheel != 0. { wheel } else { drag };
        if delta.abs() >= 1. {
            self.nav(NavigatorAction::Knob {
                outer,
                detents: if delta > 0. { 1 } else { -1 },
            });
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[test]
    fn panel_faces_emit_commands_without_advancing_state_or_reusing_ids() {
        let mut session = flightcraft_control::Session::new().unwrap();
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/packs/equipment-demo");
        session
            .install_pack(&flightcraft_control::read_pack(&path).unwrap())
            .unwrap();
        session
            .execute(
                "aircraft.load",
                serde_json::json!({"source":{"kind":"installed","id":"equipment-demonstrator"},"reset":true}),
            )
            .unwrap();
        let ctx = egui::Context::default();
        t::install(&ctx);
        let before = *session.sim().state();
        let configuration = session.aircraft_configuration().unwrap();
        let panel = configuration.panel.equipment.as_ref().unwrap();
        let glass = session.glass_state();
        let input = InstrumentInput::new(session.sim().systems(), 0.)
            .with_aircraft(configuration)
            .with_glass(&glass);
        let mut runtime = PanelUi::default();
        let switch = panel.elements.iter().find(|e| e.id == "switch_master").unwrap();
        let fit = crate::panel_preview::equipment_rect(
            panel,
            Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600., 800.)),
        );
        let [w, _] = panel.size;
        let scale = fit.width() / w as f32;
        let origin = fit.min;
        let [x, y] = switch.position;
        let [sw, sh] = switch.size;
        let position = origin + egui::vec2((x + sw * 0.5) as f32, (y + sh * 0.5) as f32) * scale;
        let mut settings = Vec::new();
        for pressed in [None, Some(true), Some(false)] {
            let events = pressed
                .map(|pressed| {
                    vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]
                })
                .unwrap_or_default();
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600., 800.))),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ctx, |ui| {
                            settings.extend(runtime.show(
                                ui,
                                panel,
                                input,
                                &crate::instruments::Registry::default(),
                            ));
                        });
                },
            );
            fn no_conflict(shape: &egui::Shape) {
                match shape {
                    egui::Shape::Text(text) => assert!(!text.galley.job.text.contains("use of widget ID")),
                    egui::Shape::Vec(shapes) => {
                        for shape in shapes {
                            no_conflict(shape);
                        }
                    }
                    _ => {}
                }
            }
            for shape in output.shapes {
                no_conflict(&shape.shape);
            }
        }
        assert!(settings.iter().any(|s| matches!(
            s,
            SystemSetting::Switch {
                switch: SystemSwitch::Master,
                on: false
            }
        )));
        assert_eq!(session.sim().state(), &before);
    }
}

fn rocker_light(switch: Rocker) -> Option<flightcraft_aero::LoadSwitch> {
    use flightcraft_aero::LoadSwitch as L;
    match switch {
        Rocker::Beacon => Some(L::Beacon),
        Rocker::Nav => Some(L::Nav),
        Rocker::Strobe => Some(L::Strobe),
        Rocker::Taxi => Some(L::Taxi),
        Rocker::Landing => Some(L::Landing),
        _ => None,
    }
}

fn navigator_columns_rect(cell: Rect, r: Rect) -> Rect {
    let x = (r.left() - cell.left()) / cell.width();
    let y = (r.top() - cell.top()) / cell.height();
    let w = r.width() / cell.width();
    let h = r.height() / cell.height();
    if (0.34..0.83).contains(&y) {
        return part(
            cell,
            0.10 + (x - 0.02) * 0.66 / 0.96,
            0.08 + (y - 0.35) * 0.84 / 0.47,
            w * 0.66 / 0.96,
            h * 0.84 / 0.47,
        );
    }
    if y < 0.26 {
        let row = if y < 0.14 { 0. } else { 1. };
        let (ny, nh) = if x < 0.17 {
            (0.03 + row * 0.32, 0.07)
        } else if x < 0.46 {
            (0.10 + row * 0.32, 0.10)
        } else if x < 0.61 {
            (0.29 + row * 0.32, 0.06)
        } else {
            (0.21 + row * 0.32, 0.08)
        };
        return part(cell, 0.79, ny, 0.19, nh);
    }
    if y < 0.34 {
        return part(cell, 0.01, 0.38 + (x - 0.55) * 1.2, 0.07, 0.14);
    }
    if x < 0.23 {
        return part(cell, 0.01, 0.08, 0.07, 0.24);
    }
    if x > 0.80 {
        return part(cell, 0.83, 0.73, 0.13, 0.25);
    }
    // Four explicit encoder step buttons remain available along the bottom edge.
    part(cell, 0.79 + (x - 0.26) * 0.34, 0.65, w * 0.30, 0.08)
}

impl PanelUi {
    fn audio_panel(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        state: &flightcraft_control::CommsSnapshot,
        font: f32,
    ) {
        self.audio_controls(ui, rect, state, font, 6);
    }
    fn audio_controls(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        state: &flightcraft_control::CommsSnapshot,
        font: f32,
        columns: usize,
    ) {
        use flightcraft_control::CommsAudioPanel as Setting;
        ui.painter().rect_filled(rect, 3., t::DIAL);
        let p = &state.panel;
        let first = [
            (
                "MIC1",
                p.transmit == 1,
                Setting {
                    transmit: Some(1),
                    ..Default::default()
                },
            ),
            (
                "MIC2",
                p.transmit == 2,
                Setting {
                    transmit: Some(2),
                    ..Default::default()
                },
            ),
            (
                "RX1",
                p.receive_com1,
                Setting {
                    receive_com1: Some(!p.receive_com1),
                    ..Default::default()
                },
            ),
            (
                "RX2",
                p.receive_com2,
                Setting {
                    receive_com2: Some(!p.receive_com2),
                    ..Default::default()
                },
            ),
            (
                "NAV ID",
                p.nav_ident,
                Setting {
                    nav_ident: Some(!p.nav_ident),
                    ..Default::default()
                },
            ),
            (
                "ICS",
                p.intercom,
                Setting {
                    intercom: Some(!p.intercom),
                    ..Default::default()
                },
            ),
        ];
        let second = [
            (
                "VOL −",
                false,
                Setting {
                    volume: Some((p.volume - 0.1).max(0.0)),
                    ..Default::default()
                },
            ),
            (
                "VOL +",
                false,
                Setting {
                    volume: Some((p.volume + 0.1).min(1.0)),
                    ..Default::default()
                },
            ),
            (
                "SQ −",
                false,
                Setting {
                    squelch: Some((p.squelch - 0.1).max(0.0)),
                    ..Default::default()
                },
            ),
            (
                "SQ +",
                false,
                Setting {
                    squelch: Some((p.squelch + 0.1).min(1.0)),
                    ..Default::default()
                },
            ),
            (
                "VOICE",
                p.speech,
                Setting {
                    speech: Some(!p.speech),
                    ..Default::default()
                },
            ),
            (
                "CAP",
                p.captions,
                Setting {
                    captions: Some(!p.captions),
                    ..Default::default()
                },
            ),
        ];
        // Retain the existing standalone face when hosting is not enabled.
        if columns == 6 {
            for (i, (label, selected, setting)) in first.into_iter().chain(second).enumerate() {
                let cell = part(rect, (i % 6) as f32 / 6., (i / 6) as f32 / 2., 1. / 6., 0.5).shrink(1.);
                if button(
                    ui,
                    (i / 6, i % 6),
                    cell,
                    label,
                    selected && state.powered,
                    font * 0.85,
                )
                .clicked()
                {
                    self.commands.push(EquipmentSetting::AudioPanel { setting });
                }
            }
            return;
        }
        let status = format!(
            "{}VOL {:.0}%  SQ {:.0}%",
            if state.powered { "" } else { "OFF  " },
            p.volume * 100.,
            p.squelch * 100.
        );
        let status_rect = part(rect, 0., 0., 1., 0.14);
        let controls_rect = part(rect, 0., 0.16, 1., 0.84);
        let cell = |i: usize| {
            part(
                controls_rect,
                (i % columns) as f32 / columns as f32,
                (i / columns) as f32 / (12 / columns) as f32,
                1. / columns as f32,
                1. / (12 / columns) as f32,
            )
            .shrink(1.)
        };
        let controls: Vec<_> = first.into_iter().chain(second).collect();
        let mut captions: Vec<_> = controls
            .iter()
            .enumerate()
            .map(|(i, (label, _, _))| (cell(i), (*label).into()))
            .collect();
        captions.push((status_rect, status.clone()));
        let size = shared_caption_size(ui, &captions, font);
        uniform_text(ui, status_rect, &status, size, t::TEXT);
        for (i, (label, selected, setting)) in controls.into_iter().enumerate() {
            if uniform_button(ui, ("audio", i), cell(i), label, selected && state.powered, size).clicked() {
                self.commands.push(EquipmentSetting::AudioPanel { setting });
            }
        }
    }
}

/// The lower console window's content size: the console region's own aspect, as wide as the
/// screen allows up to a desktop size, and no taller than the screen has room for.
pub(crate) fn console_window_size(
    ctx: &egui::Context,
    console: &flightcraft_aero::modern::EquipmentPanel,
) -> egui::Vec2 {
    let bounds = t::window_bounds(ctx);
    let aspect = (console.size[1] / console.size[0].max(1.)) as f32;
    let mut width = (bounds.width() - 24.).clamp(120., 560.);
    if width * aspect > bounds.height() - 70. {
        width = ((bounds.height() - 70.) / aspect).max(120.);
    }
    egui::vec2(width, width * aspect)
}
