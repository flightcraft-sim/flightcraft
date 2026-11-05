//! Emergency overlays on the shared planning map's terrain, airports, and projection.
use super::*;
use flightcraft_control::scenario::EmergencyDebrief;

pub(crate) fn show(ui: &mut egui::Ui, s: &mut Session, d: &EmergencyDebrief) {
    // A procedure with no glide numbers (a ground fire) has no footprint to draw.
    let Some(glide) = d.glide.as_ref() else {
        return;
    };
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 470.), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0., theme::BACKGROUND);
    if s.sim().world().choice() != glide.world {
        p.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "Load the recorded terrain region to view this debrief.",
            FontId::proportional(16.),
            theme::AMBER,
        );
        return;
    }
    let [n, e, _] = glide.origin_m;
    let Some([latitude_deg, longitude_deg]) = s.sim().world().geodetic(n, e) else {
        return;
    };
    let span = glide
        .boundary_m
        .iter()
        .copied()
        .chain(d.track_m.iter().map(|[n, e, _]| [*n, *e]))
        .map(|[pn, pe]| libm::hypot(pn - n, pe - e))
        .fold(1000., f64::max)
        * 1.2;
    let mut state = MapState {
        base_map: BaseMap::Terrain,
        centre: Some(GeoPoint {
            latitude_deg,
            longitude_deg,
        }),
        range_nm: span / 1852.,
        ..Default::default()
    };
    for (overlay, style) in &mut state.overlays {
        style.enabled = matches!(overlay, Overlay::Airports | Overlay::RangeRings);
    }
    let Some(v) = View::new(s, &state, rect) else {
        return;
    };
    terrain(&p, &v, s, &state, false, false);
    paint_features(&p, &v, s, &state);
    rings(&p, &v, &state);
    let points = glide.boundary_m.iter().map(|p| v.local(*p)).collect();
    p.add(egui::Shape::closed_line(
        points,
        Stroke::new(2.0_f32, theme::ACCENT),
    ));
    let track: Vec<_> = d.track_m.iter().map(|[n, e, _]| v.local([*n, *e])).collect();
    if track.len() > 1 {
        p.add(egui::Shape::line(track, Stroke::new(3.0_f32, theme::GLASS_GPS)));
    }
    let mut annotations = Vec::new();
    let mut field_annotations = Vec::new();
    let origin = v.local([n, e]);
    p.circle_filled(origin, 5., theme::AMBER);
    annotations.push((
        origin,
        if d.procedure.id == "engine_failure" {
            "ENGINE FAILURE"
        } else {
            "FAILURE POINT"
        }
        .to_owned(),
        theme::AMBER,
        false,
    ));
    for (index, field) in glide.fields.iter().filter(|f| !f.airport).enumerate() {
        let at = v.local(field.position_m);
        let chosen = d.chosen_field.as_deref() == Some(&field.id);
        let color = if chosen { theme::GLASS_GPS } else { theme::TEXT };
        let (sn, cs) = field.heading_grid_rad.sin_cos();
        let [fnorth, feast] = field.position_m;
        let corners: Vec<_> = [(-0.5, -0.5), (-0.5, 0.5), (0.5, 0.5), (0.5, -0.5)]
            .into_iter()
            .map(|(a, b)| {
                v.local([
                    fnorth + cs * a * field.length_m - sn * b * field.width_m,
                    feast + sn * a * field.length_m + cs * b * field.width_m,
                ])
            })
            .collect();
        p.add(egui::Shape::closed_line(
            corners,
            Stroke::new(if chosen { 3.0_f32 } else { 1.5_f32 }, color),
        ));
        let distance = libm::hypot(fnorth - n, feast - e) / 1852.;
        let bearing = flightcraft_control::scenario::compass(libm::atan2(feast - e, fnorth - n).to_degrees());
        field_annotations.push((
            at,
            format!("F{} · {distance:.1} nm {bearing}", index + 1),
            color,
            true,
        ));
    }
    if let Some(field) = d
        .chosen_field
        .as_ref()
        .and_then(|id| glide.fields.iter().find(|f| f.id == *id))
    {
        let at = v.local(field.position_m);
        p.circle_stroke(at, 12., Stroke::new(3.0_f32, theme::GLASS_GPS));
        annotations.insert(
            0,
            (
                at,
                format!("CHOSEN {}", field.id.replace(':', " runway ")),
                theme::GLASS_GPS,
                false,
            ),
        );
    }
    if let Some([n, e, _]) = d.touchdown_m {
        let at = v.local([n, e]);
        p.circle_filled(at, 5., theme::TEXT);
        annotations.push((
            at,
            if d.outcome == "crashed" {
                "IMPACT"
            } else {
                "TOUCHDOWN"
            }
            .into(),
            theme::TEXT,
            false,
        ));
    }
    let mut occupied = Vec::new();
    for (at, text, color, left) in annotations.into_iter().chain(field_annotations) {
        let galley = p.layout_no_wrap(text, FontId::proportional(13.), color);
        let size = galley.size();
        let preferred = if left {
            egui::vec2(-size.x - 10., -size.y - 8.)
        } else {
            egui::vec2(12., 12.)
        };
        let offsets = [
            preferred,
            egui::vec2(12., -size.y - 12.),
            egui::vec2(-size.x - 12., 12.),
            egui::vec2(-size.x - 12., -size.y - 12.),
            egui::vec2(12., 36.),
            egui::vec2(-size.x - 12., -size.y - 36.),
        ];
        let position = offsets
            .into_iter()
            .map(|offset| at + offset)
            .find(|pos| {
                let bounds = Rect::from_min_size(*pos, size).expand(5.);
                rect.contains_rect(bounds) && occupied.iter().all(|r: &Rect| !r.intersects(bounds))
            })
            .unwrap_or(at + preferred);
        let bounds = Rect::from_min_size(position, size).expand(3.);
        occupied.push(bounds.expand(3.));
        p.line_segment([at, bounds.clamp(at)], Stroke::new(0.7_f32, color));
        p.rect_filled(bounds, 2., theme::BACKGROUND.gamma_multiply(0.9));
        p.galley(position, galley, color);
    }
    label(
        &p,
        rect.left_top() + egui::vec2(10., 10.),
        "NORTH UP   TERRAIN / FAA AIRPORTS   •   cyan: estimated reach   •   magenta: flown track",
        theme::TEXT,
    );
    if glide.terrain_coverage_limited {
        label(
            &p,
            rect.left_bottom() + egui::vec2(10., -35.),
            "Terrain coverage limits reach estimate",
            theme::AMBER,
        );
    }
}
fn label(p: &egui::Painter, at: Pos2, text: &str, color: egui::Color32) {
    let galley = p.layout_no_wrap(text.into(), FontId::proportional(13.), color);
    let rect = Rect::from_min_size(at, galley.size()).expand(3.);
    p.rect_filled(rect, 2., theme::BACKGROUND.gamma_multiply(0.9));
    p.galley(at, galley, color);
}
