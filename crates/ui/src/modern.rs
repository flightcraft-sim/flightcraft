//! Original, declaratively placed equipment faces. Interactions emit typed commands only.
use crate::{instruments::InstrumentInput, theme as t};
use eframe::egui::{self, Align2, FontId, Rect, Sense, Stroke};
use flightcraft_aero::modern::*;
use flightcraft_control::{
    AvionicsSetting, DisplayAction, EquipmentSetting, GlassSetting, NavigatorAction, NavigatorPage, RadioBand,
};
use flightcraft_sim::{
    modern::{KeyPosition, Lane, ModernSetting, Pump},
    systems::{FuelSelector, SystemSetting, SystemSwitch},
};

#[derive(Default)]
pub(crate) struct PanelUi {
    pub commands: Vec<EquipmentSetting>,
    pub avionics: Vec<AvionicsSetting>,
    controller: crate::autopilot::Controller,
    pub maps: Vec<(String, Rect)>,
    pub map_widgets: std::collections::BTreeMap<String, crate::map::PlanningMap>,
    pub procedure_rect: Option<Rect>,
    pub procedures: crate::procedures::Controls,
    pull_owned: bool,
    key_owned: bool,
    entry: String,
}
fn text(ui: &egui::Ui, rect: Rect, label: &str, size: f32, color: egui::Color32) {
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::monospace(size),
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
        Stroke::new(1., if lit { t::ACCENT } else { t::BORDER }),
        egui::StrokeKind::Inside,
    );
    text(ui, rect, label, font, if lit { t::ACCENT } else { t::TEXT });
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
    ) -> Vec<SystemSetting> {
        self.maps.clear();
        self.procedure_rect = None;
        let (available, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
        let [width, height] = panel.size;
        let scale = (available.width() / width as f32).min(available.height() / height as f32);
        let rect = Rect::from_center_size(
            available.center(),
            egui::vec2(width as f32, height as f32) * scale,
        );
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
            Stroke::new(1., t::BORDER),
            egui::StrokeKind::Inside,
        );
        crate::panel_art::shell(ui, rect, panel);
        let Some(glass) = input.glass else {
            return Vec::new();
        };
        let modern = input.systems.modern.as_ref();
        let font = (12. * scale).max(6.);
        let mut settings = Vec::new();
        let mut held = false;
        let mut start_held = false;
        for e in &panel.elements {
            let [x, y] = e.position;
            let [w, h] = e.size;
            let cell = Rect::from_min_size(
                rect.min + egui::vec2(x as f32, y as f32) * scale,
                egui::vec2(w as f32, h as f32) * scale,
            );
            ui.push_id(&e.id, |ui| match &e.element {
                PanelElement::TouchDisplay { composition, .. } => {
                    if let Some(d) = glass.touch.displays.iter().find(|d| d.id == e.id) {
                        let screen = part(cell, 0.02, 0.03, 0.96, 0.84);
                        ui.painter().rect_filled(cell, 6., t::DIAL);
                        self.maps.extend(
                            crate::glass::paint_touch(ui, screen, glass, input, d, composition)
                                .into_iter()
                                .enumerate()
                                .map(|(i, rect)| (format!("{}-{i}", e.id), rect)),
                        );
                        for (i, (label, layout)) in [
                            ("PFD", TouchLayout::FullPfd),
                            ("PFD / MAP", TouchLayout::PfdMap),
                            ("MAP / ENG", TouchLayout::MapEngine),
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            if button(
                                ui,
                                i,
                                part(cell, 0.10 + i as f32 * 0.25, 0.89, 0.24, 0.09),
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
                            part(cell, 0.86, 0.89, 0.12, 0.09),
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
                        for (side, xpos) in [(false, 0.015), (true, 0.94)] {
                            let knob = part(cell, xpos, 0.89, 0.045, 0.09);
                            ui.painter().circle_stroke(
                                knob.center(),
                                knob.width() * 0.45,
                                Stroke::new(1.5, t::MUTED),
                            );
                            let response =
                                ui.interact(knob, ui.id().with(("corner", side)), Sense::click_and_drag());
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
                PanelElement::Navigator { .. } => self.navigator(ui, cell, input, font),
                PanelElement::Standby => crate::glass::paint_standby(ui, cell, glass, input),
                PanelElement::Autopilot => {
                    if let Some(state) = input.autopilot {
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
                        Rocker::Beacon | Rocker::Nav | Rocker::Strobe | Rocker::Taxi | Rocker::Landing => (
                            modern.is_some_and(|m| {
                                m.light_switches
                                    .iter()
                                    .any(|(light, on)| Some(*light) == rocker_light(*switch) && *on)
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
                    let face = part(cell, 0.2, 0.05, 0.6, 0.66);
                    ui.painter().rect_filled(face, 3., t::DIAL);
                    ui.painter().rect_filled(
                        part(face, 0.15, if on { 0.10 } else { 0.65 }, 0.7, 0.15),
                        1.,
                        if on { t::GREEN } else { t::BORDER },
                    );
                    text(
                        ui,
                        part(cell, 0., 0.72, 1., 0.18),
                        label,
                        font * 0.85,
                        if *backlit {
                            t::TEXT.linear_multiply(if input.systems.electrical.main_bus_voltage > 9. {
                                1.
                            } else {
                                0.22
                            })
                        } else {
                            t::TEXT
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
                        if let Some(switch) = setting {
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
                        .circle_stroke(center, radius * 1.12, Stroke::new(1., t::BORDER));
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
                    text(ui, part(cell, 0., 0., 1., 0.13), "FUEL SELECTOR", font, t::MUTED);
                    ui.painter().circle_filled(pivot, radius, t::DIAL);
                    ui.painter()
                        .circle_stroke(pivot, radius, Stroke::new(1., t::MUTED));
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
                        if button(
                            ui,
                            "pin",
                            part(cell, 0., 0., 0.35, 0.9),
                            if p.pin_inserted { "PIN IN" } else { "PIN OUT" },
                            !p.pin_inserted,
                            font,
                        )
                        .clicked()
                        {
                            self.system(ModernSetting::ParachutePin {
                                inserted: !p.pin_inserted,
                            });
                        }
                        let handle = part(cell, 0.4, 0., 0.6, 0.9);
                        let response = button(ui, "handle", handle, "HOLD TO PULL", !p.pin_inserted, font);
                        ui.painter().rect_stroke(
                            handle.shrink(2. * scale),
                            3.,
                            Stroke::new(2. * scale, t::DANGER),
                            egui::StrokeKind::Inside,
                        );
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
                element => {
                    if let Some(setting) =
                        crate::panel_art::primitive(ui, cell, &e.id, element, modern, font, scale)
                    {
                        self.system(setting);
                    }
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
        settings
    }
    fn navigator(&mut self, ui: &mut egui::Ui, cell: Rect, input: InstrumentInput<'_>, font: f32) {
        let Some(glass) = input.glass else {
            return;
        };
        let Some(n) = &glass.touch.navigator else {
            return;
        };
        ui.painter().rect(
            cell,
            5.,
            t::DIAL,
            Stroke::new(1.5, t::BORDER),
            egui::StrokeKind::Inside,
        );
        if !n.powered {
            text(ui, cell, "NO POWER", font, t::DANGER);
            return;
        }
        for (row, band, active, standby) in [
            (0, RadioBand::Com, n.com_active_khz, n.com_standby_khz),
            (1, RadioBand::Nav, n.nav_active_khz, n.nav_standby_khz),
        ] {
            let band_name = if band == RadioBand::Com { "COM" } else { "NAV" };
            let y = 0.02 + row as f32 * 0.13;
            if button(
                ui,
                ("band", row),
                part(cell, 0.02, y, 0.15, 0.11),
                band_name,
                n.knob_band == band,
                font,
            )
            .clicked()
            {
                self.nav(NavigatorAction::KnobBand { band });
            }
            text(
                ui,
                part(cell, 0.18, y, 0.28, 0.11),
                &format!("{}.{:03}", active / 1000, active % 1000),
                font * 1.15,
                t::GREEN,
            );
            if button(
                ui,
                ("flip", row),
                part(cell, 0.47, y, 0.13, 0.11),
                "SWAP",
                false,
                font * 0.75,
            )
            .clicked()
            {
                self.nav(NavigatorAction::Flip { band });
            }
            text(
                ui,
                part(cell, 0.62, y, 0.36, 0.11),
                &format!("{}.{:03}", standby / 1000, standby % 1000),
                font * 1.15,
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
                part(cell, 0.55 + index as f32 * 0.22, 0.265, 0.20, 0.07),
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
        let screen = part(cell, 0.02, 0.35, 0.96, 0.47);
        match n.page {
            NavigatorPage::Home => {
                for (i, (label, page)) in [
                    ("Flight plan", NavigatorPage::FlightPlan),
                    ("Direct-to", NavigatorPage::DirectTo),
                    ("Procedures", NavigatorPage::Procedures),
                    ("Nearest", NavigatorPage::Nearest),
                    ("Map", NavigatorPage::Map),
                    ("Waypoint info", NavigatorPage::WaypointInfo),
                    ("Frequencies", NavigatorPage::Frequencies),
                    ("Transponder", NavigatorPage::Transponder),
                ]
                .into_iter()
                .enumerate()
                {
                    if page == NavigatorPage::Transponder && !n.transponder {
                        continue;
                    }
                    if button(
                        ui,
                        i,
                        part(screen, (i % 2) as f32 * 0.5, (i / 2) as f32 * 0.25, 0.48, 0.23),
                        label,
                        false,
                        font,
                    )
                    .clicked()
                    {
                        self.nav(NavigatorAction::Page { page });
                    }
                }
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
                self.maps.push(("navigator-map".into(), screen));
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
                        "XPDR {} {:?}",
                        glass.settings.transponder_code, glass.settings.transponder_mode
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
                        part(screen, i as f32 / 3., 0.72, 0.32, 0.25),
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
            }
        }
        if button(
            ui,
            "home",
            part(cell, 0.02, 0.86, 0.20, 0.11),
            "HOME",
            false,
            font,
        )
        .clicked()
        {
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
            if button(
                ui,
                ("knob_step", i),
                part(cell, 0.26 + i as f32 * 0.12, 0.86, 0.11, 0.11),
                label,
                false,
                font * 0.85,
            )
            .clicked()
            {
                self.nav(NavigatorAction::Knob { outer, detents });
            }
        }
        let knob = part(cell, 0.81, 0.84, 0.16, 0.14);
        let radius = knob.height().min(knob.width()) * 0.48;
        ui.painter().circle_filled(knob.center(), radius, t::RAISED);
        ui.painter()
            .circle_stroke(knob.center(), radius, Stroke::new(1.5, t::MUTED));
        ui.painter()
            .circle_stroke(knob.center(), radius * 0.55, Stroke::new(1.5, t::ACCENT));
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
        let [w, h] = panel.size;
        let scale = (1600_f32 / w as f32).min(800_f32 / h as f32);
        let origin = egui::pos2((1600. - w as f32 * scale) * 0.5, (800. - h as f32 * scale) * 0.5);
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
                            settings.extend(runtime.show(ui, panel, input));
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
