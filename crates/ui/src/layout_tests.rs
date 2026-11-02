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
                        .with_aircraft(&config)
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
fn panels_cover_at_least_ninety_five_percent_of_common_view_widths() {
    let catalog = flightcraft_aero::AircraftCatalog::builtins().unwrap();
    let mut audit = Vec::new();
    for (width, height) in [(1280., 720.), (1920., 1080.), (2560., 1440.), (390., 844.)] {
        let view = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height));
        for id in ["corvid", "larrow", "corvid-glass", "corvid-standby"] {
            let panel = catalog.resolve_aircraft(id).unwrap().panel;
            let cells = crate::instruments::layout(&panel, view);
            let bounds = cells.into_iter().fold(egui::Rect::NOTHING, |a, b| a.union(b));
            assert!(bounds.intersect(view).width() / width >= 0.95, "{id}: {bounds:?}");
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
                    bounds.intersect(host).width() / width >= 0.95,
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
                    bounds.intersect(host).width() / width >= 0.95,
                    "{pack} compact={compact}: {bounds:?}"
                );
            }
            for essential in [None, Some([80., 90., 1120., 430.])] {
                panel.essential = essential;
                let fit = crate::panel_preview::equipment_rect(&panel, view);
                assert!(fit.intersect(view).width() / width >= 0.95);
                let [w, h] = panel.size;
                assert!((f64::from(fit.width() / fit.height()) - w / h).abs() < 1e-5);
                if height < fit.height()
                    && let Some([_, y, _, h]) = essential
                {
                    let cy = fit.top() + (y + h * 0.5) as f32 * fit.width() / w as f32;
                    assert!((cy - view.center().y).abs() < 1.);
                }
            }
            audit.push(serde_json::json!({"panel":pack,"view":[width,height],"width_fraction":1.}));
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
