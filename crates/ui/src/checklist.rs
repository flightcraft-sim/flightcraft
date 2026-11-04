//! The checklist panel: the active normal-procedure list with the current item, its timers and
//! warnings, "Show me", and a pulsing outline on the dash control to use next. Works the same on
//! desktop and web: everything goes through commands.
use crate::theme;
use eframe::egui::{self, Color32, Rect, RichText, Stroke};
use flightcraft_control::{
    ChecklistAbort, ChecklistComplete, ChecklistItemView, ChecklistSelect, ChecklistShowMe,
    ChecklistStateCommand, ChecklistView, CommandError, ItemStatus, Session,
};
use serde_json::{Value, json};

/// Egui temp-data key (a `String` panel element id) read by the dash painter.
pub(crate) const HIGHLIGHT_KEY: &str = "checklist-highlight";

/// A pulsing outline, drawn over a dash control.
pub(crate) fn pulse(ui: &egui::Ui, rect: Rect) {
    let t = ui.input(|i| i.time) as f32;
    let a = 0.55 + 0.45 * (t * 5.).sin();
    ui.painter().rect_stroke(
        rect,
        4.,
        Stroke::new(3_f32, theme::AMBER.gamma_multiply(a)),
        egui::StrokeKind::Outside,
    );
    ui.ctx().request_repaint();
}

/// Whether another part of the app (the walkaround) has registered a command by this name; its
/// output when it has.
fn optional_command(s: &mut Session, name: &str) -> Option<Value> {
    match s.execute(name, json!({})) {
        Ok(v) => Some(v),
        Err(CommandError::Unknown(_)) => None,
        Err(_) => Some(Value::Null),
    }
}

/// The walkaround's camera framing: `preflight.state` reports `active` and a `camera` with a
/// body-frame `target_m`, `eye_m` and `fov_deg` while the walkaround is under way. The inset uses
/// it in place of its own part framing.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WalkaroundCamera {
    pub body: Option<flightcraft_aero::exterior_framing::Framing>,
    pub world: Option<([f32; 3], [f32; 3], f32)>,
}
pub(crate) fn walkaround_camera(s: &mut Session) -> Option<WalkaroundCamera> {
    let state = optional_command(s, "preflight.state")?;
    if state.get("active") != Some(&Value::Bool(true)) {
        return None;
    }
    let camera = state.get("camera")?;
    let vec3 = |key: &str| -> Option<[f32; 3]> {
        let a = camera.get(key)?.as_array()?;
        Some([
            a.first()?.as_f64()? as f32,
            a.get(1)?.as_f64()? as f32,
            a.get(2)?.as_f64()? as f32,
        ])
    };
    let fov_deg = camera.get("fov_deg").and_then(Value::as_f64).unwrap_or(50.) as f32;
    let body = vec3("eye_m").zip(vec3("target_m")).map(|(eye_m, target_m)| {
        flightcraft_aero::exterior_framing::Framing {
            eye_m,
            target_m,
            fov_deg,
        }
    });
    body.map(|b| WalkaroundCamera {
        body: Some(b),
        world: None,
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Checklist,
    Preflight,
}

#[derive(Default)]
pub(crate) struct ChecklistUi {
    tab: Tab,
    pub error: Option<String>,
}

fn marker(status: ItemStatus) -> (&'static str, Color32) {
    match status {
        ItemStatus::Done => ("[x]", theme::GREEN),
        ItemStatus::Current => (">>", theme::ACCENT),
        ItemStatus::Failed => ("[!]", theme::DANGER),
        ItemStatus::Pending => ("[ ]", theme::MUTED),
    }
}

impl ChecklistUi {
    pub fn show(&mut self, ui: &mut egui::Ui, s: &mut Session) {
        // The Preflight tab appears only when the walkaround provides `preflight.state`.
        let preflight = optional_command(s, "preflight.state");
        if preflight.is_none() {
            self.tab = Tab::Checklist;
        }
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Checklist, "Checklist");
            if preflight.is_some() {
                ui.selectable_value(&mut self.tab, Tab::Preflight, "Preflight");
            }
        });
        ui.separator();
        match (self.tab, preflight) {
            (Tab::Preflight, Some(state)) => self.preflight(ui, s, &state),
            _ => self.checklist(ui, s),
        }
        if let Some(e) = &self.error {
            ui.label(RichText::new(e).color(theme::AMBER));
        }
    }

    fn checklist(&mut self, ui: &mut egui::Ui, s: &mut Session) {
        let Ok(view) = s.run(ChecklistStateCommand {}) else {
            return;
        };
        if view.available.is_empty() {
            ui.label(RichText::new("This aircraft has no normal-procedure checklists.").color(theme::MUTED));
            return;
        }
        ui.horizontal_wrapped(|ui| {
            for list in &view.available {
                if ui
                    .selectable_label(view.active.as_ref() == Some(&list.id), &list.title)
                    .clicked()
                {
                    self.error = s
                        .run(ChecklistSelect {
                            id: Some(list.id.clone()),
                        })
                        .err()
                        .map(|e| e.to_string());
                }
            }
        });
        if view.active.is_none() {
            ui.label(RichText::new("Choose a list to begin.").color(theme::MUTED));
            return;
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new(view.title.clone().unwrap_or_default()).strong());
            ui.label(
                RichText::new(view.poh_ref.clone().unwrap_or_default())
                    .size(11.)
                    .color(theme::MUTED),
            );
        });
        // Highlight the current item's panel control on the dash.
        let current_item = view.current.and_then(|i| view.items.get(i));
        let control = current_item.and_then(|i| i.control.clone());
        ui.ctx().data_mut(|d| match &control {
            Some(c) => d.insert_temp(egui::Id::new(HIGHLIGHT_KEY), c.clone()),
            None => {
                d.remove::<String>(egui::Id::new(HIGHLIGHT_KEY));
            }
        });
        let footer = 72.;
        let height = (ui.available_height() - footer).max(60.);
        egui::ScrollArea::vertical()
            .max_height(height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (i, item) in view.items.iter().enumerate() {
                    self.item(ui, s, item, Some(i) == view.current);
                }
                if view.complete {
                    ui.label(RichText::new("Checklist complete.").color(theme::GREEN).strong());
                }
            });
        ui.separator();
        self.warnings_and_buttons(ui, s, &view, current_item);
    }

    fn warnings_and_buttons(
        &mut self,
        ui: &mut egui::Ui,
        s: &mut Session,
        view: &ChecklistView,
        current_item: Option<&ChecklistItemView>,
    ) {
        for limit in &view.active_limits {
            ui.label(RichText::new(limit).color(theme::DANGER).strong());
        }
        for w in view.warnings.iter().rev().take(2) {
            ui.label(
                RichText::new(format!("{} ({})", w.text, w.poh_ref))
                    .size(11.)
                    .color(theme::AMBER),
            );
        }
        if let Some(a) = &view.aborted {
            ui.label(RichText::new(format!("Stopped: {}", a.reason)).color(theme::ACCENT));
        }
        ui.horizontal(|ui| {
            if ui
                .button("Stop and squawk")
                .on_hover_text("Abort the procedure: the right answer to a failed check")
                .clicked()
            {
                let at = current_item.map_or("the checklist", |i| i.text.as_str());
                self.error = s
                    .run(ChecklistAbort {
                        reason: format!("Stopped at: {at}"),
                    })
                    .err()
                    .map(|e| e.to_string());
            }
            if ui.button("Restart list").clicked() {
                self.error = s
                    .run(ChecklistSelect {
                        id: view.active.clone(),
                    })
                    .err()
                    .map(|e| e.to_string());
            }
            if ui.button("Close").clicked() {
                self.error = s.run(ChecklistSelect { id: None }).err().map(|e| e.to_string());
            }
        });
    }

    fn item(&mut self, ui: &mut egui::Ui, s: &mut Session, item: &ChecklistItemView, current: bool) {
        let (mark, colour) = marker(item.status);
        let done = item.status == ItemStatus::Done;
        let failed = item.status == ItemStatus::Failed;
        let frame = egui::Frame::new()
            .fill(if current {
                theme::RAISED
            } else {
                Color32::TRANSPARENT
            })
            .inner_margin(egui::Margin::symmetric(6, 3))
            .corner_radius(4.);
        frame.show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(mark).monospace().color(colour));
                ui.label(RichText::new(&item.text).strong().color(if done {
                    theme::MUTED
                } else {
                    theme::TEXT
                }));
                ui.label(RichText::new("-").color(theme::MUTED));
                ui.label(RichText::new(&item.response).color(if done {
                    theme::MUTED
                } else {
                    theme::ACCENT
                }));
            });
            if let Some(detail) = &item.detail {
                ui.label(RichText::new(detail).size(11.).color(if failed {
                    theme::DANGER
                } else {
                    theme::MUTED
                }));
            }
            if let Some(t) = item.timer.as_ref().filter(|_| !done)
                && (current || t.elapsed_s > 0. || t.expired)
            {
                let seconds = t.seconds.max(1.);
                ui.add(
                    egui::ProgressBar::new((t.elapsed_s / seconds).clamp(0., 1.) as f32)
                        .desired_width(180.)
                        .text(format!(
                            "{} {:.0}/{:.0} s{}",
                            t.label,
                            t.elapsed_s.min(seconds * 9.),
                            seconds,
                            if t.expired { "  EXCEEDED" } else { "" }
                        ))
                        .fill(if t.expired { theme::DANGER } else { theme::ACCENT }),
                );
            }
            if current || failed {
                ui.label(RichText::new(&item.poh_ref).size(10.).color(theme::MUTED));
                if let Some(n) = &item.note {
                    ui.label(RichText::new(n).size(10.).color(theme::MUTED));
                }
                if let Some(n) = &item.unverified {
                    ui.label(RichText::new(n).size(10.).color(theme::AMBER));
                }
                self.item_buttons(ui, s, item, failed);
            }
        });
    }

    fn item_buttons(&mut self, ui: &mut egui::Ui, s: &mut Session, item: &ChecklistItemView, failed: bool) {
        ui.horizontal(|ui| {
            if !item.verifiable && ui.button("Done").clicked() {
                self.error = s
                    .run(ChecklistComplete {
                        item: item.id.clone(),
                        force: false,
                    })
                    .err()
                    .map(|e| e.to_string());
            }
            if ui
                .button("Show me")
                .on_hover_text("The virtual pilot does this item and explains it")
                .clicked()
            {
                self.error = s
                    .run(ChecklistShowMe {
                        item: item.id.clone(),
                    })
                    .err()
                    .map(|e| e.to_string());
            }
            if item.verifiable
                && failed
                && ui
                    .small_button("Continue anyway")
                    .on_hover_text("Records that you went on past a failed check")
                    .clicked()
            {
                self.error = s
                    .run(ChecklistComplete {
                        item: item.id.clone(),
                        force: true,
                    })
                    .err()
                    .map(|e| e.to_string());
            }
        });
    }

    /// The walkaround's items, from `preflight.state`. The field names are read tolerantly.
    fn preflight(&mut self, ui: &mut egui::Ui, s: &mut Session, state: &Value) {
        let stations = state
            .get("stations")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if stations.is_empty() {
            ui.label(RichText::new("No walkaround stations reported.").color(theme::MUTED));
        }
        let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_owned);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for station in &stations {
                    let name = text(station, "name")
                        .or_else(|| text(station, "title"))
                        .or_else(|| text(station, "id"))
                        .unwrap_or_else(|| "Station".into());
                    ui.label(RichText::new(name).strong());
                    let station_id = text(station, "id");
                    for item in state
                        .get("items")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter(|i| text(i, "station") == station_id)
                    {
                        let done = item.get("done") == Some(&Value::Bool(true));
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new(if done { "[x]" } else { "[ ]" })
                                    .monospace()
                                    .color(if done { theme::GREEN } else { theme::MUTED }),
                            );
                            ui.label(text(item, "text").unwrap_or_default());
                            if let Some(r) = text(item, "poh_ref") {
                                ui.label(RichText::new(r).size(10.).color(theme::MUTED));
                            }
                            if let Some(f) = item.get("finding").filter(|f| !f.is_null()) {
                                let note = f
                                    .as_str()
                                    .map(str::to_owned)
                                    .or_else(|| text(f, "text"))
                                    .or_else(|| text(f, "description"))
                                    .unwrap_or_else(|| f.to_string());
                                ui.label(RichText::new(note).color(theme::AMBER));
                            }
                            if !done
                                && let Some(id) = text(item, "id")
                                && ui.small_button("Check").clicked()
                            {
                                self.error = s
                                    .execute("preflight.perform", json!({"item": id}))
                                    .err()
                                    .map(|e| e.to_string());
                            }
                        });
                    }
                }
            });
    }
}
