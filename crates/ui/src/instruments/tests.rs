use super::*;
use flightcraft_control::Session;
use flightcraft_sim::systems::{SystemSetting, SystemSwitch};

fn update(instrument: &mut dyn Instrument, systems: &SystemsState, time_s: f64) -> DrawList {
    instrument.update(InstrumentInput::new(systems, time_s))
}

#[test]
fn every_module_has_bounded_deterministic_lag_and_pauses() {
    let registry = Registry::default();
    for id in flightcraft_aero::INSTRUMENT_MODULES
        .iter()
        .filter(|id| !id.starts_with("electronic_") && !id.starts_with("glass_"))
    {
        let mut s = SystemsState::default();
        s.sensed.attitude_valid = true;
        s.tanks = [flightcraft_aero::Wing::Left, flightcraft_aero::Wing::Right]
            .into_iter()
            .map(|wing| flightcraft_sim::systems::TankState {
                id: format!("{wing:?}"),
                wing,
                capacity_kg: 20.,
                unusable_kg: 1.,
                quantity_kg: 0.,
                indicated_kg: 0.,
            })
            .collect();
        let mut a = registry.instrument(id).unwrap();
        let mut b = registry.instrument(id).unwrap();
        let initial = update(a.as_mut(), &s, 0.).indicated;
        update(b.as_mut(), &s, 0.);
        s.sensed.indicated_airspeed_mps = 1e6;
        s.sensed.altitude_msl_m = 1e6;
        s.sensed.vertical_speed_mps = 1e6;
        s.sensed.roll_rad = 1e6;
        s.sensed.heading_rad = 2.;
        s.sensed.turn_rate_rps = 1e6;
        s.engine_rpm = 1e6;
        s.oil_temperature_c = 1e6;
        s.oil_pressure_pa = 1e12;
        s.egt_c = 1e6;
        s.cht_c = 1e6;
        for t in &mut s.tanks {
            t.indicated_kg = 1e6;
        }
        s.fuel_flow_kg_s = 1e6;
        s.electrical.battery_current_a = -1e6;
        s.electrical.main_bus_voltage = 1e6;
        s.suction_pa = 1e12;
        let paused = update(a.as_mut(), &s, 0.);
        assert_eq!(paused.indicated, initial, "{id}");
        let coarse = update(a.as_mut(), &s, 0.5);
        for i in 1..=60 {
            update(b.as_mut(), &s, f64::from(i) / 120.);
        }
        let fine = update(b.as_mut(), &s, 0.5);
        assert!((coarse.indicated - fine.indicated).abs() < 1e-8, "{id}");
        let settled = update(a.as_mut(), &s, 1000.);
        assert!(settled.indicated.is_finite(), "{id}");
        let limit = match *id {
            "airspeed" => 200.,
            "attitude" => std::f64::consts::PI,
            "altimeter" => 30000.,
            "turn_coordinator" => 6.,
            "heading" => 2_f64.to_degrees(),
            "vertical_speed" => 2000.,
            "tachometer" => 3500.,
            "oil_temperature" => 160.,
            "oil_pressure" => 700.,
            "egt" => 1200.,
            "cht" => 350.,
            "fuel_left" | "fuel_right" => 20.,
            "fuel_flow" => 80.,
            "ammeter" => 20.,
            "volts" => 32.,
            "suction" => 25.,
            "annunciators" => 0.,
            _ => f64::NAN,
        };
        assert!((settled.indicated - limit).abs() < 1e-10, "{id}");
        assert!(settled.items.iter().all(valid_item), "{id}: {:?}", settled.items);
        if *id != "annunciators" {
            assert_ne!(coarse.indicated, initial, "{id} must move");
            assert_ne!(coarse.indicated, settled.indicated, "{id} must lag");
        }
        // Rewinding time resets presentation memory rather than integrating backwards.
        let mut fresh = registry.instrument(id).unwrap();
        assert_eq!(
            update(a.as_mut(), &s, 0.).indicated,
            update(fresh.as_mut(), &s, 0.).indicated
        );
    }
}

#[test]
fn vacuum_failure_command_invalidates_and_tumbles_the_display() {
    let mut session = Session::new().unwrap();
    session
        .execute(
            "failure.set",
            serde_json::json!({"failure":{"kind":"vacuum_pump"}}),
        )
        .unwrap();
    // Braked idle is stable while the vacuum gyro spins down.
    session
        .execute("sim.set_controls", serde_json::json!({"brake":1.0}))
        .unwrap();
    let mut indicator = Registry::default().instrument("attitude").unwrap();
    assert!(update(indicator.as_mut(), session.sim().systems(), 0.).valid);
    session
        .execute("sim.step", serde_json::json!({"steps":12000}))
        .unwrap();
    assert!(!session.sim().readout().crashed);
    let s = session.sim().systems();
    assert!(!s.sensed.attitude_valid);
    let draw = update(indicator.as_mut(), s, session.sim().state().time_s);
    assert!(!draw.valid);
    assert!(draw.indicated.abs() > 0.01);
    assert!(
        draw.items
            .iter()
            .any(|item| matches!(item,DrawItem::Label { text,.. } if text == "GYRO INVALID"))
    );
}

#[test]
fn sensed_values_drive_pitot_vsi_heading_and_electric_turn_failures() {
    let mut session = Session::new().unwrap();
    session
        .execute(
            "sim.reset",
            serde_json::json!({"start":{"kind":"air","height_m":1000.,"airspeed_mps":40.,"heading_rad":0.}}),
        )
        .unwrap();
    session
        .execute(
            "failure.set",
            serde_json::json!({"failure":{"kind":"pitot_blocked"}}),
        )
        .unwrap();
    session
        .execute(
            "failure.set",
            serde_json::json!({"failure":{"kind":"static_blocked"}}),
        )
        .unwrap();
    let registry = Registry::default();
    let mut airspeed = registry.instrument("airspeed").unwrap();
    let frozen = update(airspeed.as_mut(), session.sim().systems(), 0.).indicated;
    session
        .execute("sim.step", serde_json::json!({"steps":120}))
        .unwrap();
    assert_eq!(
        update(airspeed.as_mut(), session.sim().systems(), 1.).indicated,
        frozen
    );
    let mut vsi = registry.instrument("vertical_speed").unwrap();
    assert_eq!(update(vsi.as_mut(), session.sim().systems(), 1.).indicated, 0.);
    session
        .run(flightcraft_control::SetSystems {
            setting: SystemSetting::Switch {
                switch: SystemSwitch::Master,
                on: false,
            },
        })
        .unwrap();
    session
        .execute("sim.step", serde_json::json!({"steps":1}))
        .unwrap();
    let mut turn = registry.instrument("turn_coordinator").unwrap();
    assert!(!update(turn.as_mut(), session.sim().systems(), 2.).valid);
    assert_eq!(update(turn.as_mut(), session.sim().systems(), 3.).indicated, 0.);
    let mut s = session.sim().systems().clone();
    let mut heading = registry.instrument("heading").unwrap();
    s.sensed.heading_rad = 359_f64.to_radians();
    update(heading.as_mut(), &s, 0.);
    s.sensed.heading_rad = 1_f64.to_radians();
    let d = update(heading.as_mut(), &s, 0.1);
    assert!(d.indicated > 359. || d.indicated < 1.);
}

#[test]
fn alternator_failure_drives_discharge_needle_and_annunciator() {
    let mut session = Session::new().unwrap();
    session
        .execute(
            "failure.set",
            serde_json::json!({"failure":{"kind":"alternator"}}),
        )
        .unwrap();
    session
        .execute("sim.step", serde_json::json!({"steps":120}))
        .unwrap();
    let mut gauge = Registry::default().instrument("ammeter").unwrap();
    assert!(update(gauge.as_mut(), session.sim().systems(), 1.).indicated < 0.);
    let mut annunciator = Registry::default().instrument("annunciators").unwrap();
    let mut s = session.sim().systems().clone();
    s.electrical.main_bus_voltage = 10.;
    assert!(
        update(annunciator.as_mut(), &s, 1.)
            .items
            .iter()
            .any(|i| matches!(i,DrawItem::Label { text,ink:Ink::Warning,.. } if text == "LOW VOLT"))
    );
}

#[test]
fn skins_and_panels_load_atomically_and_reject_hostile_art() {
    let catalog = flightcraft_aero::AircraftCatalog::builtins().unwrap();
    let panel = catalog.resolve_aircraft("corvid").unwrap().panel;
    let mut runtime = InstrumentPanel::default();
    runtime.load(&panel).unwrap();
    let mut private = panel.clone();
    private.instruments[0].skin = "private-face".into();
    assert!(runtime.load(&private).is_err());
    assert_eq!(runtime.loaded.as_ref(), Some(&panel));
    runtime.registry.register_pack_skin(r#"{"id":"private-face","colors":[["needle","amber"]],"face":[{"kind":"circle","center":[100,88],"radius":66,"ink":"face"}]}"#).unwrap();
    runtime.load(&private).unwrap();
    assert!(
        runtime
            .registry
            .register_pack_skin(
                r#"{"id":"bad","face":[{"kind":"circle","center":[0,0],"radius":-10,"ink":"face"}]}"#
            )
            .is_err()
    );
    private.instruments[0].module = "missing".into();
    assert!(runtime.load(&private).is_err());
    assert!(runtime.registry.instrument("missing").is_err());
}

#[test]
fn both_aircraft_layouts_keep_aspect_and_paint_valid_geometry_when_cropped() {
    let catalog = flightcraft_aero::AircraftCatalog::builtins().unwrap();
    let ctx = egui::Context::default();
    t::install(&ctx);
    for name in ["corvid", "larrow", "corvid-standby"] {
        let panel = catalog.resolve_aircraft(name).unwrap().panel;
        for (width, height) in [(1280., 800.), (1000., 640.)] {
            for compact in [false, true] {
                let panel_height = panel_height(height, compact);
                assert!(height - 64. - 30. - panel_height - 16. >= 230.);
                let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(width - 24., panel_height - 55.));
                let cells = layout(&panel, rect);
                for (i, a) in cells.iter().enumerate() {
                    assert!(a.is_positive());
                    assert!(a.expand(0.001).contains_rect(fit(*a)));
                    for b in cells.iter().skip(i + 1) {
                        let overlap = a.intersect(*b);
                        assert!(overlap.width() < 0.001 || overlap.height() < 0.001);
                    }
                }
                let mut runtime = InstrumentPanel::default();
                let session = Session::new().unwrap();
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(width, height))),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::TopBottomPanel::bottom("instruments")
                            .exact_height(panel_height)
                            .show(ctx, |ui| {
                                runtime
                                    .show(ui, &panel, InstrumentInput::new(session.sim().systems(), 0.), 0)
                                    .unwrap();
                            });
                    },
                );
                assert!(output.shapes.len() > 200);
                for shape in output.shapes {
                    let bounds = shape.shape.visual_bounding_rect();
                    if bounds.is_positive() {
                        assert!(bounds.is_finite(), "{name}: invalid cropped shape {bounds:?}");
                    }
                }
            }
        }
    }
}

#[test]
fn panel_shortcuts_are_commands_and_adjustments_are_bounded() {
    let mut session = Session::new().unwrap();
    for action in crate::keymap::Action::SYSTEMS {
        if action == crate::keymap::Action::Starter {
            assert!(shortcut(action, session.sim().systems()).is_none());
            continue;
        }
        let setting = shortcut(action, session.sim().systems()).unwrap();
        session.run(flightcraft_control::SetSystems { setting }).unwrap();
    }
    assert!(!session.sim().systems().controls.master);
    assert!(!session.sim().systems().controls.alternator);
}

#[test]
fn mouse_wheel_on_kollsman_knob_produces_the_altimeter_command() {
    let ctx = egui::Context::default();
    t::install(&ctx);
    let mut session = Session::new().unwrap();
    let mut panel = session.aircraft_configuration().unwrap().panel.clone();
    panel.instruments.retain(|i| i.module == "altimeter");
    let instrument = panel.instruments.first_mut().unwrap();
    instrument.position = [0., 0.];
    instrument.size = [1., 1.];
    let mut systems = session.sim().systems().clone();
    systems.enabled = false; // Isolate the dial from the switch strip.
    let mut runtime = InstrumentPanel::default();
    let mut settings = Vec::new();
    for pass in 0..3 {
        let mut events = vec![egui::Event::PointerMoved(egui::pos2(155., 143.))];
        if pass == 2 {
            events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0., 10.),
                modifiers: Default::default(),
            });
        }
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(200., 180.))),
                time: Some(f64::from(pass) / 60.),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        settings = runtime
                            .show(ui, &panel, InstrumentInput::new(&systems, 0.), 0)
                            .unwrap();
                    });
            },
        );
    }
    assert!(
        settings
            .iter()
            .any(|s| matches!(s,SystemSetting::Altimeter { pressure_pa } if (*pressure_pa - 29.93 * options::PA_PER_INHG).abs() < 1e-8))
    );
    for setting in settings {
        session.run(flightcraft_control::SetSystems { setting }).unwrap();
    }
    assert!(
        (session.sim().systems().controls.altimeter_setting_pa - 29.93 * options::PA_PER_INHG).abs() < 1e-8
    );
}

#[test]
fn old_keymaps_gain_optional_systems_bindings_without_changing_custom_keys() {
    use crate::keymap::{Action, Keymap};
    let mut old = Keymap::default();
    for action in Action::SYSTEMS {
        old.bindings.remove(&action);
    }
    for action in [
        Action::ComPtt,
        Action::IntercomPtt,
        Action::RadioPanel,
        Action::Kneeboard,
        Action::RadioTransmit,
    ] {
        old.bindings.remove(&action);
    }
    old.bindings.insert(Action::Camera, vec![egui::Key::V]);
    let upgraded = Keymap::from_json(&old.json().unwrap()).unwrap();
    assert_eq!(upgraded.bindings.get(&Action::Camera), Some(&vec![egui::Key::V]));
    assert!(
        Action::SYSTEMS
            .iter()
            .all(|a| upgraded.bindings.get(a).is_some_and(Vec::is_empty))
    );
    let mut configured = upgraded;
    configured.bindings.insert(Action::Master, vec![egui::Key::O]);
    assert_eq!(
        Keymap::from_json(&configured.json().unwrap()).unwrap(),
        configured
    );
}

#[test]
fn turn_direction_labels_are_beside_the_lower_standard_rate_marks() {
    let mut turn = Registry::default().instrument("turn_coordinator").unwrap();
    let draw = update(turn.as_mut(), &SystemsState::default(), 0.);
    for (label, left) in [("L", true), ("R", false)] {
        let at = draw
            .items
            .iter()
            .find_map(|i| match i {
                DrawItem::Label { at, text, .. } if text == label => Some(*at),
                _ => None,
            })
            .unwrap();
        let [x, y] = at;
        assert!(y > 76.);
        assert_eq!(x < 100., left);
    }
}

#[test]
fn altimeter_hands_follow_standard_continuous_gearing() {
    for (feet, expected) in [
        (0., [-90., -90., -90.]),
        (450., [72., -73.8, -88.38]),
        (1250., [0., -45., -85.5]),
        (9990., [266.4, 269.64, -54.036]),
        (12340., [32.4, -5.76, -45.576]),
    ] {
        for (actual, expected) in behaviour::altimeter_angles(feet).into_iter().zip(expected) {
            assert!((actual.to_degrees() - expected).abs() < 1e-9, "{feet} ft");
        }
    }
    let mut indicator = Registry::default().instrument("altimeter").unwrap();
    let draw = update(indicator.as_mut(), &SystemsState::default(), 0.);
    for digit in 0..10 {
        assert!(
            draw.items
                .iter()
                .any(|item| matches!(item,DrawItem::Label { text,.. } if text == &digit.to_string()))
        );
    }
    assert!(
        draw.items
            .iter()
            .filter(|item| matches!(item,DrawItem::Line { width,.. } if *width == 1.))
            .count()
            >= 50
    );
}

#[test]
fn pressure_preferences_have_exact_steps_limits_and_round_trip() {
    let us = DisplayOptions::default();
    assert_eq!(us.pressure_value(101325.), "29.92");
    assert!((us.adjust_pressure(101325., 1.) - 29.93 * options::PA_PER_INHG).abs() < 1e-8);
    assert!((us.adjust_pressure(101325., -1.) - 29.91 * options::PA_PER_INHG).abs() < 1e-8);
    assert_eq!(us.adjust_pressure(101325., 1e6), 31. * options::PA_PER_INHG);
    assert_eq!(us.adjust_pressure(101325., -1e6), 28.1 * options::PA_PER_INHG);
    let si = DisplayOptions {
        pressure: PressureUnit::Hpa,
        engine: EngineUnits::Si,
    };
    assert_eq!(si.pressure_value(101325.), "1013");
    assert_eq!(si.adjust_pressure(101325., 1.), 101400.);
    assert_eq!(
        serde_json::from_str::<DisplayOptions>(&serde_json::to_string(&si).unwrap()).unwrap(),
        si
    );
}

#[test]
fn airspeed_zero_is_at_the_bottom_and_arcs_follow_variant_limits() {
    assert!((behaviour::airspeed_angle(0.) - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
    let mut aircraft = flightcraft_aero::AircraftCatalog::builtins()
        .unwrap()
        .resolve_aircraft("corvid")
        .unwrap();
    let systems = SystemsState::default();
    let render = |aircraft: &flightcraft_aero::ResolvedAircraft| {
        Registry::default()
            .instrument("airspeed")
            .unwrap()
            .update(InstrumentInput::new(&systems, 0.).with_aircraft(aircraft))
    };
    let start = |draw: &DrawList, ink| {
        draw.items
            .iter()
            .find_map(|i| match i {
                DrawItem::Line {
                    a, width, ink: role, ..
                } if *role == ink && *width == 3. => Some(*a),
                _ => None,
            })
            .unwrap()
    };
    let draw = render(&aircraft);
    for (ink, radius, mps) in [
        (Ink::Normal, 62., aircraft.variant.v_speeds.vs_mps),
        (Ink::Text, 56., aircraft.variant.v_speeds.vso_mps),
        (Ink::Warning, 62., aircraft.variant.v_speeds.vno_mps),
        (Ink::Alert, 55., aircraft.variant.v_speeds.vne_mps),
    ] {
        let angle = behaviour::airspeed_angle(mps * 1.943844);
        let [x, y] = start(&draw, ink);
        assert!((f64::from(x) - 100. - radius * angle.cos()).abs() < 1e-4);
        assert!((f64::from(y) - 88. - radius * angle.sin()).abs() < 1e-4);
    }
    aircraft.variant.v_speeds.vs_mps += 2.;
    assert_ne!(start(&draw, Ink::Normal), start(&render(&aircraft), Ink::Normal));
}

#[test]
fn engine_units_convert_only_for_display_and_each_cluster_has_operating_bands() {
    use behaviour::engine_reading;
    assert_eq!(engine_reading("oil_temperature", 100., EngineUnits::Us), "212 °F");
    assert_eq!(engine_reading("oil_temperature", 100., EngineUnits::Si), "100 °C");
    assert_eq!(
        engine_reading("oil_pressure", 689.4757293, EngineUnits::Us),
        "100 psi"
    );
    assert_eq!(
        engine_reading("fuel_left", behaviour::KG_PER_US_GALLON * 10., EngineUnits::Us),
        "10.0 US gal"
    );
    assert_eq!(
        engine_reading("fuel_flow", behaviour::KG_PER_US_GALLON * 5., EngineUnits::Us),
        "5.0 gal/h"
    );
    for id in [
        "oil_temperature",
        "oil_pressure",
        "egt",
        "cht",
        "fuel_left",
        "fuel_right",
        "fuel_flow",
        "ammeter",
        "volts",
        "suction",
    ] {
        let mut module = Registry::default().instrument(id).unwrap();
        let systems = SystemsState::default();
        let mut input = InstrumentInput::new(&systems, 0.);
        let us = module.update(input);
        input.options.engine = EngineUnits::Si;
        let si = module.update(input);
        assert_eq!(
            us.indicated, si.indicated,
            "{id}: changing units must not move the needle"
        );
        assert_eq!(us.size, [320., 22.]);
        for ink in [Ink::Normal, Ink::Warning, Ink::Alert] {
            assert!(
                us.items
                    .iter()
                    .any(|item| matches!(item,DrawItem::Line { ink:role,.. } if *role == ink)),
                "{id}: {ink:?}"
            );
        }
    }
}

#[test]
fn uninstalled_vacuum_has_neither_a_gauge_nor_an_annunciator() {
    let catalog = flightcraft_aero::AircraftCatalog::builtins().unwrap();
    for (name, vacuum) in [("corvid", true), ("larrow", false)] {
        let aircraft = catalog.resolve_aircraft(name).unwrap();
        assert_eq!(
            aircraft.panel.instruments.iter().any(|i| i.module == "suction"),
            vacuum
        );
        let mut session = Session::new().unwrap();
        session
            .execute(
                "aircraft.load",
                serde_json::json!({"source":{"kind":"installed","id":name},"reset":true}),
            )
            .unwrap();
        let draw = Registry::default()
            .instrument("annunciators")
            .unwrap()
            .update(InstrumentInput::new(session.sim().systems(), 0.).with_aircraft(&aircraft));
        assert_eq!(
            draw.items
                .iter()
                .any(|i| matches!(i,DrawItem::Label {text,..} if text == "LOW VAC")),
            vacuum
        );
        assert!(
            draw.items
                .iter()
                .any(|i| matches!(i,DrawItem::Label {text,ink:Ink::Inactive,..} if text == "LOW VOLT"))
        );
        if vacuum {
            assert!(
                draw.items
                    .iter()
                    .any(|i| matches!(i,DrawItem::Label {text,ink:Ink::Warning,..} if text == "LOW VAC"))
            );
        }
    }
}

#[test]
fn starter_is_held_and_returns_to_both_without_cancelling_scripted_start() {
    let mut session = Session::new().unwrap();
    session
        .run(flightcraft_control::SetSystems {
            setting: SystemSetting::Magnetos {
                position: flightcraft_sim::systems::Magnetos::Off,
            },
        })
        .unwrap();
    let mut hold = switches::StarterHold::default();
    for setting in hold.update(session.sim().systems(), true) {
        session.run(flightcraft_control::SetSystems { setting }).unwrap();
    }
    assert!(session.sim().systems().controls.starter);
    assert_eq!(
        session.sim().systems().controls.magnetos,
        flightcraft_sim::systems::Magnetos::Both
    );
    assert!(hold.update(session.sim().systems(), true).is_empty());
    for setting in hold.update(session.sim().systems(), false) {
        session.run(flightcraft_control::SetSystems { setting }).unwrap();
    }
    assert!(!session.sim().systems().controls.starter);
    assert_eq!(
        session.sim().systems().controls.magnetos,
        flightcraft_sim::systems::Magnetos::Both
    );
    session
        .run(flightcraft_control::SetSystems {
            setting: SystemSetting::Switch {
                switch: SystemSwitch::Starter,
                on: true,
            },
        })
        .unwrap();
    assert!(hold.update(session.sim().systems(), false).is_empty());
}

#[test]
fn mouse_starter_press_and_release_send_systems_commands() {
    let ctx = egui::Context::default();
    t::install(&ctx);
    let mut session = Session::new().unwrap();
    let panel = session.aircraft_configuration().unwrap().panel.clone();
    let mut runtime = InstrumentPanel::default();
    let mut start = None;
    for frame in 0..3 {
        let mut events = Vec::new();
        if let Some(pos) = start {
            events = vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: frame == 1,
                    modifiers: Default::default(),
                },
            ];
        }
        let mut settings = Vec::new();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280., 380.))),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    settings = runtime
                        .show(ui, &panel, InstrumentInput::new(session.sim().systems(), 0.), 0)
                        .unwrap();
                });
            },
        );
        if frame == 0 {
            start = output.shapes.iter().find_map(|clipped| match &clipped.shape {
                Shape::Text(text) if text.galley.job.text == "START" => {
                    Some(clipped.shape.visual_bounding_rect().center())
                }
                _ => None,
            });
            assert!(start.is_some());
        }
        for setting in settings {
            session.run(flightcraft_control::SetSystems { setting }).unwrap();
        }
        assert_eq!(session.sim().systems().controls.starter, frame == 1);
    }
}

#[test]
fn every_text_box_fits_without_overlapping_text_or_crossing_dial_edges() {
    let ctx = egui::Context::default();
    t::install(&ctx);
    let catalog = flightcraft_aero::AircraftCatalog::builtins().unwrap();
    for name in ["corvid", "larrow", "corvid-standby"] {
        let aircraft = catalog.resolve_aircraft(name).unwrap();
        let mut session = Session::new().unwrap();
        session
            .execute(
                "aircraft.load",
                serde_json::json!({"source":{"kind":"installed","id":name},"reset":true}),
            )
            .unwrap();
        for (width, height) in [(1280., 800.), (1000., 640.)] {
            for compact in [false, true] {
                let bounds = Rect::from_min_size(
                    Pos2::ZERO,
                    egui::vec2(width - 24., panel_height(height, compact) - 60.),
                );
                let cells = layout(&aircraft.panel, bounds);
                let _ = ctx.run(egui::RawInput { screen_rect:Some(bounds),..Default::default() },|ctx| {
                    for (feet,heading,valid) in [(0.,0.,true),(450.,17.,false),(1250.,90.,true),(9990.,177.,true),(12340.,359.,true),(30000.,23.,false)] {
                        let mut systems = session.sim().systems().clone();
                        systems.sensed.altitude_msl_m = feet/3.28084;
                        systems.sensed.heading_rad = f64::to_radians(heading);
                        for a in [&mut systems.avionics.electronic_attitude, &mut systems.avionics.electronic_hsi.reversion_attitude] {
                            a.power.powered=true; a.power.on_battery=true;
                            a.attitude_valid=valid; a.aligning=false; a.air_data_valid=valid;
                            a.heading_deg=heading; a.altitude_ft=feet; a.airspeed_kt=125.; a.roll_deg=heading-180.; a.pitch_deg=17.; a.vertical_speed_fpm=2000.;
                        }
                        systems.avionics.electronic_hsi.reversion=false;
                        systems.avionics.electronic_hsi.destination=Some("MMU".into());
                        systems.avionics.electronic_hsi.gps_fix=valid;
                        systems.avionics.electronic_hsi.distance_nm=Some(52.3);
                        systems.sensed.attitude_valid = valid;
                        systems.sensed.indicated_airspeed_mps = 200.;
                        systems.sensed.vertical_speed_mps = 200.;
                        systems.engine_rpm = aircraft.engine.propulsion.max_rpm;
                        systems.oil_temperature_c = 160.; systems.oil_pressure_pa = 700000.;
                        systems.egt_c = 1200.; systems.cht_c = 350.;
                        for tank in &mut systems.tanks { tank.indicated_kg = tank.capacity_kg; }
                        for pressure in [PressureUnit::InHg,PressureUnit::Hpa] { for engine in [EngineUnits::Us,EngineUnits::Si] {
                            let options = DisplayOptions {pressure,engine};
                            let mut global = Vec::new();
                            for (installation,cell) in aircraft.panel.instruments.iter().zip(&cells) {
                                let mut input = InstrumentInput::new(&systems,0.).with_aircraft(&aircraft); input.options = options;
                                let draw = Registry::default().instrument(&installation.module).unwrap().update(input);
                                let canvas = Rect::from_min_size(Pos2::ZERO,egui::vec2(draw.size[0],draw.size[1]));
                                let fitted = fit_size(*cell,draw.size);
                                let scale = fitted.width()/canvas.width();
                                let mut labels = Vec::new();
                                for item in &draw.items {
                                    if let DrawItem::Label {at,text,size,..} = item {
                                        let galley = ctx.fonts_mut(|fonts| fonts.layout_no_wrap(text.clone(),FontId::monospace(*size),t::TEXT));
                                        let local = Rect::from_center_size(Pos2::from(*at),galley.size());
                                        assert!(canvas.expand(0.01).contains_rect(local),"{name}/{}: {text} outside canvas {local:?}",installation.module);
                                        for (other,other_text) in &labels { assert!(!local.intersects(*other),"{name}/{}: {text} overlaps {other_text}: {local:?}, {other:?}",installation.module); }
                                        for face in &draw.items { if let DrawItem::Circle {center,radius,ink:Ink::Face} = face {
                                            let center = Pos2::from(*center);
                                            let inside = [local.left_top(),local.right_top(),local.left_bottom(),local.right_bottom()].into_iter().all(|p| p.distance(center) < *radius-0.25);
                                            let closest = egui::pos2(center.x.clamp(local.left(),local.right()),center.y.clamp(local.top(),local.bottom()));
                                            let outside = closest.distance(center) > *radius+0.25;
                                            assert!(inside || outside,"{name}/{}: {text} crosses dial edge: {local:?}",installation.module);
                                        } }
                                        let physical = Rect::from_min_max(fitted.min+local.min.to_vec2()*scale,fitted.min+local.max.to_vec2()*scale);
                                        assert!(cell.expand(0.01).contains_rect(physical));
                                        for (other,other_text) in &global { assert!(!physical.intersects(*other),"{name}: {text} overlaps {other_text}"); }
                                        global.push((physical,text.clone()));
                                        labels.push((local,text.clone()));
                                    }
                                }
                            }
                        } }
                    }
                });
            }
        }
    }
}

#[test]
fn electronic_skins_show_simulated_flags_and_shortcuts_use_avionics_commands() {
    let mut session = Session::new().unwrap();
    session
        .execute(
            "aircraft.load",
            serde_json::json!({"source":{"kind":"installed","id":"corvid-standby"},"reset":true}),
        )
        .unwrap();
    for action in crate::keymap::Action::AVIONICS {
        if action == crate::keymap::Action::StandbyDirectTo {
            continue;
        }
        let setting = avionics_shortcut(action, &session.sim().systems().avionics).unwrap();
        session.run(flightcraft_control::SetAvionics { setting }).unwrap();
    }
    for id in ["electronic_attitude", "electronic_hsi"] {
        let mut module = Registry::default().instrument(id).unwrap();
        let mut systems = session.sim().systems().clone();
        systems.avionics.electronic_hsi.reversion = false;
        for a in [
            &mut systems.avionics.electronic_attitude,
            &mut systems.avionics.electronic_hsi.reversion_attitude,
        ] {
            a.power.switch_on = true;
            a.power.powered = true;
            a.aligning = false;
            a.attitude_valid = false;
            a.air_data_valid = false;
        }
        let draw = update(module.as_mut(), &systems, 0.);
        assert!(!draw.valid);
        assert!(
            draw.items
                .iter()
                .any(|item| matches!(item,DrawItem::Line{ink:Ink::Alert,width,..} if *width==3.))
        );
        if id == "electronic_hsi" {
            assert!(
                draw.items
                    .iter()
                    .any(|item| matches!(item,DrawItem::Label{text,..} if text=="NO GPS"))
            );
        }
        assert!(draw.items.iter().all(valid_item));
    }
}

#[test]
fn electronic_speed_limit_marks_the_variant_value_only_when_in_view() {
    let aircraft = flightcraft_aero::AircraftCatalog::builtins()
        .unwrap()
        .resolve_aircraft("corvid-standby")
        .unwrap();
    let mut systems = SystemsState::default();
    systems.avionics.electronic_attitude.power.powered = true;
    let mut instrument = Registry::default().instrument("electronic_attitude").unwrap();
    for (speed, visible) in [
        (80., false),
        (aircraft.variant.v_speeds.vne_mps * 1.94384449, true),
        (180., false),
    ] {
        systems.avionics.electronic_attitude.airspeed_kt = speed;
        let draw = instrument.update(InstrumentInput::new(&systems, 0.).with_aircraft(&aircraft));
        assert_eq!(
            draw.items
                .iter()
                .any(|item| matches!(item,DrawItem::Line {ink:Ink::Alert,width,..} if *width==2.)),
            visible
        );
    }
}
