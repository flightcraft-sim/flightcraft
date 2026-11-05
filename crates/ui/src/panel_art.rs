//! Generic millimetre panel artwork and physical control faces.
use crate::theme as t;
use eframe::egui::{self, Align2, FontId, Pos2, Rect, Sense, Stroke};
use flightcraft_aero::modern::*;
use flightcraft_aero::panel_fidelity::{ControlBinding, ControlStyle, SrgbHex};
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
pub(crate) fn color(hex: Option<&SrgbHex>, fallback: egui::Color32) -> egui::Color32 {
    hex.and_then(SrgbHex::rgb)
        .map_or(fallback, |[r, g, b]| egui::Color32::from_rgb(r, g, b))
}
/// Measure actual font glyphs, then shrink into the reserved caption rectangle.
pub(crate) fn fit_text(ui: &egui::Ui, rect: Rect, label: &str, size: f32, color: egui::Color32) -> Rect {
    let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(rect));
    let mut galley = painter.layout_no_wrap(label.into(), FontId::monospace(size.max(0.01)), color);
    let ratio = (rect.width().max(0.01) / galley.size().x.max(0.01))
        .min(rect.height().max(0.01) / galley.size().y.max(0.01))
        .min(1.);
    if ratio < 1. {
        galley = painter.layout_no_wrap(
            label.into(),
            FontId::monospace((size * ratio * 0.98).max(0.01)),
            color,
        );
    }
    let bounds = Rect::from_center_size(rect.center(), galley.size());
    painter.galley(bounds.min, galley, color);
    bounds
}
fn text(ui: &egui::Ui, at: Pos2, label: &str, size: f32, color: egui::Color32) {
    ui.painter()
        .text(at, Align2::CENTER_CENTER, label, FontId::monospace(size), color);
}
pub(crate) fn outline_points(outline: &Outline) -> Vec<[f64; 2]> {
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
/// The main dash excludes the separately authored lower console and unused canvas margins.
pub(crate) fn dash_bounds(panel: &EquipmentPanel) -> Option<Rect> {
    let shell = panel.shell.as_ref().filter(|s| s.centre_console.is_some())?;
    let mut points = outline_points(&shell.outline);
    if let Some(glare) = &shell.glare_shield {
        points.extend(outline_points(glare));
    }
    let bounds = points.into_iter().fold(Rect::NOTHING, |rect, [x, y]| {
        rect.union(Rect::from_min_max(
            egui::pos2(x as f32, y as f32),
            egui::pos2(x as f32, y as f32),
        ))
    });
    bounds.is_positive().then_some(bounds)
}
pub(crate) fn installation_rect(e: &EquipmentInstallation) -> Rect {
    Rect::from_min_size(
        egui::pos2(e.position[0] as f32, e.position[1] as f32),
        egui::vec2(e.size[0] as f32, e.size[1] as f32),
    )
}
fn translated(outline: &Outline, dx: f64, dy: f64) -> Outline {
    let shift = |[x, y]: [f64; 2]| [x - dx, y - dy];
    match outline {
        Outline::Polygon { points } => Outline::Polygon {
            points: points.iter().copied().map(shift).collect(),
        },
        Outline::Cubic { curves } => Outline::Cubic {
            curves: curves.iter().map(|c| c.map(shift)).collect(),
        },
    }
}
/// Rebase only the controls below the dash for an independent, interactive console host. The
/// console region (the cabin floor and the console top, when the shell authors them) is kept as
/// the host's shell, so the throttle, brake, park valve, and sticks sit on drawn artwork.
pub(crate) fn console_panel(panel: &EquipmentPanel, dash: Rect) -> EquipmentPanel {
    let mut console = panel.clone();
    console
        .elements
        .retain(|e| !dash.expand(0.01).contains_rect(installation_rect(e)));
    let mut bounds = console
        .elements
        .iter()
        .fold(Rect::NOTHING, |a, e| a.union(installation_rect(e)));
    let floor = panel.shell.as_ref().and_then(|s| s.floor.as_ref());
    if let Some(floor) = floor {
        for [x, y] in outline_points(floor) {
            bounds = bounds.union(Rect::from_min_max(
                egui::pos2(x as f32, y as f32),
                egui::pos2(x as f32, y as f32),
            ));
        }
    }
    console.size = [
        f64::from(bounds.width().max(1.)),
        f64::from(bounds.height().max(1.)),
    ];
    let (dx, dy) = (f64::from(bounds.left()), f64::from(bounds.top()));
    console.shell = panel.shell.as_ref().and_then(|s| {
        let top = s.centre_console.as_ref()?;
        Some(PanelShell {
            palette: s.palette.clone(),
            centre_console: None,
            floor: s.floor.as_ref().map(|o| translated(o, dx, dy)),
            outline: translated(top, dx, dy),
            fill: s.fill,
            glare_shield: None,
            screws: Vec::new(),
        })
    });
    console.essential = None;
    for e in &mut console.elements {
        e.position[0] -= dx;
        e.position[1] -= dy;
    }
    console
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
        .add(egui::Shape::closed_line(points, Stroke::new(1.0_f32, t::BORDER)));
}
pub(crate) fn shell(ui: &egui::Ui, rect: Rect, panel: &EquipmentPanel) {
    let Some(shell) = &panel.shell else {
        return;
    };
    let [w, h] = panel.size;
    let point = |[x, y]: [f64; 2]| {
        rect.min + egui::vec2((x / w) as f32 * rect.width(), (y / h) as f32 * rect.height())
    };
    if let Some(floor) = &shell.floor {
        fill(
            ui,
            outline_points(floor).into_iter().map(point).collect(),
            color(shell.palette.glare_shield.as_ref(), t::DIAL),
        );
    }
    fill(
        ui,
        outline_points(&shell.outline).into_iter().map(point).collect(),
        color(
            shell.palette.face.as_ref(),
            match shell.fill {
                PanelColor::Panel => t::PANEL,
                PanelColor::Raised => t::RAISED,
                PanelColor::Dial => t::DIAL,
            },
        ),
    );
    if let Some(console) = &shell.centre_console {
        fill(
            ui,
            outline_points(console).into_iter().map(point).collect(),
            color(shell.palette.face.as_ref(), t::PANEL),
        );
    }
    if let Some(glare) = &shell.glare_shield {
        fill(
            ui,
            outline_points(glare).into_iter().map(point).collect(),
            color(shell.palette.glare_shield.as_ref(), t::DIAL),
        );
    }
    let r = 1.4 * rect.width() / w as f32;
    for p in &shell.screws {
        let p = point(*p);
        ui.painter().circle_filled(p, r, t::DIAL);
        ui.painter().line_segment(
            [p - egui::vec2(r * 0.6, 0.), p + egui::vec2(r * 0.6, 0.)],
            Stroke::new(0.6_f32, t::MUTED),
        );
    }
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn primitive(
    ui: &mut egui::Ui,
    cell: Rect,
    installation: &EquipmentInstallation,
    state: Option<&ModernState>,
    font: f32,
    scale: f32,
    placard: egui::Color32,
    registration: &str,
) -> Option<ModernSetting> {
    let id = installation.id.as_str();
    let element = &installation.element;
    let appearance = &installation.appearance;
    let value = match appearance.binding {
        Some(ControlBinding::LaneA) => state.map_or(0., |s| if s.lane_a.on { 1. } else { 0. }),
        Some(ControlBinding::LaneB) => state.map_or(0., |s| if s.lane_b.on { 1. } else { 0. }),
        None => state
            .and_then(|s| s.panel_controls.get(id))
            .copied()
            .unwrap_or(0.),
    } as f32;
    let on = value >= 0.5;
    let centre = cell.center();
    let radius = cell.width().min(cell.height())
        * if appearance.style == Some(ControlStyle::CabinHeat) {
            0.42
        } else {
            0.30
        };
    let label = |s: &str| {
        let size = font.min(cell.width() / (s.chars().count().max(1) as f32 * 0.62));
        if appearance == &Default::default() {
            text(
                ui,
                cell.center_bottom() - egui::vec2(0., font * 0.55),
                s,
                size,
                placard,
            );
        } else {
            fit_text(
                ui,
                Rect::from_min_max(egui::pos2(cell.left(), cell.bottom() - font * 1.1), cell.max),
                s,
                size,
                placard,
            );
        }
    };
    match element {
        PanelElement::Label { text: s } => text(ui, centre, s, font, t::MUTED),
        PanelElement::Placard {
            text: s,
            font_size_mm,
            color,
        } => {
            let label_color = if *color == PanelInk::Text || *color == PanelInk::White {
                placard
            } else {
                ink(*color)
            };
            let s = &flightcraft_aero::modern::expand_placard_text(s, registration);
            let size = (*font_size_mm as f32 * scale).max(5.);
            let galley = ui
                .painter()
                .layout_no_wrap(s.clone(), FontId::monospace(size), label_color);
            if galley.size().x <= cell.width() && galley.size().y <= cell.height() {
                text(ui, centre, s, size, label_color);
            } else {
                fit_text(ui, cell, s, size, label_color);
            }
        }
        PanelElement::Bracket { label: s } => {
            text(ui, centre, s, font, t::TEXT);
            let width = (s.len() as f32 * font * 0.32).min(cell.width() * 0.4);
            for sign in [-1., 1.] {
                let a = centre + egui::vec2(sign * width, 0.);
                let b = egui::pos2(if sign < 0. { cell.left() } else { cell.right() }, centre.y);
                ui.painter().line_segment([a, b], Stroke::new(0.8_f32, t::TEXT));
                ui.painter().line_segment(
                    [b, b + egui::vec2(0., cell.height() * 0.4)],
                    Stroke::new(0.8_f32, t::TEXT),
                );
            }
        }
        PanelElement::GuardBar => {
            ui.painter()
                .rect_filled(cell, cell.width().min(cell.height()) * 0.5, t::TEXT);
        }
        PanelElement::Vent => {
            if appearance.style == Some(ControlStyle::LouvredEyeball) {
                for (r, c) in [
                    (1.58, t::DIAL),
                    (1.48, t::MUTED),
                    (1.36, t::DIAL),
                    (1.22, t::BORDER),
                    (1.12, t::DIAL),
                ] {
                    ui.painter().circle_filled(centre, radius * r, c);
                }
            } else {
                ui.painter().circle_filled(centre, radius * 1.5, t::BORDER);
                ui.painter().circle_filled(centre, radius * 1.25, t::DIAL);
            }
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
                .circle_stroke(centre, radius * 0.75, Stroke::new(1.0_f32, t::BORDER));
            label(s);
        }
        PanelElement::Toggle {
            label: s,
            guarded,
            lamp,
        } => {
            let bound = appearance.binding.is_some();
            let body = if bound {
                Rect::from_min_max(
                    egui::pos2(cell.left(), cell.top() + cell.height() * 0.43),
                    cell.max,
                )
            } else {
                cell
            };
            let pivot = if bound {
                body.center()
            } else {
                centre - egui::vec2(0., font * 0.5)
            };
            let lever_radius = if bound {
                body.width().min(body.height()) * 0.30
            } else {
                radius
            };
            ui.painter().circle_filled(pivot, lever_radius * 0.4, t::BORDER);
            let tip = pivot + egui::vec2(0., if on { -lever_radius } else { lever_radius * 0.6 });
            ui.painter()
                .line_segment([pivot, tip], Stroke::new(3. * scale, t::TEXT));
            ui.painter().circle_filled(tip, 2.5 * scale, t::MUTED);
            if bound {
                fit_text(
                    ui,
                    Rect::from_min_max(
                        cell.min + egui::vec2(0., cell.height() * 0.18),
                        egui::pos2(cell.right(), cell.top() + cell.height() * 0.39),
                    ),
                    s,
                    font,
                    placard,
                );
            } else {
                label(s);
            }
            if *lamp {
                ui.painter().circle_filled(
                    if bound {
                        cell.center_top() + egui::vec2(0., cell.height() * 0.07)
                    } else {
                        cell.center_top() + egui::vec2(0., radius * 0.2)
                    },
                    if bound {
                        cell.width().min(cell.height()) * 0.055
                    } else {
                        radius * 0.17
                    },
                    if state.is_some_and(|s| match appearance.binding {
                        Some(ControlBinding::LaneA) => s.lane_a.light,
                        Some(ControlBinding::LaneB) => s.lane_b.light,
                        None => on,
                    }) {
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
                    Rect::from_min_size(body.left_top(), egui::vec2(body.width(), body.height() * 0.22));
                ui.painter().rect_stroke(
                    if open {
                        guard
                    } else if bound {
                        body.shrink2(egui::vec2(body.width() * 0.15, body.height() * 0.08))
                    } else {
                        body.shrink(body.width() * 0.15)
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
                    body.shrink2(egui::vec2(0., body.height() * 0.23)),
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
                if appearance.style == Some(ControlStyle::EltPlate) {
                    ui.painter().rect(
                        cell.shrink(cell.width() * 0.04),
                        1.,
                        t::DIAL,
                        Stroke::new(1.0_f32, t::MUTED),
                        egui::StrokeKind::Inside,
                    );
                    for p in [
                        cell.left_top() + egui::vec2(cell.width() * 0.12, cell.height() * 0.1),
                        cell.right_bottom() - egui::vec2(cell.width() * 0.12, cell.height() * 0.1),
                    ] {
                        ui.painter().circle_filled(p, cell.width() * 0.025, t::MUTED);
                    }
                }
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
                        Stroke::new(0.7_f32, t::MUTED),
                    );
                }
            } else if matches!(element, PanelElement::PushPull { .. }) {
                let cap = centre - egui::vec2(0., value * radius * 0.55);
                ui.painter()
                    .circle_stroke(centre, radius * 0.95, Stroke::new(1.0_f32, t::BORDER));
                ui.painter()
                    .line_segment([centre, cap], Stroke::new(radius * 0.45, t::TEXT));
                ui.painter().circle_filled(cap, radius * 0.85, t::MUTED);
                ui.painter()
                    .circle_stroke(cap, radius * 0.85, Stroke::new(1.0_f32, t::TEXT));
                ui.painter().circle_filled(cap, radius * 0.45, t::RAISED);
            } else {
                ui.painter()
                    .circle_filled(centre, radius, if on { t::MUTED } else { t::DIAL });
                ui.painter()
                    .circle_stroke(centre, radius, Stroke::new(1.0_f32, t::TEXT));
                for i in 0..=10 {
                    let a = std::f32::consts::PI * (0.75 + i as f32 * 0.15);
                    let v = egui::vec2(a.cos(), a.sin());
                    ui.painter().line_segment(
                        [centre + v * radius * 1.15, centre + v * radius * 1.30],
                        Stroke::new(0.7_f32, t::MUTED),
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn measured_captions_stay_in_separate_rectangles() {
        let ctx = egui::Context::default();
        t::install(&ctx);
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                // Forty-pixel lamp spacing, with each lamp reserving its own caption width.
                let a = Rect::from_min_size(egui::pos2(593., 270.), egui::vec2(39., 10.));
                let b = Rect::from_min_size(egui::pos2(633., 270.), egui::vec2(39., 10.));
                let ra = fit_text(ui, a, "CAUTION", 12., t::TEXT);
                let rb = fit_text(ui, b, "WARNING", 12., t::TEXT);
                assert!(a.contains_rect(ra) && b.contains_rect(rb));
                assert!(!ra.intersects(rb));
                let pin = Rect::from_min_size(egui::pos2(830., 640.), egui::vec2(23., 18.));
                let handle = Rect::from_min_size(egui::pos2(858., 640.), egui::vec2(42., 18.));
                let rp = fit_text(ui, pin, "PIN IN", 12., t::TEXT);
                let rh = fit_text(ui, handle, "HOLD TO PULL", 12., t::TEXT);
                assert!(pin.contains_rect(rp) && handle.contains_rect(rh));
                assert!(!rp.intersects(rh));
                for label in ["NAV + STROBE", "EMS BKUP", "FUEL PUMP 1"] {
                    let r = Rect::from_min_size(egui::pos2(600., 500.), egui::vec2(32., 8.));
                    assert!(r.contains_rect(fit_text(ui, r, label, 13., t::TEXT)));
                }
            });
        });
    }
}
