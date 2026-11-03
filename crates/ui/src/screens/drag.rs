//! Desktop-space drag state shared by independently repainting viewport canvases.
use super::*;
use std::sync::{Arc, Mutex};
#[derive(Clone, Debug)]
struct Geometry {
    outer: Rect,
    content: egui::Pos2,
    available: Rect,
    scale: [f32; 2],
    units_per_mm: [f32; 2],
    screen: Screen,
}
#[derive(Clone, Debug)]
struct Payload {
    source: u64,
    element: Element,
    grab: egui::Vec2,
    point: egui::Pos2,
}
#[derive(Default, Debug)]
pub(super) struct Transfer {
    geometry: BTreeMap<u64, Geometry>,
    active: Option<Payload>,
    completed: Option<(u64, String)>,
}
impl Transfer {
    fn target(&self) -> Option<(u64, [f32; 2], Rect)> {
        let active = self.active.as_ref()?;
        let target = self
            .geometry
            .iter()
            .find(|(id, g)| **id != active.source && g.outer.contains(active.point))?;
        let (id, g) = target;
        let source = self.geometry.get(&active.source)?;
        let [w, h] = active.element.size;
        let [su, sv] = source.units_per_mm;
        let [tu, tv] = g.units_per_mm;
        let size = egui::vec2(w / su * tu * g.scale[0], h / sv * tv * g.scale[1]);
        let grab = egui::vec2(
            active.grab.x / (w * source.scale[0]) * size.x,
            active.grab.y / (h * source.scale[1]) * size.y,
        );
        let local = active.point - g.content - g.available.min.to_vec2() - grab;
        let [cw, ch] = g.screen.layout.canvas;
        let x = (local.x / g.scale[0]).clamp(0., (cw - size.x / g.scale[0]).max(0.));
        let y = (local.y / g.scale[1]).clamp(0., (ch - size.y / g.scale[1]).max(0.));
        let outline = Rect::from_min_size(g.available.min + egui::vec2(x * g.scale[0], y * g.scale[1]), size);
        Some((*id, [x, y], outline))
    }
}
pub(super) fn track(
    ui: &egui::Ui,
    screen: &Screen,
    s: &mut Session,
    available: Rect,
    scale: [f32; 2],
    shared: &Arc<Mutex<Transfer>>,
    error: &mut Option<String>,
) {
    let viewport = ui.input(|i| i.viewport().clone());
    let outer = viewport
        .outer_rect
        .or_else(|| {
            screen
                .position
                .map(|p| Rect::from_min_size(p.into(), screen.size.into()))
        })
        .unwrap_or(available);
    let content = viewport.inner_rect.map_or(outer.min, |r| r.min);
    let mut command = None;
    if let Ok(mut state) = shared.lock() {
        state.geometry.insert(
            screen.id,
            Geometry {
                outer,
                content,
                available,
                scale,
                units_per_mm: match screen.layout.units {
                    Units::Mm => [1., 1.],
                    Units::Px => screen
                        .monitor
                        .and_then(|i| s.screens().monitors.get(i))
                        .map_or(screen.layout.pixels_per_mm, |m| m.pixels_per_mm),
                },
                screen: screen.clone(),
            },
        );
        state
            .geometry
            .retain(|id, _| s.screens().windows.iter().any(|w| w.id == *id));
        let moved = ui.input(|i| {
            i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::PointerMoved(_)
                        | egui::Event::PointerButton { .. }
                        | egui::Event::Touch { .. }
                )
            })
        });
        if moved
            && let Some(local) = ui.input(|i| i.pointer.latest_pos())
            && let Some(active) = &mut state.active
        {
            // A pointer in the target viewport takes over global tracking on entry.
            if active.source == screen.id || available.contains(local) {
                active.point = content + local.to_vec2();
            }
        }
        if ui.input(|i| {
            i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::PointerButton { pressed: false, .. }
                        | egui::Event::Touch {
                            phase: egui::TouchPhase::End,
                            ..
                        }
                )
            })
        }) && let Some((to, position, _)) = state.target()
        {
            if let Some(active) = state.active.take() {
                state.completed = Some((active.source, active.element.id.clone()));
                command = Some((active.source, to, active.element.id, position));
            }
        } else if ui.input(|i| i.pointer.any_released()) {
            state.active = None;
        }
    }
    if let Some((from, to, element, position)) = command {
        execute(
            s,
            "layout.move",
            json!({"from":from,"to":to,"element":element,"position":position}),
            error,
        );
    }
}
pub(super) fn begin(
    ui: &egui::Ui,
    screen: &Screen,
    element: &Element,
    rect: Rect,
    shared: &Arc<Mutex<Transfer>>,
) {
    if let Some(point) = ui.input(|i| i.pointer.latest_pos())
        && let Ok(mut state) = shared.lock()
    {
        let content = state
            .geometry
            .get(&screen.id)
            .map_or(egui::Pos2::ZERO, |g| g.content);
        state.completed = None;
        state.active = Some(Payload {
            source: screen.id,
            element: element.clone(),
            grab: point - rect.min,
            point: content + point.to_vec2(),
        });
    }
}
pub(super) fn completed(id: u64, element: &str, shared: &Arc<Mutex<Transfer>>) -> bool {
    shared.lock().ok().is_some_and(|state| {
        state
            .completed
            .as_ref()
            .is_some_and(|(source, item)| *source == id && item == element)
    })
}
pub(super) fn outline(ui: &egui::Ui, id: u64, shared: &Arc<Mutex<Transfer>>) {
    if let Ok(state) = shared.lock()
        && let Some((target, _, rect)) = state.target()
        && target == id
    {
        ui.painter().rect_stroke(
            rect,
            4.,
            Stroke::new(3.0_f32, theme::ACCENT),
            egui::StrokeKind::Inside,
        );
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min, egui::vec2(104., 28.)),
            2.,
            theme::DIAL,
        );
        ui.painter().text(
            rect.min + egui::vec2(8., 14.),
            Align2::LEFT_CENTER,
            "Drop here",
            FontId::proportional(16.),
            theme::TEXT,
        );
    }
}
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    #[test]
    fn two_viewport_global_drop_updates_layout_at_drop_and_survives_restart() {
        let mut s = Session::new().unwrap();
        s.execute("screen.open", json!({"page":{"kind":"map"},"size":[400,300]}))
            .unwrap();
        let mut layout = Layout::single(Page::new(PageKind::Instrument));
        layout.canvas = [400., 300.];
        let element = layout.elements.first_mut().unwrap();
        element.id = "gauge".into();
        element.position = [20., 20.];
        element.size = [80., 80.];
        s.execute("layout.set", json!({"id":0,"layout":layout})).unwrap();
        let mut dest = Layout::single(Page::new(PageKind::Map));
        dest.canvas = [400., 300.];
        dest.elements.clear();
        s.execute("layout.set", json!({"id":1,"layout":dest})).unwrap();
        let shared = Arc::new(Mutex::new(Transfer::default()));
        let mut a = Canvas {
            edit: true,
            transfer: shared.clone(),
            ..Default::default()
        };
        let mut b = Canvas {
            edit: true,
            transfer: shared.clone(),
            ..Default::default()
        };
        let ctx_a = egui::Context::default();
        let ctx_b = egui::Context::default();
        crate::theme::install(&ctx_a);
        crate::theme::install(&ctx_b);
        let mut draw = |id: u64, events: Vec<egui::Event>| {
            let window = s.screens().windows.iter().find(|w| w.id == id).unwrap().clone();
            let origin = egui::pos2(id as f32 * 500., 0.);
            let info = egui::ViewportInfo {
                outer_rect: Some(Rect::from_min_size(origin, egui::vec2(400., 300.))),
                inner_rect: Some(Rect::from_min_size(origin, egui::vec2(400., 300.))),
                ..Default::default()
            };
            let mut input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400., 300.))),
                events,
                ..Default::default()
            };
            input.viewports.insert(egui::ViewportId::ROOT, info);
            let ctx = if id == 0 { &ctx_a } else { &ctx_b };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        if id == 0 {
                            a.show(ui, &window, &mut s, true);
                        } else {
                            b.show(ui, &window, &mut s, true);
                        }
                    });
            });
        };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        draw(0, vec![]);
        draw(1, vec![]);
        draw(
            0,
            vec![
                egui::Event::PointerMoved(egui::pos2(60., 60.)),
                button(egui::pos2(60., 60.), true),
            ],
        );
        draw(0, vec![egui::Event::PointerMoved(egui::pos2(70., 60.))]);
        draw(0, vec![egui::Event::PointerMoved(egui::pos2(580., 110.))]);
        assert!(
            shared.lock().unwrap().target().is_some(),
            "{:?}",
            shared.lock().unwrap()
        );
        draw(
            1,
            vec![
                egui::Event::PointerMoved(egui::pos2(80., 110.)),
                button(egui::pos2(80., 110.), false),
            ],
        );
        let moved = s
            .screens()
            .windows
            .last()
            .unwrap()
            .layout
            .elements
            .first()
            .unwrap();
        assert_eq!(moved.id, "gauge");
        assert!(moved.position[0] < 80. && moved.position[1] < 110.);
        assert!(s.screens().windows.first().unwrap().layout.elements.is_empty());
        let saved = serde_json::to_string(s.screens()).unwrap();
        let mut restored = Session::new().unwrap();
        restored
            .restore_screens(serde_json::from_str(&saved).unwrap())
            .unwrap();
        assert_eq!(saved, serde_json::to_string(restored.screens()).unwrap());
        s.execute("layout.undo", json!({})).unwrap();
        assert_eq!(s.screens().windows.first().unwrap().layout.elements.len(), 1);
    }
}
