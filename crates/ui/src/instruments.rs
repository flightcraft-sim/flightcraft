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
    let center = egui::pos2(rect.center().x, rect.top() + 105.0);
    text(p, egui::pos2(center.x, rect.top() + 18.0), title, 11.0, t::MUTED);
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
        egui::pos2(center.x, rect.bottom() - 25.0),
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
    let center = egui::pos2(rect.center().x, rect.top() + 105.0);
    text(
        p,
        egui::pos2(center.x, rect.top() + 18.0),
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
        egui::pos2(center.x, rect.bottom() - 25.0),
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
    let center = egui::pos2(rect.center().x, rect.top() + 105.0);
    text(
        p,
        egui::pos2(center.x, rect.top() + 18.0),
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
        egui::pos2(center.x, rect.bottom() - 25.0),
        format!("{:03.0}°", if degrees < 0.5 { 360.0 } else { degrees }),
        18.0,
        t::TEXT,
    );
}

fn vertical_speed(p: &Painter, rect: Rect, speed: f64) {
    let center = egui::pos2(rect.center().x, rect.top() + 105.0);
    let radius = (rect.width() * 0.36).min(64.0);
    text(
        p,
        egui::pos2(center.x, rect.top() + 18.0),
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
        egui::pos2(center.x, rect.bottom() - 25.0),
        format!("{speed:+.0} ft/min"),
        16.0,
        t::TEXT,
    );
}

pub fn panel(ui: &mut egui::Ui, r: &Readout, trim: f64) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 210.0), Sense::hover());
    let systems_width = (rect.width() * 0.19).clamp(150.0, 245.0);
    let width = (rect.width() - systems_width) / 5.0;
    let cell = |i: f32| {
        Rect::from_min_size(
            rect.min + egui::vec2(i * width, 0.0),
            egui::vec2(width, rect.height()),
        )
    };
    let p = ui.painter();
    gauge(
        p,
        cell(0.0),
        "AIRSPEED",
        r.indicated_airspeed_mps * 1.943_844,
        160.0,
        "kt",
    );
    attitude(p, cell(1.0), r);
    gauge(
        p,
        cell(2.0),
        "ALTITUDE · MSL",
        r.altitude_msl_m * 3.280_84,
        10000.0,
        "ft",
    );
    heading(p, cell(3.0), r.heading_rad);
    vertical_speed(p, cell(4.0), r.vertical_speed_mps * 196.850_4);
    let left = rect.left() + width * 5.0;
    p.line_segment(
        [
            egui::pos2(left, rect.top() + 12.0),
            egui::pos2(left, rect.bottom() - 12.0),
        ],
        Stroke::new(1.0_f32, t::BORDER),
    );
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
