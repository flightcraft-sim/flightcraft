//! Small, resolution-independent instruments drawn with egui's painter.
use crate::theme as t;
use eframe::egui::{self, Align2, FontId, Painter, Pos2, Rect, Sense, Shape, Stroke, Vec2};
use flightcraft_sim::Readout;
use std::f32::consts::{PI, TAU};

fn text(p: &Painter, at: Pos2, text: impl ToString, size: f32, color: egui::Color32) {
    p.text(at, Align2::CENTER_CENTER, text, FontId::monospace(size), color);
}

fn radial(center: Pos2, radius: f32, angle: f32) -> Pos2 {
    center + Vec2::angled(angle) * radius
}

fn face(p: &Painter, center: Pos2, radius: f32) {
    p.circle_filled(center, radius + 4.0, t::BACKGROUND);
    p.circle_filled(center, radius, t::DIAL);
    p.circle_stroke(center, radius, Stroke::new(1.0_f32, t::BORDER));
}

fn needle(p: &Painter, center: Pos2, radius: f32, angle: f32) {
    let tip = radial(center, radius * 0.78, angle);
    let side = Vec2::angled(angle + PI / 2.0) * 3.0;
    p.add(Shape::convex_polygon(
        vec![center + side, tip, center - side],
        t::ACCENT,
        Stroke::NONE,
    ));
    p.circle_filled(center, 4.0, t::TEXT);
}

fn gauge(p: &Painter, rect: Rect, title: &str, value: f64, max: f64, unit: &str) {
    let radius = (rect.width() * 0.36).min(64.0);
    let center = egui::pos2(rect.center().x, rect.top() + 90.0);
    text(p, egui::pos2(center.x, rect.top() + 12.0), title, 11.0, t::MUTED);
    face(p, center, radius);
    for i in 0..=20 {
        let fraction = i as f32 / 20.0;
        let angle = (-240.0 + fraction * 300.0).to_radians();
        let major = i % 4 == 0;
        p.line_segment(
            [
                radial(center, radius - 4.0, angle),
                radial(center, radius - if major { 12.0 } else { 8.0 }, angle),
            ],
            Stroke::new(1.0_f32, t::MUTED),
        );
        if major {
            text(
                p,
                radial(center, radius - 22.0, angle),
                format!("{:.0}", f64::from(fraction) * max),
                9.0,
                t::MUTED,
            );
        }
    }
    needle(
        p,
        center,
        radius,
        (-240.0 + (value / max).clamp(0.0, 1.0) as f32 * 300.0).to_radians(),
    );
    text(p, center + egui::vec2(0.0, radius * 0.48), unit, 9.0, t::MUTED);
    text(
        p,
        egui::pos2(center.x, rect.bottom() - 10.0),
        format!("{value:03.0}  {unit}"),
        18.0,
        t::TEXT,
    );
}

/// Clip a circular polygon against the attitude horizon, without relying on rectangular clipping.
fn earth_polygon(center: Pos2, radius: f32, normal: Vec2, offset: f32) -> Vec<Pos2> {
    let ring: Vec<Pos2> = (0..=96)
        .map(|i| radial(center, radius, i as f32 / 96.0 * TAU))
        .collect();
    let mut polygon = Vec::new();
    for pair in ring.windows(2) {
        if let [a, b] = pair {
            let da = (*a - center).dot(normal) - offset;
            let db = (*b - center).dot(normal) - offset;
            if da >= 0.0 {
                polygon.push(*a);
            }
            if (da >= 0.0) != (db >= 0.0) {
                polygon.push(*a + (*b - *a) * (da / (da - db)));
            }
        }
    }
    polygon
}

fn attitude(p: &Painter, rect: Rect, readout: &Readout) {
    let radius = (rect.width() * 0.36).min(67.0);
    let center = egui::pos2(rect.center().x, rect.top() + 90.0);
    text(
        p,
        egui::pos2(center.x, rect.top() + 12.0),
        "ATTITUDE",
        11.0,
        t::MUTED,
    );
    face(p, center, radius);
    p.circle_filled(center, radius - 2.0, t::SKY);
    let roll = readout.roll_rad as f32;
    let [a, b, c] = flightcraft_render::attitude_horizon(roll, readout.pitch_rad as f32);
    let normal = egui::vec2(a, b);
    let right = egui::vec2(roll.cos(), -roll.sin());
    let offset = -c * radius;
    let earth = earth_polygon(center, radius - 2.0, normal, offset);
    if earth.len() >= 3 {
        p.add(Shape::convex_polygon(earth, t::EARTH, Stroke::NONE));
    }
    for pitch in [-20.0_f32, -10.0, 0.0, 10.0, 20.0] {
        let length = normal.length();
        if length < 1e-5 {
            continue;
        }
        let y = offset / length
            - pitch.to_radians() * radius / (flightcraft_render::VERTICAL_FOV_RAD * 0.5).tan();
        if y.abs() < radius - 6.0 {
            let half = if pitch == 0.0 {
                ((radius - 3.0).powi(2) - y.powi(2)).sqrt()
            } else {
                17.0
            };
            let at = center + normal.normalized() * y;
            p.line_segment(
                [at - right * half, at + right * half],
                Stroke::new(1.0_f32, t::TEXT),
            );
        }
    }
    p.line_segment(
        [center - egui::vec2(34.0, 0.0), center - egui::vec2(10.0, 0.0)],
        Stroke::new(3.0_f32, t::AMBER),
    );
    p.line_segment(
        [center + egui::vec2(10.0, 0.0), center + egui::vec2(34.0, 0.0)],
        Stroke::new(3.0_f32, t::AMBER),
    );
    p.circle_stroke(center, 4.0, Stroke::new(2.0_f32, t::AMBER));
    for angle in [-60.0_f32, -30.0, 0.0, 30.0, 60.0] {
        let a = (angle - 90.0).to_radians();
        p.line_segment(
            [radial(center, radius - 2.0, a), radial(center, radius - 9.0, a)],
            Stroke::new(2.0_f32, t::TEXT),
        );
    }
    text(
        p,
        egui::pos2(center.x, rect.bottom() - 10.0),
        format!(
            "{:+.0}°  /  {:+.0}°",
            readout.pitch_rad.to_degrees(),
            readout.roll_rad.to_degrees()
        ),
        14.0,
        t::TEXT,
    );
}

fn heading(p: &Painter, rect: Rect, heading: f64) {
    let radius = (rect.width() * 0.36).min(64.0);
    let center = egui::pos2(rect.center().x, rect.top() + 90.0);
    text(
        p,
        egui::pos2(center.x, rect.top() + 12.0),
        "HEADING",
        11.0,
        t::MUTED,
    );
    face(p, center, radius);
    for i in 0..36 {
        let angle = i as f32 * TAU / 36.0 - PI / 2.0 - heading as f32;
        p.line_segment(
            [
                radial(center, radius - 4.0, angle),
                radial(center, radius - if i % 3 == 0 { 12.0 } else { 8.0 }, angle),
            ],
            Stroke::new(1.0_f32, t::MUTED),
        );
        if i % 9 == 0 {
            let label = match i {
                0 => "N",
                9 => "E",
                18 => "S",
                _ => "W",
            };
            text(
                p,
                radial(center, radius - 24.0, angle),
                label,
                12.0,
                if i == 0 { t::ACCENT } else { t::TEXT },
            );
        }
    }
    p.line_segment(
        [center + egui::vec2(0.0, 18.0), center - egui::vec2(0.0, 21.0)],
        Stroke::new(2.0_f32, t::AMBER),
    );
    p.line_segment(
        [center - egui::vec2(16.0, 0.0), center + egui::vec2(16.0, 0.0)],
        Stroke::new(2.0_f32, t::AMBER),
    );
    p.add(Shape::convex_polygon(
        vec![
            center + egui::vec2(-4.0, -radius - 2.0),
            center + egui::vec2(4.0, -radius - 2.0),
            center + egui::vec2(0.0, -radius + 5.0),
        ],
        t::AMBER,
        Stroke::NONE,
    ));
    let degrees = heading.to_degrees().rem_euclid(360.0);
    text(
        p,
        egui::pos2(center.x, rect.bottom() - 10.0),
        format!("{:03.0}°", if degrees < 0.5 { 360.0 } else { degrees }),
        18.0,
        t::TEXT,
    );
}

fn vertical_speed(p: &Painter, rect: Rect, speed: f64) {
    let center = egui::pos2(rect.center().x, rect.top() + 90.0);
    let radius = (rect.width() * 0.36).min(64.0);
    text(
        p,
        egui::pos2(center.x, rect.top() + 12.0),
        "VERTICAL SPEED",
        11.0,
        t::MUTED,
    );
    face(p, center, radius);
    for i in -10..=10 {
        let angle = PI + i as f32 * PI / 12.0;
        p.line_segment(
            [
                radial(center, radius - 4.0, angle),
                radial(center, radius - if i % 5 == 0 { 12.0 } else { 8.0 }, angle),
            ],
            Stroke::new(1.0_f32, t::MUTED),
        );
    }
    text(p, center + egui::vec2(14.0, -28.0), "+2", 10.0, t::MUTED);
    text(p, center + egui::vec2(14.0, 28.0), "−2", 10.0, t::MUTED);
    needle(
        p,
        center,
        radius,
        PI + (speed / 2000.0).clamp(-1.0, 1.0) as f32 * PI * 0.83,
    );
    text(
        p,
        egui::pos2(center.x, rect.bottom() - 10.0),
        format!("{speed:+.0} ft/min"),
        16.0,
        t::TEXT,
    );
}

/// Exact critically damped spring response for a held input. Presentation only;
/// simulation time makes pauses, playback rates, and frame subdivisions agree.
#[derive(Default)]
pub(crate) struct SlipBall {
    position: f64,
    velocity: f64,
    time_s: Option<f64>,
}

impl SlipBall {
    pub fn update(&mut self, r: &Readout) -> f32 {
        let target = (-r.lateral_acceleration_g / 0.3).clamp(-1.0, 1.0);
        let previous = self.time_s.replace(r.time_s);
        if let Some(previous) = previous.filter(|previous| *previous <= r.time_s) {
            let dt = r.time_s - previous;
            let omega = 7.0;
            let offset = self.position - target;
            let c = self.velocity + omega * offset;
            let decay = (-omega * dt).exp();
            self.position = target + (offset + c * dt) * decay;
            self.velocity = (self.velocity - omega * c * dt) * decay;
        } else {
            self.position = target;
            self.velocity = 0.0;
        }
        self.position.clamp(-1.0, 1.0) as f32
    }
}

const STANDARD_RATE_RPS: f64 = std::f64::consts::PI / 60.0;

fn turn_symbol_angle(rate: f64) -> f32 {
    (rate / STANDARD_RATE_RPS).clamp(-2.0, 2.0) as f32 * PI / 6.0
}

fn turn_coordinator(p: &Painter, rect: Rect, rate: f64, ball: f32) {
    let center = rect.center();
    let radius = 67.0;
    text(
        p,
        egui::pos2(center.x, rect.top() + 12.0),
        "TURN COORDINATOR",
        11.0,
        t::MUTED,
    );
    face(p, center, radius);
    let pivot = center - egui::vec2(0.0, 12.0);
    // The wings meet the +/-30 degree marks at a standard-rate, two-minute turn.
    for rate in [-STANDARD_RATE_RPS, 0.0, STANDARD_RATE_RPS] {
        let angle = turn_symbol_angle(rate);
        for side in [-1.0, 1.0] {
            p.line_segment(
                [
                    radial(pivot, side * 46.0, angle),
                    radial(pivot, side * 55.0, angle),
                ],
                Stroke::new(2.0, t::TEXT),
            );
        }
    }
    let angle = turn_symbol_angle(rate);
    let rotate = |x, y| {
        pivot
            + egui::vec2(
                x * angle.cos() - y * angle.sin(),
                x * angle.sin() + y * angle.cos(),
            )
    };
    for (a, b) in [
        ((-41.0, 0.0), (-8.0, 0.0)),
        ((-8.0, 0.0), (0.0, 5.0)),
        ((0.0, 5.0), (8.0, 0.0)),
        ((8.0, 0.0), (41.0, 0.0)),
        ((0.0, -13.0), (0.0, 12.0)),
        ((-11.0, 10.0), (11.0, 10.0)),
    ] {
        p.line_segment([rotate(a.0, a.1), rotate(b.0, b.1)], Stroke::new(2.5, t::ACCENT));
    }
    text(p, center + egui::vec2(-46.0, -45.0), "L", 10.0, t::MUTED);
    text(p, center + egui::vec2(46.0, -45.0), "R", 10.0, t::MUTED);
    text(p, center + egui::vec2(0.0, -42.0), "2 MIN", 10.0, t::MUTED);
    // Circular-arc tube: apparent lateral force sends the ball opposite body-y force.
    let tube = |x: f32| {
        let horizontal = x * 39.0;
        center
            + egui::vec2(
                horizontal,
                38.0 + (75.0_f32.powi(2) - horizontal.powi(2)).sqrt() - 75.0,
            )
    };
    let points: Vec<_> = (0..=48).map(|i| tube(i as f32 / 24.0 - 1.0)).collect();
    p.add(Shape::line(points.clone(), Stroke::new(17.0, t::BORDER)));
    p.add(Shape::line(points, Stroke::new(14.0, t::RAISED)));
    for x in [-0.24, 0.24] {
        let at = tube(x);
        p.line_segment(
            [at - egui::vec2(0.0, 10.0), at + egui::vec2(0.0, 10.0)],
            Stroke::new(1.0, t::TEXT),
        );
    }
    p.circle_filled(tube(ball), 6.0, t::TEXT);
    p.circle_stroke(tube(ball), 6.0, Stroke::new(1.0, t::BACKGROUND));
    text(
        p,
        egui::pos2(center.x, rect.bottom() - 10.0),
        format!("{:+.1}°/s", rate.to_degrees()),
        14.0,
        t::TEXT,
    );
}

/// Preserve at least 230 points of scene content at the minimum desktop size.
pub(crate) fn panel_height(window_height: f32, compact: bool) -> f32 {
    if compact {
        220.0
    } else {
        (window_height * 0.46).clamp(300.0, 380.0)
    }
}

struct PanelLayout {
    // Reading order is the basic T, also used by the optional single row.
    dials: [Rect; 6],
    systems: Rect,
}

impl PanelLayout {
    fn new(rect: Rect, compact: bool) -> Self {
        let systems_width = (rect.width() * 0.22).clamp(190.0, 245.0);
        let columns = if compact { 6 } else { 3 };
        let rows = if compact { 1 } else { 2 };
        let width = (rect.width() - systems_width) / columns as f32;
        let height = rect.height() / rows as f32;
        Self {
            dials: std::array::from_fn(|i| {
                Rect::from_min_size(
                    rect.min + egui::vec2((i % columns) as f32 * width, (i / columns) as f32 * height),
                    egui::vec2(width, height),
                )
            }),
            systems: Rect::from_min_max(egui::pos2(rect.right() - systems_width, rect.top()), rect.max),
        }
    }
}

/// Paint in a fixed design space, then scale geometry, strokes, text, and clipping together.
fn scaled(p: &Painter, cell: Rect, size: Vec2, draw: impl FnOnce(&Painter, Rect)) {
    let scale = (cell.width() / size.x).min(cell.height() / size.y);
    if !scale.is_finite() || scale <= 0.0 {
        return;
    }
    let origin = cell.center() - size * scale * 0.5;
    let transform = egui::emath::TSTransform::new(origin.to_vec2(), scale);
    let clip = transform.inverse().mul_rect(cell.intersect(p.clip_rect()));
    let local = Painter::new(p.ctx().clone(), p.layer_id(), clip);
    let start = p.ctx().graphics_mut(|g| g.entry(p.layer_id()).next_idx());
    draw(&local, Rect::from_min_size(Pos2::ZERO, size));
    p.ctx().graphics_mut(|g| {
        let list = g.entry(p.layer_id());
        list.transform_range(start, list.next_idx(), transform);
    });
}

pub fn panel(ui: &mut egui::Ui, r: &Readout, trim: f64, compact: bool, ball: f32) {
    let (rect, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
    let layout = PanelLayout::new(rect, compact);
    let p = ui.painter();
    for (i, cell) in layout.dials.into_iter().enumerate() {
        scaled(p, cell, egui::vec2(200.0, 180.0), |p, rect| match i {
            0 => gauge(
                p,
                rect,
                "AIRSPEED",
                r.indicated_airspeed_mps * 1.943_844,
                160.0,
                "kt",
            ),
            1 => attitude(p, rect, r),
            2 => gauge(
                p,
                rect,
                "ALTITUDE · MSL",
                r.altitude_msl_m * 3.280_84,
                10000.0,
                "ft",
            ),
            3 => turn_coordinator(p, rect, r.turn_rate_rps, ball),
            4 => heading(p, rect, r.heading_rad),
            _ => vertical_speed(p, rect, r.vertical_speed_mps * 196.850_4),
        });
    }
    p.line_segment(
        [layout.systems.left_top(), layout.systems.left_bottom()],
        Stroke::new(1.0, t::BORDER),
    );
    scaled(p, layout.systems, egui::vec2(245.0, 210.0), |p, rect| {
        systems(p, rect, r, trim)
    });
}

fn systems(p: &Painter, rect: Rect, r: &Readout, trim: f64) {
    let left = rect.left();
    let systems_width = rect.width();
    let center_x = left + systems_width * 0.5;
    text(
        p,
        egui::pos2(center_x, rect.top() + 18.0),
        "AIRCRAFT SYSTEMS",
        11.0,
        t::MUTED,
    );
    for (i, label, value, color) in [
        (0.0, "THROTTLE", r.controls.throttle, t::ACCENT),
        (1.0, "FLAPS", r.effective_controls.flaps, t::TEXT),
        (2.0, "TRIM", trim, t::AMBER),
        (3.0, "BRAKES", r.controls.brake, t::TEXT),
    ] {
        let at = egui::pos2(left + 18.0, rect.top() + 35.0 + i * 24.0);
        p.text(at, Align2::LEFT_CENTER, label, FontId::monospace(10.0), t::MUTED);
        p.text(
            egui::pos2(rect.right() - 16.0, at.y),
            Align2::RIGHT_CENTER,
            format!("{:3.0}%", value * 100.0),
            FontId::monospace(14.0),
            color,
        );
        let bar = Rect::from_min_size(at + egui::vec2(0.0, 8.0), egui::vec2(systems_width - 34.0, 3.0));
        p.rect_filled(bar, 1.0, t::RAISED);
        let fraction = if label == "TRIM" {
            ((value + 0.3) / 0.6) as f32
        } else {
            value as f32
        };
        p.rect_filled(
            Rect::from_min_size(bar.min, egui::vec2(bar.width() * fraction.clamp(0.0, 1.0), 3.0)),
            1.0,
            color,
        );
    }
    let power = r.failures.engine_power_fraction;
    let engine = if power == 0.0 {
        "ENG FAIL".to_owned()
    } else {
        format!("ENG {:.0}%", power * 100.0)
    };
    text(
        p,
        egui::pos2(center_x, rect.top() + 131.0),
        format!("{engine}  {:.0} RPM", r.engine_rpm),
        11.0,
        if power == 0.0 {
            t::DANGER
        } else if power < 1.0 {
            t::AMBER
        } else {
            t::MUTED
        },
    );
    text(
        p,
        egui::pos2(center_x, rect.top() + 150.0),
        format!(
            "WIND FROM {:03.0}°  {:.0} kt{}",
            r.weather.wind_from_deg,
            r.weather.wind_speed_mps * 1.943_844,
            if r.weather.shear { " S" } else { "" }
        ),
        10.0,
        t::TEXT,
    );
    text(
        p,
        egui::pos2(center_x, rect.top() + 167.0),
        format!("TURB {:?}", r.weather.turbulence).to_uppercase(),
        10.0,
        t::MUTED,
    );
    let jam = |name: &str, position: Option<f64>| {
        position.map_or_else(|| format!("{name} FREE"), |v| format!("{name} JAM {v:+.2}"))
    };
    for (y, a, b) in [
        (184.0, ("E", r.failures.elevator), ("A", r.failures.aileron)),
        (200.0, ("R", r.failures.rudder), ("F", r.failures.flaps)),
    ] {
        text(
            p,
            egui::pos2(center_x, rect.top() + y),
            format!("{}   {}", jam(a.0, a.1), jam(b.0, b.1)),
            10.0,
            if a.1.is_some() || b.1.is_some() {
                t::AMBER
            } else {
                t::MUTED
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_rate_marks_match_two_minute_turns() {
        assert!((STANDARD_RATE_RPS.to_degrees() - 3.0).abs() < 1e-12);
        for direction in [-1.0, 1.0] {
            assert!(
                (turn_symbol_angle(direction * STANDARD_RATE_RPS) - direction as f32 * PI / 6.0).abs() < 1e-6
            );
            assert_eq!(turn_symbol_angle(direction * 100.0), direction as f32 * PI / 3.0);
        }
    }

    #[test]
    fn ball_damping_agrees_across_frame_rates_and_freezes_when_paused() {
        let sample = |time_s, lateral_acceleration_g| Readout {
            time_s,
            lateral_acceleration_g,
            ..Readout::default()
        };
        let mut coarse = SlipBall::default();
        let mut fine = SlipBall::default();
        coarse.update(&sample(0.0, 0.0));
        fine.update(&sample(0.0, 0.0));
        let at_one_second = coarse.update(&sample(1.0, 0.15));
        for i in 1..=120 {
            fine.update(&sample(f64::from(i) / 120.0, 0.15));
        }
        assert!((coarse.position - fine.position).abs() < 1e-12);
        assert!((coarse.velocity - fine.velocity).abs() < 1e-12);
        assert!(at_one_second < -0.49 && at_one_second > -0.5);
        assert_eq!(coarse.update(&sample(1.0, -0.15)), at_one_second);
        assert!(
            coarse.update(&sample(1.1, -0.15)) < 0.0,
            "the ball must not jump across the tube"
        );
        assert!(coarse.update(&sample(3.0, -0.15)) > 0.49);
        assert_eq!(
            coarse.update(&sample(0.0, 0.0)),
            0.0,
            "reset discards spring memory"
        );
        assert_eq!(coarse.update(&sample(100.0, 10.0)), -1.0);
    }

    #[test]
    fn basic_t_and_compact_layouts_fit_both_window_sizes() {
        for (width, height) in [(1280.0, 800.0), (1000.0, 640.0)] {
            for compact in [false, true] {
                let panel_height = panel_height(height, compact);
                // Match the header, footer, panel margin, and central-panel margin in the app.
                let scene_height = height - 64.0 - 30.0 - panel_height - 16.0;
                assert!(scene_height >= 230.0);
                let rect = Rect::from_min_size(
                    egui::pos2(12.0, height - 30.0 - panel_height + 5.0),
                    egui::vec2(width - 24.0, panel_height - 10.0),
                );
                let layout = PanelLayout::new(rect, compact);
                let cells: Vec<_> = layout.dials.into_iter().chain([layout.systems]).collect();
                for (i, a) in cells.iter().enumerate() {
                    assert!(rect.expand(0.001).contains_rect(*a));
                    for b in cells.iter().skip(i + 1) {
                        let overlap = a.intersect(*b);
                        assert!(overlap.width() <= 0.001 || overlap.height() <= 0.001);
                    }
                }
                let [airspeed, attitude, altimeter, turn, heading, vsi] = layout.dials;
                assert!(
                    airspeed.center().x < attitude.center().x && attitude.center().x < altimeter.center().x
                );
                assert!(turn.center().x < heading.center().x && heading.center().x < vsi.center().x);
                if compact {
                    assert_eq!(airspeed.center().y, vsi.center().y);
                } else {
                    assert_eq!(airspeed.center().x, turn.center().x);
                    assert_eq!(attitude.center().x, heading.center().x);
                    assert_eq!(altimeter.center().x, vsi.center().x);
                    assert!(attitude.bottom() <= heading.top());
                }
            }
        }
    }

    #[test]
    fn painted_instruments_stay_inside_their_cells() {
        let ctx = egui::Context::default();
        t::install(&ctx);
        let r = Readout {
            indicated_airspeed_mps: 40.0,
            altitude_msl_m: 1000.0,
            turn_rate_rps: STANDARD_RATE_RPS,
            roll_rad: 0.3,
            ..Readout::default()
        };
        for (width, height) in [(1280.0, 800.0), (1000.0, 640.0)] {
            for compact in [false, true] {
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(width, height))),
                        ..egui::RawInput::default()
                    },
                    |ctx| {
                        egui::TopBottomPanel::bottom("instruments")
                            .exact_height(panel_height(height, compact))
                            .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 5)))
                            .show(ctx, |ui| panel(ui, &r, 0.0, compact, 1.0));
                    },
                );
                assert!(output.shapes.len() > 100);
                for shape in output.shapes {
                    let bounds = shape.shape.visual_bounding_rect();
                    if bounds.is_positive() {
                        assert!(
                            shape.clip_rect.expand(1.0).contains_rect(bounds),
                            "{width} × {height}, compact={compact}: {bounds:?} outside {:?}",
                            shape.clip_rect
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn attitude_earth_clip_handles_inverted_and_vertical_flight() {
        let earth = |roll, pitch| {
            let [a, b, c] = flightcraft_render::attitude_horizon(roll, pitch);
            earth_polygon(Pos2::ZERO, 60.0, egui::vec2(a, b), -c * 60.0)
        };
        let level = earth(0.0, 0.0);
        assert!(level.iter().all(|p| p.y >= -1e-4));
        let inverted = earth(PI, 0.0);
        assert!(inverted.iter().all(|p| p.y <= 1e-4));
        assert!(earth(1.2, PI / 2.0).is_empty());
        assert!(earth(1.2, -PI / 2.0).len() >= 96);
    }
}
