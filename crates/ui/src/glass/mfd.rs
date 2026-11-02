use super::*;
use flightcraft_control::{GlassPage, TerrainBand};
pub(super) fn draw(s: &GlassState, options: crate::instruments::DisplayOptions) -> DrawList {
    let mut d = canvas();
    header(&mut d, "MULTI FUNCTION");
    if s.mfd_failed {
        flag(&mut d, [4., 34., 492., 340.], "MFD FAIL");
        return d;
    }
    engine_strip(&mut d, s, options);
    line(&mut d, [109., 32.], [109., 352.], Ink::Inactive, 1.);
    label(
        &mut d,
        [306., 368.],
        s.region.as_deref().unwrap_or("NO REGION / NO GPS"),
        11.,
        Ink::Muted,
    );
    match s.settings.page {
        GlassPage::Map => map(&mut d, s),
        GlassPage::Nearest | GlassPage::FlightPlan => {
            let legs = if s.settings.page == GlassPage::Nearest {
                &s.nearest
            } else {
                &s.gps.legs
            };
            label(
                &mut d,
                [302., 49.],
                if s.settings.page == GlassPage::Nearest {
                    "NEAREST AIRPORTS"
                } else {
                    "FLIGHT PLAN"
                },
                14.,
                Ink::Text,
            );
            label(
                &mut d,
                [302., 75.],
                "AIRPORT      NM     BRG    ETE",
                11.,
                Ink::Muted,
            );
            for (i, leg) in legs.iter().take(8).enumerate() {
                let y = 103. + i as f32 * 29.;
                let ete = leg.ete_s.map_or("--:--".into(), |t| {
                    format!("{:02.0}:{:02.0}", (t / 60.).floor(), t % 60.)
                });
                label(
                    &mut d,
                    [302., y],
                    format!(
                        "{:<6}  {:5.1}  {:03.0}   {}",
                        leg.airport,
                        leg.distance_nm,
                        leg.bearing_rad.to_degrees(),
                        ete
                    ),
                    12.,
                    if s.settings.page == GlassPage::FlightPlan && i == s.gps.active_leg {
                        Ink::Gps
                    } else {
                        Ink::Text
                    },
                );
            }
        }
    }
    d
}
fn map(d: &mut DrawList, s: &GlassState) {
    // Square plotting area keeps range rings and true bearings geometrically correct.
    let point = |[x, y]: [f64; 2]| [151. + x as f32 * 300., 40. + y as f32 * 300.];
    let bounds = [151., 40., 300., 300.];
    rect(d, bounds, Ink::Inactive);
    let raster = &s.terrain;
    if raster.columns > 1 && raster.rows > 1 {
        let mut vertices = Vec::with_capacity(raster.len());
        for (i, height) in raster.elevation_m.iter().enumerate() {
            let x = (i as u32 % raster.columns) as f64 / f64::from(raster.columns - 1);
            let y = (i as u32 / raster.columns) as f64 / f64::from(raster.rows - 1);
            let (ink, other, blend) = if let Some(height) = height.filter(|_| s.air_data.air_data_valid) {
                match flightcraft_control::terrain_band(s.air_data.altitude_msl_m, f64::from(height)) {
                    TerrainBand::Warning => (Ink::Alert, Ink::Alert, 0.),
                    TerrainBand::Caution => (Ink::Warning, Ink::Warning, 0.),
                    _ => (Ink::TerrainLow, Ink::TerrainHigh, (height / 1200.).clamp(0., 1.)),
                }
            } else {
                (Ink::Inactive, Ink::Inactive, 0.)
            };
            vertices.push((point([x, y]), ink, other, blend));
        }
        let mut indices = Vec::new();
        for row in 0..raster.rows - 1 {
            for col in 0..raster.columns - 1 {
                let a = row * raster.columns + col;
                let b = a + 1;
                let c = a + raster.columns;
                let e = c + 1;
                indices.extend_from_slice(&[a, b, c, b, e, c]);
            }
        }
        d.items.push(DrawItem::Mesh { vertices, indices });
    }
    // All rings, including holes and separate polygon components, retain their class ink.
    for outline in &s.outlines {
        let ink = match outline.class {
            Some(flightcraft_world::AirspaceClass::B) | Some(flightcraft_world::AirspaceClass::D) => {
                Ink::AirspaceBlue
            }
            Some(flightcraft_world::AirspaceClass::C) | Some(flightcraft_world::AirspaceClass::E) => Ink::Gps,
            _ => Ink::Muted,
        };
        for ring in &outline.rings {
            for segment in ring.windows(2) {
                if let [a, b] = segment {
                    clipped_line(d, point(*a), point(*b), bounds, ink, 1.4);
                }
            }
        }
    }
    if s.gps.valid {
        // Airport-to-airport segments use the same chart projection as airport runways.
        for legs in s.settings.flight_plan.windows(2) {
            if let [a, b] = legs
                && let (Some(a), Some(b)) = (
                    s.airports.iter().find(|v| v.ident == *a),
                    s.airports.iter().find(|v| v.ident == *b),
                )
            {
                clipped_line(d, point(a.position), point(b.position), bounds, Ink::Gps, 2.);
            }
        }
        if let Some(dest) = s
            .gps
            .destination
            .as_ref()
            .and_then(|id| s.airports.iter().find(|a| a.ident == *id))
        {
            clipped_line(d, point([0.5, 0.5]), point(dest.position), bounds, Ink::Gps, 1.);
        }
    }
    let center = point([0.5, 0.5]);
    let [center_x, center_y] = center;
    let mut previous = point([0.5, 0.125]);
    for i in 1..=72 {
        let angle = f64::from(i) * core::f64::consts::TAU / 72.;
        let next = point([0.5 + 0.375 * libm::sin(angle), 0.5 - 0.375 * libm::cos(angle)]);
        line(d, previous, next, Ink::Muted, 0.6);
        previous = next;
    }
    // Declutter airport labels in chart cells; runway geometry remains visible at every range.
    let mut labels = Vec::<[f32; 2]>::new();
    for airport in &s.airports {
        for [a, b] in &airport.runways {
            clipped_line(d, point(*a), point(*b), bounds, Ink::Text, 2.);
        }
        let [x, y] = point(airport.position);
        if (163. ..439.).contains(&x) && (57. ..322.).contains(&y) {
            circle(d, [x, y], 2., Ink::Text);
            if labels
                .iter()
                .all(|[lx, ly]| (x - lx).abs() > 40. || (y - ly).abs() > 21.)
                && (x - center_x).abs() + (y - center_y).abs() > 24.
            {
                label(d, [x, y - 10.], &airport.ident, 9., Ink::Text);
                labels.push([x, y]);
            }
        }
    }
    let nose = |x, y| rotated([x, y], center, s.aircraft_heading_relative_rad);
    polygon(
        d,
        vec![nose(0., -10.), nose(6., 8.), nose(0., 4.), nose(-6., 8.)],
        Ink::Needle,
    );
    rect(d, [155., 321., 292., 18.], Ink::Face);
    label(
        d,
        [301., 330.],
        format!(
            "{:?}   {:.0} NM   RING {:.0}",
            s.settings.orientation,
            s.settings.range_nm,
            s.settings.range_nm * 0.75
        )
        .to_uppercase(),
        10.,
        Ink::Text,
    );
    label(
        d,
        [302., 350.],
        if s.air_data.air_data_valid {
            "TERRAIN  YELLOW ≤1000 FT  RED ≤100 FT"
        } else {
            "TERRAIN INHIBITED / ADC FAIL"
        },
        9.,
        Ink::Warning,
    );
}
/// Liang–Barsky clipping avoids spilling map geometry into engine gauges or page labels.
fn clipped_line(d: &mut DrawList, a: [f32; 2], b: [f32; 2], [x, y, w, h]: [f32; 4], ink: Ink, width: f32) {
    let [ax, ay] = a;
    let [bx, by] = b;
    let (dx, dy) = (bx - ax, by - ay);
    let (mut t0, mut t1) = (0_f32, 1_f32);
    for (p, q) in [(-dx, ax - x), (dx, x + w - ax), (-dy, ay - y), (dy, y + h - ay)] {
        if p == 0. {
            if q < 0. {
                return;
            }
        } else {
            let t = q / p;
            if p < 0. {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
        }
    }
    if t0 <= t1 {
        line(
            d,
            [ax + t0 * dx, ay + t0 * dy],
            [ax + t1 * dx, ay + t1 * dy],
            ink,
            width,
        );
    }
}
