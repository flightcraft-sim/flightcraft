//! Original glass skins: behaviour snapshots enter through the instrument interface.
use eframe::egui;
mod controls;
mod mfd;
mod pfd;
use crate::instruments::{DrawItem, DrawList, Ink, Instrument, InstrumentInput};
pub use controls::{GlassControls, shortcut};
use flightcraft_control::GlassState;

pub fn create(id: &str) -> Result<Box<dyn Instrument>, String> {
    match id {
        "glass_pfd" | "glass_mfd" | "glass_standby" => Ok(Box::new(Display { id: id.into() })),
        _ => Err(format!("Unknown glass module: {id}")),
    }
}
struct Display {
    id: String,
}
impl Instrument for Display {
    fn update(&mut self, input: InstrumentInput<'_>) -> DrawList {
        let Some(state) = input.glass else {
            let mut d = canvas();
            label(&mut d, [250., 190.], "NO DISPLAY DATA", 18., Ink::Alert);
            return d;
        };
        if self.id == "glass_mfd" {
            mfd::draw(state, input.options)
        } else {
            pfd::draw(state, input, self.id == "glass_standby")
        }
    }
}
fn canvas() -> DrawList {
    DrawList {
        size: [500., 380.],
        items: Vec::new(),
        indicated: 0.,
        valid: true,
    }
}
fn label(d: &mut DrawList, at: [f32; 2], text: impl ToString, size: f32, ink: Ink) {
    d.items.push(DrawItem::Label {
        at,
        text: crate::instruments::formatting::clean_zero(&text.to_string()),
        size,
        ink,
    });
}
fn line(d: &mut DrawList, a: [f32; 2], b: [f32; 2], ink: Ink, width: f32) {
    d.items.push(DrawItem::Line { a, b, ink, width });
}
fn polygon(d: &mut DrawList, points: Vec<[f32; 2]>, ink: Ink) {
    if points.len() >= 3 {
        d.items.push(DrawItem::Polygon { points, ink });
    }
}
fn rect(d: &mut DrawList, [x, y, w, h]: [f32; 4], ink: Ink) {
    polygon(d, vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h]], ink);
}
fn circle(d: &mut DrawList, center: [f32; 2], radius: f32, ink: Ink) {
    d.items.push(DrawItem::Circle { center, radius, ink });
}
fn flag(d: &mut DrawList, [x, y, w, h]: [f32; 4], title: &str) {
    rect(d, [x, y, w, h], Ink::Face);
    line(d, [x + 4., y + 4.], [x + w - 4., y + h - 4.], Ink::Alert, 3.);
    line(d, [x + w - 4., y + 4.], [x + 4., y + h - 4.], Ink::Alert, 3.);
    rect(d, [x + w * 0.5 - 55., y + h * 0.5 - 10., 110., 20.], Ink::Face);
    label(d, [x + w * 0.5, y + h * 0.5], title, 12., Ink::Alert);
}
fn header(d: &mut DrawList, title: &str) {
    rect(d, [0., 0., 500., 380.], Ink::Face);
    rect(d, [0., 0., 500., 28.], Ink::Inactive);
    label(d, [250., 14.], title, 14., Ink::Text);
}
/// Clip convex polygons to the rectangular attitude viewport using half-plane intersections.
fn clip_polygon(mut points: Vec<[f32; 2]>, [x, y, w, h]: [f32; 4]) -> Vec<[f32; 2]> {
    for (axis, bound, greater) in [(0, x, true), (0, x + w, false), (1, y, true), (1, y + h, false)] {
        let coordinate = |[px, py]: [f32; 2]| if axis == 0 { px } else { py };
        let inside = |p| {
            if greater {
                coordinate(p) >= bound
            } else {
                coordinate(p) <= bound
            }
        };
        let Some(mut previous) = points.last().copied() else {
            break;
        };
        let mut output = Vec::new();
        for current in points {
            if inside(previous) != inside(current) {
                let fraction = (bound - coordinate(previous)) / (coordinate(current) - coordinate(previous));
                let [px, py] = previous;
                let [cx, cy] = current;
                output.push([px + (cx - px) * fraction, py + (cy - py) * fraction]);
            }
            if inside(current) {
                output.push(current);
            }
            previous = current;
        }
        points = output;
    }
    points
}
fn rotated([x, y]: [f32; 2], [cx, cy]: [f32; 2], angle: f64) -> [f32; 2] {
    let (s, c) = (libm::sin(angle) as f32, libm::cos(angle) as f32);
    [cx + x * c - y * s, cy + x * s + y * c]
}
/// Linear tape ordinate, shared by ticks, limit bands, trends, and selected bugs.
pub fn tape_y(value: f64, current: f64, center: f32, pixels_per_unit: f32) -> f32 {
    center - (value - current) as f32 * pixels_per_unit
}
fn engine_strip(d: &mut DrawList, s: &GlassState, options: crate::instruments::DisplayOptions) {
    for (i, v) in s.engine.iter().enumerate() {
        let compact = s.engine.len() > 11;
        let spacing = if compact {
            310. / s.engine.len() as f32
        } else {
            28.
        };
        let y = if compact { 38. } else { 47. } + i as f32 * spacing;
        let bar_y = y + if compact { 8. } else { 17. };
        let (value, unit) = engine_display(v, options.engine);
        if compact {
            label(
                d,
                [54., y],
                format!("{} {} {}", v.label, value, unit),
                8.,
                Ink::Text,
            );
        } else {
            label(d, [54., y], format!("{} {}", v.label, value), 11., Ink::Text);
            label(d, [54., y + 11.], unit, 7., Ink::Muted);
        }
        rect(d, [12., bar_y, 84., 3.], Ink::Inactive);
        let [normal_min, normal_max] = v.normal;
        let a = ((normal_min - v.min) / (v.max - v.min)).clamp(0., 1.) as f32;
        let b = ((normal_max - v.min) / (v.max - v.min)).clamp(0., 1.) as f32;
        rect(d, [12. + 84. * a, bar_y, 84. * (b - a), 3.], Ink::Normal);
        let f = ((v.value - v.min) / (v.max - v.min)).clamp(0., 1.) as f32;
        line(
            d,
            [12. + 84. * f, bar_y - 2.],
            [12. + 84. * f, bar_y + 5.],
            if v.value >= normal_min && v.value <= normal_max {
                Ink::Text
            } else {
                Ink::Warning
            },
            2.,
        );
    }
}
#[cfg(all(test, not(target_arch = "wasm32")))]
mod preview;
#[cfg(test)]
mod tests;

fn engine_display(
    v: &flightcraft_control::EngineValue,
    units: crate::instruments::EngineUnits,
) -> (String, String) {
    use crate::instruments::EngineUnits;
    let (value, unit, precision) = match (v.unit.as_str(), units) {
        ("kg/h", EngineUnits::Us) => (v.value / (0.72 * 3.785411784), "gal/h", 1),
        ("kg", EngineUnits::Us) => (v.value / (0.72 * 3.785411784), "US gal", 1),
        ("kPa", EngineUnits::Us) => (v.value / 6.894757293, "psi", 0),
        ("C", EngineUnits::Us) => (v.value * 1.8 + 32., "°F", 0),
        ("C", _) => (v.value, "°C", 0),
        (unit, _) => (v.value, unit, if unit == "A" || unit == "V" { 1 } else { 0 }),
    };
    (
        crate::instruments::formatting::clean_zero(&format!("{value:.precision$}")),
        unit.into(),
    )
}

/// Fill the installation viewport while keeping circular symbols and text undistorted.
fn paint_fill(ui: &egui::Ui, area: egui::Rect, mut draw: DrawList) {
    use crate::instruments::{Skin, SkinPack};
    let [w, h] = draw.size;
    let (sx, sy) = (area.width() / w, area.height() / h);
    let factor = sx.min(sy);
    let point = |[x, y]: &mut [f32; 2]| {
        *x = area.left() + *x * sx;
        *y = area.top() + *y * sy;
    };
    for item in &mut draw.items {
        match item {
            DrawItem::Circle { center, radius, .. } => {
                point(center);
                *radius *= factor;
            }
            DrawItem::Label { at, size, .. } => {
                point(at);
                *size = (*size * factor).max(5.);
            }
            DrawItem::Line { a, b, width, .. } => {
                point(a);
                point(b);
                *width = (*width * factor).max(0.5);
            }
            DrawItem::Polygon { points, .. } => {
                for p in points {
                    point(p);
                }
            }
            DrawItem::Mesh { vertices, .. } => {
                for (p, ..) in vertices {
                    point(p);
                }
            }
        }
    }
    SkinPack {
        id: "original".into(),
        colors: Vec::new(),
        face: Vec::new(),
    }
    .paint(&ui.painter().with_clip_rect(area), &draw);
}
fn touch_engine(
    ui: &egui::Ui,
    area: egui::Rect,
    state: &GlassState,
    input: InstrumentInput<'_>,
    style: flightcraft_aero::modern::EngineStripStyle,
) {
    use flightcraft_aero::modern::EngineStripStyle;
    let mut s = state.clone();
    let mut bars = area;
    if style == EngineStripStyle::DialsAndBars {
        let height = area.height() * 0.32;
        for (index, name) in ["RPM", "MANIFOLD"].into_iter().enumerate() {
            let center = egui::pos2(area.center().x, area.top() + height * (index as f32 + 0.5) * 0.5);
            let r = (height * 0.23).min(area.width() * 0.39);
            ui.painter().circle_filled(center, r, crate::theme::DIAL);
            ui.painter()
                .circle_stroke(center, r, egui::Stroke::new(1., crate::theme::MUTED));
            if let Some(v) = s.engine.iter().find(|v| v.label == name) {
                let a = std::f32::consts::PI
                    * (0.75 + 1.5 * ((v.value - v.min) / (v.max - v.min)).clamp(0., 1.) as f32);
                ui.painter().line_segment(
                    [center, center + egui::vec2(a.cos(), a.sin()) * r * 0.8],
                    egui::Stroke::new(1.3, crate::theme::ACCENT),
                );
                ui.painter().text(
                    center + egui::vec2(0., r * 0.50),
                    egui::Align2::CENTER_CENTER,
                    format!(
                        "{} {} {}",
                        name,
                        engine_display(v, input.options.engine).0,
                        engine_display(v, input.options.engine).1
                    ),
                    egui::FontId::monospace((r * 0.24).max(5.)),
                    crate::theme::TEXT,
                );
            }
        }
        s.engine.retain(|v| v.label != "RPM" && v.label != "MANIFOLD");
        bars.min.y += height;
    }
    let mut d = canvas();
    d.size = [110., 380.];
    engine_strip(&mut d, &s, input.options);
    paint_fill(ui, bars, d);
}
/// Installed touch composition. Returned map rectangles are hosted by the shared map widget.
pub(crate) fn paint_touch(
    ui: &egui::Ui,
    cell: egui::Rect,
    s: &GlassState,
    input: InstrumentInput<'_>,
    display: &flightcraft_control::TouchDisplayState,
    composition: &flightcraft_aero::modern::DisplayComposition,
) -> Vec<egui::Rect> {
    use flightcraft_aero::modern::{DisplayInset, TouchLayout};
    let mut maps = Vec::new();
    if display.failed || !display.powered {
        let mut d = canvas();
        flag(
            &mut d,
            [0., 0., 500., 380.],
            if display.failed {
                "DISPLAY FAIL"
            } else {
                "NO POWER"
            },
        );
        paint_fill(ui, cell, d);
        return maps;
    }
    let mut shared = s.clone();
    shared.pfd_failed = false;
    shared.mfd_failed = false;
    shared.reversion = false;
    let mut content = cell;
    if composition.top_data_bar {
        let bar = egui::Rect::from_min_size(cell.min, egui::vec2(cell.width(), cell.height() * 0.08));
        ui.painter().rect_filled(bar, 0., crate::theme::RAISED);
        let radio = s.touch.navigator.as_ref().map_or("COM —".into(), |n| {
            format!(
                "COM {:.3} / {:.3}",
                n.com_active_khz as f64 / 1000.,
                n.com_standby_khz as f64 / 1000.
            )
        });
        let advice = input.systems.modern.as_ref().and_then(|m| {
            m.annunciations.iter().find(|a| {
                a.active && a.source == flightcraft_aero::modern::AnnunciationSource::EngineShutdown
            })
        });
        for (fraction, label) in [
            (0.20, radio),
            (
                0.54,
                advice.map_or_else(
                    || {
                        format!(
                            "{} {:.1} NM",
                            s.gps.destination.as_deref().unwrap_or("GPS"),
                            s.gps.distance_nm
                        )
                    },
                    |a| a.label.clone(),
                ),
            ),
            (
                0.84,
                format!(
                    "{:.2}  XPDR {}",
                    s.baro_pa / 3386.389,
                    s.settings.transponder_code
                ),
            ),
        ] {
            ui.painter().text(
                egui::pos2(bar.left() + bar.width() * fraction, bar.center().y),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::monospace((bar.height() * 0.37).max(5.)),
                crate::theme::TEXT,
            );
        }
        content.min.y = bar.bottom();
    }
    let layout = if display.reversion {
        TouchLayout::FullPfd
    } else {
        display.layout
    };
    let strip = if display.engine_strip || display.reversion {
        content.width() * composition.engine_strip_fraction as f32
    } else {
        0.
    };
    let main = egui::Rect::from_min_max(content.min + egui::vec2(strip, 0.), content.max);
    if strip > 0. {
        touch_engine(
            ui,
            egui::Rect::from_min_max(content.min, egui::pos2(main.left(), content.bottom())),
            &shared,
            input,
            composition.engine_style,
        );
    }
    let primary = match layout {
        TouchLayout::FullPfd => Some(main),
        TouchLayout::PfdMap => {
            let (left, right) = main.split_left_right_at_fraction(0.5);
            maps.push(right);
            Some(left)
        }
        TouchLayout::MapEngine => {
            let (map, engine) = main.split_left_right_at_fraction(0.68);
            maps.push(map);
            touch_engine(ui, engine, &shared, input, composition.engine_style);
            None
        }
    };
    if let Some(primary) = primary {
        paint_fill(
            ui,
            primary,
            pfd::draw_composed(&shared, input, false, composition.hsi),
        );
        for inset in &composition.insets {
            let rect = egui::Rect::from_min_size(
                primary.min
                    + egui::vec2(
                        primary.width()
                            * if matches!(
                                inset.corner,
                                flightcraft_aero::modern::PfdCorner::TopLeft
                                    | flightcraft_aero::modern::PfdCorner::BottomLeft
                            ) {
                                0.01
                            } else {
                                0.72
                            },
                        primary.height()
                            * if matches!(
                                inset.corner,
                                flightcraft_aero::modern::PfdCorner::TopLeft
                                    | flightcraft_aero::modern::PfdCorner::TopRight
                            ) {
                                0.10
                            } else {
                                0.68
                            },
                    ),
                egui::vec2(primary.width() * 0.27, primary.height() * 0.25),
            );
            match inset.content {
                DisplayInset::Map => maps.push(rect),
                DisplayInset::Engine => touch_engine(ui, rect, &shared, input, composition.engine_style),
                DisplayInset::Navigation => {
                    ui.painter().rect_filled(rect, 0., crate::theme::DIAL);
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        format!(
                            "{}\nDTK {:03.0}° M\n{:.1} NM",
                            s.gps.source,
                            s.gps.desired_track_rad.to_degrees(),
                            s.gps.distance_nm
                        ),
                        egui::FontId::monospace((rect.height() * 0.12).max(5.)),
                        crate::theme::ACCENT,
                    );
                }
            }
        }
    }
    maps
}

pub(crate) fn paint_standby(ui: &egui::Ui, cell: egui::Rect, state: &GlassState, input: InstrumentInput<'_>) {
    use crate::instruments::{Skin, SkinPack, fit_size, scaled};
    let draw = pfd::draw(state, input, true);
    let skin = SkinPack {
        id: "original".into(),
        colors: Vec::new(),
        face: Vec::new(),
    };
    scaled(ui.painter(), fit_size(cell, draw.size), draw.size, |p| {
        skin.paint(p, &draw)
    });
}
