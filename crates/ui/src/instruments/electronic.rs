//! Original rectangular electronic skins. All geometry consumes displayed unit values only.
use super::{DrawItem, DrawList, Ink, Instrument, InstrumentInput};
use flightcraft_sim::avionics::electronic_attitude::ElectronicAttitude;

pub(super) fn create(id: &str) -> Result<Box<dyn Instrument>, String> {
    match id {
        "electronic_attitude" => Ok(Box::new(Attitude)),
        "electronic_hsi" => Ok(Box::new(Hsi)),
        _ => Err(format!("Unknown electronic instrument: {id}")),
    }
}
struct Attitude;
impl Instrument for Attitude {
    fn update(&mut self, input: InstrumentInput<'_>) -> DrawList {
        attitude(&input.systems.avionics.electronic_attitude, input, "ATTITUDE")
    }
}
struct Hsi;
impl Instrument for Hsi {
    fn update(&mut self, input: InstrumentInput<'_>) -> DrawList {
        let h = &input.systems.avionics.electronic_hsi;
        let a = &h.reversion_attitude;
        if h.reversion {
            return attitude(a, input, "REVERSION");
        }
        let mut d = base(a, "NAVIGATION");
        if !a.power.powered {
            return d;
        }
        d.label(
            [100., 23.],
            if a.attitude_valid {
                format!("HDG {:03.0}", a.heading_deg)
            } else if a.aligning {
                format!("ALIGNING {:02.0}s", a.alignment_remaining_s)
            } else {
                "AHRS X".into()
            },
            9.,
            if a.attitude_valid { Ink::Text } else { Ink::Alert },
        );
        let center = [100., 77.];
        for i in 0..72 {
            let angle = f64::from(i) * 5. - a.heading_deg;
            let point = |radius| compass(center, angle, radius);
            d.line(
                point(42.),
                point(if i % 6 == 0 { 37. } else { 40. }),
                Ink::Muted,
                0.7,
            );
            if i % 6 == 0 {
                let text = match i {
                    0 => "N".into(),
                    18 => "E".into(),
                    36 => "S".into(),
                    54 => "W".into(),
                    _ => format!("{}", i / 2),
                };
                d.label(point(32.), text, 7., Ink::Text);
            }
        }
        let bug = compass(center, h.heading_bug_deg - a.heading_deg, 44.);
        d.circle(bug, 2., Ink::Needle);
        let course = h.course_deg - a.heading_deg;
        d.line(
            compass(center, course, 29.),
            compass(center, course + 180., 29.),
            Ink::Normal,
            1.2,
        );
        let tip = compass(center, course, 29.);
        d.items.push(DrawItem::Polygon {
            points: vec![
                tip,
                compass(center, course - 10., 22.),
                compass(center, course + 10., 22.),
            ],
            ink: Ink::Normal,
        });
        // Deviation is perpendicular to the selected course; needle moves toward the path.
        if let Some(cdi) = h.cdi {
            let offset = -cdi as f32 * 18.;
            let [ax, ay] = compass([0., 0.], course + 90., 1.);
            let shifted = [100. + ax * offset, 77. + ay * offset];
            d.line(
                compass(shifted, course, 16.),
                compass(shifted, course + 180., 16.),
                Ink::Normal,
                2.,
            );
            for value in [-18., -9., 9., 18.] {
                d.circle([100. + ax * value, 77. + ay * value], 1., Ink::Text);
            }
        }
        d.label(
            [29., 47.],
            format!("BUG\n{:03.0}", h.heading_bug_deg),
            7.,
            Ink::Needle,
        );
        d.label(
            [171., 47.],
            format!("CRS\n{:03.0}", h.course_deg),
            7.,
            Ink::Normal,
        );
        if !h.gps_fix {
            d.label([100., 128.], "NO GPS", 9., Ink::Alert);
        } else {
            d.label(
                [100., 128.],
                format!(
                    "GPS {}  ±{:.0} NM",
                    h.destination.as_deref().unwrap_or("----"),
                    h.full_scale_nm
                ),
                8.,
                Ink::Normal,
            );
        }
        d.label(
            [51., 142.],
            format!("GS {} KT", number(h.ground_speed_kt, 0)),
            7.,
            Ink::Text,
        );
        d.label(
            [150., 142.],
            format!("TRK {}°", number(h.track_deg, 0)),
            7.,
            Ink::Text,
        );
        d.label(
            [51., 155.],
            format!("DIST {} NM", number(h.distance_nm, 1)),
            7.,
            Ink::Text,
        );
        d.label(
            [150., 155.],
            format!("BRG {}°", number(h.bearing_deg, 0)),
            7.,
            Ink::Text,
        );
        d.label(
            [100., 169.],
            format!(
                "WIND {}° / {} KT",
                number(h.wind_from_deg, 0),
                number(h.wind_speed_kt, 0)
            ),
            7.,
            Ink::Text,
        );
        if !a.attitude_valid && !a.aligning {
            red_x(&mut d, [56., 33.], [144., 120.]);
        }
        d
    }
}
fn number(v: Option<f64>, precision: usize) -> String {
    v.map_or_else(|| "---".into(), |v| format!("{v:.precision$}"))
}
fn compass([x, y]: [f32; 2], degrees: f64, radius: f32) -> [f32; 2] {
    let a = degrees.to_radians();
    [x + libm::sin(a) as f32 * radius, y - libm::cos(a) as f32 * radius]
}
fn rect(d: &mut DrawList, [x, y]: [f32; 2], [w, h]: [f32; 2], ink: Ink) {
    d.items.push(DrawItem::Polygon {
        points: vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h]],
        ink,
    });
}
fn base(a: &ElectronicAttitude, title: &str) -> DrawList {
    let mut d = DrawList {
        size: [200., 180.],
        items: Vec::new(),
        indicated: a.heading_deg,
        valid: a.attitude_valid,
    };
    rect(&mut d, [1., 1.], [198., 178.], Ink::Face);
    d.label([48., 10.], title, 8., Ink::Text);
    let status = if !a.power.switch_on {
        "OFF".into()
    } else if !a.power.powered {
        "NO POWER".into()
    } else if a.power.on_battery {
        format!(
            "BAT {:.0}%",
            a.power.battery_remaining_s / a.power.battery_endurance_s * 100.
        )
    } else {
        "BUS".into()
    };
    d.label(
        [153., 10.],
        status,
        8.,
        if a.power.on_battery {
            Ink::Warning
        } else {
            Ink::Muted
        },
    );
    d
}
fn attitude(a: &ElectronicAttitude, input: InstrumentInput<'_>, title: &str) -> DrawList {
    let mut d = base(a, title);
    d.indicated = a.roll_deg;
    if !a.power.powered {
        return d;
    }
    // Clip a banked sky/ground boundary to the rectangular attitude viewport.
    let bank = a.roll_deg.to_radians();
    let pitch = (a.pitch_deg as f32 * 1.3).clamp(-100., 100.);
    let horizon =
        |[x, y]: [f32; 2]| (x - 100.) * libm::sin(bank) as f32 + (y - 79.) * libm::cos(bank) as f32 - pitch;
    let viewport = [[52., 32.], [145., 32.], [145., 125.], [52., 125.]];
    rect(&mut d, [52., 32.], [93., 93.], Ink::Sky);
    let mut ground = Vec::new();
    for (p, q) in viewport.iter().zip(viewport.iter().cycle().skip(1)).take(4) {
        let (hp, hq) = (horizon(*p), horizon(*q));
        if hp >= 0. {
            ground.push(*p);
        }
        if (hp >= 0.) != (hq >= 0.) {
            let f = hp / (hp - hq);
            let [px, py] = *p;
            let [qx, qy] = *q;
            ground.push([px + (qx - px) * f, py + (qy - py) * f]);
        }
    }
    if ground.len() >= 3 {
        d.items.push(DrawItem::Polygon {
            points: ground,
            ink: Ink::Earth,
        });
    }
    for degrees in [-30., -20., -10., 0., 10., 20., 30.] {
        let y = (degrees - a.pitch_deg) as f32 * 1.3;
        let transform = |x: f32| {
            [
                100. + x * libm::cos(bank) as f32 - y * libm::sin(bank) as f32,
                79. - x * libm::sin(bank) as f32 - y * libm::cos(bank) as f32,
            ]
        };
        let (left, right) = (transform(-13.), transform(13.));
        let inside = |[x, y]: [f32; 2]| (54. ..=143.).contains(&x) && (34. ..=123.).contains(&y);
        if inside(left) && inside(right) {
            d.line(left, right, Ink::Text, if degrees == 0. { 1.5 } else { 0.6 });
        }
        if a.attitude_valid && degrees != 0. {
            let at = transform(23.);
            let [x, y] = at;
            if (61. ..=136.).contains(&x) && (40. ..=116.).contains(&y) {
                d.label(at, format!("{:.0}", degrees.abs()), 5., Ink::Text);
            }
        }
    }
    for angle in [-60., -30., 0., 30., 60.] {
        let top = compass([100., 75.], angle, 39.);
        let bottom = compass([100., 75.], angle, 35.);
        d.line(top, bottom, Ink::Text, 1.);
    }
    let bank = a.roll_deg.clamp(-60., 60.);
    d.items.push(DrawItem::Polygon {
        points: vec![
            compass([100., 75.], bank, 34.),
            compass([100., 75.], bank - 4., 38.),
            compass([100., 75.], bank + 4., 38.),
        ],
        ink: Ink::Needle,
    });
    d.label([100., 22.], format!("ROLL {:+.0}°", a.roll_deg), 7., Ink::Text);
    d.line([82., 79.], [94., 79.], Ink::Needle, 2.);
    d.line([106., 79.], [118., 79.], Ink::Needle, 2.);
    d.circle([100., 79.], 2., Ink::Needle);
    tape(&mut d, 25., a.airspeed_kt, 10., "KT");
    tape(&mut d, 170., a.altitude_ft, 100., "FT");
    // Variant-supplied V-speed bands share the airspeed tape's moving scale.
    if let Some(aircraft) = input.aircraft {
        let v = &aircraft.variant.v_speeds;
        for (lo, hi, x, ink) in [
            (v.vso_mps, v.vfe_mps, 47., Ink::Text),
            (v.vs_mps, v.vno_mps, 44., Ink::Normal),
            (v.vno_mps, v.vne_mps, 44., Ink::Warning),
        ] {
            let y = |speed: f64| (79. - (speed * 1.94384449 - a.airspeed_kt) as f32 * 1.3).clamp(34., 124.);
            d.line([x, y(lo)], [x, y(hi)], ink, 2.);
        }
        let red = 79. - (v.vne_mps * 1.94384449 - a.airspeed_kt) as f32 * 1.3;
        if (34. ..=124.).contains(&red) {
            d.line([40., red], [49., red], Ink::Alert, 2.);
        }
    }
    d.line(
        [49., 79.],
        [49., (79. - a.airspeed_trend_kt as f32 * 1.3).clamp(34., 124.)],
        Ink::Needle,
        2.,
    );
    d.line(
        [147., 79.],
        [147., (79. - a.altitude_trend_ft as f32 * 0.13).clamp(34., 124.)],
        Ink::Needle,
        2.,
    );
    let bug = (79. - (a.selected_altitude_ft - a.altitude_ft) as f32 * 0.13).clamp(34., 124.);
    d.line([149., bug], [156., bug], Ink::Needle, 2.);
    d.line([195., 35.], [195., 124.], Ink::Muted, 1.);
    for y in [35., 57., 79., 101., 124.] {
        d.line([191., y], [197., y], Ink::Text, 0.7);
    }
    let vsi = (79. - (a.vertical_speed_fpm / 2000.).clamp(-1., 1.) as f32 * 44.).clamp(35., 124.);
    d.line([190., vsi], [198., vsi], Ink::Needle, 2.);
    d.label(
        [50., 138.],
        format!("VS {:+.0}", a.vertical_speed_fpm),
        7.,
        Ink::Text,
    );
    d.label(
        [152., 138.],
        format!("SEL {:.0}", a.selected_altitude_ft),
        7.,
        Ink::Needle,
    );
    d.label(
        [50., 152.],
        format!(
            "{} {}",
            input.options.pressure_value(a.baro_inhg * 3386.389),
            input.options.pressure_label()
        ),
        8.,
        Ink::Text,
    );
    d.label(
        [151., 152.],
        format!("TURN {:+.1}°/s", a.turn_rate_deg_s),
        7.,
        Ink::Text,
    );
    d.line([65., 166.], [135., 166.], Ink::Muted, 1.);
    d.circle(
        [100. - (a.slip_g as f32 * 30.).clamp(-30., 30.), 166.],
        3.,
        Ink::Text,
    );
    if a.aligning {
        rect(&mut d, [54., 64.], [89., 26.], Ink::Face);
        d.label(
            [99., 76.],
            format!("ALIGNING {:.0}s", a.alignment_remaining_s),
            8.,
            Ink::Warning,
        );
    } else if !a.attitude_valid {
        red_x(&mut d, [54., 34.], [143., 123.]);
    }
    if !a.air_data_valid {
        red_x(&mut d, [5., 33.], [45., 124.]);
        red_x(&mut d, [151., 33.], [188., 124.]);
    }
    d
}
fn tape(d: &mut DrawList, x: f32, value: f64, step: f64, unit: &str) {
    rect(d, [x - 20., 32.], [40., 94.], Ink::Inactive);
    let base = libm::floor(value / step) * step;
    for offset in -3..=3 {
        let tick = base + f64::from(offset) * step;
        let y = 79. - ((tick - value) / step * 13.) as f32;
        if (37. ..=120.).contains(&y) && (y - 79.).abs() > 11. {
            d.label([x, y], format!("{tick:.0}"), 7., Ink::Muted);
        }
    }
    rect(d, [x - 20., 71.], [40., 16.], Ink::Face);
    d.label([x, 79.], format!("{value:.0}"), 9., Ink::Text);
    d.label([x, 22.], unit, 7., Ink::Muted);
}
fn red_x(d: &mut DrawList, a: [f32; 2], b: [f32; 2]) {
    d.line(a, b, Ink::Alert, 3.);
    d.line([a[0], b[1]], [b[0], a[1]], Ink::Alert, 3.);
}
