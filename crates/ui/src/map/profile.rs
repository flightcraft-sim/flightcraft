use super::*;
use flightcraft_control::{PlanProfile, ProfileAirspace};
use flightcraft_world::AirspaceClass;
use std::collections::BTreeMap;
#[derive(Clone, Debug)]
struct Shelf {
    id: String,
    class: AirspaceClass,
    start_m: f64,
    end_m: f64,
    floor_m: f64,
    ceiling_m: f64,
}
fn controlled(volume: &ProfileAirspace) -> bool {
    matches!(
        volume.class,
        Some(AirspaceClass::B | AirspaceClass::C | AirspaceClass::D)
    )
}
/// Merge adjacent samples into lateral shelf intervals. Mode C veils and Class E are excluded.
fn shelves(profile: &PlanProfile) -> Vec<Shelf> {
    let mut active = BTreeMap::<String, Shelf>::new();
    let mut output = Vec::new();
    for pair in profile.samples.windows(2) {
        let [a, b] = pair else { continue };
        if b.distance_m <= a.distance_m {
            continue;
        }
        let present: std::collections::BTreeSet<_> = a
            .airspace
            .iter()
            .filter(|v| controlled(v))
            .map(|v| v.id.clone())
            .collect();
        let ended: Vec<_> = active
            .keys()
            .filter(|id| !present.contains(*id))
            .cloned()
            .collect();
        for id in ended {
            if let Some(shelf) = active.remove(&id) {
                output.push(shelf);
            }
        }
        for volume in a.airspace.iter().filter(|v| controlled(v)) {
            let Some(class) = volume.class else { continue };
            let shelf = active.entry(volume.id.clone()).or_insert_with(|| Shelf {
                id: volume.id.clone(),
                class,
                start_m: a.distance_m,
                end_m: b.distance_m,
                floor_m: volume.floor_m,
                ceiling_m: volume.ceiling_m,
            });
            shelf.end_m = b.distance_m;
            shelf.floor_m = shelf.floor_m.min(volume.floor_m);
            shelf.ceiling_m = shelf.ceiling_m.max(volume.ceiling_m);
        }
    }
    output.extend(active.into_values());
    output
}
/// Terrain, controlled shelves, and conflict legs all come from the existing plan commands.
pub(super) fn paint(ui: &egui::Ui, s: &mut Session, rect: Rect) {
    if rect.height() < 65. || rect.width() < 160. {
        return;
    }
    let p = ui.painter().with_clip_rect(rect);
    p.rect_filled(rect, 0., theme::PANEL);
    if s.map_state().opacity(Overlay::Route) == 0. {
        return;
    }
    let Ok(profile) = s.run(flightcraft_control::ProfilePlan { spacing_m: 1000. }) else {
        return;
    };
    let conflicts = s
        .run(flightcraft_control::SummaryPlan {})
        .map(|v| v.conflicts)
        .unwrap_or_default();
    let distance = profile.samples.last().map_or(0., |s| s.distance_m);
    if distance <= 0. {
        p.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "Add two waypoints to see the route profile",
            FontId::proportional(12.),
            theme::MUTED,
        );
        return;
    }
    let shelves = shelves(&profile);
    let highest = profile
        .samples
        .iter()
        .flat_map(|s| [s.altitude_m, s.terrain_m.unwrap_or(0.)])
        .chain(shelves.iter().map(|s| s.ceiling_m))
        .fold(600., f64::max);
    let top_ft = ((highest / 0.3048 * 1.15) / 1000.).ceil() * 1000.;
    let top_m = top_ft * 0.3048;
    let plot = Rect::from_min_max(
        rect.left_top() + egui::vec2(58., 26.),
        rect.right_bottom() - egui::vec2(12., 39.),
    );
    if plot.height() < 8. {
        return;
    }
    let at = |d: f64, h: f64| {
        egui::pos2(
            plot.left() + (d / distance) as f32 * plot.width(),
            plot.bottom() - (h / top_m).clamp(0., 1.) as f32 * plot.height(),
        )
    };
    let font = FontId::proportional(10.);
    for i in 0..=4 {
        let fraction = f64::from(i) / 4.;
        let feet = top_ft * fraction;
        let y = at(0., feet * 0.3048).y;
        p.line_segment(
            [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
            Stroke::new(0.5, theme::BORDER),
        );
        p.text(
            egui::pos2(plot.left() - 5., y),
            Align2::RIGHT_CENTER,
            format!("{feet:.0}"),
            font.clone(),
            theme::MUTED,
        );
        let nm = distance / 1852. * fraction;
        let x = at(distance * fraction, 0.).x;
        p.text(
            egui::pos2(x, plot.bottom() + 3.),
            Align2::CENTER_TOP,
            format!("{nm:.1}"),
            font.clone(),
            theme::MUTED,
        );
    }
    p.text(
        rect.left_top() + egui::vec2(5., 3.),
        Align2::LEFT_TOP,
        "Feet MSL",
        font.clone(),
        theme::TEXT,
    );
    p.text(
        egui::pos2(plot.center().x, plot.bottom() + 16.),
        Align2::CENTER_TOP,
        "Nautical miles",
        font.clone(),
        theme::TEXT,
    );
    for pair in profile.samples.windows(2) {
        if let [a, b] = pair
            && let (Some(ah), Some(bh)) = (a.terrain_m, b.terrain_m)
        {
            p.add(egui::Shape::convex_polygon(
                vec![
                    at(a.distance_m, 0.),
                    at(a.distance_m, ah),
                    at(b.distance_m, bh),
                    at(b.distance_m, 0.),
                ],
                theme::TERRAIN_HIGH,
                Stroke::NONE,
            ));
        }
    }
    let mut labels = Vec::<Rect>::new();
    for shelf in &shelves {
        let color = if shelf.class == AirspaceClass::C {
            theme::GLASS_GPS
        } else {
            theme::AIRSPACE_BLUE
        };
        let bounds = Rect::from_two_pos(at(shelf.start_m, shelf.floor_m), at(shelf.end_m, shelf.ceiling_m));
        p.rect_stroke(bounds, 0., Stroke::new(1., color), egui::StrokeKind::Inside);
        let caption = format!(
            "{} {:.0}–{:.0} ft",
            class_name(Some(shelf.class)),
            shelf.floor_m / 0.3048,
            shelf.ceiling_m / 0.3048
        );
        let galley = p.layout_no_wrap(caption, font.clone(), color);
        let pos = bounds.center_top() + egui::vec2(-galley.size().x * 0.5, 2.);
        let label = Rect::from_min_size(pos, galley.size());
        if plot.contains_rect(label) && labels.iter().all(|r| !r.intersects(label.expand(2.))) {
            p.rect_filled(label, 1., theme::PANEL);
            p.galley(pos, galley, color);
            labels.push(label);
        } else {
            // Narrow shelves still retain a hover label with exact limits and their identifier.
            ui.interact(
                bounds.intersect(plot),
                ui.id().with((&shelf.id, shelf.start_m.to_bits())),
                Sense::hover(),
            )
            .on_hover_text(format!(
                "{}: floor {:.0} ft MSL, ceiling {:.0} ft MSL",
                shelf.id,
                shelf.floor_m / 0.3048,
                shelf.ceiling_m / 0.3048
            ));
        }
    }
    // Draw planned altitude last so shelf outlines cannot obscure red conflict segments.
    for pair in profile.samples.windows(2) {
        if let [a, b] = pair {
            let conflict = conflicts.iter().any(|c| c.leg == a.leg);
            p.line_segment(
                [at(a.distance_m, a.altitude_m), at(b.distance_m, b.altitude_m)],
                Stroke::new(2., if conflict { theme::DANGER } else { theme::GLASS_GPS }),
            );
        }
    }
    p.text(
        egui::pos2(plot.left() + 8., rect.top() + 3.),
        Align2::LEFT_TOP,
        format!(
            "PROFILE · {:.1} NM · {} conflicts",
            distance / 1852.,
            conflicts.len()
        ),
        font.clone(),
        theme::TEXT,
    );
    let caption = conflicts
        .iter()
        .take(1)
        .map(|c| format!("Leg {}: {}", c.leg + 1, c.detail))
        .collect::<Vec<_>>()
        .join(" · ");
    p.text(
        rect.left_bottom() + egui::vec2(6., -1.),
        Align2::LEFT_BOTTOM,
        caption,
        font,
        theme::DANGER,
    );
}
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/region_data.rs"));
    use super::*;
    #[test]
    fn veils_are_omitted_and_controlled_shelves_merge() {
        require_region_data!();
        let mut s = super::super::tests::planned_session();
        let mut profile = s
            .run(flightcraft_control::ProfilePlan { spacing_m: 1000. })
            .unwrap();
        for sample in &mut profile.samples {
            sample.airspace = vec![
                ProfileAirspace {
                    id: "veil".into(),
                    class: None,
                    floor_m: 0.,
                    ceiling_m: 3048.,
                },
                ProfileAirspace {
                    id: "shelf".into(),
                    class: Some(AirspaceClass::B),
                    floor_m: 600.,
                    ceiling_m: 1500.,
                },
            ];
        }
        let result = shelves(&profile);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "shelf");
        assert_eq!(result[0].floor_m, 600.);
        assert_eq!(result[0].ceiling_m, 1500.);
        assert!(result[0].end_m > result[0].start_m);
    }
}
