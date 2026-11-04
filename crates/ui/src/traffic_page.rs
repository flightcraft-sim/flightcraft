//! The Traffic screen page: a heading-up radar around the aircraft and the nearby traffic by range.
//! It reads the same merged target list, symbol classes, and controller highlight as the glass
//! avionics and the moving map, so all three agree with what the controller said.
use crate::theme;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, vec2};
use flightcraft_control::{Session, TrafficClass, TrafficFilter, TrafficTarget};

const RANGES_NM: [f64; 6] = [2., 5., 10., 20., 40., 80.];
#[derive(Debug)]
pub struct TrafficPage {
    range_nm: f64,
    filter: TrafficFilter,
    selected: Option<u32>,
}
impl Default for TrafficPage {
    fn default() -> Self {
        Self {
            range_nm: 10.,
            filter: TrafficFilter::Unrestricted,
            selected: None,
        }
    }
}
fn ink(t: &TrafficTarget) -> Color32 {
    match t.class {
        TrafficClass::Alert => theme::AMBER,
        TrafficClass::Proximate => theme::TEXT,
        TrafficClass::Other => theme::MUTED,
    }
}
/// Heading-up screen offset, in units of the outer range, for a target.
pub fn heading_up(t: &TrafficTarget, heading_true_deg: f64, range_nm: f64) -> [f64; 2] {
    let angle = (t.bearing_true_deg - heading_true_deg).to_radians();
    let r = t.range_nm / range_nm;
    [r * angle.sin(), -r * angle.cos()]
}
impl TrafficPage {
    pub fn show(&mut self, ui: &mut egui::Ui, s: &Session) {
        let heading = s.sim().readout().heading_grid_rad + s.sim().heading_reference().grid_north_true_rad;
        let heading_deg = heading.to_degrees().rem_euclid(360.);
        let targets: Vec<TrafficTarget> = s
            .traffic_targets()
            .into_iter()
            .filter(|t| self.filter.shows(t.relative_altitude_ft) || t.class == TrafficClass::Alert)
            .collect();
        ui.horizontal_wrapped(|ui| {
            ui.heading("Traffic");
            for r in RANGES_NM {
                if ui
                    .selectable_label(self.range_nm == r, format!("{r:.0} NM"))
                    .clicked()
                {
                    self.range_nm = r;
                }
            }
            ui.separator();
            for f in TrafficFilter::ALL {
                if ui.selectable_label(self.filter == f, f.label()).clicked() {
                    self.filter = f;
                }
            }
            ui.label(format!(
                "{} aircraft · heading up {:03.0}°",
                targets.len(),
                heading_deg
            ));
        });
        let wide = ui.available_width() > ui.available_height() * 1.25;
        let size = if wide {
            vec2(
                ui.available_height().min(ui.available_width() * 0.5),
                ui.available_height(),
            )
        } else {
            vec2(ui.available_width(), (ui.available_height() * 0.55).max(120.))
        };
        let layout = if wide {
            egui::Layout::left_to_right(egui::Align::TOP)
        } else {
            egui::Layout::top_down(egui::Align::LEFT)
        };
        ui.with_layout(layout, |ui| {
            self.radar(ui, size, &targets, heading_deg);
            egui::ScrollArea::vertical()
                .id_salt("traffic_list")
                .auto_shrink([false, false])
                .show(ui, |ui| self.list(ui, &targets));
        });
    }

    fn radar(&mut self, ui: &mut egui::Ui, size: egui::Vec2, targets: &[TrafficTarget], heading_deg: f64) {
        let (response, p) = ui.allocate_painter(size, Sense::click());
        let rect = response.rect;
        p.rect_filled(rect, 4., theme::DIAL);
        let centre = rect.center();
        let radius = (rect.width().min(rect.height()) * 0.5 - 22.).max(20.);
        for fraction in [1. / 3., 2. / 3., 1.] {
            p.circle_stroke(
                centre,
                radius * fraction as f32,
                Stroke::new(1.0_f32, theme::BORDER),
            );
        }
        p.line_segment(
            [centre - vec2(radius, 0.), centre + vec2(radius, 0.)],
            Stroke::new(0.5_f32, theme::BORDER),
        );
        p.line_segment(
            [centre - vec2(0., radius), centre + vec2(0., radius)],
            Stroke::new(0.5_f32, theme::BORDER),
        );
        p.text(
            centre + vec2(radius * 0.35, -radius * 0.35 / 1.4),
            Align2::LEFT_BOTTOM,
            format!("{:.0} NM", self.range_nm / 3.),
            FontId::monospace(10.),
            theme::MUTED,
        );
        p.text(
            centre + vec2(radius * 0.68, -radius * 0.68 / 1.4),
            Align2::LEFT_BOTTOM,
            format!("{:.0}", self.range_nm * 2. / 3.),
            FontId::monospace(10.),
            theme::MUTED,
        );
        p.text(
            Pos2::new(centre.x, rect.top() + 4.),
            Align2::CENTER_TOP,
            format!("{:03.0}", heading_deg),
            FontId::monospace(12.),
            theme::ACCENT,
        );
        // Ownship at the centre, pointing up.
        p.add(egui::Shape::convex_polygon(
            vec![
                centre + vec2(0., -9.),
                centre + vec2(6., 8.),
                centre + vec2(0., 4.),
                centre + vec2(-6., 8.),
            ],
            theme::ACCENT,
            Stroke::NONE,
        ));
        let mut hits: Vec<(Pos2, u32)> = Vec::new();
        for t in targets.iter().filter(|t| t.range_nm <= self.range_nm * 1.05) {
            let [x, y] = heading_up(t, heading_deg, self.range_nm);
            let pos = centre + vec2(x as f32, y as f32) * radius;
            let angle = (t.track_true_deg - heading_deg).to_radians();
            let (sin, cos) = (angle.sin() as f32, angle.cos() as f32);
            let along = vec2(sin, -cos);
            let across = vec2(cos, sin);
            let colour = ink(t);
            let outline = vec![
                pos + along * 10.,
                pos - along * 7. + across * 6.,
                pos - along * 3.,
                pos - along * 7. - across * 6.,
            ];
            match t.class {
                TrafficClass::Other => {
                    p.add(egui::Shape::closed_line(outline, Stroke::new(1.5_f32, colour)));
                }
                _ => {
                    p.add(egui::Shape::convex_polygon(outline, colour, Stroke::NONE));
                }
            }
            if t.highlighted || self.selected == Some(t.address) {
                let pulse = 0.5 + 0.5 * (s_time(ui) * 4.).sin() as f32;
                p.circle_stroke(
                    pos,
                    16.,
                    Stroke::new(2.0_f32, theme::GLASS_GPS.linear_multiply(0.5 + 0.5 * pulse)),
                );
            }
            let climb = match t.trend() {
                1 => "↑",
                -1 => "↓",
                _ => "",
            };
            p.text(
                pos + vec2(13., -2.),
                Align2::LEFT_BOTTOM,
                t.short_label(),
                FontId::proportional(11.),
                colour,
            );
            p.text(
                pos + vec2(13., -1.),
                Align2::LEFT_TOP,
                format!("{}{}", t.altitude_tag(), climb),
                FontId::monospace(10.),
                colour,
            );
            hits.push((pos, t.address));
        }
        if response.clicked()
            && let Some(at) = response.interact_pointer_pos()
        {
            self.selected = hits
                .iter()
                .filter(|(pos, _)| pos.distance(at) < 18.)
                .min_by(|a, b| a.0.distance(at).total_cmp(&b.0.distance(at)))
                .map(|(_, a)| *a);
        }
        if let Some(at) = response.hover_pos()
            && let Some((_, address)) = hits.iter().find(|(pos, _)| pos.distance(at) < 14.)
            && let Some(t) = targets.iter().find(|t| t.address == *address)
        {
            response.clone().on_hover_text_at_pointer(t.details());
        }
        let _: Rect = rect;
    }

    fn list(&mut self, ui: &mut egui::Ui, targets: &[TrafficTarget]) {
        egui::Grid::new("traffic_rows")
            .striped(true)
            .num_columns(8)
            .show(ui, |ui| {
                for head in [
                    "Aircraft",
                    "Type",
                    "Rng NM",
                    "Brg",
                    "Rel alt",
                    "GS kt",
                    "V/S",
                    "Frequency / intent",
                ] {
                    ui.label(egui::RichText::new(head).color(theme::MUTED).small());
                }
                ui.end_row();
                for t in targets {
                    let selected = self.selected == Some(t.address);
                    let colour = if t.highlighted { theme::GLASS_GPS } else { ink(t) };
                    let name = ui.selectable_label(
                        selected || t.highlighted,
                        egui::RichText::new(format!(
                            "{}{}",
                            t.registration,
                            match t.class {
                                TrafficClass::Alert => "  ALERT",
                                _ => "",
                            }
                        ))
                        .color(colour),
                    );
                    if name.clicked() {
                        self.selected = if selected { None } else { Some(t.address) };
                    }
                    name.on_hover_text(t.details());
                    ui.label(egui::RichText::new(&t.type_designator).color(colour));
                    ui.label(
                        egui::RichText::new(format!("{:.1}", t.range_nm))
                            .monospace()
                            .color(colour),
                    );
                    ui.label(
                        egui::RichText::new(format!("{:03.0}", t.bearing_true_deg))
                            .monospace()
                            .color(colour),
                    );
                    ui.label(egui::RichText::new(t.altitude_tag()).monospace().color(colour));
                    ui.label(
                        egui::RichText::new(format!("{:.0}", t.ground_speed_kt))
                            .monospace()
                            .color(colour),
                    );
                    ui.label(
                        egui::RichText::new(format!("{:+.0}", t.vertical_rate_fpm))
                            .monospace()
                            .color(colour),
                    );
                    ui.label(
                        egui::RichText::new(match (t.frequency_khz, &t.intent) {
                            (Some(khz), Some(intent)) => format!("{:.3}  {intent}", f64::from(khz) / 1000.),
                            (None, Some(intent)) => intent.clone(),
                            _ => String::new(),
                        })
                        .color(theme::MUTED),
                    );
                    ui.end_row();
                }
            });
        if targets.is_empty() {
            ui.label("No traffic in range. Start some with the traffic.density command.");
        }
        if let Some(t) = self
            .selected
            .and_then(|a| targets.iter().find(|t| t.address == a))
        {
            ui.separator();
            ui.label(t.details());
        }
    }
}
fn s_time(ui: &egui::Ui) -> f64 {
    ui.ctx().input(|i| i.time)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use serde_json::json;
    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/atc_fixture.rs"));

    fn target(bearing: f64, range: f64) -> TrafficTarget {
        serde_json::from_value(json!({
            "address": 1, "fleet_id": 1, "origin": "fleet", "callsign": "Cessna One", "registration": "N1",
            "aircraft_type": "Cessna", "type_designator": "C172", "latitude_deg": 0.0, "longitude_deg": 0.0,
            "position_m": [0.0, 0.0, 0.0], "altitude_msl_ft": 3000.0, "track_true_deg": 0.0,
            "track_grid_rad": 0.0, "ground_speed_kt": 100.0, "vertical_rate_fpm": 0.0, "on_ground": false,
            "range_nm": range, "bearing_true_deg": bearing, "relative_altitude_ft": 0.0, "class": "other",
            "closing": false, "squawk": null, "frequency_khz": null, "facility": null, "intent": null,
            "assigned_altitude_ft": null, "assigned_heading_deg": null, "assigned_speed_kt": null,
            "highlighted": false, "highlight_remaining_s": 0.0
        }))
        .unwrap()
    }

    #[test]
    fn radar_is_heading_up() {
        let close = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9;
        // Straight ahead is the top of the screen, half way out at half range.
        assert!(close(heading_up(&target(90., 5.), 90., 10.), [0., -0.5]));
        // Dead right is 90 degrees clockwise from the nose; behind is the bottom.
        assert!(close(heading_up(&target(180., 10.), 90., 10.), [1., 0.]));
        assert!(close(heading_up(&target(270., 10.), 90., 10.), [0., 1.]));
        assert!(close(heading_up(&target(0., 10.), 90., 10.), [-1., 0.]));
    }

    fn render_text(page: &mut TrafficPage, s: &Session) -> Vec<String> {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut text = Vec::new();
        for _ in 0..2 {
            text.clear();
            let out = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000., 700.))),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| page.show(ui, s));
                },
            );
            fn collect(shape: &egui::Shape, text: &mut Vec<String>) {
                match shape {
                    egui::Shape::Text(t) => text.push(t.galley.text().into()),
                    egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, text)),
                    _ => {}
                }
            }
            for shape in &out.shapes {
                collect(&shape.shape, &mut text);
            }
        }
        text
    }
    fn assert_rows_in_range_order(targets: &[TrafficTarget], text: &[String]) {
        // Unrestricted filter by default: every aircraft inside 9,900 ft is listed.
        let listed: Vec<&TrafficTarget> = targets
            .iter()
            .filter(|t| TrafficFilter::Unrestricted.shows(t.relative_altitude_ft))
            .collect();
        assert!(!listed.is_empty());
        let rows: Vec<usize> = listed
            .iter()
            .filter_map(|t| text.iter().position(|x| x.starts_with(&t.registration)))
            .collect();
        assert_eq!(rows.len(), listed.len(), "every listed aircraft has a row");
        // Rows are in range order.
        let order: Vec<usize> = listed
            .iter()
            .map(|t| text.iter().rposition(|x| x.starts_with(&t.registration)).unwrap())
            .collect();
        assert!(order.windows(2).all(|w| w[0] <= w[1]), "{order:?}");
        assert!(text.iter().any(|t| t.contains("heading up")));
    }

    #[test]
    fn page_lists_nearby_traffic_by_range() {
        let mut s = atc_session("KABE");
        s.execute("traffic.density", json!({"density":"busy","airport":"KABE"}))
            .unwrap();
        let targets = s.traffic_targets();
        assert!(!targets.is_empty());
        let text = render_text(&mut TrafficPage::default(), &s);
        assert_rows_in_range_order(&targets, &text);
    }

    #[test]
    fn a_paired_browser_screen_renders_the_same_rows_from_its_snapshot() {
        let mut owner = atc_session("KABE");
        owner
            .execute("traffic.density", json!({"density":"busy","airport":"KABE"}))
            .unwrap();
        let snapshot = owner.screen_snapshot();
        assert!(!snapshot.traffic.is_empty());
        let wire = serde_json::to_string(&snapshot).unwrap();
        // The remote copy has no fleet of its own: only the snapshot feeds the page.
        let mut remote = atc_session("KABE");
        assert!(remote.traffic_targets().is_empty());
        remote
            .apply_screen_snapshot(serde_json::from_str(&wire).unwrap())
            .unwrap();
        let targets = remote.traffic_targets();
        assert_eq!(targets.len(), snapshot.traffic.len());
        let text = render_text(&mut TrafficPage::default(), &remote);
        assert_rows_in_range_order(&targets, &text);
        let owner_text = render_text(&mut TrafficPage::default(), &owner);
        for t in &targets {
            assert_eq!(
                text.iter().any(|x| x.starts_with(&t.registration)),
                owner_text.iter().any(|x| x.starts_with(&t.registration))
            );
        }
    }
}
