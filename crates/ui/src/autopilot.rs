//! Original controller faceplates and flight-mode annunciation; interactions only emit commands.
mod declared;
use crate::instruments::formatting as f;
use crate::{
    instruments::{DrawItem, DrawList, Ink, heading},
    keymap::Action,
    theme as t,
};
use eframe::egui::{self, Align2, FontId, Rect, Sense, Stroke};
use flightcraft_control::AvionicsSetting;
use flightcraft_sim::equipment::{AutopilotFamily, EquipmentState, LateralMode, PanelSetting, VerticalMode};

pub fn shortcut(action: Action, state: &EquipmentState) -> Option<AvionicsSetting> {
    let setting = match action {
        Action::AutopilotDisconnect => PanelSetting::Disconnect,
        Action::AutopilotEngage => PanelSetting::Engage { on: !state.engaged },
        Action::AutopilotHeading => PanelSetting::Lateral {
            mode: LateralMode::Hdg,
        },
        Action::AutopilotAltitude => PanelSetting::Vertical {
            mode: VerticalMode::Alt,
        },
        Action::AutopilotLevel => PanelSetting::Level,
        _ => return None,
    };
    Some(AvionicsSetting::Autopilot { setting })
}
fn lateral(mode: LateralMode) -> &'static str {
    match mode {
        LateralMode::Rol => "ROL",
        LateralMode::Hdg => "HDG",
        LateralMode::Trk => "TRK",
        LateralMode::Nav => "NAV",
        LateralMode::Apr => "APR",
        LateralMode::Rev => "REV",
        LateralMode::LocBc => "LOC/BC",
    }
}
fn vertical(mode: VerticalMode) -> &'static str {
    match mode {
        VerticalMode::Pit => "PIT",
        VerticalMode::Alt => "ALT",
        VerticalMode::Vs => "VS",
        VerticalMode::Ias => "IAS",
        VerticalMode::Gp => "GP",
        VerticalMode::Gs => "GS",
    }
}
fn target(s: &EquipmentState) -> String {
    match s.active_vertical {
        Some(VerticalMode::Alt) => format!("{} FT", f::feet(s.captured_altitude_m)),
        Some(VerticalMode::Vs) => format!("{} FPM", f::vertical_speed(s.vertical_speed_mps)),
        Some(VerticalMode::Ias) => format!("{:.0} KT", s.airspeed_mps * 1.9438444924406),
        Some(VerticalMode::Pit) => format!("{:+.0}°", s.pitch_rad.to_degrees()),
        _ => String::new(),
    }
}
fn caution(s: &EquipmentState) -> String {
    let mut labels = Vec::new();
    for (on, label) in [
        (s.mistrim, "MISTRIM"),
        (s.trim_in_motion, "TRIM"),
        (s.underspeed, "UNDERSPEED"),
        (s.overspeed, "OVERSPEED"),
        (s.altitude_alert, "ALT ALERT"),
        (s.pilot_overpower, "OVERRIDE"),
    ] {
        if on {
            labels.push(label);
        }
    }
    if s.disconnect_reason.is_some() && !s.engaged && s.ap_flash {
        labels.push("AP DISC");
    }
    labels.join("  ")
}
/// PFD annunciator: lateral active/armed left, AP and YD centre, vertical and target right.
pub fn strip_draw(draw: &mut DrawList, s: &EquipmentState, [x, y, w, h]: [f32; 4]) {
    draw.items.push(DrawItem::Polygon {
        points: vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h]],
        ink: Ink::Face,
    });
    let mut label = |px: f32, py: f32, text: String, ink: Ink, size: f32| {
        draw.items.push(DrawItem::Label {
            at: [x + w * px, y + py],
            text,
            size,
            ink,
        })
    };
    label(
        0.08,
        9.,
        s.active_lateral.map_or("---", lateral).into(),
        Ink::Normal,
        10.,
    );
    label(
        0.08,
        22.,
        s.armed_lateral.map_or("", lateral).into(),
        Ink::Text,
        8.,
    );
    let ap = if s.engaged {
        "AP"
    } else if s.ap_flash {
        "AP DISC"
    } else {
        ""
    };
    label(
        0.45,
        10.,
        ap.into(),
        if s.ap_flash {
            if s.ap_flash_visible {
                Ink::Warning
            } else {
                Ink::Face
            }
        } else {
            Ink::Normal
        },
        9.,
    );
    label(
        0.54,
        10.,
        if s.yaw_damper { "YD" } else { "" }.into(),
        Ink::Normal,
        9.,
    );
    label(
        0.49,
        23.,
        if s.flight_director { "FD" } else { "" }.into(),
        Ink::Normal,
        8.,
    );
    label(
        0.68,
        9.,
        s.active_vertical.map_or("---", vertical).into(),
        Ink::Normal,
        10.,
    );
    label(
        0.68,
        22.,
        if s.alts_armed {
            "ALTS".into()
        } else {
            s.armed_vertical.map_or("", vertical).into()
        },
        Ink::Text,
        8.,
    );
    label(0.88, 12., target(s), Ink::Normal, 10.);
}
pub fn mode_strip(ui: &mut egui::Ui, s: &EquipmentState) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width().min(750.), 32.), Sense::hover());
    let mut draw = DrawList {
        size: [500., 30.],
        items: Vec::new(),
        indicated: 0.,
        valid: true,
    };
    strip_draw(&mut draw, s, [0., 0., 500., 30.]);
    crate::instruments::scaled(ui.painter(), rect, draw.size, |p| {
        use crate::instruments::Skin;
        crate::instruments::SkinPack {
            id: "original".into(),
            colors: Vec::new(),
            face: Vec::new(),
        }
        .paint(p, &draw);
    });
}
/// Command bars are presentation of director targets, never flight truth or servo commands.
pub fn command_bars(
    draw: &mut DrawList,
    s: &EquipmentState,
    center: [f32; 2],
    roll: f64,
    pitch: f64,
    scale: f32,
) {
    if !s.flight_director {
        return;
    }
    let [cx, cy] = center;
    let x = cx + ((s.command_bank_rad - roll) * 80.).clamp(-35., 35.) as f32 * scale;
    let y = cy - ((s.command_pitch_rad - pitch) * 200.).clamp(-35., 35.) as f32 * scale;
    draw.items.push(DrawItem::Line {
        a: [x, cy - 30. * scale],
        b: [x, cy + 30. * scale],
        width: 2.5 * scale,
        ink: Ink::Gps,
    });
    draw.items.push(DrawItem::Line {
        a: [cx - 42. * scale, y],
        b: [cx + 42. * scale, y],
        width: 2.5 * scale,
        ink: Ink::Gps,
    });
}
#[derive(Default)]
pub struct Controller {
    cws_owned: bool,
    heading: heading::Knob,
    altitude: Encoder,
    wheel: Encoder,
    legacy_vs: bool,
}
#[derive(Default)]
struct Encoder {
    drag: f32,
    outer: bool,
}
impl Encoder {
    fn detents(&mut self, ui: &mut egui::Ui, id: &str, rect: Rect, dual: bool) -> (i32, bool) {
        let response = ui.interact(rect, ui.id().with(id), Sense::click_and_drag());
        let outer = |p: egui::Pos2| p.distance(rect.center()) > rect.width() * 0.3;
        if response.drag_started() {
            self.drag = 0.;
            self.outer = response.interact_pointer_pos().is_some_and(outer);
        }
        let mut detents = 0;
        if response.dragged() {
            let d = ui.input(|i| i.pointer.delta());
            self.drag += d.x - d.y;
            detents = (self.drag / 6.).trunc() as i32;
            self.drag -= detents as f32 * 6.;
        }
        if response.hovered() {
            if !response.dragged() {
                self.outer = response.hover_pos().is_some_and(outer);
            }
            detents += ui.input_mut(|i| {
                let d = i.raw_scroll_delta.y;
                i.raw_scroll_delta.y = 0.;
                if d == 0. { 0 } else { d.signum() as i32 }
            });
        }
        response
            .on_hover_cursor(egui::CursorIcon::Grab)
            .on_hover_text(if dual {
                "Inner: 100 FT · outer: 1,000 FT. Wheel or drag."
            } else {
                "Wheel or drag to adjust."
            });
        (detents, dual && self.outer)
    }
}
fn text(ui: &egui::Ui, at: egui::Pos2, label: impl ToString, size: f32, color: egui::Color32) {
    let scale = ui
        .data(|data| data.get_temp::<f32>(ui.id().with("controller-font-scale")))
        .unwrap_or(1.);
    ui.painter().text(
        at,
        Align2::CENTER_CENTER,
        label,
        FontId::monospace((size * scale).max(7.)),
        color,
    );
}
fn button(ui: &mut egui::Ui, rect: Rect, label: &str, selected: bool) -> egui::Response {
    let response = ui.interact(rect, ui.id().with(label), Sense::click());
    ui.painter().rect_filled(
        rect,
        3.,
        if response.is_pointer_button_down_on() {
            t::BORDER
        } else {
            t::RAISED
        },
    );
    ui.painter().rect_stroke(
        rect,
        3.,
        Stroke::new(1., if selected { t::GREEN } else { t::BORDER }),
        egui::StrokeKind::Inside,
    );
    text(
        ui,
        rect.center(),
        label,
        11.,
        if selected { t::GREEN } else { t::TEXT },
    );
    response
}
fn knob(ui: &egui::Ui, rect: Rect, dual: bool) {
    let center = rect.center();
    let radius = rect.width() * 0.43;
    ui.painter()
        .circle(center, radius, t::DIAL, Stroke::new(2., t::BORDER));
    if dual {
        ui.painter()
            .circle(center, radius * 0.63, t::RAISED, Stroke::new(1., t::MUTED));
    }
    ui.painter().line_segment(
        [
            center - egui::vec2(0., radius - 5.),
            center - egui::vec2(0., radius * 0.45),
        ],
        Stroke::new(3., t::ACCENT),
    );
}
impl Controller {
    /// Fit the complete existing controller, including encoder hit regions, to an installation.
    pub fn show_in(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        state: &EquipmentState,
        heading_bug: f64,
        cws_key: bool,
    ) -> Vec<AvionicsSetting> {
        self.face(ui, rect, state, heading_bug, cws_key)
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        s: &EquipmentState,
        heading_bug: f64,
        cws_key: bool,
    ) -> Vec<AvionicsSetting> {
        let legacy = s.installation.family == AutopilotFamily::TwoAxisLegacy;
        let leveler = s.installation.family == AutopilotFamily::WingLeveler;
        let size = if leveler {
            egui::vec2(310., 105.)
        } else if legacy {
            egui::vec2(490., 144.)
        } else {
            egui::vec2(820., 148.)
        };
        let (plate, _) = ui.allocate_exact_size(
            egui::vec2(size.x.min(ui.available_width()), size.y),
            Sense::hover(),
        );
        self.face(ui, plate, s, heading_bug, cws_key)
    }
    fn face(
        &mut self,
        ui: &mut egui::Ui,
        plate: Rect,
        s: &EquipmentState,
        heading_bug: f64,
        cws_key: bool,
    ) -> Vec<AvionicsSetting> {
        let legacy = s.installation.family == AutopilotFamily::TwoAxisLegacy;
        let leveler = s.installation.family == AutopilotFamily::WingLeveler;
        let size = if leveler {
            egui::vec2(310., 105.)
        } else if legacy {
            egui::vec2(490., 144.)
        } else {
            egui::vec2(820., 148.)
        };
        let scale = plate.size() / size;
        let font_scale_id = ui.id().with("controller-font-scale");
        ui.data_mut(|data| data.insert_temp(font_scale_id, scale.x.min(scale.y)));
        let at = |x, y| plate.min + egui::vec2(x, y) * scale;
        let cell = |x, y, w, h| Rect::from_min_max(at(x, y), at(x + w, y + h));
        ui.painter().rect(
            plate,
            5.,
            t::PANEL,
            Stroke::new(2., t::BORDER),
            egui::StrokeKind::Inside,
        );
        for p in [
            plate.left_top() + egui::vec2(6., 6.),
            plate.right_bottom() - egui::vec2(6., 6.),
        ] {
            ui.painter().circle_filled(p, 2., t::MUTED);
        }
        let mut commands = Vec::new();
        let mut settings = Vec::new();
        text(
            ui,
            at(
                if leveler {
                    135.
                } else if legacy {
                    210.
                } else {
                    280.
                },
                15.,
            ),
            if leveler {
                "WING LEVELER"
            } else if legacy {
                "FLIGHT CONTROL · RATE"
            } else {
                "FLIGHT CONTROL · DIGITAL"
            },
            10.,
            t::MUTED,
        );
        if leveler {
            if button(ui, cell(14., 38., 45., 30.), "AP", s.engaged).clicked() {
                settings.push(PanelSetting::Engage { on: !s.engaged });
            }
            let rect = cell(78., 29., 53., 53.);
            knob(ui, rect, false);
            let (steps, _) = self.wheel.detents(ui, "turn", rect, false);
            if steps != 0 {
                settings.push(PanelSetting::Turn {
                    rate_rps: (s.turn_rate_rps + f64::from(steps) * 0.005).clamp(-0.05236, 0.05236),
                });
            }
            text(
                ui,
                at(206., 50.),
                format!("TURN {:+.0}°/s", s.turn_rate_rps.to_degrees()),
                13.,
                t::TEXT,
            );
        } else {
            let window = if legacy {
                cell(13., 29., 397., 60.)
            } else {
                cell(13., 28., 535., 57.)
            };
            ui.painter().rect_filled(window, 2., t::DIAL);
            text(
                ui,
                at(48., 42.),
                s.active_lateral.map_or("ROL", lateral),
                13.,
                t::GREEN,
            );
            text(
                ui,
                at(114., 42.),
                s.active_vertical.map_or("---", vertical),
                13.,
                t::GREEN,
            );
            text(
                ui,
                at(204., 42.),
                format!(
                    "{} {}",
                    s.armed_lateral.map_or("", lateral),
                    if s.alts_armed { "ALTS" } else { "" }
                ),
                10.,
                t::TEXT,
            );
            text(
                ui,
                at(313., 46.),
                format!("{} FT", f::selected_feet(s.altitude_m)),
                19.,
                t::TEXT,
            );
            text(
                ui,
                at(142., 72.),
                if legacy && self.legacy_vs {
                    format!("VS {}", f::vertical_speed(s.vertical_speed_mps))
                } else {
                    caution(s)
                },
                10.,
                t::AMBER,
            );
            if !legacy {
                text(ui, at(470., 48.), target(s), 13., t::GREEN);
            }
            let labels: &[&str] = if legacy {
                &["AP", "HDG", "NAV", "APR", "REV", "ALT", "UP", "DN", "ARM"]
            } else {
                &[
                    "AP", "FD", "HDG", "TRK", "NAV", "APR", "ALT", "VS", "IAS", "LVL", "YD",
                ]
            };
            for (i, &name) in labels.iter().enumerate() {
                let active = match name {
                    "AP" => s.engaged,
                    "FD" => s.flight_director,
                    "YD" => s.yaw_damper,
                    "ARM" => s.alts_armed,
                    _ => {
                        s.active_lateral.is_some_and(|m| lateral(m) == name)
                            || s.active_vertical.is_some_and(|m| vertical(m) == name)
                    }
                };
                let key = if legacy {
                    cell(14. + i as f32 * 43., 101., 39., 29.)
                } else {
                    let advance = 792. / labels.len() as f32;
                    cell(14. + i as f32 * advance, 119., advance - 4., 24.)
                };
                if button(ui, key, name, active).clicked() {
                    let action = match name {
                        "AP" => PanelSetting::Engage { on: !s.engaged },
                        "FD" => PanelSetting::FlightDirector {
                            on: !s.flight_director,
                        },
                        "YD" => PanelSetting::YawDamper { on: !s.yaw_damper },
                        "LVL" => PanelSetting::Level,
                        "HDG" => PanelSetting::Lateral {
                            mode: LateralMode::Hdg,
                        },
                        "TRK" => PanelSetting::Lateral {
                            mode: LateralMode::Trk,
                        },
                        "NAV" => PanelSetting::Lateral {
                            mode: LateralMode::Nav,
                        },
                        "APR" => PanelSetting::Lateral {
                            mode: LateralMode::Apr,
                        },
                        "REV" => PanelSetting::Lateral {
                            mode: LateralMode::Rev,
                        },
                        "ALT" => {
                            self.legacy_vs = false;
                            PanelSetting::Vertical {
                                mode: VerticalMode::Alt,
                            }
                        }
                        "VS" => PanelSetting::Vertical {
                            mode: VerticalMode::Vs,
                        },
                        "IAS" => PanelSetting::Vertical {
                            mode: VerticalMode::Ias,
                        },
                        "ARM" => PanelSetting::Altitude {
                            altitude_m: s.altitude_m,
                        },
                        _ => {
                            self.legacy_vs = true;
                            settings.push(PanelSetting::Vertical {
                                mode: VerticalMode::Vs,
                            });
                            PanelSetting::VerticalSpeed {
                                vertical_speed_mps: step_vs(
                                    s.vertical_speed_mps,
                                    if name == "UP" { 1 } else { -1 },
                                ),
                            }
                        }
                    };
                    settings.push(action);
                }
            }
            let alt_rect = if legacy {
                cell(425., 32., 51., 51.)
            } else {
                cell(742., 39., 58., 58.)
            };
            knob(ui, alt_rect, !legacy);
            text(
                ui,
                alt_rect.center() - egui::vec2(0., 35. * scale.y),
                "ALT",
                10.,
                t::TEXT,
            );
            if !legacy && plate.width() >= 500. {
                text(
                    ui,
                    alt_rect.center() + egui::vec2(0., 38. * scale.y),
                    "100 / 1,000 FT",
                    9.,
                    t::MUTED,
                );
            }
            let (steps, outer) = self.altitude.detents(ui, "altitude", alt_rect, !legacy);
            if steps != 0 {
                let feet = (libm::round(s.altitude_m / 30.48) * 100.
                    + f64::from(steps) * if outer { 1000. } else { 100. })
                .clamp(100., 32800.);
                settings.push(if legacy {
                    PanelSetting::SelectedAltitude {
                        altitude_m: feet * 0.3048,
                    }
                } else {
                    PanelSetting::Altitude {
                        altitude_m: feet * 0.3048,
                    }
                });
            }
            if !legacy {
                let hdg = cell(642., 39., 58., 58.);
                knob(ui, hdg, false);
                text(
                    ui,
                    hdg.center() - egui::vec2(0., 35. * scale.y),
                    format!("HDG {}°", heading::degrees_text(heading_bug)),
                    11.,
                    t::TEXT,
                );
                if plate.width() >= 500. {
                    text(
                        ui,
                        hdg.center() + egui::vec2(0., 38. * scale.y),
                        "PUSH SYNC",
                        9.,
                        t::MUTED,
                    );
                }
                commands.extend(self.heading.show(ui, ui.id().with("ap_hdg"), hdg));
                let wheel = cell(564., 37., 36., 49.);
                ui.painter().rect_filled(wheel, 4., t::DIAL);
                for row in 0..7 {
                    let y = wheel.top() + (5. + row as f32 * 6.) * scale.y;
                    ui.painter().line_segment(
                        [
                            egui::pos2(wheel.left() + 6. * scale.x, y),
                            egui::pos2(wheel.right() - 6. * scale.x, y),
                        ],
                        Stroke::new(1., t::MUTED),
                    );
                }
                text(ui, at(582., 22.), "UP", 9., t::TEXT);
                text(ui, at(582., 96.), "DN", 9., t::TEXT);
                let (steps, _) = self.wheel.detents(ui, "up_down", wheel, false);
                if steps != 0 {
                    settings.push(match s.active_vertical {
                        Some(VerticalMode::Ias) => PanelSetting::Airspeed {
                            airspeed_mps: (libm::round(s.airspeed_mps * 1.9438444924406) + f64::from(steps))
                                .clamp(40., 174.)
                                / 1.9438444924406,
                        },
                        Some(VerticalMode::Pit) => PanelSetting::Pitch {
                            pitch_rad: (s.pitch_rad + f64::from(steps).to_radians()).clamp(-0.25, 0.35),
                        },
                        _ => PanelSetting::VerticalSpeed {
                            vertical_speed_mps: step_vs(s.vertical_speed_mps, steps),
                        },
                    });
                }
            }
            let cws = button(
                ui,
                cell(if legacy { 353. } else { 490. }, 4., 50., 18.),
                "CWS",
                s.control_wheel_steering,
            )
            .is_pointer_button_down_on()
                || cws_key;
            if cws || self.cws_owned {
                self.cws_owned = cws;
                if cws != s.control_wheel_steering {
                    settings.push(PanelSetting::ControlWheelSteering { pressed: cws });
                }
            }
        }
        if s.ap_flash && s.ap_flash_visible {
            text(
                ui,
                at(plate.width() - 58., plate.height() - 10.),
                "AP DISC",
                10.,
                t::AMBER,
            );
        }
        commands.extend(
            settings
                .into_iter()
                .map(|setting| AvionicsSetting::Autopilot { setting }),
        );
        commands
    }
}
fn step_vs(value: f64, steps: i32) -> f64 {
    ((libm::round(value / 0.508) + f64::from(steps)) * 0.508).clamp(-7.62, 7.62)
}
