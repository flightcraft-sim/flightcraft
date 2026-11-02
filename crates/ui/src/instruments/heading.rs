//! Shared heading marker and encoder interaction for every heading instrument.
use super::{DrawItem, DrawList, Ink};
use eframe::egui::{self, Rect, Sense};
use flightcraft_sim::avionics::AvionicsSetting;

pub(crate) fn degrees_text(radians: f64) -> String {
    let degrees = libm::round(radians.to_degrees()).rem_euclid(360.);
    format!("{:03.0}", if degrees == 0. { 0. } else { degrees })
}

/// Marker remains on the rim. An arc clamps it to the nearest edge via the shortest turn.
pub(crate) fn marker(d: &mut DrawList, center: [f32; 2], radius: f32, selected: f64, heading: f64, arc: f64) {
    let delta =
        (selected - heading + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI;
    let angle = delta.clamp(-arc, arc);
    let [cx, cy] = center;
    let point = |x: f32, y: f32| {
        let (s, c) = (libm::sin(angle) as f32, libm::cos(angle) as f32);
        [cx + x * c - y * s, cy + x * s + y * c]
    };
    // Two convex halves leave a clear notch at the selected radial.
    for sign in [-1., 1.] {
        d.items.push(DrawItem::Polygon {
            points: vec![
                point(sign * 5., -radius - 3.),
                point(0., -radius + 2.),
                point(sign * 3., -radius + 5.),
                point(sign * 6., -radius + 1.),
            ],
            ink: Ink::Needle,
        });
    }
    if delta.abs() > arc {
        let tip = point(0., -radius);
        let tail = point(-delta.signum() as f32 * 8., -radius);
        d.line(tail, tip, Ink::Needle, 2.);
    }
}
pub(crate) fn knob_art(d: &mut DrawList, center: [f32; 2], radius: f32) {
    let [x, y] = center;
    d.circle(center, radius, Ink::Inactive);
    d.circle(center, radius * 0.62, Ink::Needle);
    d.line([x, y - radius + 2.], [x, y - radius * 0.55], Ink::Text, 1.);
}
#[derive(Default)]
pub(crate) struct Knob {
    previous_event_s: Option<f64>,
    drag_points: f32,
}
impl Knob {
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, id: egui::Id, rect: Rect) -> Vec<AvionicsSetting> {
        let response = ui.interact(rect, id, Sense::click_and_drag());
        let now = ui.input(|i| i.time);
        let mut commands = Vec::new();
        if response.clicked()
            && response
                .interact_pointer_pos()
                .is_some_and(|p| p.distance(rect.center()) <= rect.width() * 0.35)
        {
            commands.push(AvionicsSetting::HeadingBugSync {});
        }
        if response.drag_started() {
            self.drag_points = 0.;
        }
        let mut detents = 0;
        if response.dragged() {
            let delta = ui.input(|i| i.pointer.delta());
            self.drag_points += delta.x - delta.y;
            detents = (self.drag_points / 5.).trunc() as i32;
            self.drag_points -= detents as f32 * 5.;
        }
        if response.hovered() {
            detents += ui.input_mut(|i| {
                let delta = i.raw_scroll_delta.y;
                i.raw_scroll_delta.y = 0.;
                if delta == 0. {
                    0
                } else {
                    (delta.abs() / 20.).round().max(1.) as i32 * delta.signum() as i32
                }
            });
        }
        if detents != 0 {
            let interval_ms = self
                .previous_event_s
                .filter(|previous| now >= *previous)
                .map(|previous| ((now - previous) * 1000.).min(u32::MAX as f64) as u32);
            self.previous_event_s = Some(now);
            commands.push(AvionicsSetting::HeadingBugStep {
                detents: detents.clamp(-360, 360),
                interval_ms,
            });
        }
        let cursor = if response.dragged() {
            egui::CursorIcon::Grabbing
        } else {
            egui::CursorIcon::Grab
        };
        response
            .on_hover_cursor(cursor)
            .on_hover_text("HDG: wheel or drag = 1° per detent, fast = 10°. Push centre to sync.");
        commands
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instruments::{InstrumentInput, Registry};
    #[test]
    fn six_pack_standby_and_glass_read_the_same_selected_heading() {
        let mut s = flightcraft_control::Session::new().unwrap();
        s.execute(
            "aircraft.load",
            serde_json::json!({"source":{"kind":"installed","id":"corvid-glass"},"reset":true}),
        )
        .unwrap();
        s.execute(
            "avionics.set",
            serde_json::json!({"setting":{"kind":"heading_bug","degrees":57.}}),
        )
        .unwrap();
        let mut systems = s.sim().systems().clone();
        let h = &mut systems.avionics.electronic_hsi;
        h.reversion_attitude.power.powered = true;
        h.reversion_attitude.attitude_valid = true;
        let mut glass = s.glass_state();
        glass.air_data.attitude_valid = true;
        for id in ["heading", "electronic_hsi", "glass_pfd"] {
            let mut instrument = Registry::default().instrument(id).unwrap();
            let draw = instrument.update(InstrumentInput::new(&systems, 0.).with_glass(&glass));
            assert!(
                draw.items
                    .iter()
                    .any(|item| matches!(item,DrawItem::Label {text,..} if text=="HDG 057")),
                "{id}"
            );
        }
    }
    #[test]
    fn marker_pins_to_an_arc_edge_and_points_the_short_way() {
        let mut draw = DrawList {
            size: [200., 180.],
            items: Vec::new(),
            indicated: 0.,
            valid: true,
        };
        marker(
            &mut draw,
            [100., 90.],
            60.,
            270_f64.to_radians(),
            0.,
            60_f64.to_radians(),
        );
        let Some(DrawItem::Line {
            a: [ax, _],
            b: [bx, _],
            ..
        }) = draw.items.last()
        else {
            panic!("missing directional marker")
        };
        assert!(bx < ax);
        assert_eq!(degrees_text(359.8_f64.to_radians()), "000");
        assert_eq!(degrees_text(1_f64.to_radians()), "001");
        assert_eq!(degrees_text(-0.0), "000");
        assert_eq!(degrees_text((-0.2_f64).to_radians()), "000");
    }
    #[test]
    fn wheel_and_drag_issue_detents_and_a_centre_push_syncs() {
        let ctx = egui::Context::default();
        let mut knob = Knob::default();
        let mut settings = Vec::new();
        let position = egui::pos2(40., 40.);
        let mut frame = |time, events| {
            let _ = ctx.run(
                egui::RawInput {
                    time: Some(time),
                    events,
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200., 180.))),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        settings.extend(knob.show(
                            ui,
                            ui.id().with("test"),
                            Rect::from_center_size(position, egui::vec2(30., 30.)),
                        ));
                    });
                },
            );
        };
        frame(0., vec![egui::Event::PointerMoved(position)]);
        frame(
            1.,
            vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0., 20.),
                modifiers: egui::Modifiers::NONE,
            }],
        );
        frame(
            1.02,
            vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0., 20.),
                modifiers: egui::Modifiers::NONE,
            }],
        );
        frame(
            2.,
            vec![egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        frame(
            2.1,
            vec![egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        frame(
            3.,
            vec![egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        frame(
            3.1,
            vec![egui::Event::PointerMoved(position + egui::vec2(20., 0.))],
        );
        frame(
            3.2,
            vec![egui::Event::PointerButton {
                pos: position + egui::vec2(20., 0.),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(
            settings
                .iter()
                .any(|s| matches!(s,AvionicsSetting::HeadingBugStep {detents,..} if detents.abs()>1))
        );
        assert!(matches!(
            settings.first(),
            Some(AvionicsSetting::HeadingBugStep {
                detents: 1,
                interval_ms: None
            })
        ));
        assert!(
            settings
                .iter()
                .any(|s| matches!(s,AvionicsSetting::HeadingBugStep {interval_ms:Some(ms),..} if *ms<80))
        );
        assert!(
            settings
                .iter()
                .any(|s| matches!(s, AvionicsSetting::HeadingBugSync {}))
        );
    }
}
