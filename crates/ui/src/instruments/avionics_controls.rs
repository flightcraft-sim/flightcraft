use super::{DisplayOptions, InstrumentInput, PressureUnit};
use crate::keymap::Action;
use eframe::egui;
use flightcraft_sim::avionics::{AvionicsSetting, AvionicsState, Unit};

#[derive(Default)]
pub struct Controls {
    pub open: bool,
    query: String,
    focus_direct_to: bool,
    pending: Vec<AvionicsSetting>,
}
impl Controls {
    pub fn open_direct_to(&mut self) {
        self.open = true;
        self.focus_direct_to = true;
    }
    pub fn take_settings(&mut self) -> Vec<AvionicsSetting> {
        std::mem::take(&mut self.pending)
    }
    pub(super) fn show(&mut self, ctx: &egui::Context, input: InstrumentInput<'_>) {
        if !input.systems.avionics.installed {
            self.open = false;
            return;
        }
        let a = &input.systems.avionics.electronic_attitude;
        let h = &input.systems.avionics.electronic_hsi;
        egui::Window::new("Electronic standby controls")
            .constrain_to(crate::theme::window_bounds(ctx))
            .open(&mut self.open)
            .resizable(false)
            .show(ctx, |ui| {
                for (label, unit, power) in [
                    ("Attitude power", Unit::ElectronicAttitude, &a.power),
                    (
                        "Navigation power",
                        Unit::ElectronicHsi,
                        &h.reversion_attitude.power,
                    ),
                ] {
                    let mut on = power.switch_on;
                    if ui.checkbox(&mut on, label).changed() {
                        self.pending.push(AvionicsSetting::Power { unit, on });
                    }
                }
                let mut reversion = h.manual_reversion;
                if ui.checkbox(&mut reversion, "Attitude reversion").changed() {
                    self.pending
                        .push(AvionicsSetting::Reversion { enabled: reversion });
                }
                egui::Grid::new("standby_knobs").num_columns(2).show(ui, |ui| {
                    if let Some(degrees) =
                        knob(ui, "Heading bug (degrees)", h.heading_bug_deg, 0. ..=359., 1., 0)
                    {
                        self.pending.push(AvionicsSetting::HeadingBug { degrees });
                    }
                    if let Some(degrees) = knob(ui, "Course (degrees)", h.course_deg, 0. ..=359., 1., 0) {
                        self.pending.push(AvionicsSetting::Course { degrees });
                    }
                    let (label, value, range, step, decimals, factor) = match input.options.pressure {
                        PressureUnit::InHg => ("Baro (inHg)", a.baro_inhg, 28.10..=31., 0.01, 2, 3386.389),
                        PressureUnit::Hpa => (
                            "Baro (hPa)",
                            a.baro_inhg * 3386.389 / 100.,
                            950. ..=1050.,
                            1.,
                            0,
                            100.,
                        ),
                    };
                    if let Some(value) = knob(ui, label, value, range, step, decimals) {
                        self.pending.push(AvionicsSetting::Baro {
                            pressure_pa: value * factor,
                        });
                    }
                    if let Some(feet) = knob(
                        ui,
                        "Selected altitude (ft)",
                        a.selected_altitude_ft,
                        -1000. ..=30000.,
                        100.,
                        0,
                    ) {
                        self.pending.push(AvionicsSetting::SelectedAltitude {
                            feet: libm::round(feet / 100.) * 100.,
                        });
                    }
                });
                ui.separator();
                ui.label("Direct to airport");
                ui.horizontal(|ui| {
                    let entry = ui.add(
                        egui::TextEdit::singleline(&mut self.query)
                            .char_limit(8)
                            .hint_text("Airport ident"),
                    );
                    if self.focus_direct_to {
                        entry.request_focus();
                        self.focus_direct_to = false;
                    }
                    if ui.button("Direct to").clicked() {
                        self.pending.push(AvionicsSetting::DirectTo {
                            ident: self.query.trim().to_ascii_uppercase(),
                        });
                    }
                });
                if let Some(airports) = input.airports {
                    let query = self.query.trim().to_ascii_uppercase();
                    egui::ScrollArea::vertical().max_height(150.).show(ui, |ui| {
                        for airport in airports.airports.iter().filter(|a| {
                            query.is_empty()
                                || a.ident.contains(&query)
                                || a.icao.as_ref().is_some_and(|s| s.contains(&query))
                                || a.name.to_ascii_uppercase().contains(&query)
                        }) {
                            if ui
                                .button(format!("{} · {}", airport.ident, airport.name))
                                .clicked()
                            {
                                self.pending.push(AvionicsSetting::DirectTo {
                                    ident: airport.ident.clone(),
                                });
                            }
                        }
                    });
                } else {
                    ui.label("NO GPS · select a region with airports");
                }
            });
    }
}
fn knob(
    ui: &mut egui::Ui,
    label: &str,
    mut value: f64,
    range: std::ops::RangeInclusive<f64>,
    step: f64,
    decimals: usize,
) -> Option<f64> {
    ui.label(label);
    let changed = ui
        .add(
            egui::DragValue::new(&mut value)
                .range(range)
                .speed(step)
                .max_decimals(decimals)
                .custom_formatter(move |value, _| {
                    if step == 100. {
                        super::formatting::selected_feet(value * 0.3048)
                    } else if label.contains("degrees") {
                        format!("{}°", super::heading::degrees_text(value.to_radians()))
                    } else {
                        format!("{value:.decimals$}")
                    }
                }),
        )
        .changed();
    ui.end_row();
    changed.then_some(value)
}
pub fn shortcut(action: Action, s: &AvionicsState) -> Option<AvionicsSetting> {
    shortcut_with_options(action, s, DisplayOptions::default())
}
pub fn shortcut_with_options(
    action: Action,
    s: &AvionicsState,
    options: DisplayOptions,
) -> Option<AvionicsSetting> {
    if !s.installed {
        return None;
    }
    let a = &s.electronic_attitude;
    let h = &s.electronic_hsi;
    Some(match action {
        Action::HeadingBugSync => AvionicsSetting::HeadingBugSync {},
        Action::HeadingBugLeft | Action::HeadingBugRight => AvionicsSetting::HeadingBug {
            degrees: (h.heading_bug_deg + if action == Action::HeadingBugLeft { -1. } else { 1. })
                .rem_euclid(360.),
        },
        Action::AltimeterUp | Action::AltimeterDown => AvionicsSetting::Baro {
            pressure_pa: options.adjust_pressure(
                a.baro_inhg * 3386.389,
                if action == Action::AltimeterUp { 1. } else { -1. },
            ),
        },
        Action::StandbyCourseLeft | Action::StandbyCourseRight => AvionicsSetting::Course {
            degrees: (h.course_deg
                + if action == Action::StandbyCourseLeft {
                    -1.
                } else {
                    1.
                })
            .rem_euclid(360.),
        },
        Action::StandbyAltitudeUp | Action::StandbyAltitudeDown => AvionicsSetting::SelectedAltitude {
            feet: (a.selected_altitude_ft
                + if action == Action::StandbyAltitudeUp {
                    100.
                } else {
                    -100.
                })
            .clamp(-1000., 30000.),
        },
        Action::StandbyAttitudePower => AvionicsSetting::Power {
            unit: Unit::ElectronicAttitude,
            on: !a.power.switch_on,
        },
        Action::StandbyHsiPower => AvionicsSetting::Power {
            unit: Unit::ElectronicHsi,
            on: !h.reversion_attitude.power.switch_on,
        },
        Action::StandbyReversion => AvionicsSetting::Reversion {
            enabled: !h.manual_reversion,
        },
        _ => return None,
    })
}
