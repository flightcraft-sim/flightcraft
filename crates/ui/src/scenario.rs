//! Scenario picker, instructor controls, and an aircraft-data emergency debrief.
use crate::theme;
use eframe::egui::{self, RichText};
use flightcraft_control::{CommandError, Session, scenario::*};
#[derive(Default)]
pub struct ScenarioUi {
    pub open: bool,
    selected: usize,
    failure: usize,
    minutes: f64,
    seed: u64,
    message: String,
}
const FAILURES: &[(&str, &str)] = &[
    ("Engine power loss", "engine"),
    ("Engine fire", "engine_fire"),
    ("Electrical fire", "electrical_fire"),
    ("Cabin smoke", "cabin_smoke"),
    ("Door open", "door_open"),
    ("Carburettor icing", "carb_icing"),
    ("Airframe icing", "airframe_icing"),
    ("Flap asymmetry", "flap_asymmetry"),
    ("Trim runaway", "trim_runaway"),
    ("Trim stuck", "trim_stuck"),
    ("Both COM radios", "radio_failure"),
    ("Vacuum pump", "vacuum_pump"),
];
impl ScenarioUi {
    pub fn show(&mut self, ctx: &egui::Context, s: &mut Session) {
        let mut open = self.open;
        egui::Window::new("Scenarios and emergencies")
            .constrain_to(crate::theme::window_bounds(ctx))
            .open(&mut open)
            .default_size([900., 720.])
            .vscroll(true)
            .show(ctx, |ui| {
                if let Err(e) = self.panel(ui, s) {
                    self.message = e.to_string();
                }
            });
        self.open = open;
    }
    pub(crate) fn panel(&mut self, ui: &mut egui::Ui, s: &mut Session) -> Result<(), CommandError> {
        ui.heading("Scenario library");
        ui.label(
            RichText::new("Offline training • fictional procedures • not for navigation").color(theme::AMBER),
        );
        let docs = library()?;
        egui::ScrollArea::vertical()
            .id_salt("scenario-library")
            .max_height(260.)
            .show(ui, |ui| {
                for (i, doc) in docs.iter().enumerate() {
                    ui.selectable_value(&mut self.selected, i, &doc.title);
                }
            });
        if let Some(doc) = docs.get(self.selected) {
            ui.label(format!(
                "{} to {} • seed {}",
                doc.route.first().map_or("", |p| p.reference.as_str()),
                doc.route.last().map_or("", |p| p.reference.as_str()),
                doc.seed
            ));
            ui.label("Select a region with the scenario’s airports (New York or Eastern Pennsylvania) and position the aircraft, then start.");
            ui.horizontal(|ui| {
                if ui.button("Validate").clicked() {
                    let report = validate(doc, s);
                    self.message = if report.valid {
                        "Validated against the active region".into()
                    } else {
                        report.errors.join("\n")
                    };
                }
                if ui.button("Start on current flight").clicked() {
                    self.message = match s.run(ScenarioStart {
                        scenario: doc.clone(),
                    }) {
                        Ok(_) => "Scenario running".into(),
                        Err(e) => e.to_string(),
                    };
                }
            });
        }
        if !self.message.is_empty() {
            ui.label(RichText::new(&self.message).color(theme::AMBER));
        }
        if let Some(id) = &s.scenario_state().id {
            ui.label(format!(
                "{id} • step {} • {}",
                s.scenario_state().step,
                if s.scenario_state().complete {
                    "complete"
                } else if s.scenario_state().waiting {
                    "waiting"
                } else {
                    "armed"
                }
            ));
        }
        ui.separator();
        ui.heading("Instructor");
        egui::ComboBox::from_id_salt("instructor-failure")
            .selected_text(FAILURES.get(self.failure).map_or("Failure", |f| f.0))
            .show_ui(ui, |ui| {
                for (i, (label, _)) in FAILURES.iter().enumerate() {
                    ui.selectable_value(&mut self.failure, i, *label);
                }
            });
        ui.horizontal(|ui| {
            ui.label("Random within next");
            ui.add(
                egui::DragValue::new(&mut self.minutes)
                    .range(0.0..=60.0)
                    .suffix(" min"),
            );
            ui.label("Seed");
            ui.add(egui::DragValue::new(&mut self.seed));
        });
        ui.horizontal(|ui| {
            if ui.button("Trigger / schedule failure").clicked()
                && let Some((_,kind))=FAILURES.get(self.failure) {
                self.message=match s.execute("scenario.instructor_failure",serde_json::json!({"failure":{"kind":kind},"within_seconds":self.minutes*60.,"seed":self.seed})) {Ok(_)=>"Instructor failure armed".into(),Err(e)=>e.to_string()};
            }
            if ui.button("Clear all failures").clicked() { self.message=match s.execute("failure.clear",serde_json::json!({})) {Ok(_)=>"Failures cleared; debrief retained".into(),Err(e)=>e.to_string()}; }
        });
        indications(ui, s);
        ui.separator();
        if let Some(d) = s.scenario_state().emergency.clone() {
            debrief(ui, s, &d)?;
        } else {
            ui.label("Emergency debrief appears when an installed aircraft procedure is triggered.");
        }
        Ok(())
    }
}
/// The failures that count as an emergency indication. Pre-flight hazards (a pitot cover, chocks,
/// missing hardware) are latent until flight, so on the ground only injected failures and a
/// derived open door count; in the air the hazards' effects count too.
pub(crate) fn emergency_failures(s: &Session) -> flightcraft_aero::Failures {
    if s.sim().readout().on_ground {
        let mut f = s.sim().injected_failures();
        f.door_open |= s.sim().failures().door_open;
        f
    } else {
        s.sim().failures()
    }
}

/// Whether any failure is active, so the Emergency menu should show red.
pub(crate) fn emergency_active(s: &Session) -> bool {
    emergency_failures(s) != flightcraft_aero::Failures::default()
}

pub(crate) fn indications(ui: &mut egui::Ui, s: &Session) {
    let f = emergency_failures(s);
    for (on, label) in [
        (f.engine_fire, "ENGINE FIRE • power loss / temperature rise"),
        (f.electrical_fire, "ELECTRICAL FIRE • buses isolated"),
        (f.cabin_smoke, "CABIN SMOKE • visibility hazard"),
        (
            f.door_open,
            if s.sim().canopy().sliding {
                "CANOPY UNLATCHED • noise / increased drag"
            } else {
                "DOOR OPEN • noise / increased drag"
            },
        ),
        (f.carb_icing, "CARB ICE • monitor RPM and heat"),
        (f.airframe_icing, "ICE • reduced lift / increased drag and load"),
        (f.flap_asymmetry, "FLAP ASYMMETRY"),
        (f.trim_runaway, "TRIM RUNAWAY"),
        (f.trim_stuck, "TRIM STUCK"),
        (f.radio_failure, "COM1 / COM2 FAIL"),
        (f.pitot_blocked, "PITOT BLOCKED • airspeed unreliable"),
        (
            f.static_blocked,
            "STATIC BLOCKED • altitude and airspeed unreliable",
        ),
        (f.vacuum_pump, "VACUUM PUMP FAIL • gyro instruments"),
        (f.alternator, "ALTERNATOR FAIL • battery only"),
    ] {
        if on {
            ui.colored_label(theme::DANGER, label);
        }
    }
}
pub(crate) fn debrief(ui: &mut egui::Ui, s: &mut Session, d: &EmergencyDebrief) -> Result<(), CommandError> {
    ui.heading(format!("Emergency debrief — {}", d.procedure.title));
    ui.label(d.summary());
    if ui.button("Emergency recognized").clicked() {
        s.run(EmergencyRecord {
            observation: EmergencyObservation::Recognized,
        })?;
    }
    for item in &d.procedure.items {
        let done = d.steps.iter().find(|t| t.item == item.id);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    done.is_none(),
                    egui::Button::new(if done.is_some() { "Done" } else { "Complete" }),
                )
                .clicked()
                && let Err(e) = s.run(EmergencyRecord {
                    observation: EmergencyObservation::Checklist {
                        item: item.id.clone(),
                    },
                })
            {
                ui.label(e.to_string());
            }
            ui.label(format!(
                "{}{}",
                if item.memory { "MEMORY • " } else { "" },
                item.instruction
            ));
            if let Some(done) = done {
                ui.colored_label(
                    if done.in_order { theme::GREEN } else { theme::AMBER },
                    format!(
                        "{:.1}s{}",
                        done.elapsed_s,
                        if done.in_order { "" } else { " • out of order" }
                    ),
                );
            }
        });
    }
    glide_map(ui, s, d);
    ui.horizontal_wrapped(|ui| {
        ui.label("Choose landing area:");
        let mut open_index = 0;
        for field in &d.glide.fields {
            let label = if field.airport {
                field.id.replace(':', " runway ")
            } else {
                open_index += 1;
                format!("F{open_index}")
            };
            if ui
                .selectable_label(d.chosen_field.as_deref() == Some(&field.id), label)
                .clicked()
                && let Err(error) = s.run(EmergencyRecord {
                    observation: EmergencyObservation::ChooseField { id: field.id.clone() },
                })
            {
                ui.label(error.to_string());
            }
        }
    });
    Ok(())
}
pub(crate) fn glide_map(ui: &mut egui::Ui, s: &mut Session, d: &EmergencyDebrief) {
    crate::map::emergency::show(ui, s, d);
    ui.label("Reachable at failure • terrain candidates are unverified surfaces; inspect visually.");
    let mut open_index = 0;
    for field in &d.glide.fields {
        let prefix = if field.airport {
            String::new()
        } else {
            open_index += 1;
            format!("F{open_index} · ")
        };
        ui.label(format!("{prefix}{}", field.description(d.glide.origin_m)));
    }
    ui.label("Terrain screening: ≥1,969 ft long × 197 ft wide, local slope ≤3%, sampled along the wind. Land cover and obstacles are not verified.");
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use flightcraft_sim::hazards::GroundHazards;

    #[test]
    fn parked_hazards_are_not_an_emergency_but_a_real_failure_is() {
        let mut s = Session::new().unwrap();
        s.execute(
            "sim.reset",
            serde_json::json!({"start":{"kind":"runway"},"paused":true}),
        )
        .unwrap();
        assert!(!emergency_active(&s));
        // Covers, ropes, chocks and missing hardware are latent hazards on the ground.
        s.sim_mut().set_hazards(GroundHazards {
            pitot_cover: true,
            chocks: true,
            tie_downs: true,
            flap_hardware: true,
            aileron_hardware: true,
            ..Default::default()
        });
        assert!(s.sim().readout().on_ground);
        assert!(
            s.sim().failures().pitot_blocked,
            "the cover still blocks the pitot"
        );
        assert!(!emergency_active(&s), "but it is not an emergency");
        // A real injected failure still is, with or without the hazards.
        s.execute(
            "failure.set",
            serde_json::json!({"failure":{"kind":"engine_fire"}}),
        )
        .unwrap();
        assert!(emergency_active(&s));
        assert!(emergency_failures(&s).engine_fire);
        s.sim_mut().set_hazards(GroundHazards::default());
        assert!(emergency_active(&s));
    }

    #[test]
    fn a_blocked_pitot_injected_as_a_failure_is_listed() {
        let mut s = Session::new().unwrap();
        s.execute(
            "failure.set",
            serde_json::json!({"failure":{"kind":"pitot_blocked"}}),
        )
        .unwrap();
        assert!(emergency_active(&s));
    }
}
