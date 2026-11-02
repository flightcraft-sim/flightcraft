//! A rectangular map widget shared by the planning view and the glass MFD host.
use crate::theme;
use eframe::egui::{self, Align2, FontId, Pos2, Rect, Sense, Stroke};
use flightcraft_control::{BaseMap, MapOverlay as Overlay, MapState, MapUp, Session};
use flightcraft_world::GeoPoint;
use serde_json::{Value, json};
use std::collections::VecDeque;
mod files;
mod profile;
mod projection;
mod tiles;
mod units;
use projection::View;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod preview;
#[cfg(test)]
mod tests;

#[derive(Clone)]
struct Feature {
    position: GeoPoint,
    pixel: Pos2,
    ident: String,
    kind: &'static str,
    details: String,
    airport: bool,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    Pan,
    Waypoint(usize),
    Leg(usize),
}
#[derive(Default)]
pub struct PlanningMap {
    pub open: bool,
    pub options: crate::instruments::DisplayOptions,
    tiles: tiles::Tiles,
    selected: Option<Feature>,
    edit: bool,
    measure: bool,
    measurement: Vec<GeoPoint>,
    drag: Option<Drag>,
    pub error: Option<String>,
    track: VecDeque<GeoPoint>,
    generation: Option<u64>,
    interchange: String,
    format: String,
    files: bool,
    file_notice: String,
}
impl PlanningMap {
    pub fn with_root(root: Option<String>) -> Self {
        Self {
            tiles: tiles::Tiles::with_root(root),
            ..Default::default()
        }
    }
    pub fn zoom(&mut self, s: &mut Session, factor: f64) {
        self.command(
            s,
            "map.set",
            json!({"range_nm":(s.map_state().range_nm*factor).clamp(0.25,200.)}),
        );
    }
    fn command(&mut self, s: &mut Session, name: &str, input: Value) {
        match s.execute(name, input) {
            Ok(_) => self.error = None,
            Err(e) => self.error = Some(e.to_string()),
        }
    }
    /// All geometry and controls remain inside the supplied rectangle, including at narrow sizes.
    pub fn show(&mut self, ui: &mut egui::Ui, rect: Rect, s: &mut Session) {
        if rect.width() < 100. || rect.height() < 100. {
            return;
        }
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.set_clip_rect(rect);
            self.toolbar(ui, s);
            let top = (ui.cursor().top() + 4.).min(rect.bottom());
            let profile_height = (rect.height() * 0.24).clamp(60., 180.);
            let chart = Rect::from_min_max(
                egui::pos2(rect.left(), top),
                egui::pos2(
                    rect.right(),
                    (rect.bottom() - profile_height - 34.).max(top).min(rect.bottom()),
                ),
            );
            self.map(ui, chart, s, false);
            let profile_rect =
                Rect::from_min_max(egui::pos2(rect.left(), chart.bottom() + 24.), rect.right_bottom());
            profile::paint(ui, s, profile_rect);
            if let Some(error) = &self.error {
                ui.painter().text(
                    chart.left_top() + egui::vec2(10., 10.),
                    Align2::LEFT_TOP,
                    error,
                    FontId::proportional(12.),
                    theme::DANGER,
                );
            }
        });
        if let Some(feature) = self.selected.clone() {
            self.card(ui, s, &feature, rect);
        }
        if self.files {
            self.file_window(ui, s);
        }
    }
    /// Compact host: same projection, tiles, terrain, overlays, and gestures as the planning map.
    pub fn show_mfd(&mut self, ui: &mut egui::Ui, rect: Rect, s: &mut Session) {
        self.map(ui, rect, s, true);
        if let Some(feature) = self.selected.clone() {
            self.card(ui, s, &feature, ui.ctx().content_rect());
        }
    }
    fn toolbar(&mut self, ui: &mut egui::Ui, s: &mut Session) {
        let mut state = s.map_state().clone();
        let before = serde_json::to_value(&state).ok();
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("map_base")
                .selected_text(base_name(state.base_map))
                .show_ui(ui, |ui| {
                    for base in [BaseMap::Aero, BaseMap::Terrain, BaseMap::Sectional, BaseMap::Tac] {
                        ui.selectable_value(&mut state.base_map, base, base_name(base));
                    }
                    ui.add_enabled(false, egui::Button::new("Satellite · coming soon"));
                    ui.add_enabled(false, egui::Button::new("Street · coming soon"));
                });
            ui.menu_button("Declutter", |ui| {
                ui.label("Detail below this map span (NM across)");
                ui.add(egui::Slider::new(&mut state.declutter.fixes_across_nm, 0.0..=400.0).text("Fixes"));
                ui.add(
                    egui::Slider::new(&mut state.declutter.airway_labels_across_nm, 0.0..=400.0)
                        .text("Airway labels"),
                );
            });
            ui.menu_button("Overlays", |ui| {
                for key in Overlay::AVAILABLE {
                    if let Some(value) = state.overlays.get_mut(&key) {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut value.enabled, overlay_name(key));
                            ui.add(egui::Slider::new(&mut value.opacity, 0.0..=1.0).show_value(false));
                        });
                    }
                }
                for text in ["Live weather", "TFR", "Plates"] {
                    ui.add_enabled(false, egui::Button::new(format!("{text} · coming soon")));
                }
            });
            egui::ComboBox::from_id_salt("map_up")
                .selected_text(orientation_name(state.orientation))
                .show_ui(ui, |ui| {
                    for up in [MapUp::NorthUp, MapUp::TrackUp, MapUp::HeadingUp] {
                        ui.selectable_value(&mut state.orientation, up, orientation_name(up));
                    }
                });
            if ui.small_button("−").clicked() {
                state.range_nm = (state.range_nm * 1.25).min(200.);
            }
            if ui.small_button("+").clicked() {
                state.range_nm = (state.range_nm * 0.8).max(0.25);
            }
            if ui.button("Follow").clicked() {
                state.centre = None;
            }
            ui.checkbox(&mut state.night, "Night");
            ui.toggle_value(&mut self.edit, "Edit route");
            ui.toggle_value(&mut self.measure, "Measure");
            ui.menu_button("Waypoints", |ui| {
                for (index, point) in s.flight_plan().waypoints.clone().iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(&point.waypoint.ident);
                        let mut altitude = point.altitude_m / 0.3048;
                        if ui
                            .add(
                                egui::DragValue::new(&mut altitude)
                                    .range(-1640.0..=65616.0)
                                    .suffix(" ft MSL"),
                            )
                            .changed()
                        {
                            self.command(
                                s,
                                "plan.set_altitude",
                                json!({"index":index,"altitude_m":altitude * 0.3048}),
                            );
                        }
                    });
                }
            });
            if ui.button("Activate GPS").clicked() {
                self.command(s, "plan.activate", json!({}));
            }
            if ui.button("Import / export").clicked() {
                self.files = true;
            }
        });
        if before != serde_json::to_value(&state).ok() {
            self.command(s,"map.set",json!({"declutter":state.declutter,"base_map":state.base_map,"overlays":state.overlays,"orientation":state.orientation,"range_nm":state.range_nm,"centre":state.centre,"follow_aircraft":state.centre.is_none(),"night":state.night}));
        }
    }
    fn map(&mut self, ui: &mut egui::Ui, rect: Rect, s: &mut Session, compact: bool) {
        if rect.width() < 20. || rect.height() < 20. {
            return;
        }
        let state = s.map_state().clone();
        let p = ui.painter().with_clip_rect(rect);
        let background = if state.night {
            theme::PANEL
        } else {
            theme::MAP_DAY_OUTSIDE
        };
        p.rect_filled(rect, 0., background);
        let Some(view) = View::new(s, &state, rect) else {
            p.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Select a geographic region to use the map",
                FontId::proportional(14.),
                theme::TEXT,
            );
            return;
        };
        let response = ui.interact(
            rect,
            ui.id().with(if compact { "mfd_map" } else { "planning_map" }),
            Sense::click_and_drag(),
        );
        if self.generation != Some(s.flight_generation()) {
            self.track.clear();
            self.generation = Some(s.flight_generation());
        }
        let readout = s.sim().readout();
        let aircraft =
            readout
                .latitude_deg
                .zip(readout.longitude_deg)
                .map(|(latitude_deg, longitude_deg)| GeoPoint {
                    latitude_deg,
                    longitude_deg,
                });
        if let Some(point) = aircraft
            && self.track.back().is_none_or(|last| last.inverse(point).0 >= 50.)
        {
            if self.track.len() >= 2048 {
                self.track.pop_front();
            }
            self.track.push_back(point);
        }
        let raster = matches!(state.base_map, BaseMap::Sectional | BaseMap::Tac);
        // The original vector base remains visible wherever the selected chart has no pixels.
        terrain(&p, &view, s, &state, false, compact);
        let mut notice = format!("FlightCraft {} · not for navigation", base_name(state.base_map));
        if raster && let Some(region) = s.sim().world().region() {
            notice = format!(
                "{} · Aero outside chart coverage",
                self.tiles.paint(ui, &p, &view, &region.id, &state)
            );
        }
        if raster && state.opacity(Overlay::TerrainWarnings) > 0. {
            terrain(&p, &view, s, &state, true, compact);
        }
        let features = paint_features(&p, &view, s, &state);
        self.paint_track(&p, &view, &state);
        route(&p, &view, s, &state, self.options.engine);
        if let Some(point) = aircraft
            && state.opacity(Overlay::Aircraft) > 0.
            && let Some(c) = view.point(point)
        {
            let angle = view.aircraft_grid_heading_rad - view.angle;
            let (sin, cos) = angle.sin_cos();
            let at = |x: f64, y: f64| c + egui::vec2((x * cos - y * sin) as f32, (x * sin + y * cos) as f32);
            p.add(egui::Shape::convex_polygon(
                vec![at(0., -10.), at(6., 8.), at(0., 4.), at(-6., 8.)],
                theme::ACCENT.linear_multiply(state.opacity(Overlay::Aircraft)),
                Stroke::NONE,
            ));
        }
        rings(&p, &view, &state);
        self.paint_measure(&p, &view, s);
        if rect.width() >= 300. && rect.height() >= 180. {
            let footer_height = if compact { 30. } else { 19. };
            p.rect_filled(
                Rect::from_min_size(
                    rect.left_bottom() - egui::vec2(0., footer_height),
                    egui::vec2(rect.width(), footer_height + 2.),
                ),
                0.,
                theme::PANEL,
            );
            if compact {
                let edition = notice.split(" · not for navigation").next().unwrap_or(&notice);
                p.text(
                    rect.left_bottom() + egui::vec2(4., -16.),
                    Align2::LEFT_BOTTOM,
                    edition,
                    FontId::proportional(7.),
                    theme::MUTED,
                );
                p.text(
                    rect.left_bottom() + egui::vec2(4., -3.),
                    Align2::LEFT_BOTTOM,
                    "NOT FOR NAVIGATION",
                    FontId::proportional(8.),
                    theme::AMBER,
                );
            } else {
                p.text(
                    rect.left_bottom() + egui::vec2(6., -4.),
                    Align2::LEFT_BOTTOM,
                    notice,
                    FontId::proportional(11.),
                    theme::MUTED,
                );
            }
            p.text(
                rect.right_top() - egui::vec2(8., -8.),
                Align2::RIGHT_TOP,
                format!(
                    "{} · {} · {:.1} NM",
                    orientation_name(state.orientation),
                    view.magnetic_bearing_deg
                        .map_or("N TRUE".into(), |bearing| format!(
                            "{:03.0}° M",
                            bearing.round().rem_euclid(360.)
                        )),
                    state.range_nm
                ),
                FontId::monospace(11.),
                theme::ACCENT,
            );
        }
        self.gestures(ui, s, &view, &response, &features, compact);
    }
    fn gestures(
        &mut self,
        ui: &egui::Ui,
        s: &mut Session,
        view: &View,
        response: &egui::Response,
        features: &[Feature],
        compact: bool,
    ) {
        if response.hovered() {
            let (wheel, pinch) = ui.input_mut(|i| {
                let w = i.smooth_scroll_delta.y;
                i.smooth_scroll_delta.y = 0.;
                (w, i.zoom_delta())
            });
            if wheel != 0. || (pinch - 1.).abs() > 0.001 {
                self.zoom(
                    s,
                    ((-f64::from(wheel) / 300.).exp() / f64::from(pinch)).clamp(0.5, 2.),
                );
            }
        }
        let Some(pos) = response.interact_pointer_pos() else {
            return;
        };
        if !view.rect.contains(pos) {
            return;
        }
        if response.drag_started() {
            let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or(pos);
            self.drag = Some(if self.edit && !compact {
                route_hit(s, view, origin).unwrap_or(Drag::Pan)
            } else {
                Drag::Pan
            });
        }
        if response.dragged() && self.drag == Some(Drag::Pan) {
            let delta = ui.input(|i| i.pointer.delta());
            if let Some(centre) = view.position(view.rect.center() - delta) {
                self.command(s, "map.set", json!({"centre":centre}));
            }
        }
        if response.drag_stopped()
            && let Some(position) = view.position(pos)
        {
            match self.drag.take() {
                Some(Drag::Waypoint(index)) => {
                    self.command(
                        s,
                        "plan.move",
                        json!({"from":index,"to":index,"position":position}),
                    );
                }
                Some(Drag::Leg(index)) => {
                    let altitude = s
                        .flight_plan()
                        .waypoints
                        .get(index)
                        .map_or(1000., |w| w.altitude_m);
                    self.command(
                        s,
                        "plan.insert",
                        json!({"index":index+1,"point":point_value(position,altitude)}),
                    );
                }
                _ => {}
            }
        }
        if response.secondary_clicked() || response.long_touched() {
            if let Some(Drag::Waypoint(index)) = route_hit(s, view, pos) {
                self.command(s, "plan.remove", json!({"index":index}));
            }
        } else if response.clicked() {
            let Some(position) = view.position(pos) else {
                return;
            };
            if self.measure {
                if self.measurement.len() >= 2 {
                    self.measurement.clear();
                }
                self.measurement.push(position);
            } else if let Some(feature) = feature_at(s, view, features, pos) {
                self.selected = Some(feature);
            } else if self.edit && !compact {
                self.command(s, "plan.add", json!({"point":point_value(position,1000.)}));
            } else {
                self.selected = Some(Feature {
                    position,
                    pixel: pos,
                    ident: "POSITION".into(),
                    kind: "coordinate",
                    details: format!("{:.5}, {:.5}", position.latitude_deg, position.longitude_deg),
                    airport: false,
                });
            }
        }
    }
    fn paint_track(&self, p: &egui::Painter, v: &View, state: &MapState) {
        for (a, b) in self.track.iter().zip(self.track.iter().skip(1)) {
            if let (Some(a), Some(b)) = (v.point(*a), v.point(*b)) {
                p.line_segment(
                    [a, b],
                    Stroke::new(1., theme::ACCENT.linear_multiply(state.opacity(Overlay::Track))),
                );
            }
        }
    }
    fn paint_measure(&self, p: &egui::Painter, v: &View, s: &Session) {
        if let [a, b] = self.measurement.as_slice()
            && let (Some(x), Some(y)) = (v.point(*a), v.point(*b))
        {
            let (distance, true_bearing) = flightcraft_sim::avionics::navigation::inverse(
                [a.latitude_deg, a.longitude_deg],
                [b.latitude_deg, b.longitude_deg],
            );
            let bearing = true_bearing.to_degrees();
            let variation = flightcraft_world::geomag::MagneticModel::wmm2025()
                .and_then(|m| m.declination(a.latitude_deg, a.longitude_deg, s.simulation_date()))
                .unwrap_or(0.);
            p.line_segment([x, y], Stroke::new(2., theme::AMBER));
            p.text(
                x.lerp(y, 0.5),
                Align2::CENTER_BOTTOM,
                format!(
                    "{:.2} NM · {:03.0}° M",
                    distance / 1852.,
                    (bearing - variation).rem_euclid(360.)
                ),
                FontId::monospace(12.),
                theme::AMBER,
            );
        }
    }
    fn card(&mut self, ui: &egui::Ui, s: &mut Session, feature: &Feature, rect: Rect) {
        let mut open = true;
        egui::Window::new(format!("{} · {}",feature.ident,feature.kind)).id(ui.id().with("map_details")).open(&mut open).resizable(false).constrain_to(rect).default_pos(rect.left_top()+egui::vec2(12.,60.)).max_width((rect.width()-24.).max(100.)).max_height((rect.height()-24.).max(100.)).show(ui.ctx(),|ui|{
            ui.label(&feature.details);
            ui.label(format!("{:.5}, {:.5}",feature.position.latitude_deg,feature.position.longitude_deg));
            let waypoint=json!({"ident":feature.ident,"kind":feature.kind,"position":feature.position});
            ui.horizontal_wrapped(|ui|{
                if ui.button("Direct to").clicked(){self.command(s,"avionics.set",json!({"setting":{"kind":"glass","setting":{"kind":"direct_to_waypoint","waypoint":waypoint}}}));}
                if ui.button("Add to plan").clicked(){self.command(s,"plan.add",json!({"point":{"waypoint":waypoint,"altitude_m":1000.}}));}
                if ui.button("Start here").clicked(){self.command(s,"sim.reset",json!({"world":s.sim().world().choice(),"start":{"kind":"geographic_air","position":feature.position,"height_agl_m":600.,"airspeed_mps":45.,"heading_rad":0.},"paused":true}));}
                if ui.button("Pin").clicked() && let Ok(pins)=s.run(flightcraft_control::PinsList{}) {
                        let id=(1..=1001).map(|i|format!("map-pin-{i}")).find(|id|!pins.pins.iter().any(|p|&p.id==id)).unwrap_or_else(||"pin-limit".into());
                        self.command(s,"pins.add",json!({"pin":{"id":id,"name":feature.ident,"note":feature.details,"position":feature.position,"altitude_m":null}}));
                }
                if feature.airport {ui.add_enabled(false,egui::Button::new("Open plates · coming soon"));}
            });
        });
        if !open {
            self.selected = None;
        }
    }
    fn file_window(&mut self, ui: &egui::Ui, s: &mut Session) {
        let mut open = true;
        let dropped = ui.ctx().input(|i| i.raw.dropped_files.last().cloned());
        if let Some(file) = dropped {
            match files::dropped(&file) {
                Ok((format, text)) => {
                    self.format = format;
                    self.interchange = text;
                    self.file_notice = "File loaded; select Import to replace the editable plan".into();
                }
                Err(error) => self.error = Some(error),
            }
        }
        egui::Window::new("Flight plan import / export")
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                if self.format.is_empty() { self.format = "gpx".into(); }
                ui.horizontal(|ui| {
                    for format in ["gpx", "fpl", "json"] {
                        ui.selectable_value(&mut self.format, format.into(), format.to_uppercase());
                    }
                    if ui.button("Export file").clicked() {
                        match s.execute("plan.export", json!({"format":self.format})) {
                            Ok(value) => {
                                self.interchange = value.get("text").and_then(Value::as_str).unwrap_or_default().into();
                                match files::export(&self.format, &self.interchange) { Ok(notice) => self.file_notice = notice, Err(error) => self.error = Some(error) }
                            }
                            Err(e) => self.error = Some(e.to_string()),
                        }
                    }
                    if ui.button("Copy text").clicked() { ui.ctx().copy_text(self.interchange.clone()); }
                    if ui.button("Import").clicked() { self.command(s, "plan.import", json!({"format":self.format,"text":self.interchange})); }
                });
                ui.label("Drop a plan file or paste its text, then select Import. Desktop exports go to the local data directory; browser exports download a file.");
                if !self.file_notice.is_empty() { ui.label(&self.file_notice); }
                ui.add(egui::TextEdit::multiline(&mut self.interchange).char_limit(1024*1024).desired_rows(12).desired_width(500.));
                if let Some(error) = &self.error { ui.colored_label(theme::DANGER,error); }
            });
        self.files = open;
    }
}
fn base_name(base: BaseMap) -> &'static str {
    match base {
        BaseMap::Aero => "Aero",
        BaseMap::Terrain => "Terrain",
        BaseMap::Sectional => "VFR sectional",
        BaseMap::Tac => "Terminal area",
        BaseMap::Satellite => "Satellite",
        BaseMap::Street => "Street",
    }
}
fn orientation_name(up: MapUp) -> &'static str {
    match up {
        MapUp::NorthUp => "North up",
        MapUp::TrackUp => "Track up",
        MapUp::HeadingUp => "Heading up",
    }
}
fn class_name(class: Option<flightcraft_world::AirspaceClass>) -> &'static str {
    use flightcraft_world::AirspaceClass;
    match class {
        Some(AirspaceClass::A) => "A",
        Some(AirspaceClass::B) => "B",
        Some(AirspaceClass::C) => "C",
        Some(AirspaceClass::D) => "D",
        Some(AirspaceClass::E) => "E",
        None => "Mode C",
    }
}
fn overlay_name(overlay: Overlay) -> &'static str {
    match overlay {
        Overlay::Route => "Route",
        Overlay::Airspace => "Airspace",
        Overlay::TerrainWarnings => "Terrain warnings",
        Overlay::Airports => "Airports",
        Overlay::Navaids => "Navaids",
        Overlay::Fixes => "Fixes",
        Overlay::Airways => "Airways",
        Overlay::Pins => "Pins",
        Overlay::Aircraft => "Aircraft",
        Overlay::Track => "Track history",
        Overlay::RangeRings => "Range rings",
        Overlay::LiveWeather => "Live weather",
        Overlay::Tfr => "TFR",
        Overlay::Plates => "Plates",
    }
}
fn point_value(position: GeoPoint, altitude: f64) -> Value {
    json!({"waypoint":{"ident":"MAP","kind":"coordinate","position":position},"altitude_m":altitude})
}
fn hit(features: &[Feature], pos: Pos2) -> Option<&Feature> {
    features
        .iter()
        .filter(|f| f.pixel.distance(pos) <= 12.)
        .min_by(|a, b| a.pixel.distance_sq(pos).total_cmp(&b.pixel.distance_sq(pos)))
}
fn feature_at(s: &Session, v: &View, features: &[Feature], pos: Pos2) -> Option<Feature> {
    if let Some(feature) = hit(features, pos) {
        return Some(feature.clone());
    }
    if s.map_state().opacity(Overlay::Airspace) == 0. {
        return None;
    }
    let local = v.local_at(pos);
    let db = s.sim().world().airspace()?;
    let volume = db
        .volumes
        .iter()
        .filter(|volume| {
            volume.contains_lateral(local) || volume.boundary_distance_m(local) * v.pixels_per_m < 8.
        })
        .min_by(|a, b| {
            a.boundary_distance_m(local)
                .total_cmp(&b.boundary_distance_m(local))
        })?;
    let info = &volume.info;
    Some(Feature {
        position: v.position(pos)?,
        pixel: pos,
        ident: info.id.clone(),
        kind: "coordinate",
        details: format!(
            "{} · {:?} airspace\nFloor {:.0} ft {:?} · ceiling {:.0} ft {:?}",
            info.name,
            info.class,
            info.floor.value_ft,
            info.floor.reference,
            info.ceiling.value_ft,
            info.ceiling.reference
        ),
        airport: false,
    })
}
fn route_hit(s: &Session, v: &View, p: Pos2) -> Option<Drag> {
    for (i, w) in s.flight_plan().waypoints.iter().enumerate() {
        if v.point(w.waypoint.position).is_some_and(|q| q.distance(p) < 12.) {
            return Some(Drag::Waypoint(i));
        }
    }
    for (i, leg) in s.flight_plan().waypoints.windows(2).enumerate() {
        if let [a, b] = leg
            && let (Some(a), Some(b)) = (v.point(a.waypoint.position), v.point(b.waypoint.position))
            && segment_distance(p, a, b) < 8.
        {
            return Some(Drag::Leg(i));
        }
    }
    None
}
fn segment_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let d = b - a;
    let t = if d.length_sq() > 0. {
        ((p - a).dot(d) / d.length_sq()).clamp(0., 1.)
    } else {
        0.
    };
    p.distance(a + d * t)
}
fn terrain(p: &egui::Painter, v: &View, s: &Session, state: &MapState, warnings_only: bool, compact: bool) {
    let altitude = if compact {
        let computer = &s.sim().glass_computers().primary;
        computer.air_data_valid.then_some(computer.altitude_msl_m)
    } else {
        Some(s.sim().readout().altitude_msl_m)
    };
    let Some(region) = s.sim().world().region() else {
        return;
    };
    let [sn, se] = region.southwest_m;
    let span = f64::from(region.tile_size.saturating_sub(1)) * f64::from(region.spacing_m);
    let (nn, ee) = (
        sn + span * f64::from(region.tiles_north),
        se + span * f64::from(region.tiles_east),
    );
    let corners = [
        v.rect.left_top(),
        v.rect.right_top(),
        v.rect.left_bottom(),
        v.rect.right_bottom(),
    ]
    .map(|p| v.local_at(p));
    let visible_n = corners
        .iter()
        .map(|[n, _]| *n)
        .fold(f64::INFINITY, f64::min)
        .max(sn);
    let visible_e = corners
        .iter()
        .map(|[_, e]| *e)
        .fold(f64::INFINITY, f64::min)
        .max(se);
    let high_n = corners
        .iter()
        .map(|[n, _]| *n)
        .fold(f64::NEG_INFINITY, f64::max)
        .min(nn);
    let high_e = corners
        .iter()
        .map(|[_, e]| *e)
        .fold(f64::NEG_INFINITY, f64::max)
        .min(ee);
    if high_n <= visible_n || high_e <= visible_e {
        return;
    }
    // Sample only inside the grid, with vertices exactly on its boundary. Unknown vertices
    // never interpolate into known terrain, so the coverage edge cannot produce a dark band.
    let mut mesh = egui::Mesh::default();
    let mut known = Vec::new();
    let size = 48_u32;
    let low = if state.night {
        theme::TERRAIN_LOW
    } else {
        theme::MAP_DAY_LAND
    };
    let high = if state.night {
        theme::TERRAIN_HIGH
    } else {
        theme::MAP_DAY_HIGH
    };
    for y in 0..=size {
        for x in 0..=size {
            let n = high_n - (high_n - visible_n) * f64::from(y) / f64::from(size);
            let e = visible_e + (high_e - visible_e) * f64::from(x) / f64::from(size);
            let pos = v.local([n, e]);
            let ground = s
                .sim()
                .world()
                .terrain_ground(n.clamp(sn + 0.001, nn - 0.001), e.clamp(se + 0.001, ee - 0.001));
            known.push(ground.is_some());
            let color = if let Some(ground) = ground {
                let height = ground.height_msl_m;
                let band = altitude.map_or(flightcraft_control::TerrainBand::Clear, |altitude| {
                    flightcraft_control::terrain_band(altitude, height)
                });
                if state.opacity(Overlay::TerrainWarnings) > 0.
                    && band != flightcraft_control::TerrainBand::Clear
                {
                    (if band == flightcraft_control::TerrainBand::Warning {
                        theme::DANGER
                    } else {
                        theme::AMBER
                    })
                    .linear_multiply(state.opacity(Overlay::TerrainWarnings))
                } else if warnings_only {
                    theme::CLEAR
                } else if height <= 0. || ground.surface == flightcraft_world::Surface::Water {
                    if state.night {
                        theme::MAP_NIGHT_WATER
                    } else {
                        theme::MAP_DAY_WATER
                    }
                } else {
                    let color = egui::lerp(
                        egui::Rgba::from(low)..=egui::Rgba::from(high),
                        (height as f32 / 1400.).clamp(0., 1.),
                    );
                    let [nn, ne, nd] = ground.normal_ned;
                    let shade = if state.base_map == BaseMap::Terrain {
                        (0.70 + 0.3 * (-nn * 0.4 - ne * 0.4 - nd * 0.82)).clamp(0.4, 1.0)
                    } else {
                        1.
                    };
                    egui::Color32::from(color).linear_multiply(shade as f32)
                }
            } else if warnings_only {
                theme::CLEAR
            } else {
                theme::BORDER
            };
            mesh.colored_vertex(pos, color);
        }
    }
    for y in 0..size {
        for x in 0..size {
            let a = y * (size + 1) + x;
            for triangle in [[a, a + 1, a + size + 1], [a + 1, a + size + 2, a + size + 1]] {
                if triangle
                    .iter()
                    .all(|index| known.get(*index as usize) == Some(&true))
                {
                    mesh.indices.extend(triangle);
                }
            }
        }
    }
    p.add(egui::Shape::mesh(mesh));
}
fn paint_features(p: &egui::Painter, v: &View, s: &mut Session, state: &MapState) -> Vec<Feature> {
    let mut features = Vec::new();
    let text = if state.night {
        theme::TEXT
    } else {
        theme::MAP_DAY_TEXT
    };
    let font = FontId::proportional((v.rect.width() / 35.).clamp(5., 11.));
    let world = s.sim().world();
    let across_nm = f64::from(v.rect.width()) / v.pixels_per_m / 1852.0;
    let inset = v.rect.width().min(v.rect.height()) < 140.;
    if !inset
        && state.opacity(Overlay::Airspace) > 0.
        && let Some(db) = world.airspace()
    {
        let mut labels = Vec::<Pos2>::new();
        for volume in &db.volumes {
            let color = match volume.info.class {
                Some(flightcraft_world::AirspaceClass::B | flightcraft_world::AirspaceClass::D) => {
                    theme::AIRSPACE_BLUE
                }
                Some(flightcraft_world::AirspaceClass::C | flightcraft_world::AirspaceClass::E) => {
                    theme::GLASS_GPS
                }
                _ => theme::MUTED,
            }
            .linear_multiply(state.opacity(Overlay::Airspace));
            for ring in volume.polygons.iter().flatten() {
                let points: Vec<_> = ring.iter().map(|c| v.local(*c)).collect();
                for edge in points.windows(2) {
                    if let [a, b] = edge {
                        p.line_segment([*a, *b], Stroke::new(1.2, color));
                    }
                }
                if let Some(pos) = points.iter().find(|p| v.rect.shrink(40.).contains(**p)).copied() {
                    let floor = volume.info.floor;
                    let ceiling = volume.info.ceiling;
                    let details = format!(
                        "{} · {} airspace\nFloor {:.0} ft {:?} · ceiling {:.0} ft {:?}",
                        volume.info.name,
                        class_name(volume.info.class),
                        floor.value_ft,
                        floor.reference,
                        ceiling.value_ft,
                        ceiling.reference
                    );
                    if labels.iter().all(|other| other.distance(pos) > 75.) {
                        p.text(
                            pos,
                            Align2::LEFT_TOP,
                            format!(
                                "{} {:.0}/{:.0}",
                                class_name(volume.info.class),
                                floor.value_ft / 100.,
                                ceiling.value_ft / 100.
                            ),
                            font.clone(),
                            color,
                        );
                        labels.push(pos);
                    }
                    if let Some(position) = v.position(pos) {
                        features.push(Feature {
                            position,
                            pixel: pos,
                            ident: volume.info.id.clone(),
                            kind: "coordinate",
                            details,
                            airport: false,
                        });
                    }
                }
            }
        }
    }
    if let Some(nav) = world.navdata() {
        if !inset && state.opacity(Overlay::Airways) > 0. {
            for airway in &nav.routes {
                let color = theme::MUTED.linear_multiply(state.opacity(Overlay::Airways) * 0.25);
                for edge in airway.geometry.windows(2) {
                    if let [a, b] = edge
                        && let (Some(a), Some(b)) = (v.point(*a), v.point(*b))
                    {
                        p.line_segment([a, b], Stroke::new(0.45, color));
                    }
                }
            }
            if across_nm < state.declutter.airway_labels_across_nm {
                let mut labels = Vec::<Pos2>::new();
                for airway in &nav.routes {
                    if let Some(pos) = airway
                        .geometry
                        .iter()
                        .filter_map(|point| v.point(*point))
                        .find(|point| v.rect.shrink(24.).contains(*point))
                        && labels.iter().all(|other| other.distance(pos) > 70.)
                    {
                        p.text(
                            pos,
                            Align2::LEFT_BOTTOM,
                            &airway.ident,
                            FontId::proportional(10.),
                            theme::MUTED.linear_multiply(state.opacity(Overlay::Airways)),
                        );
                        labels.push(pos);
                    }
                }
            }
        }
        if !inset && state.opacity(Overlay::Navaids) > 0. {
            for navaid in &nav.navaids {
                if let Some(pos) = v.point(navaid.position).filter(|p| v.rect.contains(*p)) {
                    let color = theme::AIRSPACE_BLUE.linear_multiply(state.opacity(Overlay::Navaids));
                    p.circle_stroke(pos, 5., Stroke::new(1.5, color));
                    if state.range_nm < 60. {
                        p.circle_stroke(pos, 16., Stroke::new(0.7, color));
                        for i in 0..12 {
                            let a =
                                i as f32 * std::f32::consts::TAU / 12. + (v.north_grid_rad - v.angle) as f32;
                            let d = egui::vec2(a.sin(), -a.cos());
                            p.line_segment([pos + d * 13., pos + d * 17.], Stroke::new(0.7, color));
                        }
                    }
                    p.text(
                        pos + egui::vec2(8., 0.),
                        Align2::LEFT_CENTER,
                        &navaid.ident,
                        font.clone(),
                        color,
                    );
                    let frequency = navaid
                        .components
                        .iter()
                        .filter_map(|c| c.frequency_hz)
                        .map(|hz| {
                            if hz < 1e6 {
                                format!("{:.1} kHz", hz / 1000.)
                            } else {
                                format!("{:.3} MHz", hz / 1e6)
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    features.push(Feature {
                        position: navaid.position,
                        pixel: pos,
                        ident: navaid.ident.clone(),
                        kind: "navaid",
                        details: format!("{} · {:?}\n{}", navaid.name, navaid.kind, frequency),
                        airport: false,
                    });
                }
            }
        }
        if state.opacity(Overlay::Fixes) > 0. {
            for fix in &nav.fixes {
                let on_route = s
                    .flight_plan()
                    .waypoints
                    .iter()
                    .any(|w| w.waypoint.position.inverse(fix.position).0 < 100.)
                    || s.gps_map_waypoints()
                        .iter()
                        .any(|w| w.position.inverse(fix.position).0 < 100.);
                if (inset || across_nm >= state.declutter.fixes_across_nm) && !on_route {
                    continue;
                }
                if let Some(pos) = v.point(fix.position).filter(|p| v.rect.contains(*p)) {
                    let color = text.linear_multiply(state.opacity(Overlay::Fixes));
                    p.add(egui::Shape::closed_line(
                        vec![
                            pos + egui::vec2(0., -3.),
                            pos + egui::vec2(3., 3.),
                            pos + egui::vec2(-3., 3.),
                        ],
                        Stroke::new(0.7, color),
                    ));
                    if across_nm < state.declutter.fixes_across_nm || on_route {
                        p.text(
                            pos + egui::vec2(4., 0.),
                            Align2::LEFT_CENTER,
                            &fix.ident,
                            FontId::proportional(9.),
                            color,
                        );
                    }
                    features.push(Feature {
                        position: fix.position,
                        pixel: pos,
                        ident: fix.ident.clone(),
                        kind: "fix",
                        details: fix.name.clone(),
                        airport: false,
                    });
                }
            }
        }
    }
    if state.opacity(Overlay::Airports) > 0.
        && let Some(airports) = world.airports()
    {
        let mut labels = Vec::<Pos2>::new();
        let mut symbols = Vec::<Pos2>::new();
        for airport in &airports.airports {
            let position = GeoPoint {
                latitude_deg: airport.latitude_deg,
                longitude_deg: airport.longitude_deg,
            };
            if let Some(pos) = v.point(position).filter(|p| v.rect.contains(*p)) {
                if inset && symbols.iter().any(|p| p.distance(pos) < 16.) {
                    continue;
                }
                symbols.push(pos);
                let color = text.linear_multiply(state.opacity(Overlay::Airports));
                p.circle_stroke(pos, if inset { 2. } else { 4. }, Stroke::new(1., color));
                if !inset && state.range_nm < 35. {
                    for runway in &airport.runways {
                        let [a, b] = &runway.ends;
                        let point = |end: &flightcraft_world::RunwayEnd| {
                            v.point(GeoPoint {
                                latitude_deg: end.latitude_deg,
                                longitude_deg: end.longitude_deg,
                            })
                        };
                        if let (Some(a), Some(b)) = (point(a), point(b)) {
                            let axis = (b - a).normalized();
                            let side = egui::vec2(-axis.y, axis.x)
                                * ((runway.width_m * v.pixels_per_m) as f32 * 0.5).max(0.5);
                            p.add(egui::Shape::convex_polygon(
                                vec![a + side, b + side, b - side, a - side],
                                color,
                                Stroke::NONE,
                            ));
                        }
                    }
                }
                if !inset && labels.iter().all(|p| p.distance(pos) > 45.) {
                    p.text(
                        pos + egui::vec2(7., -7.),
                        Align2::LEFT_BOTTOM,
                        airport.icao.as_deref().unwrap_or(&airport.ident),
                        font.clone(),
                        color,
                    );
                    labels.push(pos);
                }
                let runways = airport
                    .runways
                    .iter()
                    .map(|r| {
                        format!(
                            "{} · {:.0} × {:.0} m · {:?}",
                            r.designator, r.length_m, r.width_m, r.surface
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                features.push(Feature {
                    position,
                    pixel: pos,
                    ident: airport.icao.as_deref().unwrap_or(&airport.ident).into(),
                    kind: "airport",
                    details: format!(
                        "{}\n{}\nFrequencies not supplied in airport catalogue",
                        airport.name, runways
                    ),
                    airport: true,
                });
            }
        }
    }
    if state.opacity(Overlay::Pins) > 0.
        && let Ok(pins) = s.run(flightcraft_control::PinsList {})
    {
        for pin in pins.pins {
            if let Some(pos) = v.point(pin.position).filter(|p| v.rect.contains(*p)) {
                let color = theme::AMBER.linear_multiply(state.opacity(Overlay::Pins));
                p.circle_filled(pos, 4., color);
                p.text(
                    pos + egui::vec2(6., 0.),
                    Align2::LEFT_CENTER,
                    &pin.name,
                    font.clone(),
                    color,
                );
                features.push(Feature {
                    position: pin.position,
                    pixel: pos,
                    ident: pin.name,
                    kind: "pin",
                    details: pin.note,
                    airport: false,
                });
            }
        }
    }
    features
}
fn route(
    p: &egui::Painter,
    v: &View,
    s: &mut Session,
    state: &MapState,
    engine_units: crate::instruments::EngineUnits,
) {
    let opacity = state.opacity(Overlay::Route);
    if opacity == 0. {
        return;
    }
    let color = theme::GLASS_GPS.linear_multiply(opacity);
    let summary = s.run(flightcraft_control::SummaryPlan {}).ok();
    for (i, leg) in s.flight_plan().waypoints.windows(2).enumerate() {
        if let [a, b] = leg
            && let (Some(a), Some(b)) = (v.point(a.waypoint.position), v.point(b.waypoint.position))
        {
            p.line_segment([a, b], Stroke::new(2.5, color));
            if v.rect.width() >= 350.
                && let Some(leg) = summary.as_ref().and_then(|s| s.legs.get(i))
            {
                let label = format!(
                    "{:03.0}° M · {:.1} NM · {} min",
                    leg.magnetic_course_deg,
                    leg.distance_m / 1852.,
                    leg.ete_s.map_or("—".into(), |t| format!("{:.0}", t / 60.))
                );
                let pos = a.lerp(b, 0.5);
                let rect = p.text(
                    pos,
                    Align2::CENTER_BOTTOM,
                    &label,
                    FontId::monospace(11.),
                    theme::PANEL,
                );
                p.rect_filled(rect.expand(3.), 2., theme::PANEL);
                p.text(pos, Align2::CENTER_BOTTOM, label, FontId::monospace(11.), color);
            }
        }
    }
    for w in &s.flight_plan().waypoints {
        if let Some(pos) = v.point(w.waypoint.position) {
            p.circle_filled(pos, 4., color);
            p.text(
                pos + egui::vec2(0., 7.),
                Align2::CENTER_TOP,
                &w.waypoint.ident,
                FontId::proportional(12.),
                color,
            );
        }
    }
    if v.rect.width() >= 400.
        && v.rect.height() >= 250.
        && let Some(summary) = summary
    {
        let text = format!(
            "ROUTE {:.1} NM · {} min · fuel {} + {} reserve · {} · {} conflicts",
            summary.distance_m / 1852.,
            summary.ete_s.map_or("—".into(), |t| format!("{:.0}", t / 60.)),
            summary
                .trip_fuel_kg
                .map_or("—".into(), |f| units::fuel(f, engine_units)),
            units::fuel(summary.reserve_fuel_kg, engine_units),
            units::speed(summary.cruise.airspeed_mps),
            summary.conflicts.len()
        );
        let galley = p.layout(
            text,
            FontId::monospace(11.),
            theme::GLASS_GPS,
            (v.rect.width() - 16.).max(20.),
        );
        let origin = v.rect.left_bottom() + egui::vec2(8., -34. - galley.size().y);
        p.rect_filled(
            Rect::from_min_size(origin, galley.size()).expand(3.),
            2.,
            theme::PANEL,
        );
        p.galley(origin, galley, theme::GLASS_GPS);
    }
}
fn rings(p: &egui::Painter, v: &View, state: &MapState) {
    let color = theme::MUTED.linear_multiply(state.opacity(Overlay::RangeRings));
    let radius = v.rect.width().min(v.rect.height()) * 0.5;
    if state.opacity(Overlay::RangeRings) > 0. {
        for f in [0.25, 0.5, 0.75] {
            p.circle_stroke(v.rect.center(), radius * f, Stroke::new(0.7, color));
        }
    }
    if v.rect.width() < 200. || v.rect.height() < 150. {
        return;
    }
    let length = (state.range_nm * 0.4).max(0.1);
    let pixels = (length * 1852. * v.pixels_per_m) as f32;
    let origin = v.rect.left_bottom() + egui::vec2(12., -60.);
    p.line_segment(
        [origin, origin + egui::vec2(pixels, 0.)],
        Stroke::new(2., theme::ACCENT),
    );
    p.text(
        origin - egui::vec2(0., 3.),
        Align2::LEFT_BOTTOM,
        format!("{length:.1} NM"),
        FontId::monospace(11.),
        theme::ACCENT,
    );
}
