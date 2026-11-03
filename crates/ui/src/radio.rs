//! Radio controls, temporary captions, and the persistent kneeboard transcript.
use crate::{
    keymap::{Action, Keymap},
    theme,
};
use eframe::egui::{self, RichText};
mod files;
use flightcraft_control::{
    CommsAudioPanel, CommsSnapshot, CommsTransmit, CommsTune, Reception, Session, Speaker,
};

#[derive(Default)]
pub struct RadioUi {
    open: bool,
    history: bool,
    generation: Option<u64>,
    last_save: Option<f64>,
    registration: String,
    spoken_type: String,
    export_message: Option<String>,
    kneeboard: bool,
    talk: bool,
    intercom: bool,
    submit: bool,
    text: String,
    tune1: String,
    tune2: String,
    airport: String,
    traffic_count: u8,
    pub error: Option<String>,
    speech: crate::radio_speech::Speech,
}
impl RadioUi {
    pub fn shortcut(&mut self, action: Action) {
        match action {
            Action::RadioTransmit => self.submit = true,
            Action::ComPtt => {
                self.talk = true;
                self.intercom = false;
            }
            Action::IntercomPtt => {
                self.talk = true;
                self.intercom = true;
            }
            Action::RadioPanel => self.open = !self.open,
            Action::Kneeboard => self.kneeboard = !self.kneeboard,
            _ => {}
        }
    }
    pub fn button(&mut self, ui: &mut egui::Ui, keymap: &Keymap) {
        if ui
            .button(format!("Radios [{}]", keymap.label(Action::RadioPanel)))
            .clicked()
        {
            self.open = !self.open;
        }
    }
    pub fn show(&mut self, ctx: &egui::Context, s: &mut Session, keymap: &Keymap, master_gain: f32) {
        if self.generation != Some(s.flight_generation()) {
            self.generation = Some(s.flight_generation());
            self.registration.clear();
        }
        let wall = ctx.input(|i| i.time);
        if self.last_save.is_none_or(|last| wall - last >= 5.0) {
            s.autosave_flight();
            self.last_save = Some(wall);
        }
        let state = s.comms_state();
        self.speech.observe(s, master_gain);
        captions(ctx, s);
        let submitted = std::mem::take(&mut self.submit);
        if self.open {
            egui::Window::new("Audio panel and radios")
                .constrain_to(crate::theme::window_bounds(ctx))
                .id(egui::Id::new("radio_panel"))
                .default_width(590.0)
                .open(&mut self.open)
                .show(ctx, |ui| {
                    if self.registration.is_empty() { self.registration = state.callsign.registration.clone(); self.spoken_type = state.callsign.spoken_type.clone(); }
                    ui.label(format!("Your call: {}", state.callsign.current));
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.registration).desired_width(95.).hint_text("Registration"));
                        ui.add(egui::TextEdit::singleline(&mut self.spoken_type).desired_width(110.).hint_text("Spoken type"));
                        if ui.button("Set callsign").clicked() { apply(s.execute("comms.callsign", serde_json::json!({"registration":self.registration,"spoken_type":self.spoken_type})), &mut self.error); }
                        if ui.button("Use pack").clicked() { apply(s.execute("comms.callsign", serde_json::json!({"reset_to_pack":true})), &mut self.error); self.registration.clear(); }
                    });
                    panel(ui, s, &state, &mut self.tune1, &mut self.tune2, &mut self.error);
                    atc_panel(ui,s,&mut self.airport,&mut self.error);
                    ui.separator();
                    if ui.button("Saved flights / debrief").clicked() { self.history = true; apply(s.run(flightcraft_control::FlightRecords { refresh: true }), &mut self.error); }
                    ui.horizontal(|ui| {
                        ui.label("Pattern airport");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.airport)
                                .hint_text("Airport identifier")
                                .desired_width(95.0),
                        );
                        ui.add(egui::Slider::new(&mut self.traffic_count, 0..=3).text("traffic"));
                        if ui.button("Set traffic").clicked() {
                            apply(
                                s.execute(
                                    "traffic.set",
                                    serde_json::json!({"airport":self.airport,"count":self.traffic_count}),
                                ),
                                &mut self.error,
                            );
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui
                            .button(format!("Talk [{}]", keymap.label(Action::ComPtt)))
                            .clicked()
                        {
                            self.talk = true;
                            self.intercom = false;
                        }
                        if ui
                            .button(format!("Kneeboard [{}]", keymap.label(Action::Kneeboard)))
                            .clicked()
                        {
                            self.kneeboard = !self.kneeboard;
                        }
                    });
                    if let Some(error) = &self.error {
                        ui.colored_label(theme::AMBER, error);
                    }
                    if let Some(error) = self.speech.error() {
                        ui.colored_label(theme::AMBER, error);
                    }
                });
        }
        if self.talk {
            let mut send = submitted;
            egui::TopBottomPanel::bottom("typed_ptt").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(if self.intercom { "INTERCOM" } else { "TRANSMIT" });
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.text)
                            .hint_text(format!(
                                "{} · use {{callsign}} · {} to transmit",
                                state.callsign.current,
                                keymap.label(Action::RadioTransmit)
                            ))
                            .desired_width(ui.available_width() - 140.0),
                    );
                    if !response.has_focus() && self.text.is_empty() {
                        response.request_focus();
                    }
                    send |= ui.button("Transmit").clicked()
                        || ((response.has_focus() || response.lost_focus())
                            && ui.input(|i| keymap.pressed(i, Action::RadioTransmit)));
                    if ui.button("Close").clicked() {
                        self.talk = false;
                    }
                });
                if let Some(error) = &self.error {
                    ui.colored_label(theme::AMBER, error);
                }
            });
            if send {
                match s.run(CommsTransmit {
                    text: self.text.clone(),
                    intercom: self.intercom,
                }) {
                    Ok(_) => {
                        self.text.clear();
                        self.talk = false;
                        self.error = None;
                    }
                    Err(error) => self.error = Some(error.to_string()),
                }
            }
        }
        if self.history {
            egui::Window::new("Saved flights · debrief")
                .constrain_to(crate::theme::window_bounds(ctx))
                .default_width(700.)
                .default_height(500.)
                .open(&mut self.history)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("Save current flight").clicked() {
                            apply(s.run(flightcraft_control::FlightSave {}), &mut self.error);
                        }
                        if ui.button("Refresh flights").clicked() {
                            apply(
                                s.run(flightcraft_control::FlightRecords { refresh: true }),
                                &mut self.error,
                            );
                        }
                        if ui.button("Export current JSON").clicked() {
                            export_record(s, None, &mut self.export_message, &mut self.error);
                        }
                    });
                    let archive = s.flight_archive_status();
                    if archive.busy {
                        ui.label("Saving or loading…");
                        ctx.request_repaint();
                    }
                    if let Some(error) = &archive.error {
                        ui.colored_label(theme::AMBER, error);
                    }
                    if let Some(message) = &self.export_message {
                        ui.label(message);
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("flight-list")
                        .max_height(180.)
                        .show(ui, |ui| {
                            for flight in &archive.flights {
                                ui.horizontal(|ui| {
                                    ui.label(format!(
                                        "{} · {} · {:.1} min · {} calls",
                                        flight.registration,
                                        flight.aircraft,
                                        flight.duration_s / 60.,
                                        flight.events
                                    ));
                                    ui.label(&flight.id);
                                    if ui
                                        .add_enabled(!archive.busy, egui::Button::new("Load debrief"))
                                        .clicked()
                                    {
                                        apply(
                                            s.run(flightcraft_control::FlightLoad {
                                                id: flight.id.clone(),
                                            }),
                                            &mut self.error,
                                        );
                                    }
                                });
                            }
                        });
                    if let Some(saved) = &archive.loaded {
                        ui.separator();
                        ui.label(format!(
                            "Loaded {} · {}",
                            saved.summary.id, saved.summary.registration
                        ));
                        if ui.button("Export loaded JSON").clicked() {
                            export_record(
                                s,
                                Some(saved.summary.id.clone()),
                                &mut self.export_message,
                                &mut self.error,
                            );
                        }
                        if let Some(debrief) = &saved.record.emergency {
                            ui.heading(format!("Emergency debrief — {}", debrief.procedure.title));
                            ui.label(debrief.summary());
                            crate::scenario::glide_map(ui, s, debrief);
                            for timing in &debrief.steps {
                                ui.label(format!(
                                    "{}: {:.1} s{}",
                                    timing.item,
                                    timing.elapsed_s,
                                    if timing.in_order { "" } else { " • out of order" }
                                ));
                            }
                        }
                        record_transcript(ui, s, &saved.record);
                    }
                    if let Some(error) = &self.error {
                        ui.colored_label(theme::AMBER, error);
                    }
                });
        }
        if self.kneeboard {
            egui::Window::new("Kneeboard · radio transcript")
                .constrain_to(crate::theme::window_bounds(ctx))
                .default_width(620.0)
                .default_height(370.0)
                .open(&mut self.kneeboard)
                .show(ctx, |ui| transcript(ui, s));
        }
    }
}
fn apply<T>(result: Result<T, flightcraft_control::CommandError>, error: &mut Option<String>) {
    *error = result.err().map(|e| e.to_string());
}
pub fn panel(
    ui: &mut egui::Ui,
    s: &mut Session,
    state: &CommsSnapshot,
    tune1: &mut String,
    tune2: &mut String,
    error: &mut Option<String>,
) {
    #[cfg(target_arch = "wasm32")]
    ui.colored_label(
        theme::MUTED,
        "Browser speech is unfiltered; this browser API does not expose voice audio to WebAudio.",
    );
    ui.horizontal(|ui| {
        ui.colored_label(
            if state.powered { theme::GREEN } else { theme::AMBER },
            if state.powered {
                "AUDIO · ON"
            } else {
                "AUDIO · NO POWER"
            },
        );
        ui.label("Transmit");
        for radio in [1, 2] {
            if ui
                .selectable_label(state.panel.transmit == radio, format!("COM{radio} MIC"))
                .clicked()
            {
                apply(
                    s.run(CommsAudioPanel {
                        transmit: Some(radio),
                        ..Default::default()
                    }),
                    error,
                );
            }
        }
    });
    ui.horizontal(|ui| {
        for (label, selected, command) in [
            (
                "COM1 RX",
                state.panel.receive_com1,
                CommsAudioPanel {
                    receive_com1: Some(!state.panel.receive_com1),
                    ..Default::default()
                },
            ),
            (
                "COM2 RX",
                state.panel.receive_com2,
                CommsAudioPanel {
                    receive_com2: Some(!state.panel.receive_com2),
                    ..Default::default()
                },
            ),
            (
                "NAV ID",
                state.panel.nav_ident,
                CommsAudioPanel {
                    nav_ident: Some(!state.panel.nav_ident),
                    ..Default::default()
                },
            ),
            (
                "ICS",
                state.panel.intercom,
                CommsAudioPanel {
                    intercom: Some(!state.panel.intercom),
                    ..Default::default()
                },
            ),
        ] {
            if ui.selectable_label(selected, label).clicked() {
                apply(s.run(command), error);
            }
        }
    });
    ui.horizontal(|ui| {
        let mut volume = state.panel.volume;
        if ui
            .add(egui::Slider::new(&mut volume, 0.0..=1.0).text("Volume"))
            .changed()
        {
            apply(
                s.run(CommsAudioPanel {
                    volume: Some(volume),
                    ..Default::default()
                }),
                error,
            );
        }
        let mut squelch = state.panel.squelch;
        if ui
            .add(egui::Slider::new(&mut squelch, 0.0..=1.0).text("Squelch"))
            .changed()
        {
            apply(
                s.run(CommsAudioPanel {
                    squelch: Some(squelch),
                    ..Default::default()
                }),
                error,
            );
        }
    });
    ui.horizontal(|ui| {
        let mut speech = state.panel.speech;
        let mut captions = state.panel.captions;
        if ui.checkbox(&mut speech, "Speech").changed() {
            apply(
                s.run(CommsAudioPanel {
                    speech: Some(speech),
                    ..Default::default()
                }),
                error,
            );
        }
        if ui.checkbox(&mut captions, "Captions").changed() {
            apply(
                s.run(CommsAudioPanel {
                    captions: Some(captions),
                    ..Default::default()
                }),
                error,
            );
        }
        ui.colored_label(theme::MUTED, "Transcript is always recorded");
    });
    ui.separator();
    for (number, radio, entry, powered) in [
        (1, &state.com1, tune1, state.com1_powered),
        (2, &state.com2, tune2, state.com2_powered),
    ] {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!(
                    "COM{number}  {:07.3}",
                    f64::from(radio.active_khz) / 1000.0
                ))
                .monospace()
                .color(if powered { theme::ACCENT } else { theme::MUTED }),
            );
            if ui
                .button("SWAP")
                .on_hover_text("Swap active and standby")
                .clicked()
            {
                apply(
                    s.run(CommsTune {
                        radio: number,
                        swap: true,
                        ..Default::default()
                    }),
                    error,
                );
            }
            ui.label(format!("STBY {:.3}", f64::from(radio.standby_khz) / 1000.0));
            ui.add(
                egui::TextEdit::singleline(entry)
                    .hint_text("MHz")
                    .desired_width(65.0),
            );
            if ui.button("Tune").clicked() {
                let parsed = entry.parse::<f64>().ok().filter(|v| {
                    v.is_finite()
                        && (118.0..137.0).contains(v)
                        && (v * 1000.0 - (v * 1000.0).round()).abs() < 1e-6
                });
                if let Some(mhz) = parsed {
                    apply(
                        s.run(CommsTune {
                            radio: number,
                            channel_khz: Some((mhz * 1000.0).round() as u32),
                            standby: true,
                            ..Default::default()
                        }),
                        error,
                    );
                } else {
                    *error = Some("Enter a COM channel in MHz, for example 122.800.".into());
                }
            }
            let mut spacing = radio.spacing;
            egui::ComboBox::from_id_salt(("spacing", number))
                .selected_text(if spacing == flightcraft_control::ComSpacing::Khz25 {
                    "25 kHz"
                } else {
                    "8.33 kHz"
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut spacing, flightcraft_control::ComSpacing::Khz25, "25 kHz");
                    ui.selectable_value(&mut spacing, flightcraft_control::ComSpacing::Khz833, "8.33 kHz");
                });
            if spacing != radio.spacing {
                apply(
                    s.run(CommsTune {
                        radio: number,
                        spacing: Some(spacing),
                        ..Default::default()
                    }),
                    error,
                );
            }
        });
    }
    if state.stations.is_empty() {
        ui.colored_label(theme::AMBER, "No FAA COM frequencies imported for this region.");
    } else {
        let nearby = s
            .sim()
            .readout()
            .latitude_deg
            .zip(s.sim().readout().longitude_deg);
        let mut stations = state.stations.iter().collect::<Vec<_>>();
        stations.sort_by(|a, b| {
            let distance = |st: &flightcraft_world::ComStation| {
                nearby.map_or(f64::MAX, |(latitude_deg, longitude_deg)| {
                    let point = s
                        .sim()
                        .world()
                        .airports()
                        .and_then(|data| {
                            data.airports.iter().find(|a| {
                                a.ident == st.airport || a.icao.as_deref() == Some(st.airport.as_str())
                            })
                        })
                        .map(|a| flightcraft_world::GeoPoint {
                            latitude_deg: a.latitude_deg,
                            longitude_deg: a.longitude_deg,
                        })
                        .unwrap_or(st.position);
                    point
                        .inverse(flightcraft_world::GeoPoint {
                            latitude_deg,
                            longitude_deg,
                        })
                        .0
                })
            };
            distance(a).total_cmp(&distance(b))
        });
        egui::ComboBox::from_id_salt("radio_stations")
            .selected_text("Nearby FAA frequencies · tune COM1")
            .show_ui(ui, |ui| {
                for station in stations.iter().take(30) {
                    if ui
                        .selectable_label(
                            false,
                            format!(
                                "{} {:.3} {}",
                                flightcraft_control::comms::phraseology::facility(station),
                                f64::from(station.channel_khz) / 1000.0,
                                station.metadata.as_ref().map_or("", |m| m.sector.as_str())
                            ),
                        )
                        .on_hover_text(format!(
                            "{} · {}",
                            station.name,
                            station.metadata.as_ref().map_or_else(
                                || "Hours unavailable".to_owned(),
                                |m| if m.hours.is_empty() {
                                    "Hours unavailable".to_owned()
                                } else {
                                    format!("Published hours: {}", m.hours)
                                }
                            )
                        ))
                        .clicked()
                    {
                        apply(
                            s.run(CommsTune {
                                radio: 1,
                                channel_khz: Some(station.channel_khz),
                                spacing: Some(flightcraft_control::ComSpacing::Khz833),
                                ..Default::default()
                            }),
                            error,
                        );
                    }
                }
            });
    }
}
pub fn captions(ctx: &egui::Context, s: &Session) {
    if !s.comms_state().panel.captions {
        return;
    }
    let now = s.sim().state().time_s;
    if let Some(event) = s
        .flight_record()
        .events
        .iter()
        .rev()
        .find(|e| e.is_spoken() && e.reception == Reception::Heard && now - e.time_s - e.duration_s < 7.0)
    {
        egui::TopBottomPanel::top("radio_caption")
            .frame(egui::Frame::new().fill(theme::RAISED).inner_margin(8))
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(format!(
                        "{}{} · {}",
                        if event.is_simulated_atc() {
                            "Simulated ATC · "
                        } else {
                            ""
                        },
                        event_speaker(event),
                        event.text
                    ))
                    .color(theme::TEXT),
                );
            });
    }
}
pub fn event_speaker(event: &flightcraft_control::Transmission) -> String {
    if event.speaker_name.is_empty() {
        speaker(&event.speaker)
    } else {
        event.speaker_name.clone()
    }
}
pub fn speaker(speaker: &Speaker) -> String {
    match speaker {
        Speaker::Pilot => "YOU".into(),
        Speaker::Instructor => "INSTRUCTOR · INTERCOM".into(),
        Speaker::Crew => "PILOT IN COMMAND · INTERCOM".into(),
        Speaker::Traffic { callsign } => callsign.clone(),
        Speaker::Facility { role, .. } => {
            flightcraft_control::comms::phraseology::facility_kind(*role).into()
        }
        Speaker::Nav { ident } => format!("NAV · {ident}"),
    }
}
pub fn transcript(ui: &mut egui::Ui, s: &Session) {
    if let Some(plan) = s.crew_state().plan {
        ui.label(RichText::new("CREW BRIEFING").strong().color(theme::ACCENT));
        ui.label(plan.briefing);
        for leg in plan.legs {
            ui.label(leg.name);
        }
        ui.separator();
    }
    ui.label(
        RichText::new("KNEEBOARD  /  COMMUNICATIONS")
            .strong()
            .color(theme::ACCENT),
    );
    if let Some(p) = s.traffic_state().airport {
        ui.label(format!(
            "{p} · {} traffic aircraft",
            s.traffic_state().aircraft.len()
        ));
    }
    record_transcript(ui, s, s.flight_record());
}
fn record_transcript(ui: &mut egui::Ui, s: &Session, record: &flightcraft_control::FlightRecord) {
    let stations = s.comms_state().stations;
    egui::ScrollArea::vertical()
        .stick_to_bottom(true)
        .max_height(450.0)
        .show(ui, |ui| {
            for event in record.events.iter().filter(|e| e.is_spoken()) {
                let color = match event.reception {
                    Reception::Heard => theme::TEXT,
                    Reception::Blocked => theme::AMBER,
                    _ => theme::MUTED,
                };
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(
                        theme::ACCENT,
                        format!(
                            "{:02.0}:{:02.0}",
                            (event.time_s / 60.0).floor(),
                            event.time_s % 60.0
                        ),
                    );
                    let name = if event.speaker_name.is_empty() {
                        flightcraft_control::speaker_label(&event.speaker, &stations)
                    } else {
                        event_speaker(event)
                    };
                    ui.label(RichText::new(name).strong().color(color));
                    ui.colored_label(
                        theme::MUTED,
                        match event.source {
                            flightcraft_control::EventSource::ScenarioAi => "scenario (ai-written)",
                            flightcraft_control::EventSource::Ai => "ai",
                            flightcraft_control::EventSource::Scenario => "scenario",
                            flightcraft_control::EventSource::Rules => "rules",
                            flightcraft_control::EventSource::Pilot => "pilot",
                        },
                    );
                    if event.is_simulated_atc() {
                        ui.colored_label(theme::AMBER, "Simulated ATC");
                    }
                    ui.colored_label(color, &event.text);
                });
                if let Some(reason) = &event.reason {
                    ui.colored_label(theme::AMBER, reason);
                }
                if let Some(grade) = &event.grade
                    && event.reason.as_deref() != Some(grade.feedback.as_str())
                {
                    ui.colored_label(
                        if grade.passed { theme::GREEN } else { theme::AMBER },
                        if grade.passed {
                            "Call correct and timely"
                        } else {
                            &grade.feedback
                        },
                    );
                }
                ui.separator();
            }
        });
}
#[cfg(test)]
mod preview;

fn export_record(
    s: &mut Session,
    id: Option<String>,
    message: &mut Option<String>,
    error: &mut Option<String>,
) {
    match s.run(flightcraft_control::FlightExport { id }) {
        Ok(export) => match files::export(&export.filename, &export.json) {
            Ok(value) => *message = Some(value),
            Err(value) => *error = Some(value),
        },
        Err(value) => *error = Some(value.to_string()),
    }
}

pub(crate) fn atc_panel(
    ui: &mut egui::Ui,
    s: &mut Session,
    airport: &mut String,
    error: &mut Option<String>,
) {
    egui::CollapsingHeader::new("Tower and approach training")
        .default_open(true)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(airport)
                        .hint_text("Towered airport")
                        .desired_width(100.),
                );
                if ui.button("Set ATC airport").clicked() {
                    apply(
                        s.execute("atc.configure", serde_json::json!({"airport":airport})),
                        error,
                    );
                }
            });
            let state = s.atc_state();
            if let Some(st) = state.current_facility {
                ui.label(format!(
                    "{} · {:.3}",
                    flightcraft_control::comms::phraseology::facility(&st),
                    f64::from(st.channel_khz) / 1000.
                ));
            }
            if let Some(atis) = state.atis {
                ui.label(format!(
                    "Information {} · runway {} · ATIS {:.3}",
                    atis.information,
                    atis.runway,
                    f64::from(atis.channel_khz) / 1000.
                ));
            }
            ui.label(state.expected_next_call);
            ui.horizontal(|ui| {
                if ui
                    .button(if state.radio_failed {
                        "Restore COM"
                    } else {
                        "Practise radio failure"
                    })
                    .clicked()
                {
                    let (name, input) = if state.radio_failed {
                        ("failure.clear", serde_json::json!({"target":"radio"}))
                    } else {
                        ("failure.set", serde_json::json!({"failure":{"kind":"radio"}}))
                    };
                    apply(s.execute(name, input), error);
                }
            });
            if state.radio_failed {
                ui.horizontal_wrapped(|ui| {
                    for (label, signal) in [
                        ("Steady green", "steady_green"),
                        ("Flashing green", "flashing_green"),
                        ("Steady red", "steady_red"),
                        ("Flashing red", "flashing_red"),
                        ("Flashing white", "flashing_white"),
                        ("Red / green", "alternating_red_green"),
                    ] {
                        if ui.button(label).clicked() {
                            apply(
                                s.execute(
                                    "atc.light_gun",
                                    serde_json::json!({"airport":airport,"signal":signal}),
                                ),
                                error,
                            );
                        }
                    }
                });
            }
            for p in state.pending_instructions {
                ui.label(format!(
                    "{} · {}",
                    if p.heard {
                        "Readback required"
                    } else {
                        "Await transmission"
                    },
                    p.phrase
                ));
            }
            if let Some(g) = state.light_gun {
                ui.colored_label(
                    theme::ACCENT,
                    format!("Tower light gun: {} · {}", g.signal.label(), g.indication),
                );
            }
            if let Some(g) = state.compliance.last() {
                ui.label(format!(
                    "{} · {}",
                    if g.passed {
                        "satisfactory"
                    } else {
                        "unsatisfactory"
                    },
                    g.observed
                ));
            }
        });
}
