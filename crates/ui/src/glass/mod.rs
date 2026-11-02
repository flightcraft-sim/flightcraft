//! Original glass skins: behaviour snapshots enter through the instrument interface.
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
        let y = 47. + i as f32 * 28.;
        let (value, unit) = engine_display(v, options.engine);
        label(d, [54., y], format!("{} {}", v.label, value), 11., Ink::Text);
        label(d, [54., y + 11.], unit, 7., Ink::Muted);
        rect(d, [12., y + 17., 84., 3.], Ink::Inactive);
        let [normal_min, normal_max] = v.normal;
        let a = ((normal_min - v.min) / (v.max - v.min)).clamp(0., 1.) as f32;
        let b = ((normal_max - v.min) / (v.max - v.min)).clamp(0., 1.) as f32;
        rect(d, [12. + 84. * a, y + 17., 84. * (b - a), 3.], Ink::Normal);
        let f = ((v.value - v.min) / (v.max - v.min)).clamp(0., 1.) as f32;
        line(
            d,
            [12. + 84. * f, y + 15.],
            [12. + 84. * f, y + 22.],
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
