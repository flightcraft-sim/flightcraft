//! Render the main shell with the browser's serialized Worker snapshot source, without a GPU.
#![allow(clippy::unwrap_used)]
use crate::{FlightCraftApp, theme};
use eframe::egui::{self, Pos2, Rect, Vec2};
use flightcraft_control::{Session, presentation::Snapshot, screens::PageKind};
use serde_json::json;

struct BrowserMain {
    owner: Session,
    app: FlightCraftApp,
    ctx: egui::Context,
    time: f64,
    text: Vec<(String, Rect)>,
}
impl BrowserMain {
    fn new() -> Self {
        let mut seed = Session::new().unwrap();
        // A control-only center needs no GPU, while retaining the complete main-window shell.
        seed.execute(
            "screen.set_page",
            json!({"id":0,"page":{"kind":PageKind::Control}}),
        )
        .unwrap();
        Self::with_seed(seed)
    }
    fn with_seed(seed: Session) -> Self {
        let owner = Session::from_worker_seed(seed.worker_seed()).unwrap();
        let snapshot: Snapshot =
            serde_json::from_str(&serde_json::to_string(&owner.screen_snapshot()).unwrap()).unwrap();
        let mut presentation = Session::new().unwrap();
        presentation.apply_screen_snapshot(snapshot).unwrap();
        let app = FlightCraftApp::from_presentation(presentation, None, "WebGL 2", None).unwrap();
        let ctx = egui::Context::default();
        theme::install(&ctx);
        Self {
            owner,
            app,
            ctx,
            time: 0.,
            text: Vec::new(),
        }
    }
    fn render(&mut self, events: Vec<egui::Event>) {
        self.time += 1. / 60.;
        let output = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 1200.))),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |ctx| {
                self.app.show_main(ctx, |_, _| Ok(()));
            },
        );
        fn collect(shape: egui::Shape, text: &mut Vec<(String, Rect)>) {
            match shape {
                egui::Shape::Text(s) => text.push((
                    s.galley.text().into(),
                    Rect::from_min_size(s.pos, s.galley.size()),
                )),
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, text);
                    }
                }
                _ => (),
            }
        }
        self.text.clear();
        for shape in output.shapes {
            collect(shape.shape, &mut self.text);
        }
    }
    fn has(&self, label: &str) -> bool {
        self.text.iter().any(|(s, _)| s == label)
    }
    fn click(&mut self, label: &str) {
        let pos = self
            .text
            .iter()
            .find(|(s, _)| s == label)
            .unwrap_or_else(|| panic!("missing {label}: {:?}", self.text))
            .1
            .center();
        self.render(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
        ]);
        self.render(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }]);
        // Windows and popup content can be deferred until the following pass.
        for _ in 0..4 {
            self.render(Vec::new());
        }
    }
}

#[test]
fn worker_snapshot_main_renders_native_header_and_windows() {
    let mut web = BrowserMain::new();
    web.render(Vec::new());
    for label in [
        "FlightCraft",
        "Weather",
        "Scenarios",
        "Procedures / NAV",
        "World",
        "Screens",
        "Preferences",
    ] {
        assert!(web.has(label), "missing main menu {label}");
    }
    let header: Vec<_> = web
        .text
        .iter()
        .filter(|(s, _)| {
            [
                "FlightCraft",
                "Weather",
                "Scenarios",
                "Procedures / NAV",
                "World",
                "Screens",
                "Preferences",
            ]
            .contains(&s.as_str())
        })
        .collect();
    let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 1200.));
    for (i, (label, rect)) in header.iter().enumerate() {
        assert!(
            viewport.contains_rect(*rect),
            "{label} is outside the main window: {rect:?}"
        );
        for (other, other_rect) in header.iter().skip(i + 1) {
            assert!(
                !rect.intersects(*other_rect),
                "header controls overlap: {label}, {other}"
            );
        }
    }
    let menu_bottom = header.iter().map(|(_, r)| r.bottom()).fold(0.0_f32, f32::max);
    assert!(web.text.iter().any(|(s, _)| s.starts_with("Map [")));
    web.click("Scenarios");
    assert!(web.app.scenarios.open, "scenario button did not open");
    assert!(web.has("Scenarios and emergencies"), "{:?}", web.text);
    assert!(web.has("Scenario library"));
    let title = web
        .text
        .iter()
        .find(|(s, _)| s == "Scenarios and emergencies")
        .map(|(_, r)| *r)
        .expect("scenario window title");
    assert!(
        title.top() > menu_bottom,
        "a floating window covers the header menus: {title:?} vs {menu_bottom}"
    );
    web.click("Scenarios");
    assert!(
        !web.app.scenarios.open,
        "the header button must close the window it opened"
    );
    web.click("Preferences");
    assert!(web.has("Preferences and input"));
    web.click("Preferences");
    web.click("Procedures / NAV");
    assert!(web.has("CDI source"));
    assert!(web.has("NAV1"));
}

#[test]
fn weather_menu_queues_commands_to_worker_without_stepping_presentation() {
    let mut web = BrowserMain::new();
    let time = web.owner.sim().state().time_s;
    let original = web.owner.sim().conditions().clone();
    web.render(Vec::new());
    web.click("Weather");
    assert!(web.has("Weather · cloud heights ft MSL"));
    web.click("Night");
    let commands = web.app.session.take_screen_commands();
    assert!(commands.iter().any(|(command, _)| command == "weather.set"));
    assert_eq!(web.owner.sim().conditions(), &original);
    assert_eq!(web.app.session.sim().state().time_s, time);
    for (command, input) in commands {
        web.owner.execute(&command, input).unwrap();
    }
    web.app
        .session
        .apply_screen_snapshot(web.owner.screen_snapshot())
        .unwrap();
    assert_eq!(
        web.app.session.sim().conditions().time_of_day,
        flightcraft_world::TimeOfDay::Night
    );
    assert_eq!(web.owner.sim().state().time_s, time);
}

#[test]
fn planning_map_releases_the_visible_panels_equipment_hold() {
    let mut seed = Session::new().unwrap();
    let pack = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/packs/equipment-demo");
    seed.install_pack(&flightcraft_control::read_pack(&pack).unwrap())
        .unwrap();
    seed.execute(
        "aircraft.load",
        json!({"source":{"kind":"installed","id":"equipment-demonstrator"},"reset":true}),
    )
    .unwrap();
    seed.execute(
        "screen.set_page",
        json!({"id":0,"page":{"kind":"panel_item","item":"key"}}),
    )
    .unwrap();
    let mut web = BrowserMain::with_seed(seed);
    web.render(Vec::new());
    web.render(Vec::new());
    let pos = web.text.iter().find(|(s, _)| s == "START").unwrap().1.center();
    web.render(vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        },
    ]);
    let start = web.app.session.take_screen_commands();
    assert!(
        start.iter().any(|(command, input)| command == "equipment.set"
            && input
                .pointer("/setting/setting/position")
                .and_then(serde_json::Value::as_str)
                == Some("start")),
        "{start:?}"
    );
    // Opening the map hides the page while its START contact is still down.
    web.app.planning_map.open = true;
    web.render(Vec::new());
    let released = web.app.session.take_screen_commands();
    assert!(
        released.iter().any(|(command, input)| command == "equipment.set"
            && input
                .pointer("/setting/setting/position")
                .and_then(serde_json::Value::as_str)
                == Some("on")),
        "{released:?}"
    );
    web.render(Vec::new());
    assert!(
        !web.app
            .session
            .take_screen_commands()
            .iter()
            .any(|(command, _)| command == "equipment.set")
    );
}
