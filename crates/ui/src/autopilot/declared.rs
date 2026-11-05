//! Pack-placed mode controls share the installed controller's command boundary.
use super::*;
use crate::{
    instruments::InstrumentInput,
    panel_art::{color, fit_text},
};
use flightcraft_aero::panel_fidelity::{ControllerAction as A, ControllerLayout};
impl Controller {
    pub(crate) fn show_declared(
        &mut self,
        ui: &mut egui::Ui,
        plate: Rect,
        size: [f64; 2],
        layout: &ControllerLayout,
        input: InstrumentInput<'_>,
        bezel: egui::Color32,
    ) -> Vec<AvionicsSetting> {
        let Some(s) = input.autopilot else {
            return Vec::new();
        };
        ui.painter().rect(
            plate,
            3.,
            bezel,
            Stroke::new(1.0_f32, t::BORDER),
            egui::StrokeKind::Inside,
        );
        let [w, h] = size;
        let rect = |[x, y, cw, ch]: [f64; 4]| {
            Rect::from_min_size(
                plate.min + egui::vec2((x / w) as f32 * plate.width(), (y / h) as f32 * plate.height()),
                egui::vec2((cw / w) as f32 * plate.width(), (ch / h) as f32 * plate.height()),
            )
        };
        for section in &layout.sections {
            let r = rect(section.rect);
            ui.painter()
                .rect_stroke(r, 0., Stroke::new(0.6_f32, t::MUTED), egui::StrokeKind::Inside);
            fit_text(
                ui,
                Rect::from_min_size(r.min, egui::vec2(r.width(), r.height() * 0.15)),
                &section.label,
                r.height() * 0.12,
                t::TEXT,
            );
        }
        let mut commands = Vec::new();
        for c in &layout.controls {
            let r = rect(c.rect);
            let label = match c.action {
                A::Ap => "AP",
                A::Fd => "FD",
                A::Yd => "YD",
                A::Lvl => "LVL",
                A::Hdg => "HDG",
                A::Nav => "NAV",
                A::Apr => "APR",
                A::Vs => "VS",
                A::Alt => "ALT",
                A::Ias => "IAS",
                A::Vnav => "VNAV",
                A::HeadingKnob => "HDG",
                A::AltitudeKnob => "ALT SEL",
                A::PitchWheel => "UP / DN",
            };
            let active = match c.action {
                A::Ap => s.engaged,
                A::Fd => s.flight_director,
                A::Yd => s.yaw_damper,
                A::Lvl => {
                    s.engaged
                        && s.active_lateral == Some(LateralMode::Rol)
                        && s.active_vertical == Some(VerticalMode::Pit)
                }
                A::Hdg => s.active_lateral == Some(LateralMode::Hdg),
                A::Nav => s.active_lateral == Some(LateralMode::Nav),
                A::Apr => s.active_lateral == Some(LateralMode::Apr),
                A::Vs => s.active_vertical == Some(VerticalMode::Vs),
                A::Alt => s.active_vertical == Some(VerticalMode::Alt),
                A::Ias => s.active_vertical == Some(VerticalMode::Ias),
                A::Vnav => {
                    s.armed_vertical == Some(VerticalMode::Gp) || s.active_vertical == Some(VerticalMode::Gp)
                }
                _ => false,
            };
            let lit = color(c.lit_color.as_ref(), t::GREEN);
            let setting = match c.action {
                A::HeadingKnob | A::AltitudeKnob => {
                    let dial = Rect::from_center_size(
                        r.center(),
                        egui::Vec2::splat(r.width().min(r.height()) * 0.62),
                    );
                    ui.painter().circle(
                        dial.center(),
                        dial.width() * 0.48,
                        t::DIAL,
                        Stroke::new(1.0_f32, t::MUTED),
                    );
                    fit_text(
                        ui,
                        Rect::from_min_size(r.min, egui::vec2(r.width(), r.height() * 0.16)),
                        label,
                        r.height() * 0.13,
                        t::TEXT,
                    );
                    fit_text(
                        ui,
                        Rect::from_min_size(
                            egui::pos2(r.left(), r.bottom() - r.height() * 0.17),
                            egui::vec2(r.width(), r.height() * 0.17),
                        ),
                        "PUSH SYNC",
                        r.height() * 0.12,
                        t::TEXT,
                    );
                    if c.action == A::HeadingKnob {
                        commands.extend(self.heading.show(ui, ui.id().with("declared_heading"), dial));
                        None
                    } else {
                        let (steps, outer) = self.altitude.detents(ui, "declared_altitude", dial, true);
                        let clicked = ui
                            .interact(dial, ui.id().with("declared_altitude"), Sense::click_and_drag())
                            .clicked();
                        if clicked {
                            Some(PanelSetting::Altitude {
                                altitude_m: input.systems.sensed.altitude_msl_m.clamp(2., 10000.),
                            })
                        } else if steps != 0 {
                            Some(PanelSetting::Altitude {
                                altitude_m: (s.altitude_m
                                    + f64::from(steps) * if outer { 304.8 } else { 30.48 })
                                .clamp(2., 10000.),
                            })
                        } else {
                            None
                        }
                    }
                }
                A::PitchWheel => {
                    ui.painter().rect_filled(r, 2., t::DIAL);
                    for i in 1..8 {
                        let y = r.top() + r.height() * i as f32 / 8.;
                        ui.painter().line_segment(
                            [egui::pos2(r.left(), y), egui::pos2(r.right(), y)],
                            Stroke::new(1.0_f32, t::MUTED),
                        );
                    }
                    fit_text(ui, r, label, r.height() * 0.13, t::TEXT);
                    let (steps, _) = self.wheel.detents(ui, "declared_pitch", r, false);
                    if steps == 0 {
                        None
                    } else {
                        Some(match s.active_vertical {
                            Some(VerticalMode::Ias) => PanelSetting::Airspeed {
                                airspeed_mps: (s.airspeed_mps + f64::from(steps) / 1.9438444924406)
                                    .clamp(20., 90.),
                            },
                            // ALT: each detent moves the held altitude (and the director's reference) 20 ft.
                            Some(VerticalMode::Alt) => PanelSetting::HoldAltitude {
                                altitude_m: (s.captured_altitude_m + f64::from(steps) * 6.096)
                                    .clamp(2., 10000.),
                            },
                            Some(VerticalMode::Pit) => PanelSetting::Pitch {
                                pitch_rad: (s.pitch_rad + f64::from(steps).to_radians()).clamp(-0.25, 0.35),
                            },
                            _ => PanelSetting::VerticalSpeed {
                                vertical_speed_mps: step_vs(s.vertical_speed_mps, steps),
                            },
                        })
                    }
                }
                _ => {
                    ui.painter().rect(
                        r,
                        2.,
                        t::DIAL,
                        Stroke::new(1.0_f32, if active { lit } else { t::BORDER }),
                        egui::StrokeKind::Inside,
                    );
                    fit_text(
                        ui,
                        r,
                        label,
                        r.height() * 0.45,
                        if active { lit } else { t::TEXT },
                    );
                    if !ui
                        .interact(r, ui.id().with(c.action as u8), Sense::click())
                        .clicked()
                    {
                        None
                    } else {
                        Some(match c.action {
                            A::Ap => PanelSetting::Engage { on: !s.engaged },
                            A::Fd => PanelSetting::FlightDirector {
                                on: !s.flight_director,
                            },
                            A::Yd => PanelSetting::YawDamper { on: !s.yaw_damper },
                            A::Lvl => PanelSetting::Level,
                            A::Hdg => PanelSetting::Lateral {
                                mode: LateralMode::Hdg,
                            },
                            A::Nav => PanelSetting::Lateral {
                                mode: LateralMode::Nav,
                            },
                            A::Apr => PanelSetting::Lateral {
                                mode: LateralMode::Apr,
                            },
                            A::Vs => PanelSetting::Vertical {
                                mode: VerticalMode::Vs,
                            },
                            A::Alt => PanelSetting::Vertical {
                                mode: VerticalMode::Alt,
                            },
                            A::Ias => PanelSetting::Vertical {
                                mode: VerticalMode::Ias,
                            },
                            _ => PanelSetting::Vertical {
                                mode: VerticalMode::Gp,
                            },
                        })
                    }
                }
            };
            if let Some(setting) = setting {
                commands.push(AvionicsSetting::Autopilot { setting });
            }
        }
        commands
    }
}
