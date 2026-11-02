//! Procedure and NAV controls return commands; receiver state remains owned by the simulation.
use eframe::egui;
use flightcraft_control::{AvionicsSetting as S, Session};
use flightcraft_sim::avionics::radio::{CdiSource, RadioSetting as R};
#[derive(Default)]
pub struct Controls {
    airport: String,
    procedure: String,
    transition: String,
    vectors: bool,
}
impl Controls {
    pub fn show(&mut self, ui: &mut egui::Ui, session: &Session) -> Vec<S> {
        let mut commands = Vec::new();
        ui.menu_button("Procedures / NAV", |ui| {
            if let Some(data) = session.sim().world().procedures() {
                ui.label(format!("FAA {} · {}", data.cycle, data.effective_date));
                let airports = data
                    .procedures
                    .iter()
                    .map(|p| p.airport.as_str())
                    .collect::<std::collections::BTreeSet<_>>();
                egui::ComboBox::from_id_salt("procedure-airport")
                    .selected_text(if self.airport.is_empty() {
                        "Airport"
                    } else {
                        &self.airport
                    })
                    .show_ui(ui, |ui| {
                        for airport in airports {
                            if ui
                                .selectable_value(&mut self.airport, airport.into(), airport)
                                .changed()
                            {
                                self.procedure.clear();
                                self.transition.clear();
                            }
                        }
                    });
                egui::ComboBox::from_id_salt("procedure-selection")
                    .selected_text(if self.procedure.is_empty() {
                        "Procedure"
                    } else {
                        &self.procedure
                    })
                    .show_ui(ui, |ui| {
                        for p in data.procedures.iter().filter(|p| p.airport == self.airport) {
                            if ui
                                .selectable_value(
                                    &mut self.procedure,
                                    p.ident.clone(),
                                    format!("{} · {:?}", p.ident, p.kind),
                                )
                                .changed()
                            {
                                self.transition.clear();
                            }
                        }
                    });
                let selected = data
                    .procedures
                    .iter()
                    .find(|p| p.airport == self.airport && p.ident == self.procedure);
                if let Some(p) = selected {
                    egui::ComboBox::from_id_salt("procedure-transition")
                        .selected_text(if self.transition.is_empty() {
                            "Common / final"
                        } else {
                            &self.transition
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.transition, String::new(), "Common / final");
                            for r in p.routes.iter().filter(|r| !r.transition.is_empty()) {
                                ui.selectable_value(
                                    &mut self.transition,
                                    r.transition.clone(),
                                    &r.transition,
                                );
                            }
                        });
                    ui.checkbox(&mut self.vectors, "Vectors to final");
                    if ui.button("Load procedure").clicked() {
                        commands.push(S::LoadProcedure {
                            airport: p.airport.clone(),
                            procedure: p.ident.clone(),
                            transition: (!self.transition.is_empty()).then(|| self.transition.clone()),
                            vectors_to_final: self.vectors,
                        });
                    }
                }
            } else {
                ui.label("No procedures in this region");
            }
            let receiver = session.sim().procedure_receiver();
            for p in &receiver.loaded {
                ui.label(format!("{} {}", p.airport, p.ident));
                if let Some(plate) = &p.plate {
                    ui.label(&plate.name);
                    ui.small(&plate.pdf);
                }
            }
            ui.horizontal_wrapped(|ui| {
                for (label, setting) in [
                    ("Activate approach", S::ActivateApproach {}),
                    ("Vectors", S::VectorsToFinal {}),
                    ("Missed", S::MissedApproach {}),
                    ("Clear", S::ClearProcedure {}),
                    ("Next leg", S::SequenceLeg {}),
                ] {
                    if ui
                        .add_enabled(!receiver.loaded.is_empty(), egui::Button::new(label))
                        .clicked()
                    {
                        commands.push(setting);
                    }
                }
            });
            if receiver.hold_entry.is_some() {
                ui.label(format!(
                    "Hold {:?} · {}",
                    receiver.hold_entry,
                    if receiver.suspended { "SUSP" } else { "sequencing" }
                ));
                if ui.button("Exit hold").clicked() {
                    commands.push(S::HoldOptions {
                        suspend: false,
                        exit: true,
                    });
                }
            }
            ui.separator();
            let radios = session.sim().radios();
            let mut source = radios.source;
            ui.horizontal(|ui| {
                ui.label("CDI source");
                ui.selectable_value(&mut source, CdiSource::Gps, "GPS");
                ui.selectable_value(&mut source, CdiSource::Nav1, "NAV1");
            });
            if source != radios.source {
                commands.push(S::CdiSource { source });
            }
            let signals = session.sim().radio_state();
            for ((radio, r), signal) in (1u8..3).zip(radios.receivers.iter()).zip(signals.iter()) {
                ui.push_id(radio, |ui| {
                    ui.label(format!(
                        "NAV{radio} {:.2} MHz · {}",
                        f64::from(r.active_hz) / 1e6,
                        signal.ident.as_deref().unwrap_or("no ident")
                    ));
                    ui.horizontal(|ui| {
                        let mut frequency = f64::from(r.standby_hz) / 1e6;
                        if ui
                            .add(
                                egui::DragValue::new(&mut frequency)
                                    .range(108. ..=117.95)
                                    .speed(0.05)
                                    .fixed_decimals(2)
                                    .suffix(" STBY MHz"),
                            )
                            .changed()
                        {
                            commands.push(S::NavRadio {
                                setting: R::Standby {
                                    radio,
                                    frequency_hz: ((frequency * 20.).round() as u32) * 50000,
                                },
                            });
                        }
                        if ui.button("Swap").clicked() {
                            commands.push(S::NavRadio {
                                setting: R::Swap { radio },
                            });
                        }
                    });
                    let mut course = r.course_deg;
                    if ui
                        .add(
                            egui::DragValue::new(&mut course)
                                .range(0. ..=360.)
                                .suffix("° OBS"),
                        )
                        .changed()
                    {
                        commands.push(S::NavRadio {
                            setting: R::Course {
                                radio,
                                degrees: course,
                            },
                        });
                    }
                    let mut bc = r.back_course;
                    if ui.checkbox(&mut bc, "Back course").changed() {
                        commands.push(S::NavRadio {
                            setting: R::BackCourse { radio, on: bc },
                        });
                    }
                    let mut ident = r.ident_enabled;
                    if ui.checkbox(&mut ident, "Ident request").changed() {
                        commands.push(S::NavRadio {
                            setting: R::Ident { radio, on: ident },
                        });
                    }
                    if let Some(dme) = signal.dme_nm {
                        ui.label(format!("DME {dme:.1} NM"));
                    }
                });
            }
            ui.small("For simulation only, not for real-world navigation");
        });
        commands
    }
}
