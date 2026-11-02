#![allow(clippy::unwrap_used)]
use crate::{instruments::*, theme as t};
use eframe::egui::{self, Rect};
use flightcraft_control::Session;
use std::{cell::Cell, rc::Rc};
fn session(pack: &str, aircraft: &str) -> Session {
    let mut s = Session::new().unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/packs")
        .join(pack);
    s.install_pack(&flightcraft_control::read_pack(&root).unwrap())
        .unwrap();
    s.execute(
        "aircraft.load",
        serde_json::json!({"source":{"kind":"installed","id":aircraft},"reset":true}),
    )
    .unwrap();
    s
}
struct TestSkin(Rc<Cell<usize>>);
impl EquipmentSkin for TestSkin {
    fn paint(&self, p: &egui::Painter, rect: Rect, s: EquipmentSnapshot<'_>) {
        assert_eq!(s.installation.id, "switch_master");
        assert!(s.input.systems.controls.master);
        self.0.set(self.0.get() + 1);
        p.rect_filled(rect, 0., t::ACCENT);
    }
}
#[test]
fn equipment_registry_artwork_preserves_host_hit_regions_and_read_only_state() {
    let s = session("equipment-demo", "equipment-demonstrator");
    let config = s.aircraft_configuration().unwrap();
    let equipment = config.panel.equipment.as_ref().unwrap();
    let e = equipment
        .elements
        .iter()
        .find(|e| e.id == "switch_master")
        .unwrap();
    let mut panel = InstrumentPanel::default();
    let calls = Rc::new(Cell::new(0));
    panel
        .registry
        .register_equipment_skin("private_art".into(), Box::new(TestSkin(calls.clone())))
        .unwrap();
    panel
        .registry
        .set_equipment_skin("switch_master".into(), "private_art".into())
        .unwrap();
    let mut invalid = config.panel.clone();
    invalid
        .equipment
        .as_mut()
        .unwrap()
        .elements
        .first_mut()
        .unwrap()
        .appearance
        .skin = Some("missing".into());
    assert!(panel.registry.validate(&invalid).is_err());
    let ctx = egui::Context::default();
    t::install(&ctx);
    let size = egui::vec2(1250., 700.);
    let [x, y] = e.position;
    let [w, h] = e.size;
    let pos = egui::pos2((x + w * 0.5) as f32, (y + h * 0.5) as f32);
    let before = s.sim().systems().clone();
    let state = s.glass_state();
    let mut settings = Vec::new();
    for pressed in [None, Some(true), Some(false)] {
        let events = pressed
            .map(|pressed| {
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]
            })
            .unwrap_or_default();
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        settings.extend(
                            panel
                                .show(
                                    ui,
                                    &config.panel,
                                    InstrumentInput::new(s.sim().systems(), 0.)
                                        .with_aircraft(config)
                                        .with_glass(&state),
                                    s.flight_generation(),
                                )
                                .unwrap(),
                        )
                    });
            },
        );
    }
    assert_eq!(calls.get(), 3);
    assert_eq!(&before, s.sim().systems());
    assert!(settings.iter().any(|s| matches!(
        s,
        flightcraft_sim::systems::SystemSetting::Switch {
            switch: flightcraft_sim::systems::SystemSwitch::Master,
            on: false
        }
    )));
}
#[test]
fn narrow_navigator_text_stays_within_installation() {
    let s = session("panel-fidelity-demo", "equipment-fidelity-demonstrator");
    for layout in [
        flightcraft_aero::panel_fidelity::NavigatorLayout::Classic,
        flightcraft_aero::panel_fidelity::NavigatorLayout::HomeColumns,
    ] {
        let mut config = s.aircraft_configuration().unwrap().clone();
        config
            .panel
            .equipment
            .as_mut()
            .unwrap()
            .elements
            .iter_mut()
            .find(|e| e.id == "navigator")
            .unwrap()
            .appearance
            .navigator_layout = layout;
        let state = s.glass_state();
        let mut panel = InstrumentPanel::default();
        let ctx = egui::Context::default();
        t::install(&ctx);
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1250., 700.))),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        panel
                            .show(
                                ui,
                                &config.panel,
                                InstrumentInput::new(s.sim().systems(), 0.)
                                    .with_aircraft(&config)
                                    .with_glass(&state)
                                    .with_autopilot(s.sim().installed_autopilot()),
                                s.flight_generation(),
                            )
                            .unwrap()
                    });
            },
        );
        let nav = Rect::from_min_size(egui::pos2(500., 185.), egui::vec2(181., 75.));
        let mut labels = 0;
        for shape in output.shapes {
            if let egui::Shape::Text(text) = shape.shape
                && nav.contains(text.pos)
            {
                let bounds = Rect::from_min_size(text.pos, text.galley.size());
                assert!(
                    nav.expand(0.1).contains_rect(bounds),
                    "{} {:?}",
                    text.galley.job.text,
                    bounds
                );
                labels += 1;
            }
        }
        assert!(labels >= 8);
        // Painting remains an observation even when small controls have command hit regions.
        assert_eq!(s.sim().readout().time_s, 0.);
    }
}

#[test]
#[ignore = "writes original panel fidelity preview meshes"]
fn render_panel_fidelity_previews() {
    let mut s = geographic_session();
    s.execute(
        "assist.set",
        serde_json::json!({"altitude_m":1200.,"airspeed_mps":45.,"heading_rad":0.5}),
    )
    .unwrap();
    s.execute("sim.step", serde_json::json!({"steps":7200})).unwrap();
    let image = crate::panel_preview::render_equipment_panel(&mut s, 1728, 968).unwrap();
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/equipment-fidelity");
    std::fs::create_dir_all(&out).unwrap();
    image.save(out.join("panel.png")).unwrap();
    let panel = s
        .aircraft_configuration()
        .unwrap()
        .panel
        .equipment
        .as_ref()
        .unwrap();
    let [w, h] = panel.size;
    let scale = (1728. / w).min(968. / h);
    let (left, top) = ((1728. - w * scale) * 0.5, (968. - h * scale) * 0.5);
    for id in ["left_display", "navigator", "autopilot"] {
        let e = panel.elements.iter().find(|e| e.id == id).unwrap();
        let [x, y] = e.position;
        let [w, h] = e.size;
        image::imageops::crop_imm(
            &image,
            (left + x * scale).round() as u32,
            (top + y * scale).round() as u32,
            (w * scale).round() as u32,
            (h * scale).round() as u32,
        )
        .to_image()
        .save(out.join(format!("{id}.png")))
        .unwrap();
    }
}

#[test]
fn declared_controller_buttons_and_knob_pushes_emit_existing_commands() {
    use flightcraft_aero::panel_fidelity::ControllerAction as A;
    use flightcraft_control::AvionicsSetting;
    use flightcraft_sim::equipment::PanelSetting;
    let s = session("panel-fidelity-demo", "equipment-fidelity-demonstrator");
    let config = s.aircraft_configuration().unwrap();
    let glass = s.glass_state();
    let e = config
        .panel
        .equipment
        .as_ref()
        .unwrap()
        .elements
        .iter()
        .find(|e| e.id == "autopilot")
        .unwrap();
    let layout = e.appearance.controller.as_ref().unwrap();
    for c in &layout.controls {
        if c.action == A::PitchWheel {
            continue;
        }
        let ctx = egui::Context::default();
        t::install(&ctx);
        let mut controller = crate::autopilot::Controller::default();
        let mut commands = Vec::new();
        let [x, y, w, h] = c.rect;
        let pos = egui::pos2((x + w * 0.5) as f32, (y + h * 0.5) as f32);
        for pressed in [None, Some(true), Some(false)] {
            let events = pressed
                .map(|pressed| {
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]
                })
                .unwrap_or_default();
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300., 120.))),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ctx, |ui| {
                            commands.extend(
                                controller.show_declared(
                                    ui,
                                    Rect::from_min_size(
                                        egui::Pos2::ZERO,
                                        egui::vec2(e.size[0] as f32, e.size[1] as f32),
                                    ),
                                    e.size,
                                    layout,
                                    InstrumentInput::new(s.sim().systems(), 0.)
                                        .with_glass(&glass)
                                        .with_autopilot(s.sim().installed_autopilot()),
                                    t::DIAL,
                                ),
                            )
                        });
                },
            );
        }
        assert!(
            commands.iter().any(|command| matches!(
                (c.action, command),
                (A::HeadingKnob, AvionicsSetting::HeadingBugSync {})
                    | (
                        A::AltitudeKnob,
                        AvionicsSetting::Autopilot {
                            setting: PanelSetting::Altitude { .. }
                        }
                    )
                    | (
                        A::Ap,
                        AvionicsSetting::Autopilot {
                            setting: PanelSetting::Engage { .. }
                        }
                    )
                    | (
                        A::Fd,
                        AvionicsSetting::Autopilot {
                            setting: PanelSetting::FlightDirector { .. }
                        }
                    )
                    | (
                        A::Yd,
                        AvionicsSetting::Autopilot {
                            setting: PanelSetting::YawDamper { .. }
                        }
                    )
                    | (
                        A::Lvl,
                        AvionicsSetting::Autopilot {
                            setting: PanelSetting::Level
                        }
                    )
                    | (
                        A::Hdg | A::Nav | A::Apr,
                        AvionicsSetting::Autopilot {
                            setting: PanelSetting::Lateral { .. }
                        }
                    )
                    | (
                        A::Vs | A::Alt | A::Ias | A::Vnav,
                        AvionicsSetting::Autopilot {
                            setting: PanelSetting::Vertical { .. }
                        }
                    )
            )),
            "no host command for {:?}",
            c.action
        );
    }
}

/// Original analytic terrain makes captures independent of downloaded regional assets.
fn geographic_session() -> Session {
    use flightcraft_world::{HeightTile, Region, RegionOrigin, Surface, World};
    let mut s = session("panel-fidelity-demo", "equipment-fidelity-demonstrator");
    let mut samples = Vec::new();
    for e in 0..129 {
        for n in 0..129 {
            let (north, east) = (-16000. + f64::from(n) * 250., -16000. + f64::from(e) * 250.);
            samples.push(
                (350. + 240. * (north / 2700.).sin() * (east / 1900.).cos() + 120. * (east / 4200.).sin())
                    .max(0.) as f32,
            );
        }
    }
    let tile = HeightTile::new(
        [-16000., -16000.],
        250.,
        129,
        samples,
        vec![Surface::Grass; 128 * 128],
    )
    .unwrap();
    let region = Region {
        id: "panel-preview".into(),
        name: "Procedural panel preview".into(),
        origin: RegionOrigin {
            latitude_deg: 40.,
            longitude_deg: -75.,
            field_elevation_m: 0.,
            runway_heading_deg: 0.,
            runway_length_m: 900.,
            runway_width_m: 23.,
        },
        southwest_m: [-16000., -16000.],
        tiles_north: 1,
        tiles_east: 1,
        spacing_m: 250.,
        tile_size: 129,
        runway_blend_m: 200.,
        source: "Original analytic hills".into(),
        licence: "Apache-2.0".into(),
        attribution: "Original FlightCraft procedural terrain".into(),
        tiles: vec!["000.fcht".into()],
        airports: None,
    };
    s.install_region(World::from_region(region, vec![tile]).unwrap())
        .unwrap();
    s.execute("sim.reset",serde_json::json!({"world":"panel-preview","start":{"kind":"air","height_m":1200.,"airspeed_mps":45.,"heading_rad":0.5}})).unwrap();
    s.execute("map.set", serde_json::json!({"range_nm":4.,"night":true}))
        .unwrap();
    s
}

#[test]
fn whole_panel_captions_do_not_overlap() {
    let mut s = geographic_session();
    s.execute(
        "assist.set",
        serde_json::json!({"altitude_m":1200.,"airspeed_mps":45.,"heading_rad":0.5}),
    )
    .unwrap();
    s.execute("sim.step", serde_json::json!({"steps":7200})).unwrap();
    let config = s.aircraft_configuration().unwrap().clone();
    let mut audit = Vec::new();
    for open in [false, true] {
        for id in ["toggle_aux", "switch_lane_b"] {
            s.execute("equipment.set",serde_json::json!({"setting":{"kind":"systems","setting":{"kind":"panel_guard","id":id,"open":open}}})).unwrap();
        }
        for size in [
            egui::vec2(1250., 700.),
            egui::vec2(1728., 968.),
            egui::vec2(1100., 620.),
        ] {
            let state = s.glass_state();
            let mut panel = InstrumentPanel::default();
            let ctx = egui::Context::default();
            t::install(&ctx);
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ctx, |ui| {
                            panel
                                .show(
                                    ui,
                                    &config.panel,
                                    InstrumentInput::new(s.sim().systems(), s.sim().readout().time_s)
                                        .with_aircraft(&config)
                                        .with_glass(&state)
                                        .with_autopilot(s.sim().installed_autopilot()),
                                    s.flight_generation(),
                                )
                                .unwrap();
                            panel.show_shared_hosts(ui, &mut s);
                        });
                },
            );
            fn collect(shape: &egui::Shape, clip: Rect, text: &mut Vec<(String, Rect)>) {
                match shape {
                    egui::Shape::Text(t) if t.opacity_factor > 0. => {
                        let rect = Rect::from_min_size(t.pos, t.galley.size()).intersect(clip);
                        if rect.width() > 0.1 && rect.height() > 0.1 {
                            text.push((t.galley.job.text.clone(), rect));
                        }
                    }
                    egui::Shape::Vec(shapes) => {
                        for shape in shapes {
                            collect(shape, clip, text);
                        }
                    }
                    _ => {}
                }
            }
            let mut labels = Vec::new();
            for shape in output.shapes {
                collect(&shape.shape, shape.clip_rect, &mut labels);
            }
            let mut overlaps = Vec::new();
            for (i, (a, ra)) in labels.iter().enumerate() {
                for (b, rb) in labels.iter().skip(i + 1) {
                    let overlap = ra.intersect(*rb);
                    if overlap.width() > 0.2 && overlap.height() > 0.2 {
                        overlaps.push(format!("{a:?} {ra:?} / {b:?} {rb:?}"));
                    }
                }
            }
            assert!(
                overlaps.is_empty(),
                "{} overlapping captions:\n{}",
                overlaps.len(),
                overlaps.join("\n")
            );
            assert!(labels.len() > 150, "whole panel must be inspected");
            audit.push(serde_json::json!({"size":[size.x,size.y],"guard_open":open,"captions_checked":labels.len(),"overlaps":overlaps.len()}));
            eprintln!(
                "{}x{} guard_open={open}: {} captions, no overlaps",
                size.x,
                size.y,
                labels.len()
            );
        }
    }
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/regression");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(
        out.join("caption-audit.json"),
        serde_json::to_string_pretty(&audit).unwrap(),
    )
    .unwrap();
}

fn inset_crops(s: &Session, image: &image::RgbaImage) -> Vec<image::RgbaImage> {
    let panel = s
        .aircraft_configuration()
        .unwrap()
        .panel
        .equipment
        .as_ref()
        .unwrap();
    let [pw, ph] = panel.size;
    let scale = (f64::from(image.width()) / pw).min(f64::from(image.height()) / ph);
    let (left, top) = (
        (f64::from(image.width()) - pw * scale) * 0.5,
        (f64::from(image.height()) - ph * scale) * 0.5,
    );
    let mut crops = Vec::new();
    for e in &panel.elements {
        if !matches!(
            e.element,
            flightcraft_aero::modern::PanelElement::TouchDisplay { .. }
        ) {
            continue;
        }
        let [x, y] = e.position;
        let [w, h] = e.size;
        let [l, t, r, b] = e.appearance.bezel.as_ref().unwrap().margins;
        let sw = w - l - r;
        let sh = (h - t - b) * 0.92;
        for corner in [0.01, 0.72] {
            let ix = left + (x + l + sw * 0.20 + sw * 0.80 * corner) * scale;
            let iy = top + (y + t + (h - t - b) * 0.08 + sh * 0.68) * scale;
            // Check interior pixels, so a border alone cannot satisfy the assertion.
            crops.push(
                image::imageops::crop_imm(
                    image,
                    ix.ceil() as u32 + 2,
                    iy.ceil() as u32 + 2,
                    (sw * 0.8 * 0.27 * scale).floor() as u32 - 4,
                    (sh * 0.25 * scale).floor() as u32 - 4,
                )
                .to_image(),
            );
        }
    }
    crops
}
#[test]
fn integrated_insets_have_real_map_content_in_geographic_and_local_worlds() {
    let mut audit = Vec::new();
    for mut s in [
        geographic_session(),
        session("panel-fidelity-demo", "equipment-fidelity-demonstrator"),
    ] {
        let geographic = s.sim().world().region().is_some();
        if !geographic {
            s.execute("sim.reset",serde_json::json!({"start":{"kind":"air","height_m":1200.,"airspeed_mps":45.,"heading_rad":0.5}})).unwrap();
        }
        let before = s.map_state().clone();
        let image = crate::panel_preview::render_equipment_panel(&mut s, 1250, 700).unwrap();
        let crops = inset_crops(&s, &image);
        assert_eq!(crops.len(), 4);
        let mut counts = Vec::new();
        for crop in &crops {
            let colors: std::collections::BTreeSet<_> = crop.pixels().map(|p| p.0).collect();
            assert!(
                colors.len() > 24,
                "inset interior must contain map content, not a uniform fill: {} colors",
                colors.len()
            );
            counts.push(colors.len());
        }
        assert_eq!(
            serde_json::to_value(before).unwrap(),
            serde_json::to_value(s.map_state()).unwrap(),
            "terrain inset must not change shared MAP settings"
        );
        s.execute("sim.step", serde_json::json!({"steps":1200})).unwrap();
        let next = crate::panel_preview::render_equipment_panel(&mut s, 1250, 700).unwrap();
        let next_crops = inset_crops(&s, &next);
        for (a, b) in crops.iter().zip(&next_crops) {
            assert_ne!(a.as_raw(), b.as_raw(), "moving map must follow aircraft position");
        }
        audit.push(
            serde_json::json!({"geographic":geographic,"inset_interior_color_counts":counts,"moving":true}),
        );
    }
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/regression");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(
        out.join("inset-audit.json"),
        serde_json::to_string_pretty(&audit).unwrap(),
    )
    .unwrap();
}

#[test]
fn navigator_tile_and_frequency_fonts_are_uniform() {
    let s = session("panel-fidelity-demo", "equipment-fidelity-demonstrator");
    let config = s.aircraft_configuration().unwrap();
    let state = s.glass_state();
    let mut panel = InstrumentPanel::default();
    let ctx = egui::Context::default();
    t::install(&ctx);
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1250., 700.))),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    panel
                        .show(
                            ui,
                            &config.panel,
                            InstrumentInput::new(s.sim().systems(), 0.)
                                .with_aircraft(config)
                                .with_glass(&state),
                            s.flight_generation(),
                        )
                        .unwrap();
                });
        },
    );
    let tile_area = Rect::from_min_max(egui::pos2(518., 191.), egui::pos2(638., 254.));
    let frequency_area = Rect::from_min_max(egui::pos2(643., 185.), egui::pos2(681., 240.));
    let (mut tile_fonts, mut frequency_fonts) = (
        std::collections::BTreeSet::new(),
        std::collections::BTreeSet::new(),
    );
    let mut tiles = 0;
    for shape in output.shapes {
        if let egui::Shape::Text(text) = shape.shape {
            let area = Rect::from_min_size(text.pos, text.galley.size());
            if tile_area.contains_rect(area) {
                for section in &text.galley.job.sections {
                    tile_fonts.insert(section.format.font_id.size.to_bits());
                }
                assert!(text.galley.rows.len() <= 2);
                tiles += 1;
            }
            if frequency_area.contains_rect(area) {
                for section in &text.galley.job.sections {
                    frequency_fonts.insert(section.format.font_id.size.to_bits());
                }
            }
        }
    }
    assert_eq!(tiles, 8);
    assert_eq!(tile_fonts.len(), 1);
    assert_eq!(
        frequency_fonts.len(),
        1,
        "frequency column must share one font size"
    );
}

#[test]
fn lane_caption_lamp_and_guard_have_separate_regions() {
    let s = session("panel-fidelity-demo", "equipment-fidelity-demonstrator");
    let config = s.aircraft_configuration().unwrap();
    let glass = s.glass_state();
    let mut panel = InstrumentPanel::default();
    let ctx = egui::Context::default();
    t::install(&ctx);
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1250., 700.))),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    panel
                        .show(
                            ui,
                            &config.panel,
                            InstrumentInput::new(s.sim().systems(), 0.)
                                .with_aircraft(config)
                                .with_glass(&glass),
                            s.flight_generation(),
                        )
                        .unwrap();
                });
        },
    );
    for e in config
        .panel
        .equipment
        .as_ref()
        .unwrap()
        .elements
        .iter()
        .filter(|e| e.appearance.binding.is_some())
    {
        let [x, y] = e.position;
        let [w, h] = e.size;
        let cell = Rect::from_min_size(egui::pos2(x as f32, y as f32), egui::vec2(w as f32, h as f32));
        let mut caption = None;
        let mut guard = None;
        let mut lamp = None;
        for shape in &output.shapes {
            match &shape.shape {
                egui::Shape::Text(text) if cell.contains(text.pos) => {
                    caption = Some(Rect::from_min_size(text.pos, text.galley.size()))
                }
                egui::Shape::Rect(rect)
                    if rect.stroke.color == t::DANGER && cell.contains_rect(rect.rect) =>
                {
                    guard = Some(rect.rect)
                }
                egui::Shape::Circle(circle)
                    if cell.contains(circle.center)
                        && circle.center.y < cell.top() + cell.height() * 0.15 =>
                {
                    lamp = Some(Rect::from_center_size(
                        circle.center,
                        egui::Vec2::splat(circle.radius * 2.),
                    ))
                }
                _ => {}
            }
        }
        let (caption, guard, lamp) = (caption.unwrap(), guard.unwrap(), lamp.unwrap());
        assert!(lamp.bottom() < caption.top());
        assert!(caption.bottom() < guard.top());
        assert!(!guard.intersects(lamp));
    }
}
