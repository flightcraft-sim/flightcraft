//! A small command-driven search panel; network and device IO require a deliberate action.
use eframe::egui;
use flightcraft_control::{CommandError, SearchKind, SearchOutput, SearchResult, Session};
use serde_json::{Value, json};

#[derive(Default)]
pub struct SearchPanel {
    text: String,
    results: Vec<SearchResult>,
    selected: Option<SearchResult>,
    open: bool,
    pub focus: bool,
    error: Option<String>,
    notice: String,
    confirm_address: Option<String>,
    busy: bool,
    completion: std::sync::Arc<std::sync::Mutex<Option<Result<Value, String>>>>,
}
impl SearchPanel {
    pub fn show(&mut self, ui: &mut egui::Ui, session: &mut Session) {
        let response = ui.add(
            egui::TextEdit::singleline(&mut self.text)
                .char_limit(512)
                .desired_width(160.0)
                .hint_text("Search places"),
        );
        if self.focus {
            response.request_focus();
            self.focus = false;
            self.open = true;
        }
        if response.changed() {
            self.open = true;
            match session
                .execute("search.query", json!({"text": self.text}))
                .and_then(|v| {
                    serde_json::from_value::<SearchOutput>(v).map_err(|e| CommandError::Failed(e.to_string()))
                }) {
                Ok(output) => {
                    self.results = output.results;
                    self.notice = format!(
                        "{} · cycle {} · snapshot {}",
                        output.notice,
                        output.effective_date.as_deref().unwrap_or("unknown"),
                        output.snapshot_date.as_deref().unwrap_or("unavailable")
                    );
                    self.error = None;
                }
                Err(e) => self.error = Some(e.to_string()),
            }
        }
        if ui.button("My location")
            .on_hover_text("Request your device's location. It stays on your device and is saved only if you choose Pin.")
            .clicked() {
            self.open = true;
            self.execute(session, "location.current", json!({"request_permission": true}), ui.ctx());
        }
        let completed = match self.completion.lock() {
            Ok(mut slot) => slot.take(),
            Err(_) => Some(Err("Location/address task unavailable".into())),
        };
        if let Some(result) = completed {
            self.busy = false;
            match result {
                Ok(value) => self.receive(value, session),
                Err(e) => self.error = Some(e),
            }
        }
        if self.open {
            let mut open = self.open;
            egui::Window::new("Places")
                .open(&mut open)
                .default_width(460.0)
                .show(ui.ctx(), |ui| self.contents(ui, session));
            self.open = open;
        }
    }
    fn contents(&mut self, ui: &mut egui::Ui, session: &mut Session) {
        ui.small(&self.notice);
        if self.busy {
            ui.spinner();
            ui.label("Waiting for permission or provider response…");
        }
        egui::ScrollArea::vertical().max_height(230.0).show(ui, |ui| {
            let mut selection = None;
            for result in &self.results {
                let distance = result
                    .distance_m
                    .map(|d| format!(" · {:.1} nm", d / 1852.0))
                    .unwrap_or_default();
                if ui
                    .button(format!(
                        "{:?} · {} {}{distance}",
                        result.kind, result.ident, result.name
                    ))
                    .clicked()
                {
                    selection = Some(result.clone());
                }
                if result.details.contains("© OpenStreetMap contributors") {
                    ui.small("© OpenStreetMap contributors");
                }
            }
            if let Some(result) = selection {
                match result.kind {
                    SearchKind::LocationAction => self.execute(
                        session,
                        "location.current",
                        json!({"request_permission": true}),
                        ui.ctx(),
                    ),
                    SearchKind::GeocodeAction => self.confirm_address = Some(result.name),
                    _ => self.selected = Some(result),
                }
            }
        });
        if let Some(address) = self.confirm_address.clone() {
            ui.separator();
            ui.label(format!("Send “{address}” to a geocoder? Only this address is sent. Results are cached on this device."));
            ui.horizontal(|ui| {
                for (provider, label) in [
                    ("census", "US Census (US)"),
                    ("nominatim", "Nominatim · © OpenStreetMap contributors"),
                ] {
                    if ui.add_enabled(!self.busy, egui::Button::new(label)).clicked() {
                        self.confirm_address = None;
                        self.execute(
                            session,
                            "geocode.lookup",
                            json!({"address": address,
                            "provider": provider, "allow_network": true}),
                            ui.ctx(),
                        );
                    }
                }
                if ui.button("Cancel").clicked() {
                    self.confirm_address = None;
                }
            });
        }
        if let Some(result) = self.selected.clone() {
            ui.separator();
            ui.label(format!("{} · {}", result.ident, result.name));
            ui.label(&result.details);
            if let Some(position) = result.position {
                ui.monospace(format!(
                    "{:.6}, {:.6}",
                    position.latitude_deg, position.longitude_deg
                ));
                self.result_actions(ui, session, &result, position);
            }
        }
        if let Some(error) = &self.error {
            ui.colored_label(crate::theme::AMBER, error);
        }
    }
    fn result_actions(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        result: &SearchResult,
        position: flightcraft_world::GeoPoint,
    ) {
        ui.horizontal(|ui| {
            let kind = match result.kind {
                SearchKind::Airport => "airport",
                SearchKind::Navaid => "navaid",
                SearchKind::Fix => "fix",
                SearchKind::Pin => "pin",
                _ => "coordinate",
            };
            let ident = if result.ident.is_empty() {
                "POSITION"
            } else {
                &result.ident
            };
            let waypoint = json!({"ident": ident, "kind": kind, "position": position});
            if ui.button("Start here").clicked() {
                self.execute(
                    session,
                    "sim.reset",
                    json!({"world": session.sim().world().choice(),
                    "start": {"kind": "geographic_air", "position": position, "height_agl_m": 600.0,
                        "airspeed_mps": 45.0, "heading_rad": 0.0}, "paused": true}),
                    ui.ctx(),
                );
            }
            if ui.button("Direct to").clicked() {
                self.execute(
                    session,
                    "avionics.set",
                    json!({"setting": {"kind": "glass", "setting": {
                    "kind": "direct_to_waypoint", "waypoint": waypoint}}}),
                    ui.ctx(),
                );
            }
            if ui.button("Add to plan").clicked() {
                self.execute(
                    session,
                    "plan.add",
                    json!({"point": {"waypoint": waypoint,
                    "altitude_m": 1000.0}}),
                    ui.ctx(),
                );
            }
            if ui.button("Pin").clicked() {
                match session.run(flightcraft_control::PinsList {}) {
                    Ok(pins) => {
                        let id = (1..=1001)
                            .map(|i| format!("pin-{i}"))
                            .find(|id| !pins.pins.iter().any(|p| &p.id == id))
                            .unwrap_or_else(|| "pin-limit".into());
                        self.execute(
                            session,
                            "pins.add",
                            json!({"pin": {"id": id, "name": result.name,
                            "note": result.details, "position": position, "altitude_m": null}}),
                            ui.ctx(),
                        );
                    }
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
        });
    }
    fn execute(&mut self, session: &mut Session, name: &str, input: Value, ctx: &egui::Context) {
        match session.execute(name, input) {
            Ok(value) => {
                self.error = None;
                self.receive(value, session);
            }
            Err(CommandError::AppPending(request)) => {
                if self.busy {
                    self.error = Some("Wait for the current location/address request".into());
                    return;
                }
                self.busy = true;
                let completion = self.completion.clone();
                let ctx = ctx.clone();
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let worker = std::thread::Builder::new().spawn(move || {
                        let output =
                            flightcraft_control::run_navigation_request(request).map_err(|e| e.to_string());
                        if let Ok(mut slot) = completion.lock() {
                            *slot = Some(output);
                        }
                        ctx.request_repaint();
                    });
                    if let Err(e) = worker {
                        self.busy = false;
                        self.error = Some(e.to_string());
                    }
                }
                #[cfg(target_arch = "wasm32")]
                wasm_bindgen_futures::spawn_local(async move {
                    let result = complete_browser_request(request).await.map_err(|e| e.to_string());
                    if let Ok(mut slot) = completion.lock() {
                        *slot = Some(result);
                    }
                    ctx.request_repaint();
                });
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }
    fn receive(&mut self, value: Value, session: &Session) {
        let readout = session.sim().readout();
        let reference =
            readout
                .latitude_deg
                .zip(readout.longitude_deg)
                .map(|(latitude_deg, longitude_deg)| flightcraft_world::GeoPoint {
                    latitude_deg,
                    longitude_deg,
                });
        if let Some(rows) = value.get("results").and_then(Value::as_array) {
            self.results.clear();
            self.selected = None;
            for row in rows {
                if let (Some(name), Some(position), Some(attribution)) = (
                    row.get("name").and_then(Value::as_str),
                    row.get("position")
                        .and_then(|p| serde_json::from_value::<flightcraft_world::GeoPoint>(p.clone()).ok()),
                    row.get("attribution").and_then(Value::as_str),
                ) {
                    self.results.push(SearchResult {
                        kind: SearchKind::Coordinate,
                        ident: String::new(),
                        name: name.into(),
                        position: Some(position),
                        distance_m: reference.map(|p| p.inverse(position).0),
                        details: attribution.into(),
                    });
                }
            }
            if self.results.is_empty() {
                self.error = Some("No address matches".into());
            }
        }
        if value.get("accuracy_m").is_some()
            && let Some(position) = value
                .get("position")
                .and_then(|p| serde_json::from_value(p.clone()).ok())
        {
            self.selected = Some(SearchResult {
                kind: SearchKind::Coordinate,
                ident: "MY-LOCATION".into(),
                name: "My location".into(),
                position: Some(position),
                distance_m: reference.map(|p| p.inverse(position).0),
                details: format!(
                    "Accuracy {} m · not saved",
                    value.get("accuracy_m").unwrap_or(&Value::Null)
                ),
            });
        }
    }
}
#[cfg(target_arch = "wasm32")]
async fn complete_browser_request(request: flightcraft_control::AppRequest) -> Result<Value, CommandError> {
    let encode = |value| serde_json::to_value(value).map_err(|e| CommandError::Failed(e.to_string()));
    match request {
        flightcraft_control::AppRequest::Location => {
            let location = flightcraft_control::location_browser().await?;
            serde_json::to_value(location).map_err(|e| CommandError::Failed(e.to_string()))
        }
        flightcraft_control::AppRequest::Geocode(args) => {
            let output = flightcraft_control::lookup_browser(args.clone()).await?;
            let mut storage = Session::new()?;
            storage.cache_geocode_result(&args, output.clone())?;
            encode(output)
        }
        _ => Err(CommandError::Failed("Unavailable app request".into())),
    }
}
