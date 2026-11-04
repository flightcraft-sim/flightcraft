//! The Sling 2 cockpit controls driven the way a pilot drives them: pointer events on the dash
//! panel, no commands and no Show me. Every setting the panel emits goes through the same
//! `systems.set` command the application, MCP, and scripts use.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::instruments::{InstrumentInput, InstrumentPanel};
use crate::modern::cockpit;
use eframe::egui::{self, Event, PointerButton, Pos2, Rect};
use flightcraft_control::{Session, SetEquipment, SetSystems};
use serde_json::{Value, json};

const SIZE: egui::Vec2 = egui::vec2(1920., 1080.);

pub(crate) struct Rig {
    pub s: Session,
    panel: InstrumentPanel,
    ctx: egui::Context,
    pub pointer: Pos2,
}

impl Rig {
    /// The distributed-layout Sling 2 pack (test fixture copy), parked with the brakes released as
    /// the application leaves it after its first frame.
    pub fn new() -> Self {
        let mut s = Session::new().unwrap();
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../control/tests/fixtures/sling-pack");
        s.install_pack(&flightcraft_control::read_pack(&root).unwrap())
            .unwrap();
        s.execute(
            "aircraft.load",
            json!({"source":{"kind":"installed","id":"sling2"},"reset":true}),
        )
        .unwrap();
        s.execute("sim.set_controls", json!({"brake": 0.})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut rig = Self {
            s,
            panel: InstrumentPanel::default(),
            ctx,
            pointer: Pos2::ZERO,
        };
        rig.frame(Vec::new());
        rig
    }

    /// Start a lesson, leaving the pilot's brake lever released.
    pub fn lesson(id: &str) -> Self {
        let mut rig = Self::new();
        rig.s
            .execute("procedure_lesson.start", json!({"id": id}))
            .unwrap();
        rig.s.execute("sim.set_controls", json!({"brake": 0.})).unwrap();
        rig.frame(Vec::new());
        rig
    }

    pub fn run(&mut self, command: &str, input: Value) -> Value {
        self.s.execute(command, input).unwrap()
    }

    pub fn step(&mut self, seconds: f64) {
        self.run("sim.step", json!({"steps": (seconds * 120.).round() as u32}));
    }

    /// One frame of the dash, then the commands it produced.
    pub fn frame(&mut self, events: Vec<Event>) {
        let config = self.s.aircraft_configuration().unwrap().clone();
        let state = self.s.glass_state();
        let pilot = self.s.sim().controls();
        let generation = self.s.flight_generation();
        let time = self.s.sim().state().time_s;
        let mut settings = Vec::new();
        let _ = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SIZE)),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        settings = self
                            .panel
                            .show(
                                ui,
                                &config.panel,
                                InstrumentInput::new(self.s.sim().systems(), time)
                                    .with_pilot(pilot)
                                    .with_aircraft(&config)
                                    .with_glass(&state),
                                generation,
                            )
                            .unwrap();
                    });
            },
        );
        for setting in settings {
            self.s.run(SetSystems { setting }).unwrap();
        }
        for setting in self.panel.take_equipment_commands() {
            self.s.run(SetEquipment { setting }).unwrap();
        }
    }

    /// The element's rectangle on screen.
    pub fn cell(&self, id: &str) -> Rect {
        let config = self.s.aircraft_configuration().unwrap();
        let equipment = config.panel.equipment.as_ref().unwrap();
        let e = equipment.elements.iter().find(|e| e.id == id).unwrap();
        let rect = crate::panel_preview::equipment_rect(equipment, Rect::from_min_size(Pos2::ZERO, SIZE));
        let scale = rect.width() / equipment.size[0] as f32;
        Rect::from_min_size(
            rect.min + egui::vec2(e.position[0] as f32, e.position[1] as f32) * scale,
            egui::vec2(e.size[0] as f32, e.size[1] as f32) * scale,
        )
    }

    fn button(&self, pressed: bool) -> Event {
        Event::PointerButton {
            pos: self.pointer,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// Press at `at` and keep the button down.
    pub fn press(&mut self, at: Pos2) {
        self.pointer = at;
        let events = vec![Event::PointerMoved(at), self.button(true)];
        self.frame(events);
    }

    /// Keep holding for `seconds` of simulated time, a frame every tenth of a second.
    pub fn hold(&mut self, seconds: f64) {
        for _ in 0..(seconds * 10.).round() as usize {
            self.frame(Vec::new());
            self.step(0.1);
        }
    }

    pub fn release(&mut self) {
        let events = vec![self.button(false)];
        self.frame(events);
        self.frame(vec![Event::PointerGone]);
    }

    pub fn click_at(&mut self, at: Pos2) {
        self.press(at);
        self.frame(Vec::new());
        self.release();
    }

    /// Click the middle of an element.
    pub fn click(&mut self, id: &str) {
        self.click_at(self.cell(id).center());
    }

    pub fn throttle_to(&mut self, value: f64) {
        let slot = cockpit::throttle_slot(self.cell("throttle"));
        let y = slot.bottom() - value as f32 * slot.height();
        self.press(egui::pos2(slot.center().x, y));
        self.frame(Vec::new());
        self.release();
    }

    pub fn controls(&self) -> flightcraft_sim::Controls {
        self.s.sim().controls()
    }

    pub fn systems(&self) -> &flightcraft_sim::systems::SystemsState {
        self.s.sim().systems()
    }
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.02
}

#[test]
fn master_switch_is_on_the_dash_and_switches_the_main_bus() {
    let mut rig = Rig::new();
    let master = rig.systems().controls.master;
    rig.click("master_switch");
    assert_ne!(
        rig.systems().controls.master,
        master,
        "the click moved the switch"
    );
    rig.click("master_switch");
    assert_eq!(rig.systems().controls.master, master);
    rig.click("master_switch");
    rig.step(0.5);
    let on = rig.systems().controls.master;
    assert_eq!(rig.systems().electrical.main_bus_voltage > 9., on);
}

#[test]
fn throttle_lever_follows_the_pointer_and_the_wheel_over_its_slot() {
    let mut rig = Rig::new();
    for value in [0.5, 1.0, 0.0, 0.25] {
        rig.throttle_to(value);
        assert!(
            near(rig.controls().throttle, value),
            "{value}: {:?}",
            rig.controls()
        );
    }
    rig.throttle_to(0.5);
    let before = rig.controls().throttle;
    let at = rig.cell("throttle").center();
    rig.frame(vec![
        Event::PointerMoved(at),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0., 1.),
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    assert!(
        rig.controls().throttle >= before,
        "the wheel never reduces power forward"
    );
    // More throttle really is more engine: fuel flow rises with the lever.
    rig.throttle_to(0.0);
    rig.step(0.5);
    let idle = rig.systems().engine_power_fraction;
    rig.throttle_to(1.0);
    rig.step(0.5);
    assert!(rig.systems().engine_power_fraction >= idle);
}

#[test]
fn flap_selector_has_a_detent_per_stage_and_sets_the_flaps() {
    let mut rig = Rig::new();
    let stages: Vec<f64> = rig
        .s
        .aircraft_configuration()
        .unwrap()
        .variant
        .flap_stages
        .iter()
        .map(|s| s.position)
        .collect();
    assert_eq!(stages.len(), 4, "POH 7.2: positions 0, 1, 2, 3");
    let cell = rig.cell("flap_selector");
    for index in [2, 3, 1, 0] {
        rig.click_at(cockpit::flap_target(cell, index, stages.len()));
        assert!(near(rig.controls().flaps, stages[index]), "detent {index}");
    }
}

#[test]
fn hand_brake_lever_brakes_while_held_and_the_park_valve_keeps_them_on() {
    let mut rig = Rig::new();
    assert!(rig.controls().brake < 0.01 && !rig.systems().controls.park_brake);
    let lever = rig.cell("brake_lever").center();
    rig.press(lever);
    rig.frame(Vec::new());
    assert!(rig.controls().brake > 0.9, "held");
    rig.release();
    assert!(rig.controls().brake < 0.01, "released with the pointer");
    assert!(!rig.systems().controls.park_brake);
    rig.click("park_brake");
    assert!(rig.systems().controls.park_brake);
    assert!(
        rig.controls().brake < 0.01,
        "the lever is not squeezed; the valve holds the pressure"
    );
    // The valve really holds the wheels: run-up power does not move the aircraft, and with the
    // valve open it rolls.
    rig.throttle_to(0.5);
    rig.step(4.);
    assert!(
        rig.s.sim().readout().ground_speed_mps < 1.,
        "parked: {:?}",
        rig.s.sim().readout().ground_speed_mps
    );
    rig.click("park_brake");
    assert!(!rig.systems().controls.park_brake);
    rig.step(4.);
    assert!(rig.s.sim().readout().ground_speed_mps > 2.);
}

#[test]
fn stick_trim_buttons_trim_nose_down_and_up_and_are_slow() {
    let mut rig = Rig::new();
    let cell = rig.cell("pilot_stick");
    assert_eq!(rig.systems().controls.trim, 0.);
    rig.press(cockpit::stick_button(cell, 3));
    rig.hold(1.0);
    rig.release();
    let up = rig.systems().controls.trim;
    assert!(up > 0.02 && up < 0.3, "button 3 trims up: {up}");
    rig.press(cockpit::stick_button(cell, 1));
    rig.hold(2.0);
    rig.release();
    let down = rig.systems().controls.trim;
    assert!(down < up, "button 1 trims down: {down} < {up}");
    // The trim is felt: the elevator the airframe sees carries it.
    assert!(rig.s.sim().effective_controls().elevator < 0.);
}

#[test]
fn every_new_control_sits_on_the_panel_and_clear_of_the_displays() {
    let rig = Rig::new();
    let config = rig.s.aircraft_configuration().unwrap();
    let equipment = config.panel.equipment.as_ref().unwrap();
    let all = |ids: &[&str]| -> Vec<Rect> { ids.iter().map(|id| rig.cell(id)).collect() };
    let displays = all(&["left_display", "right_display"]);
    let controls = all(&[
        "master_switch",
        "flap_selector",
        "pilot_stick",
        "throttle",
        "brake_lever",
        "park_brake",
    ]);
    for (i, c) in controls.iter().enumerate() {
        for d in &displays {
            assert!(!c.intersects(*d), "control {i} overlaps a display");
        }
        for (j, other) in controls.iter().enumerate() {
            assert!(
                i == j || !c.intersects(other.shrink(0.5)),
                "controls {i} and {j} overlap"
            );
        }
    }
    equipment.validate().unwrap();
}

#[test]
fn park_brake_key_toggles_and_is_bindable() {
    use crate::keymap::{Action, Keymap};
    let map = Keymap::default();
    assert!(
        map.bindings
            .get(&Action::ParkBrake)
            .is_some_and(|k| !k.is_empty())
    );
    assert!(Action::ALL.contains(&Action::ParkBrake));
    assert!(map.bindings.contains_key(&Action::Brakes));
}

impl Rig {
    fn state(&mut self) -> Value {
        self.run("checklist.state", json!({}))
    }

    /// The pilot's own tick for items the simulator cannot verify (the checklist's button).
    fn tick(&mut self, item: &str) {
        self.run("checklist.complete", json!({"item": item}));
    }

    fn fuel_selector_left(&mut self) {
        let cell = self.cell("fuel_selector");
        let radius = cell.height() * 0.32;
        let angle = 225_f32.to_radians();
        self.click_at(cell.center() + egui::vec2(angle.cos(), angle.sin()) * radius * 1.35);
    }

    /// A guarded toggle: lift the guard, then move the lever.
    fn guarded_toggle(&mut self, id: &str) {
        let cell = self.cell(id);
        self.click_at(egui::pos2(cell.center().x, cell.top() + cell.height() * 0.49));
        self.click_at(egui::pos2(cell.center().x, cell.top() + cell.height() * 0.70));
    }

    /// Hold the key at START until the engine runs, then let go.
    fn start_engine(&mut self) {
        let cell = self.cell("ignition");
        self.press(egui::pos2(
            cell.left() + cell.width() * 0.86,
            cell.top() + cell.height() * 0.60,
        ));
        for _ in 0..90 {
            self.frame(Vec::new());
            self.step(0.1);
            if self.systems().engine_running {
                break;
            }
        }
        self.release();
    }

    /// Move the lever until the engine turns between `low` and `high` rpm; false if none does.
    fn throttle_for_rpm(&mut self, low: f64, high: f64) -> bool {
        for step in 1..=40 {
            self.throttle_to(f64::from(step) * 0.025);
            self.step(3.);
            if (low..=high).contains(&self.systems().engine_rpm) {
                return true;
            }
        }
        false
    }

    /// The pilot's action for one item, by panel element only.
    fn act(&mut self, list: &str, item: &str) {
        match (list, item) {
            ("before_start", "brakes") | ("warm_up", "brakes") => {
                if self.controls().brake < 0.5 && !self.systems().controls.park_brake {
                    self.click("park_brake");
                }
            }
            ("engine_start", "master") => self.click("master_switch"),
            ("engine_start", "efis_backup") => self.click("efis_backup_switch"),
            ("engine_start", "lane_a") => self.guarded_toggle("lane_a"),
            ("engine_start", "lane_b") => self.guarded_toggle("lane_b"),
            ("engine_start", "ems_backup") | ("warm_up", "ems_off") => self.click("ems_backup"),
            ("engine_start", "lane_lights") => {
                self.step(6.);
                self.tick("lane_lights");
            }
            ("engine_start", "fuel_selector") => self.fuel_selector_left(),
            ("engine_start", "throttle_idle") => self.throttle_to(0.),
            ("engine_start", "pump_1") => self.click("pump1"),
            ("engine_start", "pump_2") => self.click("pump2"),
            ("engine_start", "starter") => self.start_engine(),
            ("engine_start", "smooth_running") | ("warm_up", "warm_2000") => {
                assert!(self.throttle_for_rpm(1900., 2100.), "no throttle gives 2,000 rpm");
                if item == "warm_2000" {
                    self.step(130.);
                }
            }
            ("engine_start", "avionics") => self.click("avionics_switch"),
            ("warm_up", "alt_b") => {
                assert!(self.throttle_for_rpm(3000., 3300.), "no throttle gives 3,000 rpm");
                self.step(3.);
            }
            ("warm_up", "warm_2500") => {
                assert!(self.throttle_for_rpm(2450., 2600.), "no throttle gives 2,500 rpm");
            }
            ("warm_up", "oil_temp") => {
                for _ in 0..60 {
                    if self.systems().oil_temperature_c >= 50. {
                        break;
                    }
                    self.step(10.);
                }
            }
            // The engine does these on its own.
            ("engine_start", "oil_pressure") => self.step(1.),
            (_, other) => {
                let st = self.state();
                let verifiable = st["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|i| i["id"] == other)
                    .is_some_and(|i| i["verifiable"] == true);
                assert!(
                    !verifiable,
                    "{list}/{other} is verifiable but the test has no panel action for it"
                );
                self.tick(other);
            }
        }
    }
}

/// POH 4.5.1, 4.5.2 (with supplement 9.4.12) and 4.5.3, from cold and dark, using nothing but the
/// panel's own controls: no Show me and no command other than the pilot's tick on items the
/// simulator cannot verify.
#[test]
fn the_cold_and_dark_lesson_is_completed_from_the_panel_alone() {
    let mut rig = Rig::lesson("sling-before-start");
    assert!(!rig.systems().controls.master, "cold and dark");
    assert!(!rig.systems().engine_running);
    let mut acted = std::collections::BTreeSet::new();
    for _ in 0..200 {
        let debrief = rig.run("procedure_lesson.debrief", json!({}));
        if debrief["complete"] == true {
            break;
        }
        let st = rig.state();
        let list = st["active"].as_str().unwrap().to_owned();
        let Some(current) = st["current"].as_u64() else {
            rig.step(1.);
            continue;
        };
        let item = st["items"][current as usize]["id"].as_str().unwrap().to_owned();
        assert!(
            acted.insert((list.clone(), item.clone())),
            "{list}/{item} did not complete after its panel action: {}",
            st["items"][current as usize]
        );
        rig.act(&list, &item);
        rig.step(1.);
    }
    let debrief = rig.run("procedure_lesson.debrief", json!({}));
    assert_eq!(debrief["complete"], true, "{debrief}");
    for task in debrief["tasks"].as_array().unwrap() {
        assert_eq!(task["result"], "pass", "{task}");
    }
    assert!(rig.systems().engine_running);
    assert!(rig.systems().controls.master);
}
