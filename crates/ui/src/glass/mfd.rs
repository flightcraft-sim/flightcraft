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
    if s.settings.page != GlassPage::Map {
        label(
            &mut d,
            [306., 368.],
            s.region.as_deref().unwrap_or("NO REGION / NO GPS"),
            11.,
            Ink::Muted,
        );
    }
    match s.settings.page {
        GlassPage::Map => map(&mut d, s),
        GlassPage::Nearest => table(&mut d, s, ListKind::Nearest, 302., 49., 1.),
        GlassPage::FlightPlan => table(&mut d, s, ListKind::FlightPlan, 302., 49., 1.),
    }
    d
}
/// The lists the MFD and the touch displays share.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum ListKind {
    Nearest,
    FlightPlan,
    Traffic,
}
/// A titled list centred on `cx`, its title at `top`; `unit` scales type and row spacing.
pub(super) fn table(d: &mut DrawList, s: &GlassState, kind: ListKind, cx: f32, top: f32, unit: f32) {
    let (title, heading) = match kind {
        ListKind::Nearest => ("NEAREST AIRPORTS", "AIRPORT      NM     BRG    ETE"),
        ListKind::FlightPlan => ("FLIGHT PLAN", "AIRPORT      NM     BRG    ETE"),
        ListKind::Traffic => ("TRAFFIC", "TARGET      NM     ALT    TRK"),
    };
    label(d, [cx, top], title, 14. * unit, Ink::Text);
    let rows: Vec<(String, Ink)> = match kind {
        ListKind::Nearest | ListKind::FlightPlan => {
            let legs = if kind == ListKind::Nearest {
                &s.nearest
            } else {
                &s.gps.legs
            };
            legs.iter()
                .take(8)
                .enumerate()
                .map(|(i, leg)| {
                    let ete = leg.ete_s.map_or("--:--".into(), |t| {
                        format!("{:02.0}:{:02.0}", (t / 60.).floor(), t % 60.)
                    });
                    (
                        format!(
                            "{:<6}  {:5.1}  {:03.0}   {}",
                            leg.airport,
                            leg.distance_nm,
                            leg.bearing_rad.to_degrees(),
                            ete
                        ),
                        if kind == ListKind::FlightPlan && i == s.gps.active_leg {
                            Ink::Gps
                        } else {
                            Ink::Text
                        },
                    )
                })
                .collect()
        }
        ListKind::Traffic => {
            if !s.traffic_installed {
                label(
                    d,
                    [cx, top + 40. * unit],
                    "NO TRAFFIC RECEIVER",
                    12. * unit,
                    Ink::Muted,
                );
                return;
            }
            let mut targets: Vec<_> = s.traffic.iter().collect();
            targets.sort_by(|a, b| a.range_nm.total_cmp(&b.range_nm));
            targets
                .into_iter()
                .take(8)
                .map(|t| {
                    let name = if t.registration.is_empty() {
                        &t.callsign
                    } else {
                        &t.registration
                    };
                    let trend = match flightcraft_control::trend(t.vertical_rate_fpm) {
                        1 => "^",
                        -1 => "v",
                        _ => " ",
                    };
                    (
                        format!(
                            "{:<7} {:5.1}  {}{}  {:03.0}",
                            name.chars().take(7).collect::<String>(),
                            t.range_nm,
                            flightcraft_control::altitude_tag(t.relative_altitude_m / 0.3048),
                            trend,
                            t.chart_track_rad.to_degrees().rem_euclid(360.)
                        ),
                        if t.class == flightcraft_control::TrafficClass::Alert {
                            Ink::Warning
                        } else {
                            Ink::Text
                        },
                    )
                })
                .collect()
        }
    };
    label(d, [cx, top + 26. * unit], heading, 11. * unit, Ink::Muted);
    if rows.is_empty() {
        let empty = if kind == ListKind::Traffic {
            "NO TRAFFIC"
        } else {
            "NO ENTRIES"
        };
        label(d, [cx, top + 60. * unit], empty, 12. * unit, Ink::Muted);
    }
    for (i, (text, ink)) in rows.into_iter().enumerate() {
        label(
            d,
            [cx, top + (54. + i as f32 * 29.) * unit],
            text,
            12. * unit,
            ink,
        );
    }
}
/// A page of the nearest group other than airports: VORs, NDBs, intersections, or airspace, each
/// with the unit's empty-list text.
pub(super) fn nearest_table(
    d: &mut DrawList,
    s: &GlassState,
    page: flightcraft_aero::modern::NearestPage,
    cx: f32,
    top: f32,
    unit: f32,
) {
    use flightcraft_aero::modern::NearestPage as P;
    use flightcraft_control::touch_nearest as near;
    label(d, [cx, top], page.label().to_uppercase(), 14. * unit, Ink::Text);
    let (heading, rows): (&str, Vec<String>) = match page {
        P::Airports => ("AIRPORT      NM     BRG    ETE", Vec::new()),
        P::Vors => (
            "VOR     BRG    NM      FREQ",
            s.nearest_vors
                .iter()
                .map(|f| near::facility_row(page, f))
                .collect(),
        ),
        P::Ndbs => (
            "NDB     BRG    NM      FREQ",
            s.nearest_ndbs
                .iter()
                .map(|f| near::facility_row(page, f))
                .collect(),
        ),
        P::Intersections => (
            "INT     BRG    NM",
            s.nearest_intersections
                .iter()
                .map(|f| near::facility_row(page, f))
                .collect(),
        ),
        P::Airspace => (
            "AIRSPACE              STATUS",
            s.nearest_airspace.iter().map(near::airspace_row).collect(),
        ),
    };
    label(d, [cx, top + 26. * unit], heading, 11. * unit, Ink::Muted);
    if rows.is_empty() {
        label(
            d,
            [cx, top + 60. * unit],
            page.empty_text(),
            12. * unit,
            Ink::Muted,
        );
    }
    for (i, text) in rows.into_iter().take(10).enumerate() {
        label(
            d,
            [cx, top + (54. + i as f32 * 29.) * unit],
            text,
            12. * unit,
            Ink::Text,
        );
    }
}
fn map(d: &mut DrawList, s: &GlassState) {
    // Equal horizontal and vertical scales preserve range rings; clip to the full content area.
    let point = |[x, y]: [f64; 2]| [110. + x as f32 * 390., 9. + y as f32 * 390.];
    let bounds = [110., 28., 390., 352.];
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
            let [px, py] = point([x, y]);
            vertices.push(([px, py.clamp(28., 380.)], ink, other, blend));
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
    for traffic in &s.traffic {
        let [x, y] = point(traffic.position);
        if (bounds[0] + 8. ..bounds[0] + bounds[2] - 8.).contains(&x)
            && (bounds[1] + 8. ..bounds[1] + bounds[3] - 8.).contains(&y)
        {
            super::traffic::symbol(d, traffic, [x, y]);
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
        if (124. ..486.).contains(&x) && (45. ..330.).contains(&y) {
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
    rect(d, [110., 340., 390., 40.], Ink::Face);
    label(
        d,
        [305., 350.],
        format!(
            "{:?}   {:.0} NM   RING {:.0}{}",
            s.settings.orientation,
            s.settings.range_nm,
            s.settings.range_nm * 0.75,
            match (s.traffic_installed, s.settings.traffic_shown) {
                (false, _) => String::new(),
                (true, false) => "   TFC OFF".into(),
                (true, true) => format!("   TFC {}", s.settings.traffic_filter.label()),
            }
        )
        .to_uppercase(),
        10.,
        Ink::Text,
    );
    label(
        d,
        [305., 369.],
        if s.air_data.air_data_valid {
            "MAP SETTINGS SHARED WITH PLANNING"
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
