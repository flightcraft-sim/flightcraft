//! Passenger-facing watching controls; every action goes through the command registry.
use crate::{
    keymap::{Action, Keymap},
    theme,
};
use eframe::egui::{self, RichText};
use flightcraft_control::*;
use serde_json::json;
pub struct CrewUi {
    last_intent: Option<String>,
    destination: String,
    runway: String,
    altitude_ft: f64,
    choice: usize,
    director: bool,
    error: Option<String>,
    map: crate::map::PlanningMap,
}
impl Default for CrewUi {
    fn default() -> Self {
        Self {
            last_intent: None,
            destination: "KABE".into(),
            runway: "24".into(),
            altitude_ft: 4500.,
            choice: 0,
            director: true,
            error: None,
            map: Default::default(),
        }
    }
}
impl CrewUi {
    #[cfg(test)]
    pub(crate) fn scenic_preview() -> Self {
        Self {
            choice: 1,
            destination: "1N7".into(),
            runway: "07".into(),
            ..Self::default()
        }
    }
    fn sync_intent(&mut self, state: &CrewState) {
        let key = state
            .intent
            .as_ref()
            .and_then(|intent| serde_json::to_string(intent).ok());
        if key == self.last_intent {
            return;
        }
        self.last_intent = key;
        self.director = state.director;
        match &state.intent {
            Some(PilotIntent::FlyToAirport {
                airport,
                runway,
                altitude_m,
            }) => {
                self.choice = 0;
                self.destination = airport.clone();
                self.runway = runway.clone();
                self.altitude_ft = altitude_m / 0.3048;
            }
            Some(PilotIntent::Scenic {
                preset,
                airport,
                runway,
            }) => {
                self.choice = match preset {
                    ScenicPreset::DelawareWaterGap => 1,
                    ScenicPreset::PoconoPlateau => 2,
                    ScenicPreset::HudsonCorridor => 3,
                };
                self.destination = airport.clone();
                self.runway = runway.clone();
            }
            Some(PilotIntent::FlightPlan {}) => self.choice = 4,
            _ => {}
        }
    }
    pub(crate) fn panel(&mut self, ui: &mut egui::Ui, s: &mut Session, keymap: &Keymap) {
        self.sync_intent(&s.crew_state());
        ui.label(RichText::new("Sit back. I'll fly.").size(22.).color(theme::TEXT));
        ui.label("Choose a flight, review the briefing, exchange controls, then Resume.");
        let previous_choice = self.choice;
        egui::ComboBox::from_id_salt("crew_destination")
            .selected_text(match self.choice {
                0 => "Destination airport",
                1 => "Delaware Water Gap and back",
                2 => "Pocono plateau and back",
                3 => "Hudson River corridor",
                _ => "Current flight plan",
            })
            .show_ui(ui, |ui| {
                for (i, label) in [
                    "Destination airport",
                    "Delaware Water Gap and back",
                    "Pocono plateau and back",
                    "Hudson River corridor",
                    "Current flight plan",
                ]
                .iter()
                .enumerate()
                {
                    ui.selectable_value(&mut self.choice, i, *label);
                }
            });
        if previous_choice != self.choice && (1..=3).contains(&self.choice) {
            self.destination = "1N7".into();
            self.runway = "07".into();
        }
        ui.horizontal(|ui| {
            ui.label(if self.choice == 0 {
                "Destination"
            } else {
                "Return airport"
            });
            ui.text_edit_singleline(&mut self.destination);
        });
        let mut runway_picker_shown = false;
        if let Some(catalogue) = s.sim().world().airports() {
            egui::ComboBox::from_id_salt("crew_airport_picker")
                .selected_text("Choose from regional airports")
                .show_ui(ui, |ui| {
                    for a in &catalogue.airports {
                        let id = a.icao.as_deref().unwrap_or(&a.ident);
                        if ui
                            .selectable_label(self.destination == id, format!("{id} · {}", a.name))
                            .clicked()
                        {
                            self.destination = id.into();
                            if let Some(end) = a.runways.first().and_then(|r| r.ends.first()) {
                                self.runway = end.ident.clone();
                            }
                        }
                    }
                });
            if let Some(a) = catalogue.airports.iter().find(|a| {
                a.ident.eq_ignore_ascii_case(&self.destination)
                    || a.icao
                        .as_ref()
                        .is_some_and(|i| i.eq_ignore_ascii_case(&self.destination))
            }) {
                runway_picker_shown = true;
                egui::ComboBox::from_id_salt("crew_runway_picker")
                    .selected_text(format!("Runway {}", self.runway))
                    .show_ui(ui, |ui| {
                        for r in &a.runways {
                            for end in &r.ends {
                                ui.selectable_value(
                                    &mut self.runway,
                                    end.ident.clone(),
                                    format!("{} · {:.0} ft", end.ident, r.length_m / 0.3048),
                                );
                            }
                        }
                    });
            }
        } else if self.choice == 0 && ui.button("Fly the original field circuit").clicked() {
            self.destination = "default".into();
            self.runway = "36".into();
            self.altitude_ft = 1033.;
        }
        if !runway_picker_shown {
            ui.horizontal(|ui| {
                ui.label("Runway");
                ui.text_edit_singleline(&mut self.runway);
            });
        }
        if self.choice == 0 {
            ui.add(
                egui::Slider::new(&mut self.altitude_ft, 1500. ..=9500.)
                    .step_by(100.)
                    .text("Cruise ft MSL"),
            );
        } else if self.choice < 4 {
            ui.label("Cruise levels follow the route and VFR altitude rules.");
        } else {
            ui.label("Altitudes follow the current flight plan.");
        }
        ui.checkbox(&mut self.director, "Director camera");
        if ui.button("Plan and brief flight").clicked() {
            let intent = match self.choice {
                0 => {
                    json!({"kind":"fly_to_airport","airport":self.destination,"runway":self.runway,"altitude_m":self.altitude_ft*0.3048})
                }
                4 => json!({"kind":"flight_plan"}),
                n => {
                    json!({"kind":"scenic","preset":match n {1=>"delaware_water_gap",2=>"pocono_plateau",_=>"hudson_corridor"},"airport":self.destination,"runway":self.runway})
                }
            };
            let result = s
                .run(SetCrewMode {
                    mode: CrewMode::Watching,
                    director: self.director,
                })
                .and_then(|_| s.execute("crew.intent", json!({"intent":intent})));
            match result {
                Ok(v) => {
                    if v.get("accepted").and_then(|v| v.as_bool()) == Some(false) {
                        self.error = Some(
                            v.get("refusal")
                                .and_then(|v| v.as_str())
                                .unwrap_or("Plan refused")
                                .into(),
                        );
                    } else {
                        self.error = None;
                    }
                }
                Err(e) => self.error = Some(e.to_string()),
            }
        }
        let state = s.crew_state();
        if let Some(plan) = &state.plan {
            ui.separator();
            ui.label(RichText::new(&plan.briefing).color(theme::TEXT));
            ui.label(format!(
                "{} · Initial cruise {:.0} ft MSL",
                s.sim().aircraft().identity.display_name,
                plan.altitude_m / 0.3048
            ));
            let (map_rect, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 210.), egui::Sense::hover());
            self.map.show_mfd_base(ui, map_rect, s, Some(BaseMap::Terrain));
            egui::CollapsingHeader::new("Details")
                .default_open(false)
                .show(ui, |ui| {
                    for check in &plan.checks {
                        ui.small(check);
                    }
                    for leg in &plan.legs {
                        ui.label(format!(
                            "{} · {:.0} ft MSL",
                            leg.name,
                            -leg.position_m.get(2).copied().unwrap_or(0.) / 0.3048
                        ));
                    }
                });
        }
        ui.separator();
        if ui.button("Open director camera").clicked() {
            open_page(s, screens::PageKind::Director, &mut self.error);
        }
        ui.label(format!(
            "{} · {}",
            state.phase.label(),
            if state.controls == ControlOwner::Crew {
                "Pilot in command has controls"
            } else {
                "You have controls"
            }
        ));
        if state.plan.as_ref().is_none_or(|p| p.briefing != state.reason) {
            ui.label(&state.reason);
        }
        let exchange_label = if state.pending_handover.is_some() {
            "Confirm exchange"
        } else if state.controls == ControlOwner::Crew {
            "My controls"
        } else {
            "Your controls"
        };
        if ui
            .button(format!(
                "{exchange_label} [{}]",
                keymap.label(Action::CrewHandover)
            ))
            .clicked()
        {
            exchange(s, &mut self.error);
        }
        ui.horizontal(|ui| {
            if ui.button("Go around").clicked() {
                match s.run(CrewIntent {
                    intent: PilotIntent::GoAround {},
                }) {
                    Ok(result) => self.error = result.refusal,
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
            if ui.button(if s.paused() { "Resume" } else { "Pause" }).clicked() {
                let result = if s.paused() {
                    s.execute("sim.resume", json!({}))
                } else {
                    s.execute("sim.pause", json!({}))
                };
                if let Err(e) = result {
                    self.error = Some(e.to_string());
                }
            }
        });
        ui.add_enabled_ui(state.phase == PilotPhase::Cruise, |ui| {
            let mut rate = s.rate();
            if ui
                .add(
                    egui::Slider::new(&mut rate, 1. ..=8.)
                        .step_by(1.)
                        .text("Cruise sim rate"),
                )
                .changed()
                && let Err(e) = s.run(SetRate { rate })
            {
                self.error = Some(e.to_string());
            }
        });
        ui.small("Crew voice, captions, and the kneeboard use the Radios audio controls. Handover can be bound in Settings > Shortcuts.");
        if let Some(t) = state.touchdown {
            ui.label(format!(
                "Touchdown: {:.0} ft from threshold, {:.0} ft from centerline, {:.0} ft/min sink · {}",
                t.distance_m / 0.3048,
                t.centreline_m / 0.3048,
                t.sink_mps * 60. / 0.3048,
                if t.within_limits {
                    "within limits"
                } else {
                    "outside limits"
                }
            ));
        }
        if let Some(error) = &self.error {
            ui.colored_label(theme::AMBER, error);
        }
    }
}

fn open_page(s: &mut Session, kind: screens::PageKind, error: &mut Option<String>) {
    match s.execute("screen.open", json!({"page":{"kind":kind},"size":[1000.,800.]})) {
        Ok(_) => *error = None,
        Err(e) => *error = Some(e.to_string()),
    }
}
pub fn page_button(ui: &mut egui::Ui, s: &mut Session, keymap: &Keymap, error: &mut Option<String>) {
    ui.ctx()
        .data_mut(|data| data.insert_temp(egui::Id::new("crew.keymap"), keymap.clone()));
    if ui.button("Fly me…").clicked() {
        open_page(s, screens::PageKind::FlyMe, error);
    }
}
pub(crate) fn director_camera(s: &Session) -> flightcraft_render::CameraMode {
    use flightcraft_render::CameraMode;
    let crew = s.crew_state();
    if !crew.director {
        return s.camera().map_or(CameraMode::Chase, |c| render_camera(c.mode));
    }
    match crew.phase {
        PilotPhase::BeforeTakeoff | PilotPhase::Takeoff | PilotPhase::GoAround => CameraMode::Cockpit,
        PilotPhase::Climb => CameraMode::Wing,
        PilotPhase::Cruise => CameraMode::Flyby,
        PilotPhase::Final if s.sim().readout().height_agl_m < 100. => CameraMode::Tower,
        PilotPhase::Flare | PilotPhase::Rollout => CameraMode::Tower,
        _ => CameraMode::Chase,
    }
}

pub(crate) fn exchange(s: &mut Session, error: &mut Option<String>) {
    let state = s.crew_state();
    let (to, confirm) = state.pending_handover.map_or(
        (
            if state.controls == ControlOwner::Crew {
                ControlOwner::Pilot
            } else {
                ControlOwner::Crew
            },
            false,
        ),
        |to| (to, true),
    );
    // The built-in pilot acknowledges readiness; the user must confirm on a second action.
    let actor = if confirm {
        ControlOwner::Pilot
    } else {
        ControlOwner::Crew
    };
    match s.run(CrewHandover { to, actor, confirm }) {
        Ok(_) => *error = None,
        Err(e) => *error = Some(e.to_string()),
    }
}

pub(crate) fn render_camera(mode: flightcraft_control::CameraMode) -> flightcraft_render::CameraMode {
    use flightcraft_render::CameraMode;
    match mode {
        flightcraft_control::CameraMode::Chase => CameraMode::Chase,
        flightcraft_control::CameraMode::Cockpit => CameraMode::Cockpit,
        flightcraft_control::CameraMode::Wing => CameraMode::Wing,
        flightcraft_control::CameraMode::Tower => CameraMode::Tower,
        flightcraft_control::CameraMode::Flyby => CameraMode::Flyby,
    }
}
