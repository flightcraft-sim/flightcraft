use crate::keymap::Action;
use eframe::egui;
use flightcraft_control::{
    BearingPointer, GlassPage, GlassSetting as S, GlassState, MapOrientation, TransponderMode,
};
#[derive(Default)]
pub struct GlassControls {
    airport: String,
    plan: String,
    squawk: String,
    pub popout: Option<String>,
}
impl GlassControls {
    pub fn show(&mut self, ui: &mut egui::Ui, state: &GlassState) -> Vec<S> {
        let mut commands = Vec::new();
        ui.horizontal_wrapped(|ui| {
            for (label, page) in [
                ("MAP", GlassPage::Map),
                ("NRST", GlassPage::Nearest),
                ("FPL", GlassPage::FlightPlan),
            ] {
                if ui.selectable_label(state.settings.page == page, label).clicked() {
                    commands.push(S::Page { page });
                }
            }
            if ui.button("− RNG").clicked() {
                commands.push(S::MapRange {
                    range_nm: (state.settings.range_nm / 1.5).max(1.),
                });
            }
            if ui.button("+ RNG").clicked() {
                commands.push(S::MapRange {
                    range_nm: (state.settings.range_nm * 1.5).min(200.),
                });
            }
            if state.traffic_installed {
                if ui
                    .selectable_label(state.settings.traffic_shown, "TFC")
                    .on_hover_text("Show traffic on the map")
                    .clicked()
                {
                    commands.push(S::Traffic {
                        shown: !state.settings.traffic_shown,
                    });
                }
                if ui
                    .button(format!("FLT {}", state.settings.traffic_filter.label()))
                    .on_hover_text("Traffic altitude filter: normal, above, below, unrestricted")
                    .clicked()
                {
                    commands.push(S::TrafficFilter {
                        filter: state.settings.traffic_filter.next(),
                    });
                }
            }
            if ui.button("N/TRK/HDG").clicked() {
                commands.push(S::Orientation {
                    orientation: match state.settings.orientation {
                        MapOrientation::NorthUp => MapOrientation::TrackUp,
                        MapOrientation::TrackUp => MapOrientation::HeadingUp,
                        MapOrientation::HeadingUp => MapOrientation::NorthUp,
                    },
                });
            }
            let mut vision = state.settings.synthetic_vision;
            if ui.checkbox(&mut vision, "SVT").changed() {
                commands.push(S::SyntheticVision { enabled: vision });
            }
            if ui.button("PFD ↗").clicked() {
                self.popout = Some("glass_pfd".into());
            }
            if ui.button("MFD ↗").clicked() {
                self.popout = Some("glass_mfd".into());
            }
            ui.menu_button("KNOBS", |ui| {
                let mut heading = state.settings.heading_bug_rad.to_degrees();
                if ui
                    .add(
                        egui::DragValue::new(&mut heading)
                            .range(0. ..=359.)
                            .max_decimals(0)
                            .custom_formatter(|v, _| format!("{:03.0}", v))
                            .suffix("° HDG"),
                    )
                    .changed()
                {
                    commands.push(S::HeadingBug {
                        heading_rad: heading.to_radians(),
                    });
                }
                let mut course = state.settings.course_rad.to_degrees();
                if ui
                    .add(
                        egui::DragValue::new(&mut course)
                            .range(0. ..=359.)
                            .max_decimals(0)
                            .custom_formatter(|v, _| format!("{:03.0}", v))
                            .suffix("° CRS"),
                    )
                    .changed()
                {
                    commands.push(S::Course {
                        course_rad: course.to_radians(),
                    });
                }
                let mut altitude = state.settings.selected_altitude_m / 0.3048;
                if ui
                    .add(
                        egui::DragValue::new(&mut altitude)
                            .speed(100.)
                            .range(100. ..=32800.)
                            .max_decimals(0)
                            .custom_formatter(|v, _| {
                                crate::instruments::formatting::selected_feet(v * 0.3048)
                            })
                            .suffix(" ft SEL"),
                    )
                    .changed()
                {
                    commands.push(S::SelectedAltitude {
                        altitude_m: libm::round(altitude / 100.) * 30.48,
                    });
                }
                let mut hpa = state.settings.pressure_hpa;
                if ui.checkbox(&mut hpa, "Baro in hPa").changed() {
                    commands.push(S::PressureUnits { hpa });
                }
                let factor = if hpa { 100. } else { 3386.389 };
                let mut pressure = state.baro_pa / factor;
                if ui
                    .add(
                        egui::DragValue::new(&mut pressure)
                            .speed(if hpa { 1. } else { 0.01 })
                            .range(80000. / factor..=110000. / factor)
                            .suffix(if hpa { " hPa" } else { " inHg" }),
                    )
                    .changed()
                {
                    commands.push(S::Baro {
                        pressure_pa: pressure * factor,
                    });
                }
                let mut bearing = state.settings.bearing_pointer == BearingPointer::Gps;
                if ui.checkbox(&mut bearing, "GPS bearing pointer").changed() {
                    commands.push(S::BearingPointer {
                        source: if bearing {
                            BearingPointer::Gps
                        } else {
                            BearingPointer::Off
                        },
                    });
                }
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.squawk)
                            .char_limit(4)
                            .desired_width(55.)
                            .hint_text(&state.settings.transponder_code),
                    );
                    for (label, mode) in [
                        ("STBY", TransponderMode::Standby),
                        ("ON", TransponderMode::On),
                        ("ALT", TransponderMode::Alt),
                        ("OFF", TransponderMode::Off),
                    ] {
                        if ui.button(label).clicked() {
                            commands.push(S::Transponder {
                                code: if self.squawk.is_empty() {
                                    state.settings.transponder_code.clone()
                                } else {
                                    self.squawk.clone()
                                },
                                mode,
                            });
                        }
                    }
                });
            });
            ui.menu_button("GPS / PLAN", |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.airport)
                            .char_limit(16)
                            .desired_width(100.)
                            .hint_text("Airport ident"),
                    );
                    if ui.button("D→").clicked() {
                        commands.push(S::DirectTo {
                            airport: self.airport.to_uppercase(),
                        });
                    }
                });
                let query = self.airport.to_uppercase();
                for airport in state
                    .airports
                    .iter()
                    .filter(|a| a.ident.contains(&query))
                    .take(12)
                {
                    if ui.button(format!("D→ {}", airport.ident)).clicked() {
                        commands.push(S::DirectTo {
                            airport: airport.ident.clone(),
                        });
                    }
                }
                ui.label("Airport list, separated by spaces:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.plan)
                        .char_limit(1088)
                        .desired_width(260.),
                );
                if ui.button("LOAD PLAN").clicked() {
                    commands.push(S::FlightPlan {
                        airports: self.plan.split_whitespace().map(str::to_uppercase).collect(),
                    });
                }
                for (i, airport) in state.settings.flight_plan.iter().enumerate() {
                    if ui
                        .selectable_label(i == state.gps.active_leg, format!("ACTIVATE {}", airport))
                        .clicked()
                    {
                        commands.push(S::ActiveLeg { index: i });
                    }
                }
            });
        });
        commands
    }
}
pub fn shortcut(action: Action, s: &GlassState) -> Option<S> {
    Some(match action {
        Action::GlassRangeIn => S::MapRange {
            range_nm: (s.settings.range_nm / 1.5).max(1.),
        },
        Action::GlassRangeOut => S::MapRange {
            range_nm: (s.settings.range_nm * 1.5).min(200.),
        },
        Action::GlassPage => S::Page {
            page: match s.settings.page {
                GlassPage::Map => GlassPage::Nearest,
                GlassPage::Nearest => GlassPage::FlightPlan,
                GlassPage::FlightPlan => GlassPage::Map,
            },
        },
        Action::GlassOrientation => S::Orientation {
            orientation: match s.settings.orientation {
                MapOrientation::NorthUp => MapOrientation::TrackUp,
                MapOrientation::TrackUp => MapOrientation::HeadingUp,
                MapOrientation::HeadingUp => MapOrientation::NorthUp,
            },
        },
        Action::HeadingBugLeft | Action::HeadingBugRight => S::HeadingBug {
            heading_rad: (s.settings.heading_bug_rad
                + if action == Action::HeadingBugLeft {
                    -1_f64.to_radians()
                } else {
                    1_f64.to_radians()
                })
            .rem_euclid(core::f64::consts::TAU),
        },
        Action::GlassAltitudeUp | Action::GlassAltitudeDown => S::SelectedAltitude {
            altitude_m: (s.settings.selected_altitude_m
                + if action == Action::GlassAltitudeUp {
                    30.48
                } else {
                    -30.48
                })
            .clamp(-500., 15000.),
        },
        Action::GlassCourseLeft | Action::GlassCourseRight => S::Course {
            course_rad: (s.settings.course_rad
                + if action == Action::GlassCourseLeft {
                    -1_f64.to_radians()
                } else {
                    1_f64.to_radians()
                })
            .rem_euclid(core::f64::consts::TAU),
        },
        _ => return None,
    })
}
