//! The primary flight display of the landscape touch EFIS, placed by the display model.
//!
//! Every element's rectangle comes from `flightcraft_aero::display_model::PfdGeometry` (measured
//! from the modelled unit's pilot's guide figures); colours are by meaning (cyan for selected
//! values, magenta for guidance, green for active, white for scales and armed modes, yellow for
//! cautions, red for warnings) and may be replaced by a skin pack's instrument palette; type sizes
//! may be scaled by a skin pack's typography. Draw order: backdrop (sky, ground, synthetic
//! vision), then `panel`, then `bars` over the whole screen.
use super::*;
use flightcraft_aero::display_model::{PfdElement as E, PfdGeometry};
use flightcraft_control::touch_pfd::{self as logic, AfcsCell, AfcsTone};

const KNOTS: f64 = 1.943_844_492_440_6;
const FEET: f64 = 1. / 0.3048;
/// Native pixels per backdrop pixel: the synthetic-vision camera works at 2.5 pixels per degree,
/// so this sets the pitch scale (7.25 native pixels per degree).
pub(crate) const BACKDROP_SCALE: f32 = 2.9;
/// Native pixels per degree of pitch on the pitch scale.
const PITCH_PX_PER_DEG: f32 = 2.5 * BACKDROP_SCALE;
/// Airspeed tape scale, native pixels per knot, and altitude tape scale, per foot.
const SPEED_PX_PER_KT: f32 = 4.2;
const ALT_PX_PER_FT: f32 = 0.43;

/// Type size multipliers from a skin pack's typography.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Typography {
    pub tape: f32,
    pub boxes: f32,
    pub data_bar: f32,
    pub annunciation: f32,
}
impl Default for Typography {
    fn default() -> Self {
        Self {
            tape: 1.,
            boxes: 1.,
            data_bar: 1.,
            annunciation: 1.,
        }
    }
}

/// What the panel needs besides the snapshot.
#[derive(Clone, Copy)]
pub(crate) struct Options {
    pub typography: Typography,
    /// A lost display: no flight director, as in the unit's reversionary mode.
    pub reversion: bool,
}

fn r4(r: [f64; 4]) -> [f32; 4] {
    r.map(|v| v as f32)
}

/// A filled rectangle with a one-pixel border.
fn bordered(d: &mut DrawList, [x, y, w, h]: [f32; 4], fill: Ink, border: Ink) {
    rect(d, [x, y, w, h], fill);
    for (a, b) in [
        ([x, y], [x + w, y]),
        ([x + w, y], [x + w, y + h]),
        ([x + w, y + h], [x, y + h]),
        ([x, y + h], [x, y]),
    ] {
        line(d, a, b, border, 1.5);
    }
}

/// Text left-aligned at `x`: labels are centred, so place the centre half a text width along.
fn left_text(d: &mut DrawList, x: f32, y: f32, text: &str, size: f32, ink: Ink) {
    label(
        d,
        [x + text.chars().count() as f32 * size * 0.3, y],
        text,
        size,
        ink,
    );
}

fn cell_ink(cell: &AfcsCell) -> Ink {
    match cell.tone {
        AfcsTone::Active => Ink::Green,
        AfcsTone::Armed => Ink::White,
        AfcsTone::Caution { visible: true } => Ink::Yellow,
        AfcsTone::Caution { visible: false } => Ink::BoxFill,
    }
}

/// Sky, ground, and synthetic vision under the panel, optical centre at the aircraft symbol.
pub(crate) fn backdrop(s: &GlassState, input: InstrumentInput<'_>, g: &PfdGeometry) -> DrawList {
    let [_, _, pw, ph] = g.panel.map(|v| v as f32);
    let size = [pw / BACKDROP_SCALE, ph / BACKDROP_SCALE];
    let center = [
        g.aircraft_symbol[0] as f32 / BACKDROP_SCALE,
        g.aircraft_symbol[1] as f32 / BACKDROP_SCALE,
    ];
    let mut d = canvas();
    d.size = size;
    pfd::backdrop(&mut d, s, input, size, center);
    d
}

/// The instruments over the backdrop, in panel pixels.
pub(crate) fn panel(s: &GlassState, input: InstrumentInput<'_>, g: &PfdGeometry, o: Options) -> DrawList {
    let [_, _, pw, ph] = g.panel.map(|v| v as f32);
    let mut d = canvas();
    d.size = [pw, ph];
    let a = &s.air_data;
    let t = o.typography;
    let [cx, cy] = g.aircraft_symbol.map(|v| v as f32);
    if a.attitude_valid {
        attitude(&mut d, input, g, a, o);
    } else {
        let aligning = a.alignment_remaining_s > 0. && a.powered && !a.ahrs_failed;
        flag(
            &mut d,
            [cx - 150., cy - 60., 300., 150.],
            if aligning { "ALIGNING" } else { "AHRS FAIL" },
        );
    }
    afcs_box(&mut d, g, input, s, t);
    if a.air_data_valid {
        airspeed(&mut d, s, input, g, t);
        altitude(&mut d, s, g, t);
    } else {
        for e in [E::AirspeedTape, E::AltitudeTape] {
            let [x, y, w, h] = r4(g.rect(e));
            flag(&mut d, [x, y, w, h], "ADC");
        }
    }
    hsi(&mut d, s, g, t);
    boxes(&mut d, s, input, g, t);
    trim_and_flaps(&mut d, input, g);
    cas_window(&mut d, input, g, t);
    d
}

/// The elevator trim and flap position indicators (pilot's guide page 81): vertical scales left
/// of the aircraft symbol, flaps (0 at the top, full at the bottom) beside trim (DN at the top,
/// UP at the bottom), each with a white pointer. They need the pilot's controls, so a host that
/// supplies none draws neither.
fn trim_and_flaps(d: &mut DrawList, input: InstrumentInput<'_>, g: &PfdGeometry) {
    let Some(pilot) = input.pilot else {
        return;
    };
    let full = input
        .aircraft
        .and_then(|a| a.variant.flap_stages.last())
        .and_then(|stage| stage.deflection_deg);
    let flap = flightcraft_control::touch_cas::flap_gauge(pilot.flaps, full);
    let trim = flightcraft_control::touch_cas::trim_gauge(input.systems.controls.trim);
    for (element, top, bottom, fraction, pointer_left, neutral) in [
        (
            E::FlapIndicator,
            "0",
            flap.full_label.as_str(),
            flap.fraction,
            true,
            false,
        ),
        (
            E::TrimIndicator,
            "DN",
            "UP",
            (trim.nose_up + 1.) / 2.,
            false,
            true,
        ),
    ] {
        let [x, y, w, h] = r4(g.rect(element));
        if w <= 0. {
            continue;
        }
        bordered(d, [x, y, w, h], Ink::BoxFill, Ink::BoxBorder);
        let size = (w * 0.5).clamp(9., 14.);
        label(d, [x + w * 0.5, y + size * 0.8], top, size, Ink::White);
        label(d, [x + w * 0.5, y + h - size * 0.8], bottom, size, Ink::White);
        // The scale runs between the two labels.
        let (top_y, bottom_y) = (y + size * 1.7, y + h - size * 1.7);
        let axis = x + w * 0.5;
        line(d, [axis, top_y], [axis, bottom_y], Ink::White, 1.5);
        for i in 0..=4 {
            let ty = top_y + (bottom_y - top_y) * i as f32 / 4.;
            line(d, [axis - 4., ty], [axis + 4., ty], Ink::White, 1.);
        }
        if neutral {
            // The neutral mark of the trim scale.
            let mid = (top_y + bottom_y) * 0.5;
            line(d, [axis - 7., mid], [axis + 7., mid], Ink::Green, 3.);
        }
        let py = top_y + (bottom_y - top_y) * fraction as f32;
        let tip = if pointer_left { axis - 2. } else { axis + 2. };
        let base = if pointer_left { x + 2. } else { x + w - 2. };
        polygon(d, vec![[tip, py], [base, py - 6.], [base, py + 6.]], Ink::White);
    }
}

/// The CAS window: the active messages, warnings red and cautions yellow, flashing until
/// acknowledged (pilot's guide 3.7). Nothing is drawn when no message is active.
fn cas_window(d: &mut DrawList, input: InstrumentInput<'_>, g: &PfdGeometry, t: Typography) {
    use flightcraft_control::touch_cas::{CasSeverity, window};
    let failures = input.autopilot.map_or(&[][..], |a| a.failures.as_slice());
    let cas = window(input.systems.modern.as_ref(), failures);
    if cas.messages.is_empty() {
        return;
    }
    let [x, y, w, h] = r4(g.rect(E::CasWindow));
    let worst = cas.messages.first().map(|m| m.severity);
    bordered(
        d,
        [x, y, w, h],
        Ink::BoxFill,
        if worst == Some(CasSeverity::Warning) {
            Ink::Red
        } else {
            Ink::BoxBorder
        },
    );
    // Unacknowledged text flashes, twice a second.
    if cas.unacknowledged && (input.time_s * 2.).floor() as i64 % 2 != 0 {
        return;
    }
    // Shrink the type so the longest visible message fits the window.
    let longest = cas
        .visible()
        .iter()
        .map(|m| m.text.chars().count())
        .max()
        .unwrap_or(1) as f32;
    let size = (15. * t.annunciation).min((w - 8.) / (longest * 0.62));
    let lines = cas.visible().len() as f32;
    let row = (h / flightcraft_control::touch_cas::CAS_WINDOW_LINES as f32).min(size * 1.25);
    let top = y + (h - row * lines) * 0.5 + row * 0.5;
    for (i, m) in cas.visible().iter().enumerate() {
        let ink = match m.severity {
            CasSeverity::Warning => Ink::Red,
            CasSeverity::Caution => Ink::Yellow,
            CasSeverity::Advisory => Ink::White,
        };
        label(d, [x + w * 0.5, top + row * i as f32], &m.text, size, ink);
    }
    if cas.overflow() {
        // More messages than the window holds: a down-pointing triangle, as the guide shows.
        polygon(
            d,
            vec![
                [x + w - 14., y + h - 11.],
                [x + w - 4., y + h - 11.],
                [x + w - 9., y + h - 4.],
            ],
            Ink::White,
        );
    }
}

fn attitude(
    d: &mut DrawList,
    input: InstrumentInput<'_>,
    g: &PfdGeometry,
    a: &flightcraft_sim::avionics::sensor::AirDataComputer,
    o: Options,
) {
    let [cx, cy] = g.aircraft_symbol.map(|v| v as f32);
    let radius = g.roll_arc_radius as f32;
    let center = [cx, cy];
    // Pitch scale: marks every 2.5 degrees through all ranges, numbered at multiples of 10.
    let horizon = |x: f32, y: f32| rotated([x, y], center, -a.roll_rad);
    for step in -12_i32..=12 {
        let deg = step as f32 * 2.5;
        if step == 0 {
            continue;
        }
        // A mark's offset from the aircraft symbol along the pitch axis.
        let y_axis =
            flightcraft_render::pfd_pitch_y(a.pitch_rad - f64::from(deg).to_radians()) * BACKDROP_SCALE;
        let (half, number) = if step % 4 == 0 {
            (46., true)
        } else if step % 2 == 0 {
            (30., false)
        } else {
            (16., false)
        };
        let p = horizon(-half, y_axis);
        let q = horizon(half, y_axis);
        let inside = |[x, y]: [f32; 2]| (y > cy - radius * 0.62 && y < cy + 88.) && (x - cx).abs() < 170.;
        if inside(p) && inside(q) {
            line(d, p, q, Ink::White, 2.);
            if number {
                let n = format!("{}", (deg.abs() as i32));
                label(
                    d,
                    horizon(-half - 18., y_axis),
                    n.clone(),
                    18. * o.typography.boxes,
                    Ink::White,
                );
                label(
                    d,
                    horizon(half + 18., y_axis),
                    n,
                    18. * o.typography.boxes,
                    Ink::White,
                );
            }
        }
    }
    // Horizon line, part of the pitch scale.
    let hy = flightcraft_render::pfd_pitch_y(a.pitch_rad) * BACKDROP_SCALE;
    line(d, horizon(-2000., hy), horizon(2000., hy), Ink::White, 2.);
    // Roll scale: 60 degrees either side, major ticks at 30 and 60, minor at 10, 20, and 45.
    let arc: Vec<[f32; 2]> = (-60..=60)
        .step_by(3)
        .map(|deg| {
            let r = f64::from(deg).to_radians();
            [
                cx + radius * libm::sin(r) as f32,
                cy - radius * libm::cos(r) as f32,
            ]
        })
        .collect();
    for w in arc.windows(2) {
        if let [p, q] = w {
            line(d, *p, *q, Ink::White, 2.);
        }
    }
    for (deg, len) in [(10., 12.), (20., 12.), (30., 22.), (45., 12.), (60., 22.)] {
        for sign in [-1., 1.] {
            let r = f64::to_radians(deg * sign);
            let (sn, cs) = (libm::sin(r) as f32, libm::cos(r) as f32);
            line(
                d,
                [cx + radius * sn, cy - radius * cs],
                [cx + (radius + len) * sn, cy - (radius + len) * cs],
                Ink::White,
                2.,
            );
        }
    }
    // Roll scale zero: an inverted white triangle outside the arc; the roll pointer rides inside.
    polygon(
        d,
        vec![
            [cx - 9., cy - radius - 20.],
            [cx + 9., cy - radius - 20.],
            [cx, cy - radius - 5.],
        ],
        Ink::White,
    );
    let p = |x: f32, y: f32| rotated([x, y], center, -a.roll_rad);
    polygon(
        d,
        vec![p(0., -radius + 3.), p(-9., -radius + 20.), p(9., -radius + 20.)],
        Ink::White,
    );
    // Aircraft symbol, yellow.
    for sign in [-1_f32, 1.] {
        line(d, [cx + sign * 190., cy], [cx + sign * 150., cy], Ink::Yellow, 7.);
        line(
            d,
            [cx + sign * 104., cy + 12.],
            [cx + sign * 10., cy + 12.],
            Ink::Yellow,
            7.,
        );
        line(d, [cx + sign * 10., cy + 12.], [cx, cy], Ink::Yellow, 7.);
    }
    circle(d, [cx, cy], 4., Ink::Yellow);
    // Flight director command bars: magenta, hollow while hand flying, solid with the autopilot on.
    if logic::command_bars_available(o.reversion)
        && let Some(ap) = input.autopilot
        && ap.flight_director
    {
        command_bars(d, ap, center, a);
    }
    // Slip/skid ball.
    let [x, y, w, h] = r4(g.rect(E::SlipBox));
    bordered(d, [x, y, w, h], Ink::TapeFill, Ink::BoxBorder);
    for off in [-0.16_f32, 0.16] {
        line(
            d,
            [x + w * (0.5 + off), y + 3.],
            [x + w * (0.5 + off), y + h - 3.],
            Ink::White,
            1.5,
        );
    }
    let ball = x + w * 0.5 - a.slip_g.clamp(-0.5, 0.5) as f32 * w * 0.8;
    circle(d, [ball, y + h * 0.5], h * 0.38, Ink::White);
    // Standard-rate turn scale: half-standard and standard marks, and the rate marker.
    let [x, y, w, h] = r4(g.rect(E::TurnRateScale));
    let mid = x + w * 0.5;
    for k in [-2_f32, -1., 1., 2.] {
        circle(d, [mid + k * w * 0.2, y + h * 0.5], 4., Ink::White);
    }
    let rate = (input.systems.sensed.turn_rate_rps.to_degrees() / 3.).clamp(-1.25, 1.25) as f32;
    polygon(
        d,
        vec![
            [mid + rate * w * 0.4, y + h * 0.15],
            [mid + rate * w * 0.4 - 8., y + h * 0.85],
            [mid + rate * w * 0.4 + 8., y + h * 0.85],
        ],
        Ink::Magenta,
    );
}

/// Single-cue command bars: a magenta chevron that moves with the pitch command and banks with the
/// roll command. Hollow when the pilot flies, solid when the autopilot does.
fn command_bars(
    d: &mut DrawList,
    ap: &flightcraft_sim::equipment::EquipmentState,
    [cx, cy]: [f32; 2],
    a: &flightcraft_sim::avionics::sensor::AirDataComputer,
) {
    let roll_error = (ap.command_bank_rad - a.roll_rad).clamp(-0.45, 0.45);
    let pitch_error_deg = (ap.command_pitch_rad - a.pitch_rad).to_degrees().clamp(-8., 8.) as f32;
    let at = [cx, cy - pitch_error_deg * PITCH_PX_PER_DEG];
    let point = |x: f32, y: f32| rotated([x, y], at, -roll_error);
    let solid = ap.engaged;
    for sign in [-1_f32, 1.] {
        let (tip, tail, thick) = ([0., 0.], [sign * 96., 18.], 7.);
        if solid {
            polygon(
                d,
                vec![
                    point(tip[0], tip[1] - thick),
                    point(tail[0], tail[1] - thick),
                    point(tail[0], tail[1] + thick),
                    point(tip[0], tip[1] + thick),
                ],
                Ink::Magenta,
            );
        } else {
            let corners = [
                point(tip[0], tip[1] - thick),
                point(tail[0], tail[1] - thick),
                point(tail[0], tail[1] + thick),
                point(tip[0], tip[1] + thick),
            ];
            for (a, b) in corners.iter().zip(corners.iter().cycle().skip(1)) {
                line(d, *a, *b, Ink::Magenta, 2.);
            }
        }
    }
}

fn afcs_box(d: &mut DrawList, g: &PfdGeometry, input: InstrumentInput<'_>, s: &GlassState, t: Typography) {
    let [x, y, w, h] = r4(g.rect(E::AfcsBox));
    bordered(d, [x, y, w, h], Ink::BoxFill, Ink::BoxBorder);
    let size = 19. * t.annunciation;
    let Some(ap) = input.autopilot else {
        return;
    };
    let b = logic::afcs_box(ap, &s.gps.source);
    // Autopilot and yaw damper at the far left, lateral modes centre, vertical modes right.
    let (left, right) = (x + w * 0.24, x + w * 0.64);
    line(d, [left, y + 4.], [left, y + h - 4.], Ink::BoxBorder, 1.);
    line(d, [right, y + 4.], [right, y + h - 4.], Ink::BoxBorder, 1.);
    let mid = y + h * 0.5;
    let put = |d: &mut DrawList, cell: &Option<AfcsCell>, at: [f32; 2]| {
        if let Some(cell) = cell {
            label(d, at, cell.text.clone(), size, cell_ink(cell));
        }
    };
    put(d, &b.autopilot, [x + w * 0.07, mid]);
    put(d, &b.yaw_damper, [x + w * 0.17, mid]);
    put(d, &b.lateral_active, [x + w * 0.44, mid]);
    if let Some(armed) = &b.lateral_armed {
        label(d, [x + w * 0.33, mid], armed.text.clone(), size * 0.8, Ink::White);
    }
    put(d, &b.vertical_active, [x + w * 0.70, mid]);
    if let Some(r) = &b.reference {
        label(d, [x + w * 0.86, mid], r.clone(), size * 0.8, Ink::Green);
    }
    if let Some(armed) = &b.vertical_armed {
        label(
            d,
            [x + w * 0.96 - 4., mid],
            armed.text.clone(),
            size * 0.7,
            Ink::White,
        );
    }
}

/// Airspeed tape, speed range strip, trend vector, and speed references.
fn airspeed(d: &mut DrawList, s: &GlassState, input: InstrumentInput<'_>, g: &PfdGeometry, t: Typography) {
    let a = &s.air_data;
    let [x, y, w, h] = r4(g.rect(E::AirspeedTape));
    bordered(d, [x, y, w, h], Ink::TapeFill, Ink::BoxBorder);
    let ias = a.indicated_airspeed_mps * KNOTS;
    let mid = y + h * 0.5;
    let at = |kt: f64| mid - (kt - ias) as f32 * SPEED_PX_PER_KT;
    for tick in (30..=((ias + 40.) as i32 / 5 * 5 + 5)).step_by(5) {
        let v = f64::from(tick);
        let ty = at(v);
        if ty < y + 4. || ty > y + h - 4. {
            continue;
        }
        let major = tick % 10 == 0;
        line(
            d,
            [x + w - 20., ty],
            [x + w - if major { 8. } else { 14. }, ty],
            Ink::TapeText,
            2.,
        );
        if major && (ty - mid).abs() > 26. {
            label(
                d,
                [x + w * 0.38, ty],
                format!("{tick}"),
                28. * t.tape,
                Ink::TapeText,
            );
        }
    }
    // The speed range strip down the right edge of the tape.
    if let Some(aircraft) = input.aircraft {
        let v = &aircraft.variant.v_speeds;
        let band = |low: f64, high: f64, ink: Ink, d: &mut DrawList| {
            let top = at(high * KNOTS).clamp(y + 2., y + h - 2.);
            let bottom = at(low * KNOTS).clamp(y + 2., y + h - 2.);
            if bottom > top {
                rect(d, [x + w - 9., top, 7., bottom - top], ink);
            }
        };
        // Low-speed awareness (red), flap range (white), normal (green), caution (yellow), never exceed (red).
        band(0., v.vso_mps, Ink::Red, d);
        band(v.vso_mps, v.vfe_mps, Ink::White, d);
        band(v.vs_mps.max(v.vfe_mps), v.vno_mps, Ink::Green, d);
        band(v.vno_mps, v.vne_mps, Ink::Yellow, d);
        band(v.vne_mps, v.vne_mps + 30., Ink::Red, d);
        // Speed references right of the tape: R, X, Y, and the never-exceed mark.
        for (value, text, ink) in [
            (v.vr_mps, "R", Ink::Cyan),
            (v.vx_mps, "X", Ink::Cyan),
            (v.vy_mps, "Y", Ink::Cyan),
        ] {
            let ry = at(value * KNOTS);
            if ry > y + 6. && ry < y + h - 6. {
                label(d, [x + w + 14., ry], text, 22. * t.tape, ink);
            }
        }
    }
    // Trend vector: vertical magenta line beside the strip, to the predicted speed in 6 seconds.
    let trend = (a.airspeed_trend_mps * KNOTS) as f32 * SPEED_PX_PER_KT;
    if trend.abs() > 1. {
        let ty = (mid - trend).clamp(y + 4., y + h - 4.);
        line(d, [x + w + 3., mid], [x + w + 3., ty], Ink::Magenta, 4.);
    }
    // The pointer: black, with the airspeed; red above the never-exceed speed.
    let [px, py, pw, ph] = r4(g.rect(E::AirspeedPointer));
    let over = input
        .aircraft
        .is_some_and(|ac| a.indicated_airspeed_mps > ac.variant.v_speeds.vne_mps);
    bordered(
        d,
        [px, py, pw, ph],
        Ink::BoxFill,
        if over { Ink::Red } else { Ink::BoxBorder },
    );
    polygon(
        d,
        vec![
            [px + pw, py + ph * 0.5 - 7.],
            [px + pw + 12., py + ph * 0.5],
            [px + pw, py + ph * 0.5 + 7.],
        ],
        if over { Ink::Red } else { Ink::BoxFill },
    );
    label(
        d,
        [px + pw * 0.5, py + ph * 0.5],
        format!("{:.0}", ias.max(0.)),
        34. * t.tape,
        if over { Ink::Red } else { Ink::White },
    );
}

fn altitude(d: &mut DrawList, s: &GlassState, g: &PfdGeometry, t: Typography) {
    let a = &s.air_data;
    let [x, y, w, h] = r4(g.rect(E::AltitudeTape));
    bordered(d, [x, y, w, h], Ink::TapeFill, Ink::BoxBorder);
    let alt = a.altitude_msl_m * FEET;
    let mid = y + h * 0.5;
    let at = |ft: f64| mid - (ft - alt) as f32 * ALT_PX_PER_FT;
    let base = (alt / 20.).floor() as i64;
    for i in (base - 16)..=(base + 16) {
        let ft = (i * 20) as f64;
        let ty = at(ft);
        if ty < y + 3. || ty > y + h - 3. {
            continue;
        }
        let major = i % 5 == 0;
        line(
            d,
            [x + 2., ty],
            [x + if major { 16. } else { 9. }, ty],
            Ink::TapeText,
            2.,
        );
        if major && (ty - mid).abs() > 26. {
            label(
                d,
                [x + w * 0.58, ty],
                crate::instruments::formatting::integer(ft),
                26. * t.tape,
                Ink::TapeText,
            );
        }
    }
    // Selected altitude bug on the tape's left edge, pinned to the nearest edge when off the scale.
    let selected = s.settings.selected_altitude_m * FEET;
    let by = at(selected).clamp(y + 8., y + h - 8.);
    polygon(
        d,
        vec![
            [x, by - 11.],
            [x + 17., by - 11.],
            [x + 17., by + 11.],
            [x, by + 11.],
        ],
        Ink::Cyan,
    );
    polygon(
        d,
        vec![[x + 17., by - 11.], [x + 28., by], [x + 17., by + 11.]],
        Ink::Cyan,
    );
    // Vertical navigation indicator: a scale of dots (100 ft each) beside the tape's left edge and
    // a magenta diamond that sits below the middle when the aircraft is above the path, so the
    // pilot flies toward it. Green once the autopilot is tracking the path.
    if let Some(v) = &s.vnav {
        let vx = x - 14.;
        let spacing = h * 0.5 / (flightcraft_sim::avionics::vnav::FULL_SCALE_DOTS as f32 + 0.5);
        line(
            d,
            [vx, mid - spacing * 2.5],
            [vx, mid + spacing * 2.5],
            Ink::TapeText,
            1.,
        );
        for dot in [-2_f32, -1., 1., 2.] {
            circle(d, [vx, mid + dot * spacing], 3.5, Ink::White);
        }
        line(d, [vx - 8., mid], [vx + 8., mid], Ink::White, 2.);
        let dy = mid + v.dots as f32 * spacing;
        let ink = if v.tracking { Ink::Green } else { Ink::Magenta };
        polygon(
            d,
            vec![[vx - 9., dy], [vx, dy - 11.], [vx + 9., dy], [vx, dy + 11.]],
            ink,
        );
        label(d, [vx, y - 9.], "VNAV", 15. * t.tape, ink);
    }
    // Barometric minimums bug: an amber marker on the tape's left edge, pinned when off the scale.
    if let Some(minimums) = s.settings.minimums_m {
        let my = at(minimums * FEET).clamp(y + 8., y + h - 8.);
        polygon(d, vec![[x, my - 7.], [x + 14., my], [x, my + 7.]], Ink::Yellow);
        line(d, [x, my], [x + 22., my], Ink::Yellow, 2.);
    }
    // Altitude trend vector, to the predicted altitude in 6 seconds.
    let trend = (a.altitude_trend_m * FEET) as f32 * ALT_PX_PER_FT;
    if trend.abs() > 1. {
        let ty = (mid - trend).clamp(y + 4., y + h - 4.);
        line(d, [x - 3., mid], [x - 3., ty], Ink::Magenta, 4.);
    }
    // Vertical speed scale: compressed above 1,000 fpm, with the pointer and a digital readout.
    let [vx, vy, vw, vh] = r4(g.rect(E::VerticalSpeedScale));
    bordered(d, [vx, vy, vw, vh], Ink::TapeFill, Ink::BoxBorder);
    let vmid = vy + vh * 0.5;
    let half = vh * 0.5 - 8.;
    let scale = |fpm: f64| -> f32 {
        let f = fpm.abs();
        let u = if f <= 1000. {
            f / 1000. * 0.58
        } else {
            0.58 + ((f - 1000.) / 1000.).min(1.) * 0.42
        };
        (u as f32) * half * fpm.signum() as f32
    };
    for (fpm, text) in [(2000., "2"), (1000., "1"), (500., ".5")] {
        for sign in [-1_f64, 1.] {
            let ty = vmid - scale(fpm * sign);
            line(d, [vx + 2., ty], [vx + 12., ty], Ink::TapeText, 2.);
            label(d, [vx + vw * 0.68, ty], text, 20. * t.tape, Ink::TapeText);
        }
    }
    line(d, [vx + 2., vmid], [vx + 14., vmid], Ink::TapeText, 2.);
    let fpm = a.vertical_speed_mps * FEET * 60.;
    let py = vmid - scale(fpm.clamp(-2000., 2000.));
    polygon(
        d,
        vec![
            [vx + vw - 2., py],
            [vx + vw - 16., py - 8.],
            [vx + vw - 16., py + 8.],
        ],
        Ink::White,
    );
    if fpm.abs() >= 100. {
        let ry = py.clamp(vy + 14., vy + vh - 14.);
        bordered(
            d,
            [vx + vw + 4., ry - 12., 66., 24.],
            Ink::BoxFill,
            Ink::BoxBorder,
        );
        label(
            d,
            [vx + vw + 37., ry],
            format!("{:+.0}", (fpm / 10.).round() * 10.),
            20. * t.boxes,
            Ink::Green,
        );
    }
    // MINIMUMS: amber text within 100 ft above the minimums, and solid amber at or below them.
    match flightcraft_control::minimums::state(a.altitude_msl_m, s.settings.minimums_m) {
        flightcraft_control::minimums::MinimumsState::Approaching => {
            bordered(d, [x + 3., y + 3., w - 6., 24.], Ink::BoxFill, Ink::Yellow);
            label(d, [x + w * 0.5, y + 15.], "MINIMUMS", 18. * t.boxes, Ink::Yellow);
        }
        flightcraft_control::minimums::MinimumsState::Reached => {
            rect(d, [x + 3., y + 3., w - 6., 24.], Ink::Yellow);
            label(d, [x + w * 0.5, y + 15.], "MINIMUMS", 18. * t.boxes, Ink::BoxFill);
        }
        _ => {}
    }
    // The pointer: black, with the altitude; its tens digits roll.
    let [px, py, pw, ph] = r4(g.rect(E::AltitudePointer));
    bordered(d, [px, py, pw, ph], Ink::BoxFill, Ink::BoxBorder);
    polygon(
        d,
        vec![
            [px, py + ph * 0.5 - 8.],
            [px - 14., py + ph * 0.5],
            [px, py + ph * 0.5 + 8.],
        ],
        Ink::BoxFill,
    );
    label(
        d,
        [px + pw * 0.5, py + ph * 0.5],
        crate::instruments::formatting::integer((alt / 20.).round() * 20.),
        34. * t.tape,
        Ink::White,
    );
}

/// The boxes around the tapes and the compass: true airspeed, ground speed, selected altitude,
/// baro, selected heading, selected course, current heading, wind.
fn boxes(d: &mut DrawList, s: &GlassState, input: InstrumentInput<'_>, g: &PfdGeometry, t: Typography) {
    let a = &s.air_data;
    let size = 21. * t.boxes;
    let small = 15. * t.boxes;
    let boxed = |d: &mut DrawList, e: E, text: String, ink: Ink, size: f32| {
        let r = r4(g.rect(e));
        bordered(d, r, Ink::BoxFill, Ink::BoxBorder);
        label(d, [r[0] + r[2] * 0.5, r[1] + r[3] * 0.5], text, size, ink);
    };
    let [x, y, w, h] = r4(g.rect(E::TrueAirspeedBox));
    bordered(d, [x, y, w, h], Ink::BoxFill, Ink::BoxBorder);
    left_text(d, x + 5., y + h * 0.5, "TAS", small, Ink::White);
    label(
        d,
        [x + w * 0.66, y + h * 0.5],
        if a.air_data_valid {
            format!("{:.0}KT", a.true_airspeed_mps * KNOTS)
        } else {
            "---".into()
        },
        size,
        Ink::White,
    );
    let [x, y, w, h] = r4(g.rect(E::GroundSpeedBox));
    bordered(d, [x, y, w, h], Ink::BoxFill, Ink::BoxBorder);
    left_text(d, x + 5., y + h * 0.5, "GS", small, Ink::White);
    label(
        d,
        [x + w * 0.66, y + h * 0.5],
        format!("{:.0}KT", s.ground_speed_mps * KNOTS),
        size,
        Ink::Magenta,
    );
    // Selected altitude with its bug icon, and the baro setting.
    let r = r4(g.rect(E::SelectedAltitudeBox));
    bordered(d, r, Ink::BoxFill, Ink::BoxBorder);
    polygon(
        d,
        vec![
            [r[0] + 4., r[1] + 6.],
            [r[0] + 14., r[1] + 6.],
            [r[0] + 18., r[1] + r[3] * 0.5],
            [r[0] + 14., r[1] + r[3] - 6.],
            [r[0] + 4., r[1] + r[3] - 6.],
        ],
        Ink::Cyan,
    );
    label(
        d,
        [r[0] + r[2] * 0.6, r[1] + r[3] * 0.5],
        format!(
            "{}FT",
            crate::instruments::formatting::integer((s.settings.selected_altitude_m * FEET).round())
        ),
        size,
        Ink::Cyan,
    );
    boxed(
        d,
        E::BaroBox,
        if s.settings.pressure_hpa {
            format!("{:.0}HPA", s.baro_pa / 100.)
        } else {
            format!("{:.2}IN", s.baro_pa / 3386.389)
        },
        Ink::Cyan,
        size,
    );
    // The minimums box under the baro box: the set altitude, or dashes.
    boxed(
        d,
        E::MinimumsBox,
        s.settings.minimums_m.map_or("MIN ----".into(), |m| {
            format!(
                "MIN {}",
                crate::instruments::formatting::integer((m * FEET).round())
            )
        }),
        if s.settings.minimums_m.is_some() {
            Ink::Yellow
        } else {
            Ink::White
        },
        15. * t.boxes,
    );
    // Selected heading (cyan) and selected course (magenta for GPS, green for a radio).
    boxed(
        d,
        E::SelectedHeadingBox,
        format!(
            "HDG {}°",
            crate::instruments::heading::degrees_text(s.settings.heading_bug_rad)
        ),
        Ink::Cyan,
        size,
    );
    let course = if s.gps.valid && !s.settings.course_manual {
        s.gps.desired_track_rad
    } else {
        s.settings.course_rad
    };
    boxed(
        d,
        E::SelectedCourseBox,
        format!("CRS {}°", crate::instruments::heading::degrees_text(course)),
        nav_ink(s),
        size,
    );
    // Current heading.
    boxed(
        d,
        E::HeadingBox,
        if a.attitude_valid {
            format!("{}°", crate::instruments::heading::degrees_text(a.heading_rad))
        } else {
            "---".into()
        },
        Ink::White,
        26. * t.boxes,
    );
    // Wind: an arrow and speed over direction, relative to the aircraft's heading.
    let [x, y, w, h] = r4(g.rect(E::WindBox));
    bordered(d, [x, y, w, h], Ink::BoxFill, Ink::BoxBorder);
    let speed = s.wind_speed_mps * KNOTS;
    if speed >= 0.5 && a.attitude_valid {
        let angle = s.wind_from_rad - a.heading_rad + std::f64::consts::PI;
        let c = [x + 22., y + h * 0.5];
        let tip = rotated([0., -14.], c, angle);
        let tail = rotated([0., 14.], c, angle);
        line(d, tail, tip, Ink::White, 3.);
        polygon(
            d,
            vec![
                rotated([0., -17.], c, angle),
                rotated([-7., -6.], c, angle),
                rotated([7., -6.], c, angle),
            ],
            Ink::White,
        );
    } else {
        label(d, [x + 22., y + h * 0.5], "-", size, Ink::White);
    }
    label(
        d,
        [x + w * 0.68, y + h * 0.3],
        format!("{speed:.0}KT"),
        18. * t.boxes,
        Ink::White,
    );
    label(
        d,
        [x + w * 0.68, y + h * 0.72],
        format!("{:03.0}°M", s.wind_from_rad.to_degrees().rem_euclid(360.)),
        18. * t.boxes,
        Ink::White,
    );
    let _ = input;
}

/// Colour of the selected navigation source: magenta for GPS, green for a radio.
fn nav_ink(s: &GlassState) -> Ink {
    if s.gps.source == "GPS" {
        Ink::Magenta
    } else {
        Ink::Green
    }
}

/// The compass rose, heading-up, with track, selected heading, course, and deviation.
fn hsi(d: &mut DrawList, s: &GlassState, g: &PfdGeometry, t: Typography) {
    let a = &s.air_data;
    let [cx, cy] = g.hsi_center.map(|v| v as f32);
    let r = g.hsi_radius as f32;
    if !a.attitude_valid {
        flag(d, [cx - r, cy - r, r * 2., r * 2.], "HDG");
        return;
    }
    circle(d, [cx, cy], r + 4., Ink::Overlay);
    let heading = a.heading_rad;
    let rim = |deg: f64, radius: f32| -> [f32; 2] {
        let angle = deg.to_radians() - heading;
        [
            cx + libm::sin(angle) as f32 * radius,
            cy - libm::cos(angle) as f32 * radius,
        ]
    };
    // Major ticks every 10 degrees, minor every 5; numbers every 30 and cardinal letters.
    for deg in (0..360).step_by(5) {
        let major = deg % 10 == 0;
        line(
            d,
            rim(f64::from(deg), r - if major { 14. } else { 8. }),
            rim(f64::from(deg), r),
            Ink::White,
            if major { 2. } else { 1.5 },
        );
        if deg % 30 == 0 {
            let text = match deg {
                0 => "N".to_owned(),
                90 => "E".to_owned(),
                180 => "S".to_owned(),
                270 => "W".to_owned(),
                _ => format!("{}", deg / 10),
            };
            label(d, rim(f64::from(deg), r - 30.), text, 26. * t.boxes, Ink::White);
        }
    }
    // Selected heading bug (cyan) on the rim.
    let bug = s.settings.heading_bug_rad.to_degrees();
    polygon(
        d,
        vec![
            rim(bug - 3., r + 1.),
            rim(bug + 3., r + 1.),
            rim(bug + 3., r - 9.),
            rim(bug, r - 15.),
            rim(bug - 3., r - 9.),
        ],
        Ink::Cyan,
    );
    // Current track: a magenta triangle on the rim.
    let track = s.ground_track_rad.to_degrees();
    polygon(
        d,
        vec![
            rim(track - 2.5, r + 1.),
            rim(track + 2.5, r + 1.),
            rim(track, r - 14.),
        ],
        Ink::Magenta,
    );
    // Course pointer and deviation bar, in the colour of the navigation source.
    let ink = nav_ink(s);
    let course = if s.gps.valid && !s.settings.course_manual {
        s.gps.desired_track_rad
    } else {
        s.settings.course_rad
    };
    let rotate = course - heading;
    let p = |x: f32, y: f32| rotated([x, y], [cx, cy], rotate);
    line(d, p(0., -r + 14.), p(0., -r * 0.42), ink, 4.);
    line(d, p(0., r * 0.42), p(0., r - 14.), ink, 4.);
    polygon(d, vec![p(0., -r + 12.), p(-9., -r + 30.), p(9., -r + 30.)], ink);
    for f in [-2_f32, -1., 1., 2.] {
        circle(d, p(f * r * 0.26, 0.), 4., Ink::White);
    }
    if s.gps.valid {
        let x = s.gps.cdi_deviation as f32 * r * 0.52;
        line(d, p(x, -r * 0.40), p(x, r * 0.40), ink, 6.);
    }
    // Source and scale annunciations.
    label(
        d,
        [cx - r * 0.5, cy - r * 0.32],
        s.gps.source.clone(),
        17. * t.boxes,
        ink,
    );
    if s.gps.source == "GPS" {
        label(
            d,
            [cx + r * 0.52, cy - r * 0.32],
            format!("{:.2}nm", s.gps.cdi_full_scale_nm),
            15. * t.boxes,
            ink,
        );
    }
    // Aircraft symbol, white.
    line(d, [cx - 22., cy + 2.], [cx + 22., cy + 2.], Ink::White, 5.);
    line(d, [cx, cy - 20.], [cx, cy + 22.], Ink::White, 5.);
    line(d, [cx - 8., cy + 20.], [cx + 8., cy + 20.], Ink::White, 4.);
}

/// A PFD's top data bar and bottom status bar, over the whole screen (`screen` is the model's
/// pixel size). The split and full button is drawn by the touch layer.
pub(crate) fn bars(
    s: &GlassState,
    input: InstrumentInput<'_>,
    g: &PfdGeometry,
    t: Typography,
    knob_hints: [(&str, &str); 2],
) -> DrawList {
    let mut d = canvas();
    d.size = [1280., 768.];
    let [x, y, w, h] = r4(g.rect(E::DataBar));
    rect(&mut d, [x, y, w, h], Ink::BoxFill);
    line(&mut d, [x, y + h], [x + w, y + h], Ink::BoxBorder, 1.5);
    let size = 28. * t.data_bar;
    let small = 15. * t.data_bar;
    // Communication frequencies: active, then standby.
    let com = r4(g.rect(E::ComBox));
    let (active, standby) = if s.comms.com1_powered {
        (
            format!("{:.3}", f64::from(s.comms.com1.active_khz) / 1000.),
            format!("{:.3}", f64::from(s.comms.com1.standby_khz) / 1000.),
        )
    } else {
        ("---.---".to_owned(), "---.---".to_owned())
    };
    let audio = s.touch.navigator.as_ref().is_some_and(|n| n.audio_panel);
    bordered(
        &mut d,
        [com[0] + 2., com[1] + 3., com[2] * 0.5 - 3., com[3] - 6.],
        Ink::BoxFill,
        Ink::BoxBorder,
    );
    label(
        &mut d,
        [com[0] + com[2] * 0.25, com[1] + 13.],
        "COM 1",
        small,
        Ink::White,
    );
    label(
        &mut d,
        [com[0] + com[2] * 0.25, com[1] + com[3] * 0.62],
        active,
        size * 0.88,
        if audio { Ink::Green } else { Ink::White },
    );
    bordered(
        &mut d,
        [
            com[0] + com[2] * 0.5 + 1.,
            com[1] + 3.,
            com[2] * 0.5 - 3.,
            com[3] - 6.,
        ],
        Ink::BoxFill,
        Ink::BoxBorder,
    );
    label(
        &mut d,
        [com[0] + com[2] * 0.75, com[1] + 13.],
        "STBY",
        small,
        Ink::White,
    );
    label(
        &mut d,
        [com[0] + com[2] * 0.75, com[1] + com[3] * 0.62],
        standby,
        size * 0.88,
        Ink::Cyan,
    );
    // The transponder box shows in the split arrangement, where the data fields are fewer.
    let xp = r4(g.rect(E::TransponderBox));
    if xp[2] > 0. {
        bordered(
            &mut d,
            [xp[0], xp[1] + 3., xp[2], xp[3] - 6.],
            Ink::BoxFill,
            Ink::BoxBorder,
        );
        label(
            &mut d,
            [xp[0] + xp[2] * 0.3, xp[1] + xp[3] * 0.52],
            s.settings.transponder_code.clone(),
            size,
            Ink::Green,
        );
        label(
            &mut d,
            [xp[0] + xp[2] * 0.76, xp[1] + xp[3] * 0.52],
            format!("{:?}", s.settings.transponder_mode).to_uppercase(),
            size * 0.6,
            Ink::Green,
        );
    }
    for (field, fx) in &g.data_fields {
        use flightcraft_aero::display_model::DataField as F;
        let gps = &s.gps;
        let duration = |seconds: f64| {
            let minutes = (seconds.max(0.) / 60.) as u64;
            format!("{:02}:{:02}", minutes / 60, minutes % 60)
        };
        // The minimum safe altitude slot is the user field: dashes, the clock, or the flight timer.
        let (label_text, ink) = match (field, s.settings.data_bar_field) {
            (F::SafeAltitude, flightcraft_control::DataBarField::Clock) => {
                (s.clock.label.as_str(), Ink::Magenta)
            }
            (F::SafeAltitude, flightcraft_control::DataBarField::Timer) => (
                "TMR",
                if s.clock.timer_expired {
                    Ink::Yellow
                } else {
                    Ink::Magenta
                },
            ),
            _ => (field.label(), Ink::Magenta),
        };
        let value = match field {
            F::Waypoint => gps.destination.clone().unwrap_or_else(|| "_____".into()),
            F::Bearing => {
                if gps.valid && gps.distance_valid {
                    format!("{:03.0}°", gps.bearing_rad.to_degrees().rem_euclid(360.))
                } else {
                    "___°".into()
                }
            }
            F::Distance => {
                if gps.distance_valid {
                    format!("{:.1}NM", gps.distance_nm)
                } else {
                    "__._NM".into()
                }
            }
            F::TimeEnRoute => gps.ete_s.map_or("--:--".into(), duration),
            F::GroundSpeed => format!("{:.0}KT", s.ground_speed_mps * KNOTS),
            F::Track => format!("{:03.0}°", s.ground_track_rad.to_degrees().rem_euclid(360.)),
            F::SafeAltitude => match s.settings.data_bar_field {
                flightcraft_control::DataBarField::SafeAltitude => "-----".into(),
                flightcraft_control::DataBarField::Clock => {
                    flightcraft_sim::avionics::glass_aids::hms(s.clock.seconds)
                }
                flightcraft_control::DataBarField::Timer => {
                    flightcraft_sim::avionics::glass_aids::timer_text(s.clock.timer_s)
                }
            },
            // Vertical speed required to meet the next altitude constraint, once the flight plan has one.
            F::VerticalSpeedRequired => s
                .vnav
                .as_ref()
                .and_then(|v| v.required_vs_fpm)
                .map_or("-----".into(), |fpm| {
                    format!("{:+.0}FPM", (fpm / 10.).round() * 10.)
                }),
        };
        label(&mut d, [*fx as f32, y + 14.], label_text, small, Ink::White);
        label(&mut d, [*fx as f32, y + h * 0.68], value, size, ink);
    }
    // Status bar: knob functions at the ends, outside air temperature and messages in the middle.
    let [bx, by, bw, bh] = r4(g.rect(E::StatusBar));
    rect(&mut d, [bx, by, bw, bh], Ink::BoxFill);
    let sz = 16. * t.data_bar;
    let width = |text: &str| text.chars().count() as f32 * sz * 0.55;
    let cy = by + bh * 0.5;
    for (i, (inner, outer)) in knob_hints.iter().enumerate() {
        // Inner function, the dual-knob glyph, then the outer function.
        let total = width(inner) + 34. + width(outer);
        let x0 = if i == 0 { 10. } else { bw - 10. - total };
        label(&mut d, [x0 + width(inner) * 0.5, cy], *inner, sz, Ink::White);
        ring_glyph(&mut d, [x0 + width(inner) + 17., cy], bh * 0.3);
        label(
            &mut d,
            [x0 + width(inner) + 34. + width(outer) * 0.5, cy],
            *outer,
            sz,
            Ink::White,
        );
    }
    let oat = if input.options.engine == crate::instruments::EngineUnits::Us {
        format!("OAT {:.0}°F", s.air_data.outside_temperature_c * 1.8 + 32.)
    } else {
        format!("OAT {:.0}°C", s.air_data.outside_temperature_c)
    };
    label(&mut d, [bw * 0.5 - 60., by + bh * 0.5], oat, sz, Ink::White);
    label(
        &mut d,
        [bw * 0.5 + 70., by + bh * 0.5],
        "Messages",
        sz,
        Ink::Yellow,
    );
    d
}

/// The instrument-face colours and type scales a skin pack sets. Without a skin, or for a field the
/// skin leaves out, the built-in meaning colours and sizes apply.
pub(crate) fn face_style(
    skin: Option<&flightcraft_control::SkinContents>,
) -> (Vec<(Ink, egui::Color32)>, Typography) {
    let mut colors = Vec::new();
    let mut typography = Typography::default();
    let Some(skin) = skin else {
        return (colors, typography);
    };
    let style = &skin.manifest.instruments;
    let p = &style.palette;
    let mut add = |value: &Option<flightcraft_aero::panel_fidelity::SrgbHex>, ink: Ink, alpha: u8| {
        if let Some([r, g, b]) = value.as_ref().and_then(|v| v.rgb()) {
            colors.push((ink, egui::Color32::from_rgba_unmultiplied(r, g, b, alpha)));
        }
    };
    add(&p.sky, Ink::Sky, 255);
    add(&p.ground, Ink::Earth, 255);
    // Tapes are translucent over the attitude display, boxes nearly opaque.
    add(&p.tape_fill, Ink::TapeFill, 165);
    add(&p.tape_text, Ink::TapeText, 255);
    add(&p.box_fill, Ink::BoxFill, 235);
    add(&p.box_border, Ink::BoxBorder, 255);
    add(&p.box_text, Ink::BoxText, 255);
    add(&p.white, Ink::White, 255);
    add(&p.cyan, Ink::Cyan, 255);
    add(&p.magenta, Ink::Magenta, 255);
    add(&p.green, Ink::Green, 255);
    add(&p.yellow, Ink::Yellow, 255);
    add(&p.red, Ink::Red, 255);
    let t = &style.typography;
    typography.tape = t.tape_text.unwrap_or(1.);
    typography.boxes = t.box_text.unwrap_or(1.);
    typography.data_bar = t.data_bar_text.unwrap_or(1.);
    typography.annunciation = t.annunciation_text.unwrap_or(1.);
    (colors, typography)
}

/// The dual-knob glyph between the two functions: a ring with a smaller disc inside.
fn ring_glyph(d: &mut DrawList, at: [f32; 2], r: f32) {
    circle(d, at, r, Ink::Cyan);
    circle(d, at, r * 0.62, Ink::BoxFill);
    circle(d, at, r * 0.32, Ink::Cyan);
}
