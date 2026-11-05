//! Panel and keyboard share one typed command mapping.
use crate::keymap::Action;
use eframe::egui;
use flightcraft_sim::systems::{
    FuelSelector, Magnetos, PRIMER_STROKE, SystemSetting, SystemSwitch, SystemsState,
};

pub fn shortcut(action: Action, s: &SystemsState) -> Option<SystemSetting> {
    shortcut_with_options(action, s, super::DisplayOptions::default())
}
pub fn shortcut_with_options(
    action: Action,
    s: &SystemsState,
    options: super::DisplayOptions,
) -> Option<SystemSetting> {
    use Action::*;
    let c = &s.controls;
    let toggle = match action {
        Master => Some((SystemSwitch::Master, c.master)),
        Avionics => Some((SystemSwitch::Avionics, c.avionics)),
        Alternator => Some((SystemSwitch::Alternator, c.alternator)),
        FuelPump => Some((SystemSwitch::FuelPump, c.fuel_pump)),
        Lights => Some((SystemSwitch::Lights, c.lights)),
        PitotHeat => Some((SystemSwitch::PitotHeat, c.pitot_heat)),
        CarbHeat => Some((SystemSwitch::CarbHeat, c.carb_heat)),
        AlternateStatic => Some((SystemSwitch::AlternateStatic, c.alternate_static)),

        _ => None,
    };
    if let Some((switch, on)) = toggle {
        return Some(SystemSetting::Switch { switch, on: !on });
    }
    Some(match action {
        Magnetos => SystemSetting::Magnetos {
            position: match c.magnetos {
                self::Magnetos::Off => self::Magnetos::Right,
                self::Magnetos::Right => self::Magnetos::Left,
                self::Magnetos::Left => self::Magnetos::Both,
                self::Magnetos::Both => self::Magnetos::Off,
            },
        },
        FuelSelector => SystemSetting::FuelSelector {
            position: match c.fuel_selector {
                self::FuelSelector::Off => self::FuelSelector::Left,
                self::FuelSelector::Left => self::FuelSelector::Both,
                self::FuelSelector::Both => self::FuelSelector::Right,
                self::FuelSelector::Right => self::FuelSelector::Off,
            },
        },
        MixtureRich | MixtureLean => SystemSetting::Mixture {
            value: (c.mixture + if action == MixtureRich { 0.05 } else { -0.05 }).clamp(0., 1.),
        },
        Primer => SystemSetting::Primer {
            charge: (s.primer_charge + PRIMER_STROKE).min(1.),
        },
        AltimeterUp | AltimeterDown => SystemSetting::Altimeter {
            pressure_pa: options.adjust_pressure(
                c.altimeter_setting_pa,
                if action == AltimeterUp { 1. } else { -1. },
            ),
        },
        HeadingBugLeft | HeadingBugRight => SystemSetting::HeadingBug {
            heading_rad: (c.heading_bug_rad
                + if action == HeadingBugRight {
                    1_f64.to_radians()
                } else {
                    -1_f64.to_radians()
                })
            .rem_euclid(std::f64::consts::TAU),
        },
        _ => return None,
    })
}

/// Only release a starter hold that this UI owns; scripted panel commands remain independent.
#[derive(Default)]
pub(super) struct StarterHold {
    owned: bool,
}
impl StarterHold {
    pub(super) fn update(&mut self, s: &SystemsState, held: bool) -> Vec<SystemSetting> {
        let mut settings = Vec::new();
        if held {
            if s.controls.magnetos != Magnetos::Both {
                settings.push(SystemSetting::Magnetos {
                    position: Magnetos::Both,
                });
            }
            if !s.controls.starter {
                settings.push(SystemSetting::Switch {
                    switch: SystemSwitch::Starter,
                    on: true,
                });
            }
            self.owned = true;
        } else if self.owned {
            if s.controls.starter {
                settings.push(SystemSetting::Switch {
                    switch: SystemSwitch::Starter,
                    on: false,
                });
            }
            self.owned = false;
        }
        settings
    }
}

pub(super) fn show(ui: &mut egui::Ui, s: &SystemsState) -> (Vec<SystemSetting>, bool) {
    let mut settings = Vec::new();
    let mut starter_held = false;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(5., 3.);
        ui.spacing_mut().button_padding = egui::vec2(4., 2.);
        let c = &s.controls;
        ui.horizontal(|ui| {
            for (action, label, on) in [
                (Action::Master, "MASTER", c.master),
                (Action::Avionics, "AVIONICS", c.avionics),
                (Action::Alternator, "ALT", c.alternator),
                (Action::FuelPump, "PUMP", c.fuel_pump),
                (Action::Lights, "LIGHTS", c.lights),
                (Action::PitotHeat, "PITOT HEAT", c.pitot_heat),
                (Action::CarbHeat, "CARB HEAT", c.carb_heat),
                (Action::AlternateStatic, "ALT STATIC", c.alternate_static),
            ] {
                if action == Action::CarbHeat && !s.carb_heat_installed {
                    continue;
                }
                if ui
                    .selectable_label(on, egui::RichText::new(label).size(11.))
                    .clicked()
                    && let Some(setting) = shortcut(action, s)
                {
                    settings.push(setting);
                }
            }
            ui.label(egui::RichText::new("MIXTURE").size(11.));
            let mut percent = c.mixture * 100.;
            if ui
                .add(egui::Slider::new(&mut percent, 0. ..=100.).suffix("% rich"))
                .changed()
            {
                settings.push(SystemSetting::Mixture {
                    value: percent / 100.,
                });
            }
            let primed = (s.primer_charge / PRIMER_STROKE).round();
            let label = if primed > 0. {
                format!("PRIME ({primed:.0})")
            } else {
                "PRIME".to_owned()
            };
            if ui
                .small_button(egui::RichText::new(label).size(11.))
                .on_hover_text(
                    "One push is one primer stroke; the number is the primed fuel left, in strokes. \
                     Cranking uses it up and it evaporates over a few minutes. More than three \
                     strokes floods the engine: to clear it, mixture idle cut-off, throttle full \
                     open, and crank.",
                )
                .clicked()
            {
                settings.push(SystemSetting::Primer {
                    charge: (s.primer_charge + PRIMER_STROKE).min(1.),
                });
            }
        });
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("MAGNETOS").size(11.));
            for (position, label) in [
                (Magnetos::Off, "OFF"),
                (Magnetos::Right, "R"),
                (Magnetos::Left, "L"),
                (Magnetos::Both, "BOTH"),
            ] {
                if ui
                    .selectable_label(c.magnetos == position && !c.starter, label)
                    .clicked()
                {
                    settings.push(SystemSetting::Magnetos { position });
                    if c.starter {
                        settings.push(SystemSetting::Switch {
                            switch: SystemSwitch::Starter,
                            on: false,
                        });
                    }
                }
            }
            starter_held = ui
                .add(egui::Button::new("START").selected(c.starter))
                .on_hover_text("Hold to crank; release returns to BOTH")
                .is_pointer_button_down_on();
            ui.separator();
            ui.label(egui::RichText::new("FUEL").size(11.));
            for (position, label) in [
                (FuelSelector::Left, "LEFT"),
                (FuelSelector::Both, "BOTH"),
                (FuelSelector::Right, "RIGHT"),
                (FuelSelector::Off, "OFF"),
            ] {
                if ui.selectable_label(c.fuel_selector == position, label).clicked() {
                    settings.push(SystemSetting::FuelSelector { position });
                }
            }
        });
    });
    (settings, starter_held)
}
