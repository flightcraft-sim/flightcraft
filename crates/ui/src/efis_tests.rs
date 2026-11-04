//! The landscape touch EFIS drawn and operated: geometry in any cell, key and knob events,
//! skin fallback, and every painted shape staying inside the installation at desktop and phone sizes.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::efis;
use crate::{instruments::*, modern::PanelUi, theme};
use eframe::egui::{self, Rect};
use flightcraft_aero::display_model::{BezelControl, DisplayModel};
use flightcraft_aero::modern::{MfdPage, PanelElement};
use flightcraft_aero::skin_pack::SkinPackManifest;
use flightcraft_control::touch_efis::{EfisKey, Knob, Ring};
use flightcraft_control::{DisplayAction, EquipmentSetting, PackContents, Session, SkinContents};
use serde_json::json;

const MODEL: DisplayModel = DisplayModel::LandscapeTouchEfis;

fn session() -> Session {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/packs/equipment-demo");
    let mut pack: PackContents = flightcraft_control::read_pack(&root).unwrap();
    for source in pack.files.values_mut() {
        let mut document: serde_json::Value = serde_json::from_str(source).unwrap();
        if document["kind"] != "panel" {
            continue;
        }
        for e in document["equipment"]["elements"].as_array_mut().unwrap() {
            if e["element"]["kind"] == "touch_display" {
                e["appearance"] = json!({"bezel":{"model":"landscape_touch_efis"}});
            }
        }
        *source = serde_json::to_string(&document).unwrap();
    }
    let mut s = Session::new().unwrap();
    s.install_pack(&pack).unwrap();
    s.execute(
        "aircraft.load",
        json!({"source":{"kind":"installed","id":"equipment-demonstrator"},"reset":true}),
    )
    .unwrap();
    s
}

/// The left display alone, filling a panel with the bezel's own aspect.
fn isolated(s: &Session) -> flightcraft_aero::modern::EquipmentPanel {
    let mut p = s
        .aircraft_configuration()
        .unwrap()
        .panel
        .equipment
        .clone()
        .unwrap();
    let [w, h] = [272.0, 195.0];
    p.size = [w, h];
    p.essential = None;
    p.shell = None;
    p.elements
        .retain(|e| e.id == "left_display" && matches!(e.element, PanelElement::TouchDisplay { .. }));
    for e in &mut p.elements {
        e.position = [0., 0.];
        e.size = [w, h];
    }
    p
}

fn frame(
    ctx: &egui::Context,
    view: &mut PanelUi,
    panel: &flightcraft_aero::modern::EquipmentPanel,
    s: &Session,
    size: egui::Vec2,
    events: Vec<egui::Event>,
    time: f64,
) -> egui::FullOutput {
    let state = s.glass_state();
    let skins: Vec<&SkinContents> = s.skins().collect();
    ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            time: Some(time),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    view.show(
                        ui,
                        panel,
                        InstrumentInput::new(s.sim().systems(), 0.)
                            .with_glass(&state)
                            .with_skins(&skins),
                        &Registry::default(),
                    );
                });
        },
    )
}

fn press(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    }
}

/// Centre of a control in a panel that fills `size`.
fn control_center(
    panel: &flightcraft_aero::modern::EquipmentPanel,
    size: egui::Vec2,
    c: BezelControl,
) -> egui::Pos2 {
    let cell = crate::panel_preview::equipment_rect(panel, Rect::from_min_size(egui::Pos2::ZERO, size));
    efis::control_rect(efis::bezel_rect(cell, MODEL), MODEL, c)
        .unwrap()
        .center()
}

fn display_actions(view: &mut PanelUi) -> Vec<DisplayAction> {
    std::mem::take(&mut view.commands)
        .into_iter()
        .filter_map(|c| match c {
            EquipmentSetting::Display { action, .. } => Some(action),
            _ => None,
        })
        .collect()
}

#[test]
fn bezel_and_screen_keep_the_models_aspect_in_any_cell() {
    for cell in [
        Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1920., 1080.)),
        Rect::from_min_size(egui::pos2(10., 20.), egui::vec2(390., 700.)),
        Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(272., 195.)),
        Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200., 600.)),
    ] {
        let bezel = efis::bezel_rect(cell, MODEL);
        assert!(cell.expand(0.01).contains_rect(bezel), "{cell:?}");
        assert!((bezel.width() / bezel.height() - MODEL.bezel_aspect() as f32).abs() < 1e-3);
        let screen = efis::screen_rect(bezel, MODEL);
        assert!(bezel.contains_rect(screen));
        assert!(
            (screen.width() / screen.height() - 1280. / 768.).abs() < 1e-3,
            "screen aspect {}",
            screen.width() / screen.height()
        );
        // The same fractions at every size: the screen's pixels scale uniformly.
        let ppx = screen.width() / 1280.;
        assert!((screen.height() / 768. - ppx).abs() < 1e-4);
        for c in BezelControl::ALL {
            let r = efis::control_rect(bezel, MODEL, c).unwrap();
            assert!(
                bezel.contains_rect(r.shrink(0.001)),
                "{c:?} outside the bezel in {cell:?}"
            );
            assert!(r.top() >= screen.bottom() - 0.01, "{c:?} overlaps the screen");
        }
    }
}

#[test]
fn keys_and_knobs_send_the_models_actions() {
    let s = session();
    let panel = isolated(&s);
    let size = egui::vec2(816., 585.);
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let mut view = PanelUi::default();
    frame(&ctx, &mut view, &panel, &s, size, Vec::new(), 0.0);
    view.commands.clear();
    let mut t = 1.0;
    for (control, key) in [
        (BezelControl::NearestKey, EfisKey::Nearest),
        (BezelControl::DirectKey, EfisKey::Direct),
        (BezelControl::MenuKey, EfisKey::Menu),
    ] {
        let at = control_center(&panel, size, control);
        frame(
            &ctx,
            &mut view,
            &panel,
            &s,
            size,
            vec![egui::Event::PointerMoved(at)],
            t,
        );
        frame(&ctx, &mut view, &panel, &s, size, vec![press(at, true)], t + 0.1);
        frame(&ctx, &mut view, &panel, &s, size, vec![press(at, false)], t + 0.2);
        t += 1.0;
        let actions = display_actions(&mut view);
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, DisplayAction::Key { key: k, hold: false } if *k == key)),
            "{control:?}: {actions:?}"
        );
    }
    // A wheel turn over the outer ring of the left knob is an outer-ring detent; over the centre, inner.
    let knob = control_center(&panel, size, BezelControl::LeftKnob);
    let rect = efis::control_rect(
        efis::bezel_rect(
            crate::panel_preview::equipment_rect(&panel, Rect::from_min_size(egui::Pos2::ZERO, size)),
            MODEL,
        ),
        MODEL,
        BezelControl::LeftKnob,
    )
    .unwrap();
    let outer_point = knob + egui::vec2(rect.width() * 0.4, 0.);
    for (at, ring) in [(outer_point, Ring::Outer), (knob, Ring::Inner)] {
        t += 1.0;
        frame(
            &ctx,
            &mut view,
            &panel,
            &s,
            size,
            vec![egui::Event::PointerMoved(at)],
            t,
        );
        frame(
            &ctx,
            &mut view,
            &panel,
            &s,
            size,
            vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0., 1.),
                modifiers: Default::default(),
            }],
            t + 0.1,
        );
        let actions = display_actions(&mut view);
        assert!(
            actions.iter().any(|a| matches!(a,
                DisplayAction::Knob { knob: Knob::Left, ring: r, detents: 1, .. } if *r == ring)),
            "{ring:?}: {actions:?}"
        );
    }
}

#[test]
fn back_distinguishes_a_press_from_a_hold() {
    let s = session();
    let panel = isolated(&s);
    let size = egui::vec2(816., 585.);
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let at = control_center(&panel, size, BezelControl::BackKey);
    // A short press.
    let mut view = PanelUi::default();
    frame(&ctx, &mut view, &panel, &s, size, Vec::new(), 0.0);
    frame(
        &ctx,
        &mut view,
        &panel,
        &s,
        size,
        vec![egui::Event::PointerMoved(at)],
        1.0,
    );
    frame(&ctx, &mut view, &panel, &s, size, vec![press(at, true)], 1.1);
    frame(&ctx, &mut view, &panel, &s, size, vec![press(at, false)], 1.3);
    let short = display_actions(&mut view);
    assert!(
        short
            .iter()
            .any(|a| matches!(a, DisplayAction::Key { hold: false, .. })),
        "{short:?}"
    );
    assert!(
        !short
            .iter()
            .any(|a| matches!(a, DisplayAction::Key { hold: true, .. }))
    );
    // A hold fires once while the button is still down, and the release adds nothing.
    let mut view = PanelUi::default();
    frame(
        &ctx,
        &mut view,
        &panel,
        &s,
        size,
        vec![egui::Event::PointerMoved(at)],
        5.0,
    );
    frame(&ctx, &mut view, &panel, &s, size, vec![press(at, true)], 5.1);
    frame(&ctx, &mut view, &panel, &s, size, Vec::new(), 5.4);
    frame(&ctx, &mut view, &panel, &s, size, Vec::new(), 5.9);
    frame(&ctx, &mut view, &panel, &s, size, Vec::new(), 6.1);
    frame(&ctx, &mut view, &panel, &s, size, vec![press(at, false)], 6.2);
    let held = display_actions(&mut view);
    let holds = held
        .iter()
        .filter(|a| {
            matches!(
                a,
                DisplayAction::Key {
                    key: EfisKey::Back,
                    hold: true
                }
            )
        })
        .count();
    assert_eq!(holds, 1, "{held:?}");
    assert!(
        !held
            .iter()
            .any(|a| matches!(a, DisplayAction::Key { hold: false, .. })),
        "{held:?}"
    );
}

#[test]
fn the_page_bar_and_split_button_send_page_and_split_actions() {
    let mut s = session();
    // Put the left display on the MFD so its page bar shows.
    s.execute(
        "equipment.set",
        json!({"setting":{"kind":"display","id":"left_display","action":{"kind":"layout","layout":"mfd"}}}),
    )
    .unwrap();
    let panel = isolated(&s);
    let size = egui::vec2(1088., 780.);
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let mut view = PanelUi::default();
    let cell = crate::panel_preview::equipment_rect(&panel, Rect::from_min_size(egui::Pos2::ZERO, size));
    let bezel = efis::bezel_rect(cell, MODEL);
    let screen = efis::screen_rect(bezel, MODEL);
    let bar = efis::page_bar(screen, flightcraft_control::touch_efis::ScreenMode::FullMfd).unwrap();
    // The last tab is the engine page.
    let tab = egui::pos2(bar.right() - bar.width() * 0.7 / 14., bar.center().y);
    frame(&ctx, &mut view, &panel, &s, size, Vec::new(), 0.0);
    frame(
        &ctx,
        &mut view,
        &panel,
        &s,
        size,
        vec![egui::Event::PointerMoved(tab)],
        1.0,
    );
    frame(&ctx, &mut view, &panel, &s, size, vec![press(tab, true)], 1.1);
    frame(&ctx, &mut view, &panel, &s, size, vec![press(tab, false)], 1.2);
    let actions = display_actions(&mut view);
    assert!(
        actions.iter().any(|a| matches!(
            a,
            DisplayAction::MfdPage {
                page: MfdPage::Engine
            }
        )),
        "{actions:?}"
    );
    // SPLIT is in the screen's upper right.
    let split = efis::native_rect(
        screen,
        MODEL
            .pfd_geometry(flightcraft_aero::display_model::PfdMode::Full)
            .screen_rect(flightcraft_aero::display_model::PfdElement::SplitButton),
    )
    .center();
    frame(
        &ctx,
        &mut view,
        &panel,
        &s,
        size,
        vec![egui::Event::PointerMoved(split)],
        2.0,
    );
    frame(&ctx, &mut view, &panel, &s, size, vec![press(split, true)], 2.1);
    frame(&ctx, &mut view, &panel, &s, size, vec![press(split, false)], 2.2);
    let actions = display_actions(&mut view);
    assert!(
        actions.iter().any(|a| matches!(a, DisplayAction::ToggleSplit {})),
        "{actions:?}"
    );
}

fn bounds(output: egui::FullOutput) -> Vec<Rect> {
    output
        .shapes
        .into_iter()
        .map(|s| s.shape.visual_bounding_rect())
        .filter(|r| r.is_finite())
        .collect()
}

#[test]
fn every_page_and_overlay_stays_inside_the_installation_at_desktop_and_phone_sizes() {
    let mut s = session();
    let panel = isolated(&s);
    for size in [
        egui::vec2(1920., 1080.),
        egui::vec2(390., 280.),
        egui::vec2(272., 195.),
    ] {
        let cell = crate::panel_preview::equipment_rect(&panel, Rect::from_min_size(egui::Pos2::ZERO, size));
        for (layout, key) in [
            ("integrated_pfd", None),
            ("mfd", None),
            ("split", None),
            ("mfd", Some("menu")),
            ("integrated_pfd", Some("direct")),
            ("split", Some("nearest")),
        ] {
            s.execute(
                "equipment.set",
                json!({"setting":{"kind":"display","id":"left_display","action":{"kind":"layout","layout":layout}}}),
            )
            .unwrap();
            if let Some(key) = key {
                s.execute(
                    "equipment.set",
                    json!({"setting":{"kind":"display","id":"left_display","action":{"kind":"key","key":key}}}),
                )
                .unwrap();
            }
            let ctx = egui::Context::default();
            theme::install(&ctx);
            let rects = bounds(frame(
                &ctx,
                &mut PanelUi::default(),
                &panel,
                &s,
                size,
                Vec::new(),
                0.0,
            ));
            assert!(!rects.is_empty());
            for r in rects {
                assert!(
                    cell.expand(1.5).contains_rect(r),
                    "{layout}/{key:?} at {size:?}: {r:?} outside {cell:?}"
                );
            }
        }
    }
}

fn skin(font: &[u8]) -> SkinContents {
    let manifest = json!({"kind":"skin_pack","format_version":1,"id":"test-skin","version":"1",
        "display_name":"Test","licence":"Apache-2.0","display_model":"landscape_touch_efis",
        "palette":{"bezel":"#201010","key_face":"#101020","key_text":"#ffffff"},
        "labels":{"nearest_key":"NEAR"},
        "font":"fonts/a.ttf","assets":[{"path":"fonts/a.ttf","kind":"font","licence":"Apache-2.0"}]});
    SkinContents {
        manifest: SkinPackManifest::from_json(&manifest.to_string()).unwrap(),
        assets: [("fonts/a.ttf".to_owned(), font.to_vec())].into(),
    }
}

fn labels(output: egui::FullOutput) -> Vec<String> {
    output
        .shapes
        .into_iter()
        .filter_map(|s| match s.shape {
            egui::Shape::Text(t) => Some(t.galley.job.text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_skin_changes_labels_and_a_broken_font_falls_back_without_failing() {
    let mut s = session();
    let panel = isolated(&s);
    let size = egui::vec2(1088., 780.);
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let built_in = labels(frame(
        &ctx,
        &mut PanelUi::default(),
        &panel,
        &s,
        size,
        Vec::new(),
        0.0,
    ));
    for l in ["NRST", "DIRECT", "MENU", "BACK"] {
        assert!(built_in.iter().any(|t| t == l), "{l} in {built_in:?}");
    }
    // A skin that names itself for the display: its labels and a real font apply.
    s.install_skin(skin(include_bytes!("../assets/DroidSans.ttf")))
        .unwrap();
    s.execute(
        "equipment.set",
        json!({"setting":{"kind":"display","id":"left_display","action":{"kind":"skin","skin":"test-skin"}}}),
    )
    .unwrap();
    let mut view = PanelUi::default();
    frame(&ctx, &mut view, &panel, &s, size, Vec::new(), 0.1);
    let skinned = labels(frame(&ctx, &mut view, &panel, &s, size, Vec::new(), 0.2));
    assert!(skinned.iter().any(|t| t == "NEAR"), "{skinned:?}");
    assert!(!skinned.iter().any(|t| t == "NRST"));
    // Garbage font bytes: still draws, with the built-in family.
    s.install_skin(skin(b"not a font at all")).unwrap();
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let fallback = labels(frame(
        &ctx,
        &mut PanelUi::default(),
        &panel,
        &s,
        size,
        Vec::new(),
        0.0,
    ));
    assert!(
        fallback.iter().any(|t| t == "NEAR"),
        "labels still apply: {fallback:?}"
    );
    // Removing the pilot's choice with no recommendation returns to the built-in skin.
    s.execute(
        "equipment.set",
        json!({"setting":{"kind":"display","id":"left_display","action":{"kind":"skin","skin":"built-in"}}}),
    )
    .unwrap();
    let back = labels(frame(
        &ctx,
        &mut PanelUi::default(),
        &panel,
        &s,
        size,
        Vec::new(),
        0.3,
    ));
    assert!(back.iter().any(|t| t == "NRST"), "{back:?}");
}
