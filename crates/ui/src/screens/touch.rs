//! Per-contact captures: touching a lever never jumps its value, and fingers remain independent.
use crate::theme;
use eframe::egui::{self, Align2, FontId, Pos2, Rect, Sense, Stroke, TouchPhase};
use flightcraft_control::screens::{ControlKind, TouchControl};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Default)]
pub struct Lever {
    contacts: BTreeMap<String, (Pos2, f64)>,
    mouse_start: Option<(f64, Pos2)>,
    pub locked: bool,
    off_guard: bool,
    wheel_velocity: f64,
    touch_dragged: bool,
}
impl Lever {
    pub fn events(
        &mut self,
        events: &[egui::Event],
        rect: Rect,
        value: f64,
        config: &TouchControl,
    ) -> Option<f64> {
        self.events_at(events, rect, rect.height(), value, config)
    }
    fn events_at(
        &mut self,
        events: &[egui::Event],
        rect: Rect,
        travel: f32,
        value: f64,
        config: &TouchControl,
    ) -> Option<f64> {
        let mut next = None;
        for event in events {
            if let egui::Event::Touch {
                device_id,
                id,
                phase,
                pos,
                ..
            } = event
            {
                let key = format!("{device_id:?}:{id:?}");
                match phase {
                    TouchPhase::Start if rect.contains(*pos) && !self.locked && !config.friction_locked => {
                        self.contacts.insert(key, (*pos, value));
                    }
                    TouchPhase::Move => {
                        if let Some((origin, start)) = self.contacts.get(&key) {
                            if origin.distance(*pos) < 6. {
                                continue;
                            }
                            self.touch_dragged = true;
                            next = Some(detent(
                                *start + f64::from((origin.y - pos.y) / travel.max(1.)),
                                &config.detents,
                            ));
                        }
                    }
                    TouchPhase::End | TouchPhase::Cancel => {
                        self.contacts.remove(&key);
                    }
                    _ => (),
                }
            }
        }
        next
    }
    pub fn show(&mut self, ui: &mut egui::Ui, rect: Rect, value: f64, config: &TouchControl) -> Option<f64> {
        self.show_defined(ui, rect, value, config, None)
    }
    pub fn show_defined(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        value: f64,
        config: &TouchControl,
        definition: Option<&flightcraft_aero::TouchControlDefinition>,
    ) -> Option<f64> {
        use flightcraft_aero::TouchControlStyle as Style;
        let style = definition.map_or(
            match config.kind {
                ControlKind::Mixture => Style::RedLockPlunger,
                ControlKind::CarbHeat => Style::PushPull,
                ControlKind::Trim => Style::TrimWheel,
                ControlKind::Flaps => Style::FlapLever,
                ControlKind::FuelSelector => Style::FuelSelector,
                _ => Style::BlackPlunger,
            },
            |d| d.style,
        );
        let title = definition.map_or(config.kind.label(), |d| d.label.as_str());
        let mut config = config.clone();
        if let Some(d) = definition {
            config.detents = d.detents.iter().map(|(v, _)| *v).collect();
        }
        let p = ui.painter();
        p.rect_filled(rect, 4., theme::PANEL);
        p.text(
            rect.center_top() + egui::vec2(0., 18.),
            Align2::CENTER_CENTER,
            title,
            FontId::proportional(16.),
            theme::TEXT,
        );
        if rect.width() < 80. || rect.height() < 200. {
            p.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Enlarge control",
                FontId::proportional(12.),
                theme::TEXT,
            );
            self.contacts.clear();
            self.mouse_start = None;
            self.wheel_velocity = 0.;
            self.touch_dragged = false;
            return None;
        }
        let track = rect.shrink2(egui::vec2(18., 76.));
        let center = egui::pos2(track.center().x, track.bottom() - track.height() * value as f32);
        let cap = Rect::from_center_size(center, egui::vec2(64., 64.));
        match style {
            Style::FuelSelector => {
                let c = track.center();
                let radius = (rect.width() * 0.32)
                    .min((rect.width() * 0.5 - 38.).max(20.))
                    .min(track.height() * 0.32);
                p.circle_filled(c, radius, theme::DIAL);
                p.circle_stroke(c, radius, Stroke::new(3.0_f32, theme::MUTED));
                for (v, label) in [(0., "OFF"), (0.33, "LEFT"), (0.6, "RIGHT"), (1., "BOTH")] {
                    let a = fuel_angle(v);
                    let q = c + egui::vec2(a.cos(), a.sin()) * (radius + 14.);
                    p.text(
                        q,
                        Align2::CENTER_CENTER,
                        label,
                        FontId::monospace(12.),
                        theme::TEXT,
                    );
                }
                let angle = fuel_angle(value);
                let end = c + egui::vec2(angle.cos(), angle.sin()) * radius * 0.8;
                p.line_segment([c, end], Stroke::new(16.0_f32, theme::ACCENT));
                p.circle_filled(c, 12., theme::BORDER);
            }
            Style::TrimWheel => {
                let wheel = Rect::from_center_size(track.center(), egui::vec2(58., track.height()));
                p.rect_filled(wheel, 22., theme::DIAL);
                p.rect_stroke(
                    wheel,
                    22.,
                    Stroke::new(2.0_f32, theme::MUTED),
                    egui::StrokeKind::Inside,
                );
                for i in 0..18 {
                    let y = wheel.top() + (i as f32 + ((value as f32 * 20.) % 1.)) * wheel.height() / 18.;
                    p.line_segment(
                        [
                            egui::pos2(wheel.left() + 4., y),
                            egui::pos2(wheel.right() - 4., y),
                        ],
                        Stroke::new(2.0_f32, theme::MUTED),
                    );
                }
                p.text(
                    track.center_top() - egui::vec2(0., 22.),
                    Align2::CENTER_CENTER,
                    "NOSE UP",
                    FontId::monospace(12.),
                    theme::TEXT,
                );
                p.text(
                    track.center_bottom() + egui::vec2(0., 22.),
                    Align2::CENTER_CENTER,
                    "NOSE DOWN",
                    FontId::monospace(12.),
                    theme::TEXT,
                );
                let y = track.bottom() - track.height() * value as f32;
                p.line_segment(
                    [
                        egui::pos2(track.right() - 6., y),
                        egui::pos2(track.right() + 8., y),
                    ],
                    Stroke::new(4.0_f32, theme::ACCENT),
                );
            }
            Style::FlapLever | Style::Quadrant => {
                p.rect_filled(
                    Rect::from_center_size(track.center(), egui::vec2(12., track.height())),
                    6.,
                    theme::DIAL,
                );
                p.line_segment(
                    [track.center_bottom(), center],
                    Stroke::new(10.0_f32, theme::MUTED),
                );
                p.rect_filled(
                    Rect::from_center_size(center, egui::vec2(56., 48.)),
                    8.,
                    theme::DIAL,
                );
                p.rect_stroke(
                    Rect::from_center_size(center, egui::vec2(56., 48.)),
                    8.,
                    Stroke::new(2.0_f32, theme::TEXT),
                    egui::StrokeKind::Inside,
                );
            }
            _ => {
                p.line_segment(
                    [track.center_bottom(), center],
                    Stroke::new(10.0_f32, theme::MUTED),
                );
                let color = if style == Style::RedLockPlunger {
                    theme::DANGER
                } else {
                    theme::DIAL
                };
                p.circle_filled(center, 30., color);
                p.circle_stroke(center, 30., Stroke::new(3.0_f32, theme::MUTED));
                for y in [-16., -8., 0., 8., 16.] {
                    p.line_segment(
                        [center + egui::vec2(-23., y), center + egui::vec2(23., y)],
                        Stroke::new(1.0_f32, theme::BORDER),
                    );
                }
                if style == Style::RedLockPlunger {
                    p.circle_filled(center, 13., theme::DIAL);
                    p.rect_filled(
                        Rect::from_center_size(center, egui::vec2(18., 12.)),
                        3.,
                        theme::MUTED,
                    );
                }
            }
        }
        if let Some(d) = definition
            && (style == Style::FlapLever || style == Style::PushPull)
        {
            for (v, label) in &d.detents {
                let y = track.bottom() - track.height() * (*v as f32);
                p.text(
                    egui::pos2(rect.right() - 6., y),
                    Align2::RIGHT_CENTER,
                    label,
                    FontId::monospace(13.),
                    theme::TEXT,
                );
            }
        }
        let value_text = match style {
            Style::TrimWheel => format!("{:+.2} trim", value * 0.6 - 0.3),
            Style::FlapLever => definition
                .and_then(|d| {
                    d.detents
                        .iter()
                        .min_by(|(a, _), (b, _)| (a - value).abs().total_cmp(&(b - value).abs()))
                })
                .map_or_else(|| format!("{:.0}%", value * 100.), |(_, s)| s.clone()),
            Style::FuelSelector => if value < 0.2 {
                "OFF"
            } else if value < 0.45 {
                "LEFT"
            } else if value < 0.7 {
                "RIGHT"
            } else {
                "BOTH"
            }
            .into(),
            Style::PushPull => if value >= 0.5 { "PULLED" } else { "PUSHED" }.into(),
            _ => format!("{:.0}%", value * 100.),
        };
        p.text(
            rect.center_bottom() - egui::vec2(0., 20.),
            Align2::CENTER_CENTER,
            value_text,
            FontId::monospace(16.),
            theme::TEXT,
        );
        if !ui.is_enabled() {
            self.contacts.clear();
            self.mouse_start = None;
            self.touch_dragged = false;
            self.wheel_velocity = 0.;
            return None;
        }
        if style == Style::FuelSelector {
            let guard = Rect::from_center_size(
                rect.center_bottom() - egui::vec2(0., 60.),
                egui::vec2(rect.width() - 12., 48.),
            );
            if ui
                .put(
                    guard,
                    egui::Button::new(if self.off_guard {
                        "OFF guard lifted"
                    } else {
                        "Lift OFF guard"
                    }),
                )
                .clicked()
            {
                self.off_guard = !self.off_guard;
            }
        }
        let capture = match style {
            Style::RedLockPlunger => cap,
            Style::FuelSelector => {
                Rect::from_center_size(track.center(), egui::Vec2::splat((rect.width() - 12.).max(48.)))
            }
            _ => track.expand2(egui::vec2(18., 32.)),
        };
        let events = ui.input(|i| i.events.clone());
        let has_touch =
            events.iter().any(|e| matches!(e, egui::Event::Touch { .. })) || !self.contacts.is_empty();
        let mut next = self.events_at(&events, capture, track.height(), value, &config);
        if style == Style::FuelSelector {
            for event in &events {
                if let egui::Event::Touch {
                    id,
                    device_id,
                    phase: TouchPhase::Move,
                    pos,
                    ..
                } = event
                {
                    let key = format!("{device_id:?}:{id:?}");
                    if let Some((origin, start)) = self.contacts.get(&key)
                        && origin.distance(*pos) >= 6.
                    {
                        next = Some(fuel_value(
                            fuel_angle(*start) + angle_delta(*origin, *pos, track.center()),
                        ));
                    }
                }
            }
        }
        let response = ui.interact(capture, ui.id().with("lever"), Sense::drag());
        response.context_menu(|ui| {
            ui.set_min_height(48.);
            ui.checkbox(&mut self.locked, "Friction lock");
        });
        if !has_touch && !self.locked && !config.friction_locked {
            if response.drag_started() {
                self.mouse_start = response.interact_pointer_pos().map(|pos| (value, pos));
            }
            if response.dragged() {
                next = self.mouse_start.and_then(|(start, origin)| {
                    response.interact_pointer_pos().map(|pos| {
                        if style == Style::FuelSelector {
                            fuel_value(fuel_angle(start) + angle_delta(origin, pos, track.center()))
                        } else {
                            detent(
                                start + f64::from((origin.y - pos.y) / track.height().max(1.)),
                                &config.detents,
                            )
                        }
                    })
                });
            }
            if response.drag_stopped() {
                self.mouse_start = None;
                if style == Style::TrimWheel {
                    self.wheel_velocity =
                        f64::from(-ui.input(|i| i.pointer.velocity().y) / track.height().max(1.)) * 0.016;
                }
            }
        }
        if style == Style::TrimWheel
            && self.touch_dragged
            && events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Touch {
                        phase: TouchPhase::End,
                        ..
                    }
                )
            })
        {
            self.wheel_velocity =
                f64::from(-ui.input(|i| i.pointer.velocity().y) / track.height().max(1.)) * 0.016;
            self.touch_dragged = false;
        }
        if style == Style::TrimWheel
            && next.is_none()
            && self.wheel_velocity.abs() > 0.001
            && !self.locked
            && !config.friction_locked
        {
            next = Some((value + self.wheel_velocity).clamp(0., 1.));
            self.wheel_velocity *= 0.85;
            ui.ctx().request_repaint();
        }
        if let Some(v) = next {
            let v = if style == Style::FlapLever || style == Style::PushPull {
                config
                    .detents
                    .iter()
                    .copied()
                    .min_by(|a, b| (a - v).abs().total_cmp(&(b - v).abs()))
                    .unwrap_or(v)
            } else {
                v
            };
            if style == Style::FuelSelector && v < 0.2 {
                if !self.off_guard {
                    return None;
                }
                self.off_guard = false;
            }
            return Some(v);
        }
        None
    }
}
fn fuel_angle(value: f64) -> f32 {
    if value < 0.2 {
        std::f32::consts::FRAC_PI_2
    } else if value < 0.45 {
        std::f32::consts::PI
    } else if value < 0.7 {
        0.
    } else {
        -std::f32::consts::FRAC_PI_2
    }
}
fn fuel_value(angle: f32) -> f64 {
    [0., 0.33, 0.6, 1.]
        .into_iter()
        .min_by(|a, b| {
            let delta = |v| {
                (fuel_angle(v) - angle + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
                    - std::f32::consts::PI
            };
            delta(*a).abs().total_cmp(&delta(*b).abs())
        })
        .unwrap_or(1.)
}
fn angle_delta(origin: Pos2, pos: Pos2, center: Pos2) -> f32 {
    let a = origin - center;
    let b = pos - center;
    let delta = b.y.atan2(b.x) - a.y.atan2(a.x);
    (delta + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}
fn detent(value: f64, detents: &[f64]) -> f64 {
    let value = value.clamp(0., 1.);
    detents
        .iter()
        .copied()
        .find(|d| (*d - value).abs() < 0.025)
        .unwrap_or(value)
}
pub fn lever_command(kind: ControlKind, value: f64, item: Option<&str>) -> (&'static str, Value) {
    let value = value.clamp(0., 1.);
    match kind {
        ControlKind::Throttle => ("sim.set_controls", json!({"throttle":value})),
        ControlKind::Mixture => ("systems.set", json!({"setting":{"kind":"mixture","value":value}})),
        ControlKind::Primer => ("systems.set", json!({"setting":{"kind":"primer","charge":value}})),
        ControlKind::Magnetos => (
            "systems.set",
            json!({"setting":{"kind":"magnetos","position":if value<0.2{"off"}else if value<0.45{"left"}else if value<0.7{"right"}else{"both"}}}),
        ),
        ControlKind::CarbHeat => (
            "systems.set",
            json!({"setting":{"kind":"switch","switch":"carb_heat","on":value>=0.5}}),
        ),
        ControlKind::Trim => (
            "systems.set",
            json!({"setting":{"kind":"trim","value":value*0.6-0.3}}),
        ),
        ControlKind::Flaps => ("sim.set_controls", json!({"flaps":value})),
        ControlKind::ParkingBrake => (
            "systems.set",
            json!({"setting":{"kind":"park_brake","on":value>=0.5}}),
        ),
        ControlKind::FuelSelector => {
            let position = if value < 0.2 {
                "off"
            } else if value < 0.45 {
                "left"
            } else if value < 0.7 {
                "right"
            } else {
                "both"
            };
            (
                "systems.set",
                json!({"setting":{"kind":"fuel_selector","position":position}}),
            )
        }
        _ => (
            "equipment.set",
            json!({"setting":{"kind":"systems","setting":{"kind":"panel_control","id":item.unwrap_or(match kind{ControlKind::Prop=>"prop",_=>"trim"}),"value":value}}}),
        ),
    }
}
pub fn knob_command(kind: ControlKind, detents: f64, current: f64) -> (&'static str, Value) {
    let setting = match kind {
        ControlKind::Baro => {
            json!({"kind":"baro","pressure_pa":(current+detents*33.8639).clamp(80000.,110000.)})
        }
        ControlKind::Course => {
            json!({"kind":"course","course_rad":(current+detents.to_radians()).rem_euclid(std::f64::consts::TAU)})
        }
        ControlKind::Altitude => {
            json!({"kind":"selected_altitude","altitude_m":(current+detents*30.48).clamp(-500.,15000.)})
        }
        ControlKind::Range => {
            json!({"kind":"map_range","range_nm":(current*1.1_f64.powf(detents)).clamp(1.,200.)})
        }
        ControlKind::Fms => json!({"kind":"active_leg","index":(current+detents).max(0.) as usize}),
        _ => {
            json!({"kind":"heading_bug","heading_rad":(current+detents.to_radians()).rem_euclid(std::f64::consts::TAU)})
        }
    };
    (
        "avionics.set",
        json!({"setting":{"kind":"glass","setting":setting}}),
    )
}
pub fn knob_push(kind: ControlKind, reference: f64) -> (&'static str, Value) {
    match kind {
        ControlKind::Baro => (
            "avionics.set",
            json!({"setting":{"kind":"glass","setting":{"kind":"baro","pressure_pa":101325.}}}),
        ),
        ControlKind::Altitude => (
            "avionics.set",
            json!({"setting":{"kind":"glass","setting":{"kind":"selected_altitude","altitude_m":reference.clamp(-500.,15000.)}}}),
        ),
        ControlKind::Fms => (
            "avionics.set",
            json!({"setting":{"kind":"glass","setting":{"kind":"page","page":"flight_plan"}}}),
        ),
        ControlKind::Range => (
            "avionics.set",
            json!({"setting":{"kind":"glass","setting":{"kind":"orientation","orientation":"north_up"}}}),
        ),
        _ => ("avionics.set", json!({"setting":{"kind":"heading_bug_sync"}})),
    }
}
#[derive(Default)]
pub struct Knob {
    pub enlarged: bool,
    remainder: f32,
}
impl Knob {
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        kind: ControlKind,
        current: f64,
        reference: f64,
    ) -> Vec<(&'static str, Value)> {
        let mut commands = Vec::new();
        let turn = |ticks: f64| {
            if kind == ControlKind::Fms {
                if reference < 1. {
                    return knob_push(kind, reference);
                }
                let selected = (current + ticks).clamp(0., reference - 1.);
                knob_command(kind, selected - current, current)
            } else {
                knob_command(kind, ticks, current)
            }
        };
        let (rect, response) = ui.allocate_exact_size(egui::vec2(74., 74.), Sense::click_and_drag());
        ui.painter().circle_filled(rect.center(), 32., theme::DIAL);
        ui.painter()
            .circle_stroke(rect.center(), 32., Stroke::new(2.0_f32, theme::MUTED));
        if kind == ControlKind::Fms {
            ui.painter()
                .circle_stroke(rect.center(), 19., Stroke::new(2.0_f32, theme::MUTED));
        }
        for i in 0..16 {
            let a = i as f32 * std::f32::consts::TAU / 16.;
            let v = egui::vec2(a.cos(), a.sin());
            ui.painter().line_segment(
                [rect.center() + v * 26., rect.center() + v * 30.],
                Stroke::new(1.0_f32, theme::MUTED),
            );
        }
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            kind.label(),
            FontId::monospace(11.),
            theme::TEXT,
        );
        if !ui.is_enabled() {
            return commands;
        }
        if response.clicked() {
            commands.push(knob_push(kind, reference));
        }
        if response.drag_started() || response.clicked() {
            self.enlarged = true;
        }
        let rotation = ui
            .input(|i| i.multi_touch())
            .filter(|_| response.hovered() || response.dragged())
            .map_or(0., |t| t.rotation_delta.to_degrees());
        if response.dragged() || rotation != 0. {
            let delta = ui.input(|i| i.pointer.delta());
            let circular = ui
                .input(|i| i.pointer.latest_pos())
                .filter(|p| p.distance(rect.center()) > 20.)
                .map_or(0., |p| angle_delta(p - delta, p, rect.center()).to_degrees());
            let coarse = if kind == ControlKind::Fms
                && ui
                    .input(|i| i.pointer.latest_pos())
                    .is_some_and(|p| p.distance(rect.center()) > 20.)
            {
                5.
            } else {
                1.
            };
            self.remainder += (if circular.abs() > 0.3 {
                circular * 0.25
            } else {
                (delta.x - delta.y) * 0.15
            } + rotation)
                * coarse;
            let ticks = self.remainder.trunc();
            if ticks != 0. {
                self.remainder -= ticks;
                commands.push(turn(f64::from(ticks)));
            }
        }
        if self.enlarged {
            egui::Window::new(format!("{} knob", kind.label()))
                .id(ui.id().with("magnified-knob"))
                .default_size([300., 210.])
                .open(&mut self.enlarged)
                .show(ui.ctx(), |ui| {
                    ui.label("Drag to turn • use + / − for single detents");
                    let r = ui.add_sized([270., 85.], egui::Button::new("↶  TURN  ↷").sense(Sense::drag()));
                    if r.dragged() {
                        let ticks = ui.input(|i| i.pointer.delta().x - i.pointer.delta().y);
                        if ticks.abs() >= 1. {
                            commands.push(turn(f64::from(ticks.round())));
                        }
                    }
                    ui.horizontal(|ui| {
                        for (label, ticks) in [("−", -1.), ("+", 1.)] {
                            if ui.add_sized([76., 48.], egui::Button::new(label)).clicked() {
                                commands.push(turn(ticks));
                            }
                        }
                        if ui
                            .add_sized([100., 48.], egui::Button::new("PUSH / SYNC"))
                            .clicked()
                        {
                            commands.push(knob_push(kind, reference));
                        }
                    });
                });
        }
        commands
    }
}
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    fn touch(id: u64, phase: TouchPhase, x: f32, y: f32) -> egui::Event {
        egui::Event::Touch {
            device_id: egui::TouchDeviceId(1),
            id: egui::TouchId(id),
            phase,
            pos: egui::pos2(x, y),
            force: None,
        }
    }
    #[test]
    fn two_fingers_drag_two_levers_without_tap_jumps() {
        let a = Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(100., 200.));
        let b = a.translate(egui::vec2(120., 0.));
        let mut throttle = Lever::default();
        let mut mixture = Lever::default();
        let config = TouchControl::default();
        let start = [
            touch(1, TouchPhase::Start, 50., 150.),
            touch(2, TouchPhase::Start, 170., 150.),
        ];
        assert_eq!(throttle.events(&start, a, 0.2, &config), None);
        assert_eq!(mixture.events(&start, b, 0.5, &config), None);
        let tremor = [
            touch(1, TouchPhase::Move, 50., 148.),
            touch(2, TouchPhase::Move, 170., 148.),
        ];
        assert_eq!(throttle.events(&tremor, a, 0.2, &config), None);
        assert_eq!(mixture.events(&tremor, b, 0.5, &config), None);
        let moves = [
            touch(1, TouchPhase::Move, 50., 50.),
            touch(2, TouchPhase::Move, 170., 100.),
        ];
        assert_eq!(throttle.events(&moves, a, 0.2, &config), Some(0.7));
        assert_eq!(mixture.events(&moves, b, 0.5, &config), Some(0.75));
        let (name, input) = lever_command(ControlKind::Throttle, 0.7, None);
        let mut s = flightcraft_control::Session::new().unwrap();
        s.execute(name, input).unwrap();
        assert_eq!(s.sim().controls().throttle, 0.7);
        let (name, input) = lever_command(ControlKind::Mixture, 0.75, None);
        s.execute(name, input).unwrap();
        assert_eq!(s.sim().systems().controls.mixture, 0.75);
    }
    #[test]
    fn fuel_selector_requires_a_lifted_guard_before_rotating_to_off() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(300., 500.));
        let mut selector = Lever::default();
        let config = TouchControl {
            kind: ControlKind::FuelSelector,
            ..Default::default()
        };
        let draw = |selector: &mut Lever, events| {
            let mut next = None;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(rect),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        next = selector.show(ui, rect, 0.6, &config);
                    });
                },
            );
            next
        };
        draw(&mut selector, vec![]);
        draw(&mut selector, vec![touch(9, TouchPhase::Start, 150., 440.)]);
        assert!(
            selector.contacts.is_empty(),
            "the guard is separate from the rotary capture"
        );
        draw(&mut selector, vec![touch(9, TouchPhase::End, 150., 440.)]);
        assert_eq!(
            draw(&mut selector, vec![touch(1, TouchPhase::Start, 240., 250.)]),
            None
        );
        assert_eq!(
            draw(&mut selector, vec![touch(1, TouchPhase::Move, 150., 340.)]),
            None
        );
        draw(&mut selector, vec![touch(1, TouchPhase::End, 150., 340.)]);
        selector.off_guard = true;
        draw(&mut selector, vec![touch(2, TouchPhase::Start, 240., 250.)]);
        assert_eq!(
            draw(&mut selector, vec![touch(2, TouchPhase::Move, 150., 340.)]),
            Some(0.)
        );
        assert!(!selector.off_guard);
    }
    #[test]
    fn friction_lock_and_knob_commands() {
        let mut lever = Lever {
            locked: true,
            ..Default::default()
        };
        let events = [
            touch(1, TouchPhase::Start, 50., 150.),
            touch(1, TouchPhase::Move, 50., 10.),
        ];
        assert_eq!(
            lever.events(
                &events,
                Rect::from_min_size(Pos2::ZERO, egui::vec2(100., 200.)),
                0.5,
                &TouchControl::default()
            ),
            None
        );
        let (name, input) = knob_command(ControlKind::Heading, 3., 0.);
        let mut s = flightcraft_control::Session::new().unwrap();
        s.execute(name, input).unwrap();
        assert!((s.glass_state().settings.heading_bug_rad - 3_f64.to_radians()).abs() < 1e-12);
        let (name, input) = knob_push(ControlKind::Fms, 0.);
        s.execute(name, input).unwrap();
        assert_eq!(
            s.glass_state().settings.page,
            flightcraft_control::GlassPage::FlightPlan
        );
        let (name, input) = knob_command(ControlKind::Fms, 1., 1.);
        assert_eq!(name, "avionics.set");
        assert_eq!(input.pointer("/setting/setting/index"), Some(&json!(2)));
    }
    #[test]
    fn dragging_a_knob_emits_a_valid_heading_command_and_opens_magnifier() {
        let ctx = egui::Context::default();
        let mut knob = Knob::default();
        let mut session = flightcraft_control::Session::new().unwrap();
        let origin = egui::pos2(40., 40.);
        let button = |pressed| egui::Event::PointerButton {
            pos: origin,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        for events in [
            vec![],
            vec![egui::Event::PointerMoved(origin), button(true)],
            vec![egui::Event::PointerMoved(egui::pos2(65., 20.))],
        ] {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(640., 480.))),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let current = session.glass_state().settings.heading_bug_rad;
                        for (name, input) in knob.show(ui, ControlKind::Heading, current, 0.) {
                            session.execute(name, input).unwrap();
                        }
                    });
                },
            );
        }
        assert!(knob.enlarged);
        assert!(session.glass_state().settings.heading_bug_rad > 0.);
    }
}
