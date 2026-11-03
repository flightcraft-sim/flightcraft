//! Instructor controls and measured IFR debrief; mutations use public commands.
use crate::theme;
use eframe::egui::{self, Pos2, Rect, Stroke};
use flightcraft_control::*;
#[derive(Default)]
pub(crate) struct InstrumentUi {
    selected: usize,
    failure: usize,
    earliest_s: f64,
    latest_s: f64,
    pitch_deg: f64,
    bank_deg: f64,
    message: String,
}
impl InstrumentUi {
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, s: &mut Session) {
        ui.heading("Instrument lessons");
        ui.label("FAA-S-ACS-8 · simulated instrument practice");
        if let Ok(lessons) = lesson_library() {
            egui::ComboBox::from_id_salt("ifr.lesson")
                .selected_text(
                    lessons
                        .get(self.selected)
                        .map_or("Choose lesson", |l| l.title.as_str()),
                )
                .show_ui(ui, |ui| {
                    for (i, lesson) in lessons.iter().enumerate() {
                        ui.selectable_value(&mut self.selected, i, &lesson.title);
                    }
                });
            if let Some(lesson) = lessons.get(self.selected) {
                ui.label(&lesson.briefing);
                ui.label(format!(
                    "{} · {:.0} knots · minimum {:.0} feet MSL",
                    lesson.setup.airport, lesson.setup.airspeed_kt, lesson.setup.minimum_ft
                ));
                if ui.button("Start IFR lesson").clicked() {
                    self.message = match s.run(LessonStart {
                        lesson: lesson.clone(),
                    }) {
                        Ok(_) => "Lesson ready. Review the briefing, then Resume.".into(),
                        Err(e) => e.to_string(),
                    };
                }
            }
        }
        ui.horizontal(|ui| {
            for (label, enabled) in [("Hood on", true), ("Look up", false)] {
                if ui.button(label).clicked() {
                    self.result(s.execute("view.hood", serde_json::json!({"enabled":enabled})));
                }
            }
            if ui.button("Finish and grade").clicked() {
                self.result(s.execute("lesson.finish", serde_json::json!({})));
            }
        });
        ui.separator();
        ui.label("Instrument failure");
        egui::ComboBox::from_id_salt("ifr.failure")
            .selected_text(failure_name(self.failure))
            .show_ui(ui, |ui| {
                for i in 0..3 {
                    ui.selectable_value(&mut self.failure, i, failure_name(i));
                }
            });
        ui.horizontal(|ui| {
            ui.add(
                egui::DragValue::new(&mut self.earliest_s)
                    .range(0. ..=3600.)
                    .suffix(" seconds from now"),
            );
            ui.add(
                egui::DragValue::new(&mut self.latest_s)
                    .range(self.earliest_s..=3600.)
                    .suffix(" latest"),
            );
        });
        ui.small("Equal times select an exact moment; a window selects a repeatable random moment.");
        if ui.button("Arm instrument failure").clicked() {
            self.result(s.execute("lesson.instructor_failure", serde_json::json!({"failure":match self.failure {0=>"vacuum",1=>"ahrs",_=>"display"},"earliest_s":self.earliest_s,"latest_s":self.latest_s.max(self.earliest_s),"seed":3407})));
        }
        ui.horizontal(|ui| {
            ui.add(egui::Slider::new(&mut self.pitch_deg, -25. ..=25.).text("Pitch °"));
            ui.add(egui::Slider::new(&mut self.bank_deg, -60. ..=60.).text("Bank °"));
        });
        if ui.button("Set unusual attitude and hand back").clicked() {
            self.result(s.execute(
                "lesson.instructor_attitude",
                serde_json::json!({"pitch_deg":self.pitch_deg,"bank_deg":self.bank_deg}),
            ));
        }
        ui.small("At least 2,000 feet above terrain. Setup freezes the flight under the hood; Resume begins recovery.");
        if !self.message.is_empty() {
            ui.colored_label(theme::AMBER, &self.message);
        }
        if s.lesson_debrief().title.is_some() {
            task_grades(ui, s.lesson_debrief());
        }
    }
    fn result(&mut self, result: Result<serde_json::Value, CommandError>) {
        self.message = match result {
            Ok(_) => "Instructor action applied.".into(),
            Err(e) => e.to_string(),
        };
    }
}
fn failure_name(i: usize) -> &'static str {
    match i {
        0 => "Vacuum pump · six-pack",
        1 => "AHRS · glass",
        _ => "Primary flight display · glass",
    }
}
pub(crate) fn debrief(ui: &mut egui::Ui, d: &LessonDebrief) {
    let Some(title) = &d.title else {
        return;
    };
    theme::page(ui);
    egui::Frame::new()
        .fill(theme::PANEL)
        .inner_margin(egui::Margin::symmetric(16, 12))
        .show(ui, |ui| debrief_content(ui, d, title));
}
fn debrief_content(ui: &mut egui::Ui, d: &LessonDebrief, title: &str) {
    let bounds = ui
        .available_rect_before_wrap()
        .intersect(theme::window_bounds(ui.ctx()));
    let height = bounds.height().max(640.);
    let map_height = (height * 0.36).clamp(210., 560.);
    let plot_height = ((height - map_height - 340.) / 5.).clamp(52., 180.);
    ui.heading(title);
    ui.label(if d.complete {
        "Debrief · final grades"
    } else {
        "Live instrument grading"
    });
    task_grades(ui, d);
    track(ui, d, map_height);
    ui.small("Regional procedure geometry · north up · missed approach in amber");
    bands(ui, d, plot_height, "CDI / full scale", 0.75, |s| s.cdi_scale);
    bands(ui, d, plot_height, "Glidepath / full scale", 0.75, |s| {
        s.vertical_scale.unwrap_or(0.)
    });
    bands(ui, d, plot_height, "Airspeed deviation · knots", 10., |s| {
        s.airspeed_error_kt
    });
    bands(ui, d, plot_height, "Heading deviation · degrees", 10., |s| {
        s.heading_error_deg
    });
    bands(ui, d, plot_height, "Altitude deviation · feet", 100., |s| {
        s.altitude_error_ft
    });
    ui.small("Green bands show ACS tolerances. Deviation plots retain the measured flight, including setup; grading begins at the final approach fix. Altitude bands apply to level segments, and CDI bands to the instrument final.");
    if let Some(h) = &d.hold {
        ui.label(format!(
            "Hold: {} circuits · inbound {} · maximum leg {:.1} NM · wind correction {:.0}°",
            h.laps,
            h.inbound_time_s
                .map_or("pending".into(), |t| format!("{t:.0} seconds")),
            h.outbound_distance_nm,
            h.wind_correction_deg
        ));
        for r in &h.reasons {
            ui.colored_label(theme::AMBER, r);
        }
    }
}
fn task_grades(ui: &mut egui::Ui, d: &LessonDebrief) {
    for t in &d.tasks {
        let (label, color) = match t.result {
            TaskResult::Pending => ("PENDING", theme::TEXT),
            TaskResult::Pass => ("PASS", theme::GREEN),
            TaskResult::Fail => ("FAIL", theme::DANGER),
        };
        ui.colored_label(color, format!("{label} · {} · {}", t.title, t.acs));
        if t.reasons.is_empty() && t.result == TaskResult::Pass {
            ui.small("Required outcome observed within the assessed tolerances.");
        }
        for reason in &t.reasons {
            ui.label(reason);
        }
    }
}
fn track(ui: &mut egui::Ui, d: &LessonDebrief, height: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 4., theme::PANEL);
    let points: Vec<_> = d
        .geometry
        .iter()
        .map(|g| g.position)
        .chain(d.samples.iter().filter_map(|s| s.position))
        .collect();
    let Some(origin) = points.first() else {
        return;
    };
    let cos = libm::cos(origin.latitude_deg.to_radians());
    let xy = |g: &flightcraft_world::GeoPoint| {
        (
            (g.longitude_deg - origin.longitude_deg) * cos,
            g.latitude_deg - origin.latitude_deg,
        )
    };
    let (mut lo_x, mut hi_x, mut lo_y, mut hi_y) =
        (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
    for point in &points {
        let (x, y) = xy(point);
        lo_x = lo_x.min(x);
        hi_x = hi_x.max(x);
        lo_y = lo_y.min(y);
        hi_y = hi_y.max(y);
    }
    let scale = ((rect.width() - 60.) as f64 / (hi_x - lo_x).max(0.01))
        .min((rect.height() - 60.) as f64 / (hi_y - lo_y).max(0.01));
    let project = |g: &flightcraft_world::GeoPoint| {
        let (x, y) = xy(g);
        rect.center()
            + egui::vec2(
                ((x - (hi_x + lo_x) / 2.) * scale) as f32,
                (-(y - (hi_y + lo_y) / 2.) * scale) as f32,
            )
    };
    for pair in d.geometry.windows(2) {
        if let [a, b] = pair {
            p.line_segment(
                [project(&a.position), project(&b.position)],
                Stroke::new(2.0_f32, if b.missed { theme::AMBER } else { theme::TEXT }),
            );
        }
    }
    for fix in &d.geometry {
        let at = project(&fix.position);
        p.circle_stroke(at, 4., Stroke::new(1.0_f32, theme::TEXT));
        p.text(
            at + egui::vec2(6., -8.),
            egui::Align2::LEFT_BOTTOM,
            &fix.name,
            egui::FontId::proportional(12.),
            theme::TEXT,
        );
    }
    let flown: Vec<Pos2> = d
        .samples
        .iter()
        .filter_map(|s| s.position.as_ref().map(project))
        .collect();
    if flown.len() > 1 {
        p.add(egui::Shape::line(flown, Stroke::new(2.0_f32, theme::GREEN)));
    }
}
fn bands(
    ui: &mut egui::Ui,
    d: &LessonDebrief,
    height: f32,
    label: &str,
    limit: f64,
    value: impl Fn(&InstrumentSample) -> f64,
) {
    ui.label(format!("{label} · ±{limit}"));
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 3., theme::PANEL);
    let band = Rect::from_center_size(rect.center(), egui::vec2(rect.width(), rect.height() / 2.));
    p.rect_filled(band, 0., theme::GREEN.gamma_multiply(0.12));
    for y in [band.top(), band.bottom()] {
        p.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            Stroke::new(1.0_f32, theme::GREEN),
        );
    }
    let (Some(first), Some(last)) = (d.samples.first(), d.samples.last()) else {
        return;
    };
    let span = (last.time_s - first.time_s).max(1.);
    let points: Vec<_> = d
        .samples
        .iter()
        .map(|s| {
            egui::pos2(
                rect.left() + ((s.time_s - first.time_s) / span) as f32 * rect.width(),
                rect.center().y - (value(s) / limit).clamp(-1.9, 1.9) as f32 * rect.height() / 4.,
            )
        })
        .collect();
    if points.len() > 1 {
        p.add(egui::Shape::line(points, Stroke::new(1.5_f32, theme::TEXT)));
    }
}
