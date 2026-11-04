//! Layout regression captures use original analytic terrain and the live egui font atlas.
#![allow(clippy::unwrap_used)]
use crate::instruments::{InstrumentInput, InstrumentPanel};
use eframe::egui;

#[test]
#[ignore = "writes full HD before/after layout previews"]
fn render_layout_previews() {
    let stage = std::env::var("FLIGHTCRAFT_LAYOUT_STAGE").unwrap_or("after".into());
    assert!(matches!(stage.as_str(), "before" | "after"));
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../out/layout")
        .join(stage);
    std::fs::create_dir_all(&out).unwrap();
    let external = std::env::var_os("FLIGHTCRAFT_LAYOUT_PACK");
    for (name, id) in [
        ("equipment-demo", "equipment-demonstrator"),
        ("panel-fidelity-demo", "equipment-fidelity-demonstrator"),
        ("corvid", "corvid-glass"),
        ("six-pack", "corvid"),
        ("sling", "sling2"),
    ] {
        let mut s = crate::panel_fidelity_tests::geographic_session();
        if name == "sling" {
            let Some(path) = external.as_ref() else { continue };
            s.install_pack(&flightcraft_control::read_pack(std::path::Path::new(path)).unwrap())
                .unwrap();
        } else if name == "equipment-demo" {
            let root =
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/packs/equipment-demo");
            s.install_pack(&flightcraft_control::read_pack(&root).unwrap())
                .unwrap();
        }
        s.execute(
            "aircraft.load",
            serde_json::json!({"source":{"kind":"installed","id":id},"reset":true}),
        )
        .unwrap();
        s.execute("sim.reset", serde_json::json!({"world":"panel-preview","start":{"kind":"air","height_m":1200.,"airspeed_mps":45.,"heading_rad":0.5}})).unwrap();
        s.execute(
            "assist.set",
            serde_json::json!({"altitude_m":1200.,"airspeed_mps":45.,"heading_rad":0.5}),
        )
        .unwrap();
        s.execute("sim.step", serde_json::json!({"steps":7200})).unwrap();
        let config = s.aircraft_configuration().unwrap().clone();
        // FLIGHTCRAFT_SKINS_DIR adds display skin packs, as in the application.
        if let Some(dir) = std::env::var_os("FLIGHTCRAFT_SKINS_DIR") {
            assert!(s.load_skins_directory(std::path::Path::new(&dir)).is_empty());
        }
        let owned_skins: Vec<flightcraft_control::SkinContents> = s.skins().cloned().collect();
        let skins: Vec<&flightcraft_control::SkinContents> = owned_skins.iter().collect();
        let state = s.glass_state();
        if name == "corvid" {
            for display in ["glass_pfd", "glass_mfd"] {
                let mut map = crate::map::PlanningMap::default();
                let image = crate::panel_preview::rasterize(1920, 1080, |ui| {
                    let draw = crate::glass::create(display).unwrap().update(
                        InstrumentInput::new(s.sim().systems(), s.sim().state().time_s)
                            .with_aircraft(&config)
                            .with_glass(&state)
                            .with_autopilot(s.sim().installed_autopilot()),
                    );
                    let rect = ui.max_rect();
                    crate::glass::paint_fill(ui, rect, draw);
                    if display == "glass_mfd" {
                        let area = egui::Rect::from_min_max(
                            rect.min + egui::vec2(rect.width() * 0.22, rect.height() * 28. / 380.),
                            rect.max,
                        );
                        map.show_mfd(ui, area, &mut s);
                    }
                })
                .unwrap();
                image.save(out.join(format!("corvid-{display}.png"))).unwrap();
            }
        }
        let mut panel = InstrumentPanel::default();
        let mut map = crate::map::PlanningMap::default();
        let image = crate::panel_preview::rasterize(1920, 1080, |ui| {
            panel
                .show(
                    ui,
                    &config.panel,
                    InstrumentInput::new(s.sim().systems(), s.sim().state().time_s)
                        .with_pilot(s.sim().controls())
                        .with_aircraft(&config)
                        .with_skins(&skins)
                        .with_glass(&state)
                        .with_autopilot(s.sim().installed_autopilot()),
                    s.flight_generation(),
                )
                .unwrap();
            panel.show_shared_hosts(ui, &mut s);
            if let Some(area) = panel.map_rect() {
                map.show_mfd(ui, area, &mut s);
            }
        })
        .unwrap();
        image.save(out.join(format!("{name}-panel.png"))).unwrap();
    }
}

fn pfd_boxes() -> Vec<[f32; 4]> {
    vec![
        [0., 0., 500., 28.],
        [0., 30., 90., 190.],
        [390., 30., 110., 190.],
        [0., 222., 168., 70.],
        [368., 222., 132., 70.],
        [208., 256., 84., 24.],
        [0., 394., 168., 22.],
        [344., 392., 156., 24.],
        [0., 418., 500., 22.],
    ]
}
fn plain_background_fraction(
    image: &image::RgbaImage,
    area: egui::Rect,
    boxes: &[[f32; 4]],
    canvas: [f32; 2],
) -> f64 {
    let [cw, ch] = canvas;
    let plain = [crate::theme::DIAL, crate::theme::BACKGROUND, crate::theme::PANEL].map(|c| c.to_array());
    let mut count = 0_u64;
    let mut total = 0_u64;
    for (x, y, p) in image.enumerate_pixels() {
        let at = egui::pos2(x as f32 + 0.5, y as f32 + 0.5);
        if !area.contains(at) {
            continue;
        }
        total += 1;
        let local = (at - area.min) / area.size() * egui::vec2(cw, ch);
        let reserved = boxes.iter().any(|[x, y, w, h]| {
            egui::Rect::from_min_size(egui::pos2(*x, *y), egui::vec2(*w, *h))
                .expand(1.)
                .contains(local.to_pos2())
        });
        if !reserved && plain.contains(&p.0) {
            count += 1;
        }
    }
    count as f64 / total.max(1) as f64
}
#[test]
fn pfd_content_has_under_one_percent_unassigned_plain_background() {
    use flightcraft_aero::modern::{DisplayComposition, TouchLayout};
    let mut s = crate::panel_fidelity_tests::geographic_session();
    s.execute("sim.step", serde_json::json!({"steps":7200})).unwrap();
    let mut state = s.glass_state();
    state.pfd_failed = false;
    state.mfd_failed = false;
    state.reversion = false;
    state.air_data.attitude_valid = true;
    state.air_data.air_data_valid = true;
    state.air_data.powered = true;
    state.standby.attitude_valid = true;
    let systems = s.sim().systems().clone();
    let input = InstrumentInput::new(&systems, 60.).with_glass(&state);
    let mut audit = Vec::new();
    for id in ["glass_pfd", "glass_standby"] {
        let image = crate::panel_preview::rasterize(500, 440, |ui| {
            let draw = crate::glass::create(id).unwrap().update(input);
            crate::glass::paint_fill(ui, ui.max_rect(), draw);
        })
        .unwrap();
        let (boxes, canvas) = if id == "glass_standby" {
            (
                vec![[58., 0., 84., 20.], [0., 135., 200., 24.], [40., 158., 120., 22.]],
                [200., 180.],
            )
        } else {
            (pfd_boxes(), [500., 440.])
        };
        let fraction = plain_background_fraction(
            &image,
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500., 440.)),
            &boxes,
            canvas,
        );
        assert!(fraction < 0.01, "{id}: {fraction:.3} unassigned background");
        audit.push(serde_json::json!({"display":id,"plain_fraction":fraction}));
    }
    for layout in [
        TouchLayout::FullPfd,
        TouchLayout::IntegratedPfd,
        TouchLayout::PfdMap,
    ] {
        let mut display = state.touch.displays.first().unwrap().clone();
        display.layout = layout;
        display.engine_strip = false;
        display.failed = false;
        display.powered = true;
        display.reversion = false;
        display.synthetic_vision = false;
        let composition = DisplayComposition::default();
        let mut maps = crate::map::PlanningMap::default();
        let image = crate::panel_preview::rasterize(1000, 600, |ui| {
            let viewports =
                crate::glass::paint_touch(ui, ui.max_rect(), &state, input, &display, &composition);
            // These insets are painted after the PFD just as in the installed panel host.
            for viewport in viewports {
                maps.show_mfd_base(ui, viewport.rect, &mut s, viewport.base);
            }
        })
        .unwrap();
        let integrated = layout == TouchLayout::IntegratedPfd;
        let area = egui::Rect::from_min_max(
            egui::pos2(
                if integrated { 200. } else { 0. },
                if integrated { 48. } else { 0. },
            ),
            egui::pos2(
                if layout == TouchLayout::PfdMap {
                    500.
                } else {
                    1000.
                },
                600.,
            ),
        );
        let mut boxes = pfd_boxes();
        if integrated {
            boxes.extend([[5., 299., 135., 93.], [360., 299., 135., 93.]]);
        }
        let fraction = plain_background_fraction(&image, area, &boxes, [500., 440.]);
        assert!(
            fraction < 0.01,
            "touch {layout:?}: {fraction:.3} unassigned background"
        );
        audit.push(serde_json::json!({"display":format!("{layout:?}"),"plain_fraction":fraction}));
    }
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/layout");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(
        out.join("coverage.json"),
        serde_json::to_string_pretty(&audit).unwrap(),
    )
    .unwrap();
}

#[test]
fn panels_use_available_width_without_clipping_in_short_hosts() {
    let catalog = flightcraft_aero::AircraftCatalog::builtins().unwrap();
    let mut audit = Vec::new();
    for (width, height) in [(1280., 720.), (1920., 1080.), (2560., 1440.), (390., 844.)] {
        let view = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height));
        for id in ["corvid", "larrow", "corvid-glass", "corvid-standby"] {
            let panel = catalog.resolve_aircraft(id).unwrap().panel;
            let cells = crate::instruments::layout(&panel, view);
            let bounds = cells.into_iter().fold(egui::Rect::NOTHING, |a, b| a.union(b));
            assert!(view.expand(0.1).contains_rect(bounds), "{id}: {bounds:?}");
            for compact in [false, true] {
                let host = egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(
                        width,
                        crate::instruments::view_panel_height(&panel, view.size(), compact),
                    ),
                );
                let cells = crate::instruments::layout(&panel, host);
                let session = flightcraft_control::Session::new().unwrap();
                let state = session.glass_state();
                let bounds = panel.instruments.iter().zip(cells).fold(
                    egui::Rect::NOTHING,
                    |a, (installation, cell)| {
                        let draw = crate::instruments::Registry::default()
                            .instrument(&installation.module)
                            .unwrap()
                            .update(InstrumentInput::new(session.sim().systems(), 0.).with_glass(&state));
                        a.union(crate::instruments::fit_size(cell, draw.size))
                    },
                );
                assert!(
                    host.expand(0.1).contains_rect(bounds),
                    "{id} compact={compact}: {bounds:?}"
                );
            }
            audit.push(serde_json::json!({"panel":id,"view":[width,height],"width_fraction":bounds.intersect(view).width()/width}));
        }
        for pack in ["equipment-demo", "panel-fidelity-demo"] {
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../examples/packs")
                .join(pack);
            let mut s = flightcraft_control::Session::new().unwrap();
            s.install_pack(&flightcraft_control::read_pack(&root).unwrap())
                .unwrap();
            let id = if pack == "equipment-demo" {
                "equipment-demonstrator"
            } else {
                "equipment-fidelity-demonstrator"
            };
            s.execute(
                "aircraft.load",
                serde_json::json!({"source":{"kind":"installed","id":id},"reset":true}),
            )
            .unwrap();
            let mut panel = s
                .aircraft_configuration()
                .unwrap()
                .panel
                .equipment
                .clone()
                .unwrap();
            assert!(crate::panel_preview::equipment_transform(&panel, 0, 720).is_err());
            for compact in [false, true] {
                let configuration = &s.aircraft_configuration().unwrap().panel;
                let host = egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(
                        width,
                        crate::instruments::view_panel_height(configuration, view.size(), compact),
                    ),
                );
                let bounds = crate::panel_preview::equipment_rect(&panel, host);
                assert!(
                    host.expand(0.1).contains_rect(bounds),
                    "{pack} compact={compact}: {bounds:?}"
                );
            }
            for essential in [None, Some([80., 90., 1120., 430.])] {
                panel.essential = essential;
                let fit = crate::panel_preview::equipment_rect(&panel, view);
                assert!(view.expand(0.1).contains_rect(fit));
                assert!(
                    (fit.width() - view.width()).abs() < 0.1 || (fit.height() - view.height()).abs() < 0.1
                );
                let [w, h] = panel.size;
                assert!((f64::from(fit.width() / fit.height()) - w / h).abs() < 1e-5);
            }
            audit.push(serde_json::json!({"panel":pack,"view":[width,height],"width_fraction":crate::panel_preview::equipment_rect(&panel,view).width()/width}));
        }
    }
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/layout");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(
        out.join("panel-fit.json"),
        serde_json::to_string_pretty(&audit).unwrap(),
    )
    .unwrap();
}

#[test]
fn wide_pfd_keeps_hsi_cardinal_labels_on_a_circle() {
    let s = crate::panel_fidelity_tests::geographic_session();
    let mut state = s.glass_state();
    state.pfd_failed = false;
    state.mfd_failed = false;
    state.reversion = false;
    state.air_data.attitude_valid = true;
    state.air_data.air_data_valid = true;
    state.air_data.heading_rad = 0.;
    let ctx = egui::Context::default();
    crate::theme::install(&ctx);
    for size in [egui::vec2(1000., 440.), egui::vec2(500., 880.)] {
        let frame = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        let draw = crate::glass::create("glass_pfd")
                            .unwrap()
                            .update(InstrumentInput::new(s.sim().systems(), 0.).with_glass(&state));
                        crate::glass::paint_fill(ui, ui.max_rect(), draw);
                    });
            },
        );
        let centre = egui::pos2(size.x * 0.5, size.y * 350. / 440.);
        let scale = (size.x / 500.).min(size.y / 440.);
        let mut cardinals = 0;
        for shape in frame.shapes {
            if let egui::Shape::Text(text) = shape.shape
                && matches!(text.galley.job.text.as_str(), "N" | "E" | "S" | "W")
            {
                let at = text.pos + text.galley.size() * 0.5;
                assert!(
                    (at.distance(centre) - 66. * scale).abs() < 1.,
                    "{} {:?}",
                    text.galley.job.text,
                    at
                );
                cardinals += 1;
            }
        }
        assert_eq!(cardinals, 4);
    }
}

/// The packaged Sling panel is kept locally so this regression runs offline in CI.
pub(crate) fn sling_panel() -> flightcraft_aero::Panel {
    let file = serde_json::from_str(include_str!("../tests/fixtures/sling2-panel.json")).unwrap();
    let panel = match file {
        flightcraft_aero::LayerFile::Panel(panel) => Some(panel),
        _ => None,
    };
    panel.unwrap()
}

#[test]
fn main_outside_and_complete_panels_cover_central_area_at_desktop_sizes() {
    use flightcraft_control::Session;
    for (width, height) in [(1280., 800.), (1440., 900.), (1920., 1080.)] {
        for id in ["sling2", "corvid", "larrow"] {
            let mut seed = Session::new().unwrap();
            if id != "sling2" {
                seed.execute(
                    "aircraft.load",
                    serde_json::json!({"source":{"kind":"installed","id":id},"reset":true}),
                )
                .unwrap();
            }
            let mut snapshot = seed.screen_snapshot();
            if id == "sling2" {
                snapshot.configuration.as_mut().unwrap().panel = sling_panel();
            }
            let panel = snapshot.configuration.as_ref().unwrap().panel.clone();
            let mut presentation = Session::new().unwrap();
            presentation.apply_screen_snapshot(snapshot).unwrap();
            let mut app = crate::FlightCraftApp::from_presentation(presentation, None, "test", None).unwrap();
            let ctx = egui::Context::default();
            crate::theme::install(&ctx);
            for _ in 0..3 {
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, height),
                        )),
                        ..Default::default()
                    },
                    |ctx| app.show_main(ctx, |_, _| Ok(())),
                );
            }
            let outside = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("page-rect", "page".to_owned()))))
                .unwrap();
            let host = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("page-rect", "panel".to_owned()))))
                .unwrap();
            let central = ctx.available_rect();
            // Use the actual page allocations made by the main canvas, including shell margins.
            assert!(
                outside.left() <= 8. && width - outside.right() <= 8.,
                "{id} {width}: {outside:?}"
            );
            assert!(
                host.left() <= 8. && width - host.right() <= 8.,
                "{id} {width}: {host:?}"
            );
            assert!((outside.bottom() - host.top()).abs() <= 1.);
            assert!(outside.height() > height * 0.3);
            assert!(host.bottom() <= height - 29.);
            assert!(
                central.expand(8.).contains_rect(outside.union(host)),
                "{central:?}"
            );
            assert!((outside.top() - central.top()).abs() <= 8.);
            assert!((host.bottom() - central.bottom()).abs() <= 8.);
            assert!(
                crate::theme::window_bounds(&ctx).top() <= 60.,
                "header wrapped for {id}"
            );
            if let Some(equipment) = &panel.equipment {
                let dash = crate::panel_art::dash_bounds(equipment);
                let transform = if dash.is_some() {
                    crate::panel_preview::dash_rect(equipment, host)
                } else {
                    crate::panel_preview::equipment_rect(equipment, host)
                };
                let scale = transform.width() / equipment.size[0] as f32;
                let visible = dash.map_or(transform, |r| {
                    egui::Rect::from_min_size(transform.min + r.min.to_vec2() * scale, r.size() * scale)
                });
                let painted = ctx
                    .data(|data| data.get_temp::<egui::Rect>(egui::Id::new("painted-equipment-bounds")))
                    .unwrap();
                assert!(
                    (painted.min - visible.min).length() < 0.1 && (painted.max - visible.max).length() < 0.1,
                    "main host did not paint the intended dash: {painted:?} vs {visible:?}"
                );
                assert!(
                    host.expand(0.1).contains_rect(visible),
                    "clipped dash: {visible:?} in {host:?}"
                );
                assert!(
                    visible.width() >= host.width() * 0.92 || (visible.height() - host.height()).abs() < 0.1,
                    "undersized dash {id}: {visible:?} in {host:?}"
                );
                for element in &equipment.elements {
                    let logical = crate::panel_art::installation_rect(element);
                    if dash.is_some_and(|r| !r.expand(0.01).contains_rect(logical)) {
                        continue;
                    }
                    let rect = egui::Rect::from_min_size(
                        transform.min + logical.min.to_vec2() * scale,
                        logical.size() * scale,
                    );
                    assert!(
                        host.expand(0.1).contains_rect(rect),
                        "clipped dash control {}: {rect:?}",
                        element.id
                    );
                }
                if let Some(dash) = dash {
                    let console = crate::panel_art::console_panel(equipment, dash);
                    assert!(console.elements.iter().any(|e| e.id == "fuel_selector"));
                    for e in &console.elements {
                        assert!(
                            egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(console.size[0] as f32, console.size[1] as f32)
                            )
                            .expand(0.1)
                            .contains_rect(crate::panel_art::installation_rect(e))
                        );
                    }
                }
            } else {
                // Switches occupy their own row before the conventional instrument grid.
                let instrument_host = egui::Rect::from_min_max(host.min + egui::vec2(0., 65.), host.max);
                for cell in crate::instruments::layout(&panel, instrument_host) {
                    assert!(instrument_host.expand(0.1).contains_rect(cell));
                }
            }
        }
    }
}

#[test]
fn crew_subtitles_are_at_most_two_lines_inside_view_and_do_not_move_shell() {
    use flightcraft_control::{Session, presentation::Snapshot};
    let mut seed = Session::new().unwrap();
    seed.execute("crew.mode", serde_json::json!({"mode":"watching"}))
        .unwrap();
    seed.execute(
        "crew.intent",
        serde_json::json!({"intent":{"kind":"take_off","airport":"default","runway":"36"}}),
    )
    .unwrap();
    let mut snapshot: Snapshot = seed.screen_snapshot();
    let event = snapshot.record.events.last_mut().unwrap();
    event.text = "A long crew subtitle for a full briefing. ".repeat(80);
    let mut s = Session::new().unwrap();
    s.apply_screen_snapshot(snapshot).unwrap();
    let mut app = crate::FlightCraftApp::from_presentation(s, None, "test", None).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install(&ctx);
    let mut bounds = Vec::new();
    for captions in [false, true] {
        let mut snapshot = app.session.screen_snapshot();
        snapshot.comms.panel.captions = captions;
        app.session.apply_screen_snapshot(snapshot).unwrap();
        let mut output = None;
        for _ in 0..3 {
            output = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280., 800.),
                    )),
                    ..Default::default()
                },
                |ctx| app.show_main(ctx, |_, _| Ok(())),
            ));
        }
        let outside = ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("page-rect", "page".to_owned()))))
            .unwrap();
        bounds.push(outside);
        let texts: Vec<_> = output
            .unwrap()
            .shapes
            .into_iter()
            .filter_map(|s| {
                if let egui::Shape::Text(t) = s.shape {
                    Some(t)
                } else {
                    None
                }
            })
            .collect();
        let caption = texts
            .iter()
            .find(|t| t.galley.text().contains("A long crew subtitle"));
        assert_eq!(caption.is_some(), captions);
        if let Some(caption) = caption {
            assert!(caption.galley.rows.len() <= 2);
            let rect = egui::Rect::from_min_size(caption.pos, caption.galley.size());
            assert!(outside.contains_rect(rect));
            assert!(rect.bottom() <= outside.bottom() - 24.);
            assert!(rect.top() > outside.center().y);
        }
    }
    assert_eq!(bounds[0], bounds[1], "captions changed layout");
}

#[test]
#[ignore = "local corrected Sling bundle; writes CPU dash preview without an outside scene"]
fn render_corrected_sling_dash_preview() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut s = flightcraft_control::Session::new().unwrap();
    s.install_region(flightcraft_control::read_region(&root.join("assets/terrain/eastern-pa")).unwrap())
        .unwrap();
    s.install_pack(&flightcraft_control::read_pack(&root.join("out/polish/packs/sling2")).unwrap())
        .unwrap();
    s.execute(
        "aircraft.load",
        serde_json::json!({"source":{"kind":"installed","id":"sling2"},"reset":true}),
    )
    .unwrap();
    s.execute("sim.reset",serde_json::json!({"world":"eastern-pa","start":{"kind":"airport_runway","airport":"1N7","end":"07"}})).unwrap();
    s.execute("sim.step", serde_json::json!({"steps":1})).unwrap();
    let config = s.aircraft_configuration().unwrap().clone();
    let state = s.glass_state();
    let map = state.engine.iter().find(|e| e.label == "MANIFOLD").unwrap();
    assert!((10. ..12.).contains(&(map.value / 3.386389)), "{} kPa", map.value);
    let mut panel = InstrumentPanel::default();
    panel.dash_only = true;
    let image = crate::panel_preview::rasterize(1280, 396, |ui| {
        panel
            .show(
                ui,
                &config.panel,
                InstrumentInput::new(s.sim().systems(), s.sim().state().time_s)
                    .with_aircraft(&config)
                    .with_glass(&state)
                    .with_autopilot(s.sim().installed_autopilot()),
                s.flight_generation(),
            )
            .unwrap();
    })
    .unwrap();
    image.save(root.join("out/polish/revised-dash-cpu.png")).unwrap();
}
