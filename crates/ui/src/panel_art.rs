//! Generic millimetre panel artwork and physical control faces.
use crate::theme as t;
use eframe::egui::{self, Align2, FontId, Pos2, Rect, Sense, Stroke};
use flightcraft_aero::modern::*;
use flightcraft_sim::modern::{ModernSetting, ModernState};
pub(crate) fn ink(color: PanelInk) -> egui::Color32 {
    match color {
        PanelInk::Text | PanelInk::White => t::TEXT,
        PanelInk::Muted => t::MUTED,
        PanelInk::Amber => t::AMBER,
        PanelInk::Red => t::DANGER,
        PanelInk::Accent => t::ACCENT,
    }
}
fn text(ui: &egui::Ui, at: Pos2, label: &str, size: f32, color: egui::Color32) {
    ui.painter()
        .text(at, Align2::CENTER_CENTER, label, FontId::monospace(size), color);
}
fn outline_points(outline: &Outline) -> Vec<[f64; 2]> {
    match outline {
        Outline::Polygon { points } => points.clone(),
        Outline::Cubic { curves } => curves
            .iter()
            .flat_map(|[a, b, c, d]| {
                (0..12).map(move |i| {
                    let q = f64::from(i) / 12.;
                    let p = 1. - q;
                    let [ax, ay] = a;
                    let [bx, by] = b;
                    let [cx, cy] = c;
                    let [dx, dy] = d;
                    [
                        p * p * p * ax + 3. * p * p * q * bx + 3. * p * q * q * cx + q * q * q * dx,
                        p * p * p * ay + 3. * p * p * q * by + 3. * p * q * q * cy + q * q * q * dy,
                    ]
                })
            })
            .collect(),
    }
}
fn cross(a: Pos2, b: Pos2, c: Pos2) -> f32 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}
/// Bounded ear clipping supports concave shell outlines without assuming a convex panel.
fn fill(ui: &egui::Ui, points: Vec<Pos2>, color: egui::Color32) {
    if points.len() < 3 {
        return;
    }
    let area: f32 = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(a, b)| a.x * b.y - b.x * a.y)
        .sum();
    let sign = area.signum();
    let mut mesh = egui::Mesh::default();
    let mut triangle = |a, b, c| {
        let base = mesh.vertices.len() as u32;
        for p in [a, b, c] {
            mesh.colored_vertex(p, color);
        }
        mesh.add_triangle(base, base + 1, base + 2);
    };
    let mut pending = points.clone();
    while pending.len() > 3 {
        let mut ear = None;
        for i in 0..pending.len() {
            let n = pending.len();
            let (Some(&a), Some(&b), Some(&c)) = (
                pending.get((i + n - 1) % n),
                pending.get(i),
                pending.get((i + 1) % n),
            ) else {
                return;
            };
            if cross(a, b, c) * sign <= 0.00001 {
                continue;
            }
            let inside = pending.iter().enumerate().any(|(j, &p)| {
                j != i
                    && j != (i + n - 1) % n
                    && j != (i + 1) % n
                    && cross(a, b, p) * sign >= 0.
                    && cross(b, c, p) * sign >= 0.
                    && cross(c, a, p) * sign >= 0.
            });
            if !inside {
                ear = Some((i, a, b, c));
                break;
            }
        }
        let Some((i, a, b, c)) = ear else {
            break;
        };
        triangle(a, b, c);
        pending.remove(i);
    }
    if let [a, b, c] = pending.as_slice() {
        triangle(*a, *b, *c);
    }
    ui.painter().add(egui::Shape::mesh(mesh));
    ui.painter()
        .add(egui::Shape::closed_line(points, Stroke::new(1., t::BORDER)));
}
pub(crate) fn shell(ui: &egui::Ui, rect: Rect, panel: &EquipmentPanel) {
    let Some(shell) = &panel.shell else {
        return;
    };
    let [w, h] = panel.size;
    let point = |[x, y]: [f64; 2]| {
        rect.min + egui::vec2((x / w) as f32 * rect.width(), (y / h) as f32 * rect.height())
    };
    fill(
        ui,
        outline_points(&shell.outline).into_iter().map(point).collect(),
        match shell.fill {
            PanelColor::Panel => t::PANEL,
            PanelColor::Raised => t::RAISED,
            PanelColor::Dial => t::DIAL,
        },
    );
    if let Some(glare) = &shell.glare_shield {
        fill(
            ui,
            outline_points(glare).into_iter().map(point).collect(),
            t::DIAL,
        );
    }
    let r = 1.4 * rect.width() / w as f32;
    for p in &shell.screws {
        let p = point(*p);
        ui.painter().circle_filled(p, r, t::DIAL);
        ui.painter().line_segment(
            [p - egui::vec2(r * 0.6, 0.), p + egui::vec2(r * 0.6, 0.)],
            Stroke::new(0.6, t::MUTED),
        );
    }
}
pub(crate) fn primitive(
    ui: &mut egui::Ui,
    cell: Rect,
    id: &str,
    element: &PanelElement,
    state: Option<&ModernState>,
    font: f32,
    scale: f32,
) -> Option<ModernSetting> {
    let value = state
        .and_then(|s| s.panel_controls.get(id))
        .copied()
        .unwrap_or(0.) as f32;
    let on = value >= 0.5;
    let centre = cell.center();
    let radius = cell.width().min(cell.height()) * 0.30;
    let label = |s: &str| {
        text(
            ui,
            cell.center_bottom() - egui::vec2(0., font * 0.55),
            s,
            font.min(cell.width() / (s.chars().count().max(1) as f32 * 0.62)),
            t::TEXT,
        )
    };
    match element {
        PanelElement::Label { text: s } => text(ui, centre, s, font, t::MUTED),
        PanelElement::Placard {
            text: s,
            font_size_mm,
            color,
        } => text(ui, centre, s, (*font_size_mm as f32 * scale).max(5.), ink(*color)),
        PanelElement::Bracket { label: s } => {
            text(ui, centre, s, font, t::TEXT);
            let width = (s.len() as f32 * font * 0.32).min(cell.width() * 0.4);
            for sign in [-1., 1.] {
                let a = centre + egui::vec2(sign * width, 0.);
                let b = egui::pos2(if sign < 0. { cell.left() } else { cell.right() }, centre.y);
                ui.painter().line_segment([a, b], Stroke::new(0.8, t::TEXT));
                ui.painter().line_segment(
                    [b, b + egui::vec2(0., cell.height() * 0.4)],
                    Stroke::new(0.8, t::TEXT),
                );
            }
        }
        PanelElement::GuardBar => {
            ui.painter()
                .rect_filled(cell, cell.width().min(cell.height()) * 0.5, t::TEXT);
        }
        PanelElement::Vent => {
            ui.painter().circle_filled(centre, radius * 1.5, t::BORDER);
            ui.painter().circle_filled(centre, radius * 1.25, t::DIAL);
            for step in -2..=2 {
                let y = step as f32 * radius * 0.32;
                let half = (radius * radius - y * y).max(0.).sqrt();
                ui.painter().line_segment(
                    [centre + egui::vec2(-half, y), centre + egui::vec2(half, y)],
                    Stroke::new(radius * 0.10, t::MUTED),
                );
            }
        }
        PanelElement::Lamp { label: s, color } => {
            let centre = centre - egui::vec2(0., font * 0.5);
            ui.painter().circle_filled(
                centre,
                radius * 0.65,
                ink(*color).linear_multiply(if on { 1. } else { 0.18 }),
            );
            ui.painter()
                .circle_stroke(centre, radius * 0.75, Stroke::new(1., t::BORDER));
            label(s);
        }
        PanelElement::Toggle {
            label: s,
            guarded,
            lamp,
        } => {
            let pivot = centre - egui::vec2(0., font * 0.5);
            ui.painter().circle_filled(pivot, radius * 0.4, t::BORDER);
            let tip = pivot + egui::vec2(0., if on { -radius } else { radius * 0.6 });
            ui.painter()
                .line_segment([pivot, tip], Stroke::new(3. * scale, t::TEXT));
            ui.painter().circle_filled(tip, 2.5 * scale, t::MUTED);
            label(s);
            if *lamp {
                ui.painter().circle_filled(
                    cell.center_top() + egui::vec2(0., radius * 0.2),
                    radius * 0.17,
                    if on {
                        t::DANGER
                    } else {
                        t::DANGER.linear_multiply(0.15)
                    },
                );
            }
            if *guarded {
                let open = state
                    .and_then(|s| s.panel_guards.get(id))
                    .copied()
                    .unwrap_or(false);
                let guard =
                    Rect::from_min_size(cell.left_top(), egui::vec2(cell.width(), cell.height() * 0.22));
                ui.painter().rect_stroke(
                    if open {
                        guard
                    } else {
                        cell.shrink(cell.width() * 0.15)
                    },
                    2.,
                    Stroke::new(2. * scale, t::DANGER),
                    egui::StrokeKind::Inside,
                );
                if ui
                    .interact(guard, ui.id().with("guard"), Sense::click())
                    .clicked()
                {
                    return Some(ModernSetting::PanelGuard {
                        id: id.into(),
                        open: !open,
                    });
                }
                if !open {
                    return None;
                }
            }
            if ui
                .interact(
                    cell.shrink2(egui::vec2(0., cell.height() * 0.23)),
                    ui.id().with("toggle"),
                    Sense::click(),
                )
                .clicked()
            {
                return Some(ModernSetting::PanelControl {
                    id: id.into(),
                    value: if on { 0. } else { 1. },
                });
            }
        }
        PanelElement::Dimmer { label: s }
        | PanelElement::PushPull { label: s }
        | PanelElement::PitchWheel { label: s }
        | PanelElement::Elt { label: s } => {
            let centre = centre - egui::vec2(0., font * 0.4);
            if matches!(
                element,
                PanelElement::Elt { .. } | PanelElement::PitchWheel { .. }
            ) {
                let face = cell.shrink2(egui::vec2(cell.width() * 0.28, cell.height() * 0.20));
                ui.painter().rect_filled(
                    face,
                    2.,
                    if matches!(element, PanelElement::Elt { .. }) {
                        t::DANGER.linear_multiply(if on { 1. } else { 0.35 })
                    } else {
                        t::DIAL
                    },
                );
                for step in 1..5 {
                    let y = face.top() + face.height() * step as f32 / 5.;
                    ui.painter().line_segment(
                        [egui::pos2(face.left(), y), egui::pos2(face.right(), y)],
                        Stroke::new(0.7, t::MUTED),
                    );
                }
            } else if matches!(element, PanelElement::PushPull { .. }) {
                let cap = centre - egui::vec2(0., value * radius * 0.55);
                ui.painter()
                    .circle_stroke(centre, radius * 0.95, Stroke::new(1., t::BORDER));
                ui.painter()
                    .line_segment([centre, cap], Stroke::new(radius * 0.45, t::TEXT));
                ui.painter().circle_filled(cap, radius * 0.85, t::MUTED);
                ui.painter()
                    .circle_stroke(cap, radius * 0.85, Stroke::new(1., t::TEXT));
                ui.painter().circle_filled(cap, radius * 0.45, t::RAISED);
            } else {
                ui.painter()
                    .circle_filled(centre, radius, if on { t::MUTED } else { t::DIAL });
                ui.painter()
                    .circle_stroke(centre, radius, Stroke::new(1., t::TEXT));
                for i in 0..=10 {
                    let a = std::f32::consts::PI * (0.75 + i as f32 * 0.15);
                    let v = egui::vec2(a.cos(), a.sin());
                    ui.painter().line_segment(
                        [centre + v * radius * 1.15, centre + v * radius * 1.30],
                        Stroke::new(0.7, t::MUTED),
                    );
                }
                let a = std::f32::consts::PI * (0.75 + value * 1.5);
                ui.painter().line_segment(
                    [centre, centre + egui::vec2(a.cos(), a.sin()) * radius * 0.8],
                    Stroke::new(2. * scale, t::ACCENT),
                );
            }
            label(s);
            let response = ui.interact(cell, ui.id().with("physical_control"), Sense::click_and_drag());
            let delta = if response.hovered() {
                ui.input(|i| i.raw_scroll_delta.y.signum()) * 0.05
            } else {
                0.
            };
            let drag = if response.dragged() {
                -ui.input(|i| i.pointer.delta().y) / cell.height().max(1.)
            } else {
                0.
            };
            if response.clicked() {
                return Some(ModernSetting::PanelControl {
                    id: id.into(),
                    value: if on { 0. } else { 1. },
                });
            }
            if delta != 0. || drag != 0. {
                return Some(ModernSetting::PanelControl {
                    id: id.into(),
                    value: f64::from((value + delta + drag).clamp(0., 1.)),
                });
            }
        }
        _ => {}
    }
    None
}
