//! ADS-B traffic symbology for the MFD map: hollow arrow for other traffic, filled for proximate,
//! amber for a traffic alert, a relative-altitude tag in hundreds of feet, and a climb or descent
//! arrow above 500 fpm. A magenta ring marks an aircraft the controller just named.
use super::*;
use flightcraft_control::{MapTraffic, TrafficClass, altitude_tag, trend};

/// Arrow outline pointing along `angle` (clockwise from the screen top) at `at`.
pub(super) fn arrow(at: [f32; 2], angle: f64, size: f32) -> Vec<[f32; 2]> {
    [[0., -1.4], [0.9, 1.], [0., 0.5], [-0.9, 1.]]
        .iter()
        .map(|[x, y]| rotated([x * size, y * size], at, angle))
        .collect()
}
fn ring(d: &mut DrawList, at: [f32; 2], radius: f32, ink: Ink) {
    let mut previous = [at[0] + radius, at[1]];
    for i in 1..=16 {
        let a = f64::from(i) * core::f64::consts::TAU / 16.;
        let next = [
            at[0] + radius * libm::cos(a) as f32,
            at[1] + radius * libm::sin(a) as f32,
        ];
        line(d, previous, next, ink, 1.4);
        previous = next;
    }
}
/// Climb (`1`) or descent (`-1`) arrow, drawn right of the altitude tag.
fn trend_arrow(d: &mut DrawList, at: [f32; 2], direction: i8, ink: Ink) {
    let [x, y] = at;
    let tip = if direction > 0 { y - 5. } else { y + 5. };
    let base = if direction > 0 { y + 5. } else { y - 5. };
    line(d, [x, base], [x, tip], ink, 1.3);
    let head = if direction > 0 { tip + 3.5 } else { tip - 3.5 };
    line(d, [x, tip], [x - 3., head], ink, 1.3);
    line(d, [x, tip], [x + 3., head], ink, 1.3);
}
pub(super) fn symbol(d: &mut DrawList, t: &MapTraffic, at: [f32; 2]) {
    let ink = match t.class {
        TrafficClass::Alert => Ink::Warning,
        _ => Ink::Text,
    };
    let outline = arrow(at, t.chart_track_rad, 6.);
    match t.class {
        TrafficClass::Other => {
            for (i, a) in outline.iter().enumerate() {
                if let Some(b) = outline.get((i + 1) % outline.len()) {
                    line(d, *a, *b, ink, 1.3);
                }
            }
        }
        TrafficClass::Proximate => polygon(d, outline, ink),
        TrafficClass::Alert => {
            circle(d, at, 8., Ink::Warning);
            polygon(d, arrow(at, t.chart_track_rad, 5.), Ink::Face);
        }
    }
    if t.highlighted {
        ring(d, at, 12., Ink::Gps);
    }
    let [x, y] = at;
    let tag = altitude_tag(t.relative_altitude_m / 0.3048);
    label(d, [x, y + 15.], &tag, 8., ink);
    let direction = trend(t.vertical_rate_fpm);
    if direction != 0 {
        trend_arrow(d, [x + 15., y + 15.], direction, ink);
    }
    if t.highlighted || t.class == TrafficClass::Alert {
        label(
            d,
            [x, y - 14.],
            &t.registration,
            8.,
            if t.highlighted { Ink::Gps } else { ink },
        );
    }
}
