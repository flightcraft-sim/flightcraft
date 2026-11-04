//! Cockpit flight-control faces: throttle lever, hand brake lever, park-brake valve, flap
//! selector, and the control-stick grip with its trim buttons. Each emits the same typed
//! `SystemSetting` the keyboard shortcuts, MCP, and scripts use; none touches the simulation.
use super::{PanelUi, part, text};
use crate::{instruments::InstrumentInput, theme as t};
use eframe::egui::{self, Rect, Sense, Stroke};
use flightcraft_aero::modern::PanelElement;
use flightcraft_sim::systems::SystemSetting;

/// Trim travel while a stick trim button is held, as a fraction of the 0.3 range per second.
/// The keyboard binding runs at the same rate.
pub(crate) const TRIM_RATE_PER_S: f64 = 0.15;
const TRIM_LIMIT: f64 = 0.3;
/// A tap on a trim button moves the tab by this much even if the press is shorter than a frame.
const TRIM_TAP: f64 = 0.01;

/// The throttle's slot: full power at the top, idle at the bottom.
pub(crate) fn throttle_slot(cell: Rect) -> Rect {
    part(cell, 0.44, 0.13, 0.12, 0.60)
}
/// The flap selector's pivot and knob radius.
pub(crate) fn flap_dial(cell: Rect) -> (egui::Pos2, f32) {
    (
        part(cell, 0., 0.30, 1., 0.50).center(),
        cell.width().min(cell.height()) * 0.17,
    )
}
/// Where detent `index` of `count` is marked on the flap selector: an arc across the top.
pub(crate) fn flap_target(cell: Rect, index: usize, count: usize) -> egui::Pos2 {
    let (pivot, radius) = flap_dial(cell);
    let fraction = if count > 1 {
        index as f32 / (count - 1) as f32
    } else {
        0.
    };
    let angle = (-150. + 120. * fraction).to_radians();
    pivot + egui::vec2(angle.cos(), angle.sin()) * radius * 1.95
}
/// The grip, the pad centre, the pad button spacing, and the button radius of the stick.
fn stick_geometry(cell: Rect) -> (Rect, egui::Pos2, f32, f32) {
    let side = cell.width().min(cell.height() * 0.8);
    let grip = Rect::from_center_size(
        part(cell, 0., 0.0, 1., 0.84).center(),
        egui::vec2(side * 0.80, side * 1.05),
    );
    let pad = grip.center() + egui::vec2(0., grip.height() * 0.08);
    (grip, pad, grip.width() * 0.27, grip.width() * 0.13)
}
/// The centre of POH stick button 1 (trim down, forward) or 3 (trim up, aft).
#[cfg(test)]
pub(crate) fn stick_button(cell: Rect, number: u8) -> egui::Pos2 {
    let (_, pad, step, _) = stick_geometry(cell);
    pad + egui::vec2(0., if number == 1 { -step } else { step })
}

fn plate(ui: &egui::Ui, cell: Rect) {
    ui.painter().rect(
        cell.shrink(cell.width() * 0.03),
        4.,
        t::DIAL,
        Stroke::new(1.0_f32, t::BORDER),
        egui::StrokeKind::Inside,
    );
}

impl PanelUi {
    /// Draw and operate one cockpit control. Returns false if `element` is not one.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn cockpit_control(
        &mut self,
        ui: &mut egui::Ui,
        cell: Rect,
        element: &PanelElement,
        input: InstrumentInput<'_>,
        font: f32,
        scale: f32,
        settings: &mut Vec<SystemSetting>,
    ) -> bool {
        let pilot = input.pilot.unwrap_or_default();
        match element {
            PanelElement::Throttle { label } => {
                plate(ui, cell);
                let slot = throttle_slot(cell);
                ui.painter().rect(
                    slot,
                    2.,
                    t::PANEL,
                    Stroke::new(1.0_f32, t::BORDER),
                    egui::StrokeKind::Inside,
                );
                for (name, y) in [("FULL", 0.07), ("IDLE", 0.78)] {
                    text(ui, part(cell, 0., y, 1., 0.07), name, font * 0.6, t::MUTED);
                }
                let value = pilot.throttle as f32;
                let y = slot.bottom() - value * slot.height();
                let radius = cell.width() * 0.17;
                let knob = egui::pos2(slot.center().x, y);
                ui.painter().line_segment(
                    [egui::pos2(slot.center().x, slot.bottom()), knob],
                    Stroke::new(2. * scale, t::MUTED),
                );
                ui.painter().circle_filled(knob, radius, t::MUTED);
                ui.painter()
                    .circle_stroke(knob, radius, Stroke::new(1.5_f32, t::TEXT));
                text(ui, part(cell, 0., 0.86, 1., 0.12), label, font * 0.8, t::TEXT);
                let area = part(cell, 0.10, 0.04, 0.80, 0.80);
                let response = ui
                    .interact(area, ui.id().with("throttle"), Sense::click_and_drag())
                    .on_hover_text("Throttle lever: forward for more power (PageUp / PageDown)");
                if response.hovered() {
                    let wheel = ui.input(|i| i.raw_scroll_delta.y);
                    if wheel != 0. {
                        settings.push(SystemSetting::Throttle {
                            value: (pilot.throttle + f64::from(wheel.signum()) * 0.05).clamp(0., 1.),
                        });
                    }
                }
                if response.is_pointer_button_down_on()
                    && let Some(pointer) = ui.input(|i| i.pointer.interact_pos())
                {
                    let target =
                        (((slot.bottom() - pointer.y) / slot.height()).clamp(0., 1.) * 100.).round() / 100.;
                    if (f64::from(target) - pilot.throttle).abs() > 0.004 {
                        settings.push(SystemSetting::Throttle {
                            value: f64::from(target),
                        });
                    }
                }
                true
            }
            PanelElement::BrakeLever { label } => {
                plate(ui, cell);
                let squeezed = pilot.brake > 0.5;
                let lever = part(cell, 0.28, 0.10, 0.44, 0.62);
                ui.painter().rect(
                    lever,
                    cell.width() * 0.2,
                    if squeezed { t::BORDER } else { t::MUTED },
                    Stroke::new(1.5_f32, if squeezed { t::ACCENT } else { t::TEXT }),
                    egui::StrokeKind::Inside,
                );
                text(
                    ui,
                    lever,
                    if squeezed { "ON" } else { "HOLD" },
                    font * 0.8,
                    if squeezed { t::ACCENT } else { t::DIAL },
                );
                text(ui, part(cell, 0., 0.78, 1., 0.18), label, font * 0.8, t::TEXT);
                let response = ui
                    .interact(cell, ui.id().with("brake"), Sense::click_and_drag())
                    .on_hover_text("Hand brake lever: hold to brake (B)");
                let down = response.is_pointer_button_down_on();
                ui.ctx().data_mut(|d| {
                    if down {
                        d.insert_temp(egui::Id::new(super::PANEL_BRAKE_HELD), true);
                    } else if self.brake_owned {
                        d.remove::<bool>(egui::Id::new(super::PANEL_BRAKE_HELD));
                    }
                });
                if down && !self.brake_owned {
                    settings.push(SystemSetting::WheelBrake { value: 1. });
                    self.brake_owned = true;
                } else if !down && self.brake_owned {
                    settings.push(SystemSetting::WheelBrake { value: 0. });
                    self.brake_owned = false;
                }
                true
            }
            PanelElement::ParkBrake { label } => {
                plate(ui, cell);
                let on = input.systems.controls.park_brake;
                let centre = part(cell, 0., 0.10, 1., 0.55).center();
                let radius = cell.width().min(cell.height()) * 0.27;
                let pull = if on { cell.height() * 0.12 } else { 0. };
                let cap = centre + egui::vec2(0., pull);
                ui.painter()
                    .line_segment([centre, cap], Stroke::new(radius * 0.5, t::MUTED));
                ui.painter()
                    .circle_filled(cap, radius, if on { t::DANGER } else { t::MUTED });
                ui.painter()
                    .circle_stroke(cap, radius, Stroke::new(1.5_f32, t::TEXT));
                text(
                    ui,
                    part(cell, 0., 0.64, 1., 0.12),
                    if on { "ON" } else { "OFF" },
                    font * 0.75,
                    if on { t::DANGER } else { t::GREEN },
                );
                text(ui, part(cell, 0., 0.78, 1., 0.18), label, font * 0.8, t::TEXT);
                if ui
                    .interact(cell, ui.id().with("park"), Sense::click())
                    .on_hover_text(if on {
                        "Brake intercept valve closed: park brake on. Click to release (N)"
                    } else {
                        "Brake intercept valve open. Click to set the park brake (N)"
                    })
                    .clicked()
                {
                    settings.push(SystemSetting::ParkBrake { on: !on });
                }
                true
            }
            PanelElement::FlapSelector { label } => {
                plate(ui, cell);
                let stages: Vec<f64> = input
                    .aircraft
                    .map(|a| a.variant.flap_stages.iter().map(|s| s.position).collect())
                    .unwrap_or_default();
                let (pivot, radius) = flap_dial(cell);
                ui.painter().circle_filled(pivot, radius, t::MUTED);
                ui.painter()
                    .circle_stroke(pivot, radius, Stroke::new(1.5_f32, t::TEXT));
                let selected = stages
                    .iter()
                    .enumerate()
                    .min_by(|(_, a), (_, b)| (*a - pilot.flaps).abs().total_cmp(&(*b - pilot.flaps).abs()))
                    .map(|(i, _)| i);
                let count = stages.len();
                for (i, position) in stages.iter().enumerate() {
                    let fraction = if count > 1 {
                        i as f32 / (count - 1) as f32
                    } else {
                        0.
                    };
                    let angle = (-150. + 120. * fraction).to_radians();
                    let dir = egui::vec2(angle.cos(), angle.sin());
                    let target = flap_target(cell, i, count);
                    let lit = selected == Some(i);
                    ui.painter().line_segment(
                        [pivot + dir * radius * 1.15, pivot + dir * radius * 1.45],
                        Stroke::new(1.0_f32, t::MUTED),
                    );
                    let label_rect = Rect::from_center_size(target, egui::vec2(radius * 0.9, radius * 0.9));
                    text(
                        ui,
                        label_rect,
                        &i.to_string(),
                        font * 0.9,
                        if lit { t::ACCENT } else { t::TEXT },
                    );
                    if lit {
                        ui.painter().line_segment(
                            [pivot, pivot + dir * radius * 0.9],
                            Stroke::new(3. * scale, t::ACCENT),
                        );
                    }
                    if ui
                        .interact(
                            label_rect.expand(radius * 0.25),
                            ui.id().with(("flap", i)),
                            Sense::click(),
                        )
                        .on_hover_text(format!("Flap position {i}"))
                        .clicked()
                    {
                        settings.push(SystemSetting::Flaps { value: *position });
                    }
                }
                text(ui, part(cell, 0., 0.83, 1., 0.15), label, font * 0.65, t::TEXT);
                let knob = Rect::from_center_size(pivot, egui::vec2(radius * 2., radius * 2.));
                let response = ui.interact(knob, ui.id().with("flap-knob"), Sense::hover());
                if response.hovered() {
                    let wheel = ui.input(|i| i.raw_scroll_delta.y);
                    if wheel != 0. {
                        let current = selected.unwrap_or(0);
                        let next = if wheel > 0. {
                            (current + 1).min(count.saturating_sub(1))
                        } else {
                            current.saturating_sub(1)
                        };
                        if let Some(position) = stages.get(next) {
                            settings.push(SystemSetting::Flaps { value: *position });
                        }
                    }
                }
                true
            }
            PanelElement::ControlStick { label } => {
                let trim = input.systems.controls.trim;
                let (grip, pad, step, radius) = stick_geometry(cell);
                ui.painter().rect(
                    grip,
                    grip.width() * 0.27,
                    t::DIAL,
                    Stroke::new(1.5_f32, t::MUTED),
                    egui::StrokeKind::Inside,
                );
                let dt = f64::from(ui.input(|i| i.stable_dt)).clamp(0., 0.1);
                // POH 7.2 button allocation, seen from above with the nose up the page:
                // 1 trim down (forward), 3 trim up (aft), 2 autopilot, 4 not allocated, 5 PTT.
                for (number, offset, name, direction) in [
                    (1, egui::vec2(0., -step), "Trim down (nose down)", Some(-1.)),
                    (3, egui::vec2(0., step), "Trim up (nose up)", Some(1.)),
                    (2, egui::vec2(step, 0.), "Autopilot control", None),
                    (4, egui::vec2(-step, 0.), "Not allocated", None),
                ] {
                    let centre = pad + offset;
                    let rect = Rect::from_center_size(centre, egui::vec2(radius * 2.4, radius * 2.4));
                    let response =
                        ui.interact(rect, ui.id().with(("stick", number)), Sense::click_and_drag());
                    let down = response.is_pointer_button_down_on() && direction.is_some();
                    ui.painter().circle_filled(
                        centre,
                        radius,
                        if down {
                            t::ACCENT
                        } else if direction.is_some() {
                            t::MUTED
                        } else {
                            t::BORDER
                        },
                    );
                    ui.painter()
                        .circle_stroke(centre, radius, Stroke::new(1.0_f32, t::TEXT));
                    text(
                        ui,
                        Rect::from_center_size(centre, egui::vec2(radius * 2., radius * 2.)),
                        &number.to_string(),
                        font * 0.7,
                        t::DIAL,
                    );
                    let response = response.on_hover_text(format!(
                        "Stick button {number}: {name}{}",
                        if direction.is_some() {
                            format!(" (trim {trim:+.2}; Home / End)")
                        } else {
                            " (not wired to the dash)".into()
                        }
                    ));
                    if let Some(direction) = direction
                        && (down || response.clicked())
                    {
                        let step = if response.clicked() {
                            TRIM_TAP
                        } else {
                            TRIM_RATE_PER_S * dt
                        };
                        settings.push(SystemSetting::Trim {
                            value: (trim + direction * step).clamp(-TRIM_LIMIT, TRIM_LIMIT),
                        });
                    }
                }
                let ptt = Rect::from_center_size(
                    egui::pos2(grip.center().x, grip.top() + grip.height() * 0.10),
                    egui::vec2(grip.width() * 0.42, grip.height() * 0.10),
                );
                ui.painter().rect(
                    ptt,
                    2.,
                    t::BORDER,
                    Stroke::new(1.0_f32, t::TEXT),
                    egui::StrokeKind::Inside,
                );
                text(ui, ptt, "5", font * 0.6, t::TEXT);
                ui.interact(ptt, ui.id().with("stick-ptt"), Sense::hover())
                    .on_hover_text("Stick button 5: radio push to talk (T, or the Radios window)");
                text(ui, part(cell, 0., 0.86, 1., 0.12), label, font * 0.8, t::TEXT);
                true
            }
            _ => false,
        }
    }
}
