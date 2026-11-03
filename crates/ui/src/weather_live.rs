//! Endpoint tests show exact matched text before a source can be saved or used.
use crate::theme;
use eframe::egui;
use flightcraft_control::live_weather::{Extraction, Selector, SourceStatus, WeatherSource};
use flightcraft_control::{
    Session, WeatherLive, WeatherNearby, WeatherPhone, WeatherSourceSave, WeatherSourceTest, WeatherSources,
};

pub struct LiveUi {
    pub open: bool,
    pub airport: String,
    selected: String,
    add: bool,
    name: String,
    url: String,
    preset: usize,
    selector: String,
    allow_http: bool,
    cors: bool,
    disclaimer: String,
    draft: Option<WeatherSource>,
    error: Option<String>,
}
impl Default for LiveUi {
    fn default() -> Self {
        Self {
            open: false,
            airport: "1N7".into(),
            selected: "1n7-digiwx".into(),
            add: false,
            name: String::new(),
            url: String::new(),
            preset: 0,
            selector: String::new(),
            allow_http: false,
            cors: false,
            disclaimer: flightcraft_control::live_weather::source::DISCLAIMER.into(),
            draft: None,
            error: None,
        }
    }
}
impl LiveUi {
    pub fn show(&mut self, ctx: &egui::Context, session: &mut Session) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        egui::Window::new("Weather → Live")
            .open(&mut open)
            .default_width(660.)
            .vscroll(true)
            .show(ctx, |ui| {
                if let Err(e) = self.panel(ui, session) {
                    self.error = Some(e.to_string());
                }
            });
        self.open = open;
        if self.add {
            let mut add = self.add;
            egui::Window::new("Add a weather source")
                .open(&mut add)
                .default_width(580.)
                .vscroll(true)
                .show(ctx, |ui| {
                    if let Err(e) = self.add_panel(ui, session) {
                        self.error = Some(e.to_string());
                    }
                });
            self.add &= add;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
    pub(crate) fn panel(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
    ) -> Result<(), flightcraft_control::CommandError> {
        ui.label("Live station weather · for simulation only");
        ui.horizontal(|ui| {
            ui.label("Airport");
            if ui.text_edit_singleline(&mut self.airport).changed() {
                self.airport.make_ascii_uppercase();
                self.selected.clear();
            }
            let airports = session
                .sim()
                .world()
                .airports()
                .map(|f| f.airports.clone())
                .unwrap_or_default();
            egui::ComboBox::from_id_salt("live-airport")
                .selected_text("Choose airport")
                .show_ui(ui, |ui| {
                    for airport in &airports {
                        if ui
                            .selectable_label(
                                self.airport == airport.ident,
                                format!("{} · {}", airport.ident, airport.name),
                            )
                            .clicked()
                        {
                            self.airport = airport.ident.clone();
                            self.selected.clear();
                        }
                    }
                });
        });
        let result = session.run(WeatherSources {
            airport: Some(self.airport.trim().into()),
        })?;
        if self.selected.is_empty() {
            self.selected = result
                .sources
                .first()
                .map(|s| s.source.id.clone())
                .unwrap_or_default();
        }
        egui::ComboBox::from_id_salt("live-source")
            .selected_text(
                result
                    .sources
                    .iter()
                    .find(|s| s.source.id == self.selected)
                    .map(|s| s.source.name.as_str())
                    .unwrap_or("Choose a source"),
            )
            .show_ui(ui, |ui| {
                for source in &result.sources {
                    let label = result
                        .nearby
                        .as_ref()
                        .and_then(|n| {
                            n.stations
                                .iter()
                                .find(|n| n.observation.source.id == source.source.id)
                        })
                        .map(|n| n.label())
                        .unwrap_or_else(|| source.source.name.clone());
                    ui.selectable_value(&mut self.selected, source.source.id.clone(), label);
                }
            });
        if let Some(status) = result.sources.iter().find(|s| s.source.id == self.selected) {
            if let Some(nearby) = result.nearby.as_ref().and_then(|n| {
                n.stations
                    .iter()
                    .find(|n| n.observation.source.id == self.selected)
            }) {
                ui.colored_label(theme::AMBER, nearby.label());
                ui.label(format!(
                    "This reports weather at {}, not at {}.",
                    nearby.station, self.airport
                ));
            }
            source_status(ui, status);
            if ui
                .add_enabled(
                    !status.fetching
                        && flightcraft_control::live_weather::runtime::now() >= status.next_attempt_at,
                    egui::Button::new(if status.fetching {
                        "Fetching…"
                    } else {
                        "Test / refresh"
                    }),
                )
                .clicked()
            {
                session.run(WeatherSourceTest {
                    source_id: Some(self.selected.clone()),
                    source: None,
                })?;
                if session.sim().world().airports().is_some_and(|f| {
                    f.airports
                        .iter()
                        .any(|a| a.ident == self.airport || a.icao.as_deref() == Some(self.airport.as_str()))
                }) {
                    session.run(WeatherNearby {
                        airport: self.airport.clone(),
                    })?;
                }
            }
            let ready = !status.fetching && status.last_good.is_some() && !status.stale;
            ui.horizontal(|ui| -> Result<(), flightcraft_control::CommandError> {
                if ui.add_enabled(ready, egui::Button::new("Use now")).clicked() {
                    session.run(WeatherLive::UseNow {
                        source_id: self.selected.clone(),
                    })?;
                }
                if ui.add_enabled(ready, egui::Button::new("Follow")).clicked() {
                    session.run(WeatherLive::Follow {
                        source_id: self.selected.clone(),
                    })?;
                }
                if ui.button("Stop following").clicked() {
                    session.run(WeatherLive::Stop)?;
                }
                Ok(())
            })
            .inner?;
            if result.following.as_ref() == Some(&self.selected) {
                ui.label("Following · updates blend over one minute of flight time");
            }
            if status.last_good.is_some() {
                ui.separator();
                ui.label("AWOS / weather-by-phone transcript");
                let phone = session.run(WeatherPhone {
                    airport: status.source.airport.clone(),
                })?;
                ui.label(phone.text);
            }
        } else {
            ui.label("No sources for this airport. Add a local endpoint below.");
        }
        ui.separator();
        if ui
            .button("Find / compare nearest official reporting stations")
            .clicked()
        {
            session.run(WeatherNearby {
                airport: self.airport.clone(),
            })?;
        }
        ui.label("Nearby reports describe their own stations. Conditions at this airport may differ.");
        if let Some(nearby) = &result.nearby {
            if nearby.fetching {
                ui.spinner();
                ui.label("Finding nearby official METARs…");
            }
            if let Some(e) = &nearby.error {
                ui.colored_label(theme::AMBER, e);
            }
            let local = result
                .sources
                .iter()
                .find(|s| {
                    s.source.id == self.selected
                        && !nearby
                            .stations
                            .iter()
                            .any(|n| n.observation.source.id == s.source.id)
                })
                .and_then(|s| s.last_good.as_ref());
            comparison(ui, local, nearby);
        }
        if ui.button("Add a source…").clicked() {
            self.add = true;
            self.name = format!("{} local weather", self.airport);
            self.url.clear();
            self.selector.clear();
            self.allow_http = false;
            self.cors = false;
            self.preset = 0;
            self.error = None;
            self.draft = None;
        }
        for error in result.errors {
            ui.colored_label(theme::AMBER, error);
        }
        if let Some(error) = &self.error {
            ui.colored_label(theme::AMBER, error);
        }
        Ok(())
    }
    fn source(&self) -> WeatherSource {
        let extraction = match self.preset {
            1 => Extraction::MetarText,
            2 => Extraction::Aviationweather,
            3 => Extraction::Metar {
                selector: Selector::ElementId {
                    element_id: self.selector.clone(),
                },
                fields: Default::default(),
            },
            4 => Extraction::Metar {
                selector: Selector::Regex {
                    regex: self.selector.clone(),
                },
                fields: Default::default(),
            },
            _ => Extraction::Digiwx,
        };
        let mut source = WeatherSource::preset(self.airport.trim(), self.url.trim(), extraction);
        let slug = self
            .name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect::<String>();
        source.id = format!("{}-{}", self.airport.to_ascii_lowercase(), slug.trim_matches('-'));
        source.name = self.name.clone();
        source.allow_http = self.allow_http;
        source.cors = self.cors;
        source.disclaimer = self.disclaimer.clone();
        source
    }
    fn add_panel(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
    ) -> Result<(), flightcraft_control::CommandError> {
        ui.label(format!("Reporting airport: {}", self.airport));
        ui.label("Source name");
        ui.text_edit_singleline(&mut self.name);
        ui.label("URL");
        ui.text_edit_singleline(&mut self.url);
        const PRESETS: [&str; 5] = [
            "DigiWx Advisor",
            "Plain METAR text",
            "Aviation Weather Center JSON",
            "METAR element id",
            "METAR regex (one capture group)",
        ];
        egui::ComboBox::from_id_salt("weather-preset")
            .selected_text(PRESETS.get(self.preset).copied().unwrap_or("DigiWx Advisor"))
            .show_ui(ui, |ui| {
                for (i, name) in PRESETS.iter().enumerate() {
                    ui.selectable_value(&mut self.preset, i, *name);
                }
            });
        if self.preset >= 3 {
            ui.label(if self.preset == 3 { "Element id" } else { "Regex" });
            ui.text_edit_singleline(&mut self.selector);
        }
        ui.checkbox(
            &mut self.allow_http,
            "Allow unencrypted HTTP (weather and your IP address can be observed)",
        );
        ui.checkbox(&mut self.cors, "Server allows browser CORS requests");
        ui.label("Advisory-only disclaimer (shown with every observation)");
        ui.text_edit_multiline(&mut self.disclaimer);
        ui.label("FlightCraft sends only a GET for this URL and its redirects. No aircraft position, flight plan, or simulator state is sent. The server sees your IP address.");
        let source = self.source();
        let sources = session.run(WeatherSources::default())?;
        let fetching = sources
            .sources
            .iter()
            .any(|s| s.source.id == source.id && s.fetching);
        if ui
            .add_enabled(
                !fetching,
                egui::Button::new(if fetching { "Fetching…" } else { "Test" }),
            )
            .clicked()
        {
            self.error = None;
            session.run(WeatherSourceTest {
                source_id: None,
                source: Some(source.clone()),
            })?;
            self.draft = Some(source.clone());
        }
        let status = sources
            .sources
            .iter()
            .find(|s| Some(&s.source) == self.draft.as_ref());
        if let Some(status) = status {
            source_status(ui, status);
        }
        let can_save = status.is_some_and(|status| {
            source == status.source
                && !status.fetching
                && status.error.is_none()
                && status.last_good.is_some()
        });
        if ui
            .add_enabled(can_save, egui::Button::new("Save tested source"))
            .clicked()
        {
            session.run(WeatherSourceSave {
                source: source.clone(),
            })?;
            self.selected = source.id;
            self.add = false;
        }
        if let Some(e) = &self.error {
            ui.colored_label(theme::AMBER, e);
        }
        Ok(())
    }
}
pub(crate) fn source_status(ui: &mut egui::Ui, status: &SourceStatus) {
    ui.label(&status.source.name);
    if status.source.allow_http {
        ui.colored_label(
            theme::AMBER,
            "Unencrypted HTTP · source data can be changed in transit",
        );
    }
    if status.source.advisory_only {
        ui.colored_label(theme::AMBER, &status.source.disclaimer);
    }
    if status.fetching {
        ui.spinner();
        ui.label("Fetching station observation…");
    }
    if let Some(error) = &status.error {
        ui.colored_label(theme::AMBER, format!("Refresh failed. {error}"));
    }
    if let Some(live) = &status.last_good {
        let o = &live.extracted.observation;
        let age = status.age_seconds.unwrap_or(0);
        let issued = live
            .observed_at
            .and_then(time_string)
            .unwrap_or_else(|| "Observation time unavailable; age is since retrieval".into());
        ui.label(format!(
            "{issued} · age {} min {} sec{}",
            age / 60,
            age % 60,
            if status.stale {
                " · STALE — automatic use disabled"
            } else {
                ""
            }
        ));
        if let Some(w) = &o.wind {
            ui.label(format!(
                "Wind {} at {:.0} knots{}",
                w.from_true_deg
                    .map(|v| format!("{v:03.0}° true"))
                    .unwrap_or_else(|| if w.variable {
                        "variable".into()
                    } else {
                        "direction unavailable".into()
                    }),
                w.speed_kt,
                w.gust_kt.map(|v| format!(", gusts {v:.0}")).unwrap_or_default()
            ));
            if let Some([a, b]) = w.variable_sector_deg {
                ui.label(format!("Variable from {a:03.0}° to {b:03.0}° true"));
            }
        }
        if let Some(v) = o.altimeter_inhg {
            ui.label(format!("Altimeter {v:.2} inHg"));
        }
        if let Some(v) = o.visibility_sm {
            ui.label(format!("Visibility {v:.2} statute miles"));
        }
        if let Some(layers) = &o.layers {
            if layers.is_empty() {
                ui.label("Sky clear");
            }
            for l in layers {
                ui.label(format!(
                    "{} · {}",
                    match l.coverage.as_str() {
                        "FEW" => "Few",
                        "SCT" => "Scattered",
                        "BKN" => "Broken",
                        "VV" => "Vertical visibility",
                        _ => "Overcast",
                    },
                    l.base_agl_ft
                        .map(|v| format!("{v:.0} feet AGL"))
                        .unwrap_or_else(|| "height unavailable".into())
                ));
            }
        }
        if let Some(v) = o.temperature_c {
            ui.label(format!("Temperature {v:.0}° C"));
        }
        if let Some(v) = o.dewpoint_c {
            ui.label(format!("Dew point {v:.0}° C"));
        }
        if let Some(v) = &o.runway_in_use {
            ui.label(format!("Indicated runway {v}"));
        }
        ui.collapsing("Exact matched text", |ui| {
            for (field, value) in &live.extracted.matches {
                ui.label(field_label(field));
                ui.monospace(value);
            }
        });
    } else {
        ui.label("No observation yet. Test the source to fetch and inspect it.");
    }
}
fn field_label(key: &str) -> &str {
    match key {
        "metar" => "METAR",
        "wind" => "Wind",
        "gust" => "Gust",
        "altimeter" => "Altimeter",
        "visibility" => "Visibility",
        "layers" => "Cloud layers",
        "ceiling" => "Ceiling",
        "temperature" => "Temperature",
        "dew_point" => "Dew point",
        "runway_in_use" => "Indicated runway",
        "date" => "Date",
        "time" => "Time",
        _ => "Matched text",
    }
}
fn time_string(timestamp: i64) -> Option<String> {
    // The control layer formats UTC consistently on native and wasm.
    flightcraft_control::live_weather::runtime::utc_label(timestamp)
}

pub(crate) fn comparison(
    ui: &mut egui::Ui,
    local: Option<&flightcraft_control::LiveObservation>,
    nearby: &flightcraft_control::live_weather::nearby::NearbyStatus,
) {
    let now = flightcraft_control::live_weather::runtime::now();
    ui.label("Cross-check with nearby official observations");
    if let Some(local) = local
        && local.source.advisory_only
    {
        ui.colored_label(theme::AMBER, &local.source.disclaimer);
    }
    egui::Grid::new("weather-comparison")
        .striped(true)
        .show(ui, |ui| {
            ui.label("Reporting station");
            ui.label("Altimeter");
            ui.label("Wind (true)");
            ui.label("Ceiling AGL");
            ui.label("Age");
            ui.end_row();
            if let Some(local) = local {
                comparison_row(
                    ui,
                    &format!("{} · {}", local.source.airport, local.source.name),
                    local,
                    now,
                );
            }
            for n in &nearby.stations {
                comparison_row(ui, &n.label(), &n.observation, now);
            }
        });
    if let Some(local) = local {
        let check = flightcraft_control::live_weather::nearby::compare(local, &nearby.stations, now);
        if check.altimeter_disagreement {
            ui.colored_label(theme::AMBER,"Altimeter disagreement: local setting differs by more than 0.05 inHg from the nearby stations' mean.");
        }
        if check.time_disagreement {
            ui.colored_label(theme::AMBER,"Observation time disagreement: local report is more than 60 minutes older than nearby official observations.");
        }
        if check.official_altimeter_mean_inhg.is_none() {
            ui.label("No fresh nearby altimeter reports to compare.");
        }
    }
}
fn comparison_row(ui: &mut egui::Ui, label: &str, live: &flightcraft_control::LiveObservation, now: i64) {
    let o = &live.extracted.observation;
    ui.label(label);
    ui.label(
        o.altimeter_inhg
            .map(|v| format!("{v:.2} inHg"))
            .unwrap_or_else(|| "Unavailable".into()),
    );
    ui.label(
        o.wind
            .as_ref()
            .map(|w| {
                format!(
                    "{} / {:.0} knots",
                    w.from_true_deg
                        .map(|v| format!("{v:03.0}°"))
                        .unwrap_or_else(|| if w.variable {
                            "Variable".into()
                        } else {
                            "Unavailable".into()
                        }),
                    w.speed_kt
                )
            })
            .unwrap_or_else(|| "Unavailable".into()),
    );
    ui.label(
        o.layers
            .as_ref()
            .map(|layers| {
                layers
                    .iter()
                    .filter(|l| matches!(l.coverage.as_str(), "BKN" | "OVC" | "VV"))
                    .filter_map(|l| l.base_agl_ft)
                    .min_by(f64::total_cmp)
                    .map(|v| format!("{v:.0} feet"))
                    .unwrap_or_else(|| {
                        if layers.iter().any(|l| {
                            matches!(l.coverage.as_str(), "BKN" | "OVC" | "VV") && l.base_agl_ft.is_none()
                        }) {
                            "Unknown height".into()
                        } else {
                            "No ceiling".into()
                        }
                    })
            })
            .unwrap_or_else(|| "Unavailable".into()),
    );
    ui.label(format!(
        "{} min{}",
        live.age_seconds(now) / 60,
        if live.stale(now) { " · Stale" } else { "" }
    ));
    ui.end_row();
}
