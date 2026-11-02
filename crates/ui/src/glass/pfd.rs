use super::*;
use flightcraft_control::BearingPointer;
const KNOTS: f64 = 1.9438444924406;
const FEET: f64 = 1. / 0.3048;
pub(super) fn draw(s: &GlassState, input: InstrumentInput<'_>, standby: bool) -> DrawList {
    draw_composed(s, input, standby, true)
}
pub(super) fn draw_composed(
    s: &GlassState,
    input: InstrumentInput<'_>,
    standby: bool,
    show_hsi: bool,
) -> DrawList {
    if standby {
        return standby_draw(s, input);
    }
    let mut d = canvas();
    d.size = [500., 440.];
    header(
        &mut d,
        if standby {
            "STANDBY ATTITUDE"
        } else {
            "PRIMARY FLIGHT"
        },
    );
    rect(&mut d, [0., 380., 500., 60.], Ink::Face);
    let a = if standby { &s.standby } else { &s.air_data };
    d.valid = a.attitude_valid && a.air_data_valid;
    d.indicated = a.indicated_airspeed_mps;
    if !standby && s.pfd_failed {
        flag(&mut d, [5., 34., 490., 338.], "PFD FAIL");
        return d;
    }
    if let Some(ap) = input.autopilot {
        crate::autopilot::strip_draw(&mut d, ap, [0., 0., 500., 28.]);
    } else {
        label(
            &mut d,
            [68., 14.],
            format!(
                "HDG {}",
                crate::instruments::heading::degrees_text(s.settings.heading_bug_rad)
            ),
            11.,
            Ink::Needle,
        );
        label(
            &mut d,
            [434., 14.],
            format!(
                "SEL {}",
                crate::instruments::formatting::selected_feet(s.settings.selected_altitude_m)
            ),
            11.,
            Ink::Needle,
        );
    }
    // The full-width attitude underlays both tapes, with a fixed viewport at every pitch and bank.
    rect(&mut d, [0., 30., 500., 190.], Ink::Sky);
    let horizon = |x, y| rotated([x, y], [250., 125.], -a.roll_rad);
    let pitch = flightcraft_render::pfd_pitch_y(a.pitch_rad);
    polygon(
        &mut d,
        clip_polygon(
            vec![
                horizon(-900., pitch),
                horizon(900., pitch),
                horizon(900., 1000.),
                horizon(-900., 1000.),
            ],
            [0., 30., 500., 190.],
        ),
        Ink::Earth,
    );
    if s.settings.synthetic_vision
        && a.attitude_valid
        && a.air_data_valid
        && a.powered
        && let Some((world, state)) = input.vision
        && let Some(view) =
            flightcraft_render::SyntheticVision::new(world, state, a.roll_rad, a.pitch_rad, a.altitude_msl_m)
    {
        d.items.push(DrawItem::SyntheticVision {
            min: [0., 30.],
            max: [500., 220.],
            view: Box::new(view.clone()),
        });
        let inside = |[x, y]: [f32; 2]| (100. ..400.).contains(&x) && (50. ..195.).contains(&y);
        let mut occupied = Vec::<[f32; 2]>::new();
        for (at, text) in view.labels() {
            let [x, y] = at;
            if inside(at)
                && occupied
                    .iter()
                    .all(|[px, py]| (x - px).abs() > 75. || (y - py).abs() > 16.)
            {
                line(&mut d, [x, y - 4.], [x, y - 9.], Ink::Text, 1.);
                label(&mut d, [x, y], text, 9., Ink::Text);
                occupied.push(at);
            }
        }
        if let Some(at) = view.flight_path_marker().filter(|at| inside(*at)) {
            let [x, y] = at;
            // Open circle preserves terrain visibility through the velocity vector.
            let mut vertices = Vec::with_capacity(48);
            let mut indices = Vec::with_capacity(144);
            for i in 0..24_u32 {
                let angle = i as f32 * std::f32::consts::TAU / 24.;
                for radius in [5.75, 4.25] {
                    vertices.push((
                        [x + radius * angle.cos(), y + radius * angle.sin()],
                        Ink::Normal,
                        Ink::Normal,
                        0.,
                    ));
                }
                let a = i * 2;
                let b = ((i + 1) % 24) * 2;
                indices.extend([a, b, a + 1, a + 1, b, b + 1]);
            }
            d.items.push(DrawItem::Mesh { vertices, indices });
            line(&mut d, [x - 13., y], [x - 5., y], Ink::Normal, 1.5);
            line(&mut d, [x + 5., y], [x + 13., y], Ink::Normal, 1.5);
            line(&mut d, [x, y - 5.], [x, y - 10.], Ink::Normal, 1.5);
        }
    }
    if a.attitude_valid {
        for degrees in [-30_i32, -20, -10, -5, 0, 5, 10, 20, 30] {
            let y = flightcraft_render::pfd_pitch_y(a.pitch_rad - f64::from(degrees).to_radians());
            let width = if degrees % 10 == 0 { 35. } else { 20. };
            let p = horizon(-width, y);
            let q = horizon(width, y);
            if [p, q]
                .iter()
                .all(|[x, y]| (100. ..=400.).contains(x) && (55. ..=190.).contains(y))
            {
                line(&mut d, p, q, Ink::Text, if degrees == 0 { 2. } else { 1. });
                if degrees != 0 && degrees % 10 == 0 {
                    let pos = horizon(-width - 19., y);
                    let [_, y] = pos;
                    if y > 68. && y < 183. {
                        label(&mut d, pos, degrees.abs(), 9., Ink::Text);
                    }
                }
            }
        }
        for deg in [-60_f64, -45., -30., -20., -10., 0., 10., 20., 30., 45., 60.] {
            let point = |r: f32| {
                [
                    250. + r * libm::sin(deg.to_radians()) as f32,
                    125. - r * libm::cos(deg.to_radians()) as f32,
                ]
            };
            line(&mut d, point(85.), point(92.), Ink::Text, 1.);
        }
        let roll = rotated([0., -79.], [250., 125.], -a.roll_rad);
        let [rx, ry] = roll;
        polygon(
            &mut d,
            vec![roll, [rx - 4., ry + 7.], [rx + 4., ry + 7.]],
            Ink::Needle,
        );
        line(&mut d, [211., 123.], [236., 123.], Ink::Needle, 3.);
        line(&mut d, [264., 123.], [289., 123.], Ink::Needle, 3.);
        line(&mut d, [236., 123.], [243., 130.], Ink::Needle, 3.);
        line(&mut d, [257., 130.], [264., 123.], Ink::Needle, 3.);
        circle(&mut d, [250., 125.], 2., Ink::Needle);
        if let Some(ap) = input.autopilot {
            crate::autopilot::command_bars(&mut d, ap, [250., 125.], a.roll_rad, a.pitch_rad, 1.0);
        }
        line(&mut d, [219., 204.], [281., 204.], Ink::Text, 1.);
        circle(
            &mut d,
            [250. - a.slip_g.clamp(-0.5, 0.5) as f32 * 55., 204.],
            4.,
            Ink::Text,
        );
    } else {
        flag(
            &mut d,
            [99., 50., 302., 140.],
            if a.alignment_remaining_s > 0. && a.powered && !a.ahrs_failed {
                "ALIGNING"
            } else {
                "AHRS FAIL"
            },
        );
        if a.powered {
            label(
                &mut d,
                [250., 207.],
                format!("{:.0} s", a.alignment_remaining_s),
                10.,
                Ink::Warning,
            );
        }
    }
    if a.air_data_valid {
        let ias = a.indicated_airspeed_mps * KNOTS;
        let alt = a.altitude_msl_m * FEET;
        tape(&mut d, 8., ias, 10., 2., false);
        tape(&mut d, 401., alt, 100., 0.2, true);
        if let Some(aircraft) = input.aircraft {
            let v = &aircraft.variant.v_speeds;
            for (low, high, ink, x) in [
                (v.vso_mps, v.vfe_mps, Ink::Text, 11.),
                (v.vs_mps, v.vno_mps, Ink::Normal, 15.),
                (v.vno_mps, v.vne_mps, Ink::Warning, 15.),
            ] {
                let top = tape_y(high * KNOTS, ias, 128., 2.).clamp(40., 216.);
                let bottom = tape_y(low * KNOTS, ias, 128., 2.).clamp(40., 216.);
                rect(&mut d, [x, top, 3., (bottom - top).max(0.)], ink);
            }
            for (value, ink) in [
                (v.vr_mps, Ink::Needle),
                (v.vx_mps, Ink::Normal),
                (v.vy_mps, Ink::Normal),
                (v.vne_mps, Ink::Alert),
            ] {
                let y = tape_y(value * KNOTS, ias, 128., 2.);
                if (43. ..213.).contains(&y) {
                    polygon(&mut d, vec![[75., y], [82., y - 3.], [82., y + 3.]], ink);
                }
            }
            if !s.reversion || standby {
                label(
                    &mut d,
                    [151., 233.],
                    format!(
                        "Vr {:.0}  Vx {:.0}  Vy {:.0}",
                        v.vr_mps * KNOTS,
                        v.vx_mps * KNOTS,
                        v.vy_mps * KNOTS
                    ),
                    9.,
                    Ink::Needle,
                );
            }
        }
        trend(&mut d, 83., a.airspeed_trend_mps * KNOTS * 2.);
        trend(&mut d, 395., a.altitude_trend_m * FEET * 0.2);
        let y = tape_y(s.settings.selected_altitude_m * FEET, alt, 128., 0.2).clamp(43., 213.);
        polygon(
            &mut d,
            vec![[401., y], [393., y - 4.], [393., y + 4.]],
            Ink::Needle,
        );
        rect(&mut d, [401., 30., 68., 18.], Ink::Face);
        label(
            &mut d,
            [435., 39.],
            crate::instruments::formatting::selected_feet(s.settings.selected_altitude_m),
            11.,
            Ink::Needle,
        );
        label(
            &mut d,
            [454., 232.],
            format!(
                "VS {}",
                crate::instruments::formatting::vertical_speed(a.vertical_speed_mps)
            ),
            10.,
            Ink::Text,
        );
        rect(&mut d, [472., 37., 28., 181.], Ink::Face);
        for tick in -8..=8 {
            let value = f64::from(tick) * 250.;
            let y = 128. - value as f32 * 0.04;
            line(&mut d, [494., y], [499., y], Ink::Text, 1.);
            if tick % 4 == 0 {
                label(&mut d, [481., y], format!("{:+.0}", value / 1000.), 7., Ink::Text);
            }
        }
        let y = 128. - (a.vertical_speed_mps * FEET * 60.).clamp(-2000., 2000.) as f32 * 0.04;
        polygon(
            &mut d,
            vec![[498., y], [491., y - 4.], [491., y + 4.]],
            Ink::Needle,
        );
    } else {
        flag(&mut d, [5., 37., 80., 180.], "ADC");
        flag(&mut d, [399., 37., 99., 180.], "ADC");
    }
    let baro = if s.settings.pressure_hpa {
        format!("{:.0} hPa", s.baro_pa / 100.)
    } else {
        format!("{:.2} inHg", s.baro_pa / 3386.389)
    };
    label(&mut d, [444., 253.], baro, 11., Ink::Needle);
    let cy = if s.reversion && !standby { 335. } else { 320. };
    let radius = if s.reversion && !standby { 55. } else { 64. };
    if show_hsi {
        hsi(&mut d, s, cy, radius, a.attitude_valid, a.heading_rad);
    }
    if !s.reversion || standby {
        label(
            &mut d,
            [250., 225.],
            if a.attitude_valid {
                format!("{}°", crate::instruments::heading::degrees_text(a.heading_rad))
            } else {
                "---°".into()
            },
            12.,
            Ink::Text,
        );
    }
    crate::instruments::heading::knob_art(&mut d, [359., 404.], 11.);
    label(
        &mut d,
        [421., 404.],
        format!(
            "HDG {}",
            crate::instruments::heading::degrees_text(s.settings.heading_bug_rad)
        ),
        11.,
        Ink::Needle,
    );
    let gps = &s.gps;
    for (row, text, ink) in [
        (
            0,
            if gps.valid {
                format!(
                    "{} {}{}",
                    gps.source,
                    gps.destination.as_deref().unwrap_or(""),
                    if gps.waypoint_alert && gps.source == "GPS" {
                        " WPT"
                    } else {
                        ""
                    }
                )
            } else {
                if gps.source == "GPS" {
                    "NO GPS LEG"
                } else {
                    "NO NAV SIGNAL"
                }
                .into()
            },
            Ink::Gps,
        ),
        (
            1,
            format!("DTK {:03.0}", gps.desired_track_rad.to_degrees()),
            Ink::Gps,
        ),
        (
            2,
            if gps.distance_valid {
                format!("DIS {:.1} NM", gps.distance_nm)
            } else {
                "DIS —".into()
            },
            Ink::Text,
        ),
        (3, format!("XTK {:+.2} NM", gps.cross_track_nm), Ink::Text),
        (
            4,
            if gps.source == "GPS" {
                format!(
                    "{} CDI ±{:.1} NM{}",
                    gps.annunciation,
                    gps.cdi_full_scale_nm,
                    if gps.suspended { " SUSP" } else { "" }
                )
            } else {
                format!("{} CDI ±2.5 DOT", gps.annunciation)
            },
            Ink::Muted,
        ),
    ] {
        label(&mut d, [80., 267. + row as f32 * 18.], text, 11., ink);
    }
    label(
        &mut d,
        [422., 294.],
        format!(
            "WIND {:03.0}/{:.0}",
            s.wind_from_rad.to_degrees(),
            s.wind_speed_mps * KNOTS
        ),
        11.,
        Ink::Text,
    );
    let wind = |len| rotated([0., len], [422., 319.], s.wind_from_rad - a.heading_rad);
    line(&mut d, wind(-9.), wind(9.), Ink::Text, 2.);
    let tip = wind(9.);
    line(
        &mut d,
        tip,
        rotated([-4., 4.], [422., 319.], s.wind_from_rad - a.heading_rad),
        Ink::Text,
        2.,
    );
    label(
        &mut d,
        [422., 350.],
        format!(
            "OAT {}",
            if input.options.engine == crate::instruments::EngineUnits::Us {
                format!("{:.0} °F", a.outside_temperature_c * 1.8 + 32.)
            } else {
                format!("{:.0} °C", a.outside_temperature_c)
            }
        ),
        11.,
        Ink::Text,
    );
    label(
        &mut d,
        [421., 378.],
        format!(
            "XPDR {} {:?}",
            s.settings.transponder_code, s.settings.transponder_mode
        )
        .to_uppercase(),
        11.,
        Ink::Text,
    );
    if s.reversion && !standby {
        rect(&mut d, [3., 221., 407., 37.], Ink::Face);
        for (i, v) in s.engine.iter().take(10).enumerate() {
            let x = 43. + (i % 5) as f32 * 81.;
            let y = 230. + (i / 5) as f32 * 17.;
            label(
                &mut d,
                [x, y],
                format!("{} {:.0}", v.label, v.value),
                8.,
                Ink::Warning,
            );
        }
    }
    label(
        &mut d,
        [91., 427.],
        if a.air_data_valid {
            format!("TAS {:.0} KT", a.true_airspeed_mps * KNOTS)
        } else {
            "TAS ---".into()
        },
        11.,
        Ink::Text,
    );
    label(
        &mut d,
        [251., 427.],
        format!("GS {:.0} KT", s.ground_speed_mps * KNOTS),
        11.,
        Ink::Text,
    );
    label(
        &mut d,
        [421., 427.],
        format!("TRK {:03.0}", s.ground_track_rad.to_degrees()),
        11.,
        Ink::Text,
    );
    d
}
fn tape(d: &mut DrawList, x: f32, value: f64, step: f64, scale: f32, altitude: bool) {
    rect(d, [x, 37., 68., 181.], Ink::Face);
    for tick in -6..=6 {
        let v = (value / step).floor() * step + f64::from(tick) * step;
        let y = tape_y(v, value, 128., scale);
        let top = if altitude { 60. } else { 46. };
        if !(top..=210.).contains(&y) || (!altitude && v < 0.) {
            continue;
        }
        line(d, [x + 56., y], [x + 65., y], Ink::Text, 1.);
        if (y - 128.).abs() > 16. {
            label(
                d,
                [x + 34., y],
                crate::instruments::formatting::integer(v),
                12.,
                Ink::Text,
            );
        }
    }
    rect(d, [x + 5., 115., 57., 26.], Ink::Inactive);
    label(
        d,
        [x + 34., 128.],
        crate::instruments::formatting::integer(value),
        16.,
        Ink::Text,
    );
}
fn trend(d: &mut DrawList, x: f32, delta: f64) {
    let y = (128. - delta as f32).clamp(41., 213.);
    line(d, [x, 128.], [x, y], Ink::Gps, 3.);
    line(d, [x - 3., y], [x + 3., y], Ink::Gps, 2.);
}
fn hsi(d: &mut DrawList, s: &GlassState, cy: f32, r: f32, valid: bool, heading: f64) {
    if !valid {
        flag(d, [183., cy - r - 4., 134., r * 2. + 8.], "HDG FAIL");
        return;
    }
    for deg in (0..360).step_by(10) {
        let angle = f64::from(deg).to_radians() - heading;
        let point = |rad: f32| {
            [
                250. + libm::sin(angle) as f32 * rad,
                cy - libm::cos(angle) as f32 * rad,
            ]
        };
        line(d, point(r - 4.), point(r), Ink::Text, 1.);
        if deg % 30 == 0 {
            let text = match deg {
                0 => "N".into(),
                90 => "E".into(),
                180 => "S".into(),
                270 => "W".into(),
                _ => format!("{}", deg / 10),
            };
            label(d, point(r + 10.), text, 9., Ink::Text);
        }
    }
    crate::instruments::heading::marker(
        d,
        [250., cy],
        r,
        s.settings.heading_bug_rad,
        heading,
        std::f64::consts::PI,
    );
    let course = if s.gps.valid && !s.settings.course_manual {
        s.gps.desired_track_rad
    } else {
        s.settings.course_rad
    };
    let point = |x, y| rotated([x, y], [250., cy], course - heading);
    line(d, point(0., -r + 8.), point(0., r - 8.), Ink::Gps, 2.);
    polygon(
        d,
        vec![point(0., -r + 4.), point(-4., -r + 13.), point(4., -r + 13.)],
        Ink::Gps,
    );
    for f in [-1_f32, -0.5, 0.5, 1.] {
        circle(d, point(f * r * 0.5, 0.), 1.5, Ink::Text);
    }
    if s.gps.valid {
        let x = s.gps.cdi_deviation as f32 * r * 0.5;
        line(d, point(x, -r * 0.35), point(x, r * 0.35), Ink::Gps, 3.);
    }
    if let Some(dots) = s.gps.vertical_dots {
        let y = cy + ((dots / 2.5).clamp(-1., 1.) as f32) * r * 0.65;
        line(
            d,
            [250. + r + 12., cy - r * 0.65],
            [250. + r + 12., cy + r * 0.65],
            Ink::Muted,
            1.,
        );
        line(d, [250. + r + 6., y], [250. + r + 12., y - 5.], Ink::Gps, 2.);
        line(d, [250. + r + 12., y - 5.], [250. + r + 18., y], Ink::Gps, 2.);
        line(d, [250. + r + 18., y], [250. + r + 12., y + 5.], Ink::Gps, 2.);
        line(d, [250. + r + 12., y + 5.], [250. + r + 6., y], Ink::Gps, 2.);
    }
    if s.settings.bearing_pointer == BearingPointer::Gps && s.gps.valid {
        line(
            d,
            rotated([0., -r + 8.], [250., cy], s.gps.bearing_rad - heading),
            rotated([0., r - 8.], [250., cy], s.gps.bearing_rad - heading),
            Ink::Needle,
            1.,
        );
    }
    line(d, [241., cy], [259., cy], Ink::Text, 2.);
    line(d, [250., cy - 7.], [250., cy + 8.], Ink::Text, 2.);
}

fn standby_draw(s: &GlassState, input: InstrumentInput<'_>) -> DrawList {
    let a = &s.standby;
    let mut d = DrawList {
        size: [200., 180.],
        items: Vec::new(),
        indicated: a.indicated_airspeed_mps,
        valid: a.attitude_valid,
    };
    rect(&mut d, [0., 0., 200., 180.], Ink::Face);
    label(&mut d, [100., 11.], "STANDBY", 11., Ink::Text);
    rect(&mut d, [4., 23., 192., 110.], Ink::Sky);
    let horizon = |x, y| rotated([x, y], [100., 78.], -a.roll_rad);
    let pitch = a.pitch_rad.to_degrees() as f32 * 1.5;
    polygon(
        &mut d,
        clip_polygon(
            vec![
                horizon(-500., pitch),
                horizon(500., pitch),
                horizon(500., 500.),
                horizon(-500., 500.),
            ],
            [4., 23., 192., 110.],
        ),
        Ink::Earth,
    );
    if a.attitude_valid {
        for angle in [-20, -10, 0, 10, 20] {
            let y = pitch - angle as f32 * 1.5;
            let p = horizon(-18., y);
            let q = horizon(18., y);
            if [p, q]
                .iter()
                .all(|[x, y]| (7. ..193.).contains(x) && (28. ..128.).contains(y))
            {
                line(&mut d, p, q, Ink::Text, 1.);
            }
        }
        line(&mut d, [67., 80.], [91., 80.], Ink::Needle, 2.);
        line(&mut d, [109., 80.], [133., 80.], Ink::Needle, 2.);
        circle(&mut d, [100., 80.], 2., Ink::Needle);
        if let Some(ap) = input.autopilot {
            crate::autopilot::command_bars(&mut d, ap, [100., 80.], a.roll_rad, a.pitch_rad, 0.6);
        }
        circle(
            &mut d,
            [100. - a.slip_g.clamp(-0.5, 0.5) as f32 * 30., 124.],
            3.,
            Ink::Text,
        );
    } else {
        flag(
            &mut d,
            [5., 26., 190., 100.],
            if a.powered { "ALIGNING" } else { "OFF" },
        );
    }
    label(
        &mut d,
        [46., 146.],
        format!("IAS {:.0} KT", a.indicated_airspeed_mps * KNOTS),
        10.,
        Ink::Text,
    );
    label(
        &mut d,
        [150., 146.],
        format!("ALT {}", crate::instruments::formatting::feet(a.altitude_msl_m)),
        10.,
        Ink::Text,
    );
    label(
        &mut d,
        [100., 166.],
        if s.standby_on_backup {
            format!("BAT {:.0} MIN", s.standby_remaining_s / 60.)
        } else {
            String::new()
        },
        10.,
        if s.standby_on_backup {
            Ink::Warning
        } else {
            Ink::Normal
        },
    );
    d
}
