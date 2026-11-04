//! Regression tests for the ground layout (checklist beside a small exterior view, dash below)
//! and the exterior inset: the dash stays fully visible and readable from phone to desktop.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use eframe::egui;
use flightcraft_control::Session;
use serde_json::json;

const SIZES: [(f32, f32); 5] = [
    (1920., 1080.),
    (1440., 900.),
    (1280., 800.),
    (1024., 700.),
    (390., 844.),
];

struct Frame {
    texts: Vec<(String, egui::Rect)>,
    outlines: Vec<egui::Rect>,
    page: Option<egui::Rect>,
    host: egui::Rect,
    checklist: Option<egui::Rect>,
    exterior: Option<egui::Rect>,
    inset: Option<egui::Rect>,
    painted: Option<egui::Rect>,
}

fn rect(ctx: &egui::Context, id: &str) -> Option<egui::Rect> {
    ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("page-rect", id.to_owned()))))
}

/// Lay out the main window once at `size`, with the Sling dash fixture installed.
fn render(size: (f32, f32), setup: impl Fn(&mut Session), highlight: Option<&str>) -> Frame {
    let seed = Session::new().unwrap();
    let mut snapshot = seed.screen_snapshot();
    let configuration = snapshot.configuration.as_mut().unwrap();
    configuration.panel = crate::layout_tests::sling_panel();
    if let Some(control) = highlight {
        // The first item of the first list names the dash control to outline.
        let procedures = configuration.installed_systems.procedures.as_mut().unwrap();
        procedures.checklists[0].items[0].control = Some(control.to_owned());
    }
    let mut presentation = Session::new().unwrap();
    presentation.apply_screen_snapshot(snapshot).unwrap();
    let mut app = crate::FlightCraftApp::from_presentation(presentation, None, "test", None).unwrap();
    setup(&mut app.session);
    let ctx = egui::Context::default();
    crate::theme::install(&ctx);
    let mut output = None;
    for _ in 0..4 {
        output = Some(ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(size.0, size.1),
                )),
                ..Default::default()
            },
            |ctx| app.show_main(ctx, |_, _| Ok(())),
        ));
    }
    let shapes = output.unwrap().shapes;
    let outlines = shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Rect(r)
                if (r.stroke.width - 3.).abs() < 1e-3 && r.fill == egui::Color32::TRANSPARENT =>
            {
                Some(r.rect)
            }
            _ => None,
        })
        .collect();
    let texts = shapes
        .into_iter()
        .filter_map(|s| match s.shape {
            egui::Shape::Text(t) => Some((
                t.galley.job.text.clone(),
                egui::Rect::from_min_size(t.pos, t.galley.size()),
            )),
            _ => None,
        })
        .collect();
    Frame {
        page: rect(&ctx, "page"),
        host: rect(&ctx, "panel").unwrap(),
        checklist: rect(&ctx, "checklist"),
        exterior: rect(&ctx, "exterior"),
        inset: rect(&ctx, "exterior-inset"),
        painted: ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new("painted-equipment-bounds"))),
        texts,
        outlines,
    }
}

fn ground(session: &mut Session) {
    session
        .execute("checklist.select", json!({"id": "before_start"}))
        .unwrap();
}

/// The dash must be inside its host and as large as the host allows, whatever the arrangement.
fn assert_dash_fully_visible(f: &Frame, size: (f32, f32), what: &str) {
    let panel = crate::layout_tests::sling_panel();
    let equipment = panel.equipment.as_ref().unwrap();
    let dash = crate::panel_art::dash_bounds(equipment).unwrap();
    let transform = crate::panel_preview::dash_rect(equipment, f.host);
    let scale = transform.width() / equipment.size[0] as f32;
    let visible = egui::Rect::from_min_size(transform.min + dash.min.to_vec2() * scale, dash.size() * scale);
    let painted = f.painted.unwrap();
    assert!(
        (painted.min - visible.min).length() < 0.5 && (painted.max - visible.max).length() < 0.5,
        "{what} {size:?}: dash painted {painted:?}, expected {visible:?}"
    );
    assert!(
        f.host.expand(0.1).contains_rect(visible),
        "{what} {size:?}: clipped dash"
    );
    assert!(
        visible.width() >= f.host.width() * 0.92 || (visible.height() - f.host.height()).abs() < 0.5,
        "{what} {size:?}: undersized dash {visible:?} in {:?}",
        f.host
    );
    assert!(
        f.host.bottom() <= size.1 - 29.,
        "{what} {size:?}: dash under the footer"
    );
    for e in &equipment.elements {
        let logical = crate::panel_art::installation_rect(e);
        if !dash.expand(0.01).contains_rect(logical) {
            continue;
        }
        let r = egui::Rect::from_min_size(
            transform.min + logical.min.to_vec2() * scale,
            logical.size() * scale,
        );
        assert!(
            f.host.expand(0.1).contains_rect(r),
            "{what} {size:?}: clipped control {}",
            e.id
        );
    }
}

#[test]
fn ground_layout_splits_the_band_and_the_dash_is_unchanged_and_fully_visible() {
    for size in SIZES {
        let normal = render(size, |_| {}, None);
        let f = render(size, ground, None);
        let checklist = f.checklist.expect("checklist box");
        let exterior = f.exterior.expect("exterior box");
        assert!(
            normal.checklist.is_none() && normal.exterior.is_none(),
            "normal layout has neither"
        );
        assert!(
            normal.page.is_some() && f.page.is_none(),
            "the full outside view is replaced"
        );
        // The dash is identical to the normal layout's: same host, same painted bounds.
        assert!((f.host.min - normal.host.min).length() < 1. && (f.host.max - normal.host.max).length() < 1.);
        assert_dash_fully_visible(&f, size, "ground");
        let band_top = normal.page.unwrap().top();
        let band_bottom = normal.page.unwrap().bottom();
        for r in [checklist, exterior] {
            assert!(
                (r.top() - band_top).abs() < 1.5 || r.top() > band_top,
                "{size:?} {r:?}"
            );
            assert!(
                r.bottom() <= band_bottom + 1.5 && r.bottom() <= f.host.top() + 1.5,
                "{size:?}"
            );
        }
        assert!(
            !checklist.intersects(exterior.shrink(0.5)),
            "{size:?}: boxes overlap"
        );
        if size.0 >= 700. {
            // Side by side: the checklist is the larger box and the exterior sits on the other side.
            assert!(checklist.width() > exterior.width(), "{size:?}");
            assert!(exterior.left() >= checklist.right() - 1.);
            assert!(
                checklist.left() <= 8. && size.0 - exterior.right() <= 8.,
                "{size:?}"
            );
            assert!(
                checklist.height() >= 150. && checklist.width() >= size.0 * 0.5,
                "{size:?}: readable checklist"
            );
        } else {
            // A phone held upright stacks them, exterior underneath.
            assert!(exterior.top() >= checklist.bottom() - 1.5, "{size:?}");
            assert!(checklist.width() >= size.0 - 16.);
        }
        // The panel says what it is.
        assert!(
            f.texts
                .iter()
                .any(|(t, r)| t.contains("Before start") && checklist.expand(2.).contains_rect(*r)),
            "{size:?}: checklist title shown inside its box"
        );
    }
}

#[test]
fn ground_layout_text_is_not_clipped_to_a_sliver_at_laptop_sizes() {
    let f = render((1280., 800.), ground, None);
    let checklist = f.checklist.unwrap();
    // Several items are listed and each row is readable (at least 11 px tall).
    let rows: Vec<_> = f
        .texts
        .iter()
        .filter(|(t, r)| {
            checklist.contains_rect(*r)
                && (t.contains("Pre-flight") || t.contains("Walk-round") || t.contains("Brakes"))
        })
        .collect();
    assert!(!rows.is_empty());
    assert!(rows.iter().all(|(_, r)| r.height() >= 10.), "{rows:?}");
}

#[test]
fn ground_layout_returns_to_normal_in_the_air_or_by_choice() {
    let size = (1440., 900.);
    let f = render(
        size,
        |s| {
            ground(s);
            s.execute(
                "sim.reset",
                json!({"start":{"kind":"air","height_m":500.0,"airspeed_mps":50.0,"heading_rad":0.0}}),
            )
            .unwrap();
        },
        None,
    );
    assert!(
        f.checklist.is_none() && f.page.is_some(),
        "takeoff returns to the normal layout"
    );
    let f = render(
        size,
        |s| {
            ground(s);
            s.execute("view.layout", json!({"mode":"normal"})).unwrap();
        },
        None,
    );
    assert!(f.checklist.is_none() && f.page.is_some(), "normal by choice");
    let f = render(
        size,
        |s| {
            s.execute("view.layout", json!({"mode":"ground"})).unwrap();
        },
        None,
    );
    assert!(
        f.checklist.is_some() && f.exterior.is_some(),
        "ground by choice from the View menu"
    );
    let f = render(
        size,
        |s| {
            ground(s);
            s.execute("view.exterior_inset", json!({"mode":"off"})).unwrap();
        },
        None,
    );
    assert!(
        f.checklist.is_some() && f.exterior.is_none(),
        "inset off: the checklist takes the band"
    );
    assert!(f.checklist.unwrap().width() > size.0 - 20.);
}

#[test]
fn exterior_inset_shows_per_mode_and_never_covers_the_dash() {
    for size in SIZES {
        for (mode, on_ground, in_air) in [
            ("off", false, false),
            ("ground", true, false),
            ("always", true, true),
        ] {
            let g = render(
                size,
                |s| {
                    s.execute("view.exterior_inset", json!({"mode": mode})).unwrap();
                },
                None,
            );
            assert_eq!(g.inset.is_some(), on_ground, "{mode} on the ground {size:?}");
            let a = render(
                size,
                |s| {
                    s.execute("view.exterior_inset", json!({"mode": mode})).unwrap();
                    s.execute("sim.reset", json!({"start":{"kind":"air","height_m":500.0,"airspeed_mps":50.0,"heading_rad":0.0}}))
                    .unwrap();
                },
                None,
            );
            assert_eq!(a.inset.is_some(), in_air, "{mode} in flight {size:?}");
            for f in [&g, &a] {
                if let Some(inset) = f.inset {
                    let page = f.page.unwrap();
                    assert!(
                        page.expand(0.5).contains_rect(inset),
                        "{size:?}: inset inside the outside view"
                    );
                    assert!(
                        inset.bottom() <= f.host.top() + 0.5,
                        "{size:?}: inset never covers the dash"
                    );
                    if f.painted.is_some() {
                        assert_dash_fully_visible(f, size, mode);
                    }
                }
            }
        }
    }
}

#[test]
fn inset_placement_is_clamped_inside_the_outside_view() {
    let f = render(
        (1280., 800.),
        |s| {
            s.execute(
                "view.exterior_inset",
                json!({"mode":"always","placement":{"x":0.9,"y":0.95,"w":0.5,"h":0.5}}),
            )
            .unwrap();
        },
        None,
    );
    let (inset, page) = (f.inset.unwrap(), f.page.unwrap());
    assert!(page.expand(0.5).contains_rect(inset));
    assert!(inset.bottom() <= f.host.top() + 0.5);
}

#[test]
fn the_current_items_control_gets_a_pulsing_outline_on_the_dash() {
    let size = (1440., 900.);
    let plain = render(size, ground, None);
    let lit = render(size, ground, Some("lane_a"));
    let panel = crate::layout_tests::sling_panel();
    let equipment = panel.equipment.as_ref().unwrap();
    let transform = crate::panel_preview::dash_rect(equipment, lit.host);
    let scale = transform.width() / equipment.size[0] as f32;
    let e = equipment.elements.iter().find(|e| e.id == "lane_a").unwrap();
    let cell = egui::Rect::from_min_size(
        transform.min + egui::vec2(e.position[0] as f32, e.position[1] as f32) * scale,
        egui::vec2(e.size[0] as f32, e.size[1] as f32) * scale,
    );
    assert!(
        lit.host.contains_rect(cell),
        "the highlighted control is on screen"
    );
    let around = |f: &Frame| {
        f.outlines
            .iter()
            .any(|r| r.expand(1.).contains_rect(cell) && r.width() < cell.width() * 2.)
    };
    assert!(
        around(&lit),
        "pulsing outline around lane_a: {:?} vs {cell:?}",
        lit.outlines
    );
    assert!(!around(&plain), "no outline without a current control");
    // An id that is not on the dash highlights nothing and does not panic.
    let missing = render(size, ground, Some("no_such_control"));
    assert!(!around(&missing));
}

/// Every control a Sling 2 checklist item names resolves on the dash: a pulsing outline around the
/// control itself when it is on the dash, or around the Console button when it is on the lower
/// console. Checked at a desktop and a phone size.
#[test]
fn every_checklist_control_resolves_to_a_pulse_on_the_dash_or_the_console_button() {
    let systems: serde_json::Value = serde_json::from_str(include_str!(
        "../../control/tests/fixtures/sling-pack/systems.json"
    ))
    .unwrap();
    let mut controls = std::collections::BTreeSet::new();
    let mut items = 0;
    for list in systems["procedures"]["checklists"].as_array().unwrap() {
        for item in list["items"].as_array().unwrap() {
            if let Some(c) = item["control"].as_str() {
                controls.insert(c.to_owned());
                items += 1;
            }
        }
    }
    assert!(items > 40 && controls.len() >= 14, "{items} items, {controls:?}");
    for id in [
        "master_switch",
        "throttle",
        "park_brake",
        "brake_lever",
        "flap_selector",
        "pilot_stick",
        "fuel_selector",
    ] {
        assert!(controls.contains(id), "{id} is highlighted by no item");
    }
    let panel = crate::layout_tests::sling_panel();
    let equipment = panel.equipment.as_ref().unwrap();
    let dash = crate::panel_art::dash_bounds(equipment).unwrap();
    for size in [(1440., 900.), (390., 844.)] {
        let plain = render(size, ground, None);
        for id in &controls {
            let e = equipment.elements.iter().find(|e| &e.id == id).unwrap();
            let lit = render(size, ground, Some(id));
            let transform = crate::panel_art::dash_bounds(equipment)
                .map(|_| crate::panel_preview::dash_rect(equipment, lit.host));
            let transform = transform.unwrap();
            let scale = transform.width() / equipment.size[0] as f32;
            let on_dash = dash
                .expand(0.01)
                .contains_rect(crate::panel_art::installation_rect(e));
            let target = if on_dash {
                egui::Rect::from_min_size(
                    transform.min + egui::vec2(e.position[0] as f32, e.position[1] as f32) * scale,
                    egui::vec2(e.size[0] as f32, e.size[1] as f32) * scale,
                )
            } else {
                egui::Rect::from_min_size(
                    lit.host.right_bottom() - egui::vec2(94., 31.),
                    egui::vec2(88., 25.),
                )
            };
            let around = |f: &Frame| {
                f.outlines
                    .iter()
                    .any(|r| r.expand(1.).contains_rect(target) && r.width() < target.width() * 2.)
            };
            assert!(
                lit.host.expand(0.1).contains_rect(target),
                "{size:?}: {id} (or the Console button) is on screen"
            );
            assert!(
                around(&lit),
                "{size:?}: no pulse for {id}: {:?} vs {target:?}",
                lit.outlines
            );
            assert!(!around(&plain), "{size:?}: pulse without a current control");
        }
    }
}

/// CPU render of the ground layout with the real Sling 2 pack data (fixture copy): checklist on
/// the left with the run-up current, the exterior box on the right (blank here: the 3D scene needs
/// a GPU), and the whole dash below.
#[test]
#[ignore = "writes out/ground-layout/ground-1920x1080.png"]
fn render_ground_layout_preview() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut s = Session::new().unwrap();
    let pack =
        flightcraft_control::read_pack(&root.join("crates/control/tests/fixtures/sling-pack")).unwrap();
    s.install_pack(&pack).unwrap();
    s.execute(
        "aircraft.load",
        json!({"source":{"kind":"installed","id":"sling2"},"reset":true}),
    )
    .unwrap();
    s.execute("procedure_lesson.start", json!({"id":"sling-runup"}))
        .unwrap();
    s.execute("checklist.complete", json!({"item":"into_wind"}))
        .unwrap();
    s.execute("sim.step", json!({"steps": 120})).unwrap();
    let mut screen = s.screens().windows[0].clone();
    screen.layout = crate::screens::main_layout(&egui::Context::default(), &s, &screen.layout, false);
    let mut canvas = crate::screens::Canvas::default();
    let image = crate::panel_preview::rasterize(1920, 1080, |ui| {
        canvas.show(ui, &screen, &mut s, true);
    })
    .unwrap();
    let dir = root.join("out/ground-layout");
    std::fs::create_dir_all(&dir).unwrap();
    image.save(dir.join("ground-1920x1080.png")).unwrap();
}
