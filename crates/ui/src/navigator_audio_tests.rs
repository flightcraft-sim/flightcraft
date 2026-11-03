//! Audio hosting uses live panel controls and an in-memory opt-in for read-only pack previews.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::{instruments::*, modern::PanelUi, theme};
use eframe::egui::{self, Rect};
use flightcraft_aero::{modern::PanelElement, panel_fidelity::NavigatorLayout};
use flightcraft_control::{PackContents, Session};
use serde_json::json;

fn enable_audio(pack: &mut PackContents) {
    let mut found = false;
    for source in pack.files.values_mut() {
        let mut document: serde_json::Value = serde_json::from_str(source).unwrap();
        if document["kind"] != "panel" {
            continue;
        }
        for e in document["equipment"]["elements"].as_array_mut().unwrap() {
            if e["element"]["kind"] == "navigator" {
                e["element"]["audio_panel"] = json!(true);
                found = true;
            }
        }
        *source = serde_json::to_string(&document).unwrap();
    }
    assert!(found);
}
fn session() -> Session {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/packs/equipment-demo");
    let mut pack = flightcraft_control::read_pack(&root).unwrap();
    enable_audio(&mut pack);
    let mut s = Session::new().unwrap();
    s.install_pack(&pack).unwrap();
    s.execute(
        "aircraft.load",
        json!({"source":{"kind":"installed","id":"equipment-demonstrator"},"reset":true}),
    )
    .unwrap();
    s
}
fn isolated_panel(
    s: &Session,
    layout: NavigatorLayout,
    transponder: bool,
) -> flightcraft_aero::modern::EquipmentPanel {
    let mut p = s
        .aircraft_configuration()
        .unwrap()
        .panel
        .equipment
        .clone()
        .unwrap();
    p.size = [235., 98.];
    p.essential = None;
    p.shell = None;
    p.elements
        .retain(|e| matches!(e.element, PanelElement::Navigator { .. }));
    for e in &mut p.elements {
        e.position = [0., 0.];
        e.size = [235., 98.];
        e.appearance.navigator_layout = layout;
        if let PanelElement::Navigator { transponder: t, .. } = &mut e.element {
            *t = transponder;
        }
    }
    p
}
fn frame(
    ctx: &egui::Context,
    view: &mut PanelUi,
    panel: &flightcraft_aero::modern::EquipmentPanel,
    s: &Session,
    events: Vec<egui::Event>,
    scale: f32,
    transponder: bool,
) -> egui::FullOutput {
    let mut state = s.glass_state();
    state.touch.navigator.as_mut().unwrap().transponder = transponder;
    ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(235., 98.) * scale,
            )),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    view.show(
                        ui,
                        panel,
                        InstrumentInput::new(s.sim().systems(), 0.).with_glass(&state),
                        &Registry::default(),
                    );
                });
        },
    )
}
fn texts(output: egui::FullOutput) -> Vec<(String, Rect, f32)> {
    output
        .shapes
        .into_iter()
        .filter_map(|shape| {
            if let egui::Shape::Text(text) = shape.shape {
                Some((
                    text.galley.job.text.clone(),
                    Rect::from_min_size(text.pos, text.galley.size()),
                    text.galley.job.sections.first().unwrap().format.font_id.size,
                ))
            } else {
                None
            }
        })
        .collect()
}
#[test]
fn navigator_audio_captions_do_not_overlap_and_use_shared_fonts() {
    let mut s = session();
    for layout in [NavigatorLayout::Classic, NavigatorLayout::HomeColumns] {
        for transponder in [false, true] {
            let panel = isolated_panel(&s, layout, transponder);
            for page in ["home", "audio"] {
                s.execute(
                    "equipment.set",
                    json!({"setting":{"kind":"navigator","action":{"kind":"page","page":page}}}),
                )
                .unwrap();
                for scale in [1., 0.77] {
                    let ctx = egui::Context::default();
                    theme::install(&ctx);
                    let labels = texts(frame(
                        &ctx,
                        &mut PanelUi::default(),
                        &panel,
                        &s,
                        Vec::new(),
                        scale,
                        transponder,
                    ));
                    let bounds = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(235., 98.) * scale);
                    for (i, (label, rect, _)) in labels.iter().enumerate() {
                        assert!(
                            bounds.expand(0.1).contains_rect(*rect),
                            "{layout:?} {page}: {label} outside navigator: {rect:?}"
                        );
                        for (other, r, _) in labels.iter().skip(i + 1) {
                            let overlap = rect.intersect(*r);
                            assert!(
                                overlap.width() <= 0.1 || overlap.height() <= 0.1,
                                "{layout:?} {page}: {label} overlaps {other}"
                            );
                        }
                    }
                    let grid: Vec<_> = labels
                        .iter()
                        .filter(|(label, _, _)| {
                            if page == "audio" {
                                [
                                    "MIC1", "MIC2", "RX1", "RX2", "NAV ID", "ICS", "VOL −", "VOL +", "SQ −",
                                    "SQ +", "VOICE", "CAP",
                                ]
                                .contains(&label.as_str())
                            } else {
                                [
                                    "Flight plan",
                                    "Direct-to",
                                    "Procedures",
                                    "Nearest",
                                    "Map",
                                    "Waypoint info",
                                    "Frequencies",
                                    "Transponder",
                                    "Audio",
                                ]
                                .iter()
                                .any(|tile| label.replace('\n', " ") == *tile)
                            }
                        })
                        .collect();
                    assert_eq!(
                        grid.len(),
                        if page == "audio" {
                            12
                        } else if transponder {
                            9
                        } else {
                            8
                        }
                    );
                    assert!(grid.iter().all(|(_, _, size)| *size == grid.first().unwrap().2));
                }
            }
        }
    }
}
fn click(
    ctx: &egui::Context,
    view: &mut PanelUi,
    panel: &flightcraft_aero::modern::EquipmentPanel,
    s: &mut Session,
    pos: egui::Pos2,
) {
    for pressed in [true, false] {
        let _ = frame(
            ctx,
            view,
            panel,
            s,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            1.,
            false,
        );
        for command in std::mem::take(&mut view.commands) {
            s.set_equipment(command).unwrap();
        }
    }
}
#[test]
fn navigator_audio_home_tile_and_every_control_change_shared_comms() {
    let mut s = session();
    let panel = isolated_panel(&s, NavigatorLayout::HomeColumns, false);
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let mut view = PanelUi::default();
    let labels = texts(frame(&ctx, &mut view, &panel, &s, Vec::new(), 1., false));
    let tile = labels.iter().find(|(label, _, _)| label == "Audio").unwrap().1;
    click(&ctx, &mut view, &panel, &mut s, tile.center());
    assert_eq!(
        s.touch_state().navigator.unwrap().page,
        flightcraft_control::NavigatorPage::Audio
    );
    for (label, field, value) in [
        ("MIC2", "transmit", json!(2)),
        ("MIC1", "transmit", json!(1)),
        ("RX1", "receive_com1", json!(false)),
        ("RX2", "receive_com2", json!(true)),
        ("NAV ID", "nav_ident", json!(true)),
        ("ICS", "intercom", json!(false)),
        ("VOL −", "volume", json!(0.6)),
        ("VOL +", "volume", json!(0.7)),
        ("SQ −", "squelch", json!(0.1)),
        ("SQ +", "squelch", json!(0.2)),
        ("VOICE", "speech", json!(false)),
        ("CAP", "captions", json!(false)),
    ] {
        let labels = texts(frame(&ctx, &mut view, &panel, &s, Vec::new(), 1., false));
        let pos = labels
            .iter()
            .find(|(text, _, _)| text == label)
            .unwrap()
            .1
            .center();
        click(&ctx, &mut view, &panel, &mut s, pos);
        let shared = serde_json::to_value(s.comms_state().panel).unwrap();
        if let Some(expected) = value.as_f64() {
            assert!(
                (shared[field].as_f64().unwrap() - expected).abs() < 1e-9,
                "{label}"
            );
        } else {
            assert_eq!(shared[field], value, "{label}");
        }
    }
}
#[test]
fn hosted_audio_suppresses_separate_face_even_on_home() {
    let s = session();
    let mut panel = isolated_panel(&s, NavigatorLayout::HomeColumns, false);
    let mut face = panel.elements.first().unwrap().clone();
    face.id = "audio_face".into();
    face.element = PanelElement::AudioPanel;
    panel.elements.push(face);
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let labels = texts(frame(
        &ctx,
        &mut PanelUi::default(),
        &panel,
        &s,
        Vec::new(),
        1.,
        false,
    ));
    assert!(labels.iter().any(|(label, _, _)| label == "Audio"));
    assert!(!labels.iter().any(|(label, _, _)| label == "MIC1"));
    for e in &mut panel.elements {
        if let PanelElement::Navigator { audio_panel, .. } = &mut e.element {
            *audio_panel = false;
        }
    }
    let labels = texts(frame(
        &ctx,
        &mut PanelUi::default(),
        &panel,
        &s,
        Vec::new(),
        1.,
        false,
    ));
    assert!(labels.iter().any(|(label, _, _)| label == "MIC1"));
}

#[test]
#[ignore = "loads an external read-only pack and writes navigator audio previews"]
fn render_sling_navigator_audio_preview() {
    let root = std::path::PathBuf::from(std::env::var_os("FLIGHTCRAFT_SLING_PACK").unwrap());
    let mut pack = flightcraft_control::read_pack(&root).unwrap();
    let original = flightcraft_control::read_pack(&root).unwrap().files;
    enable_audio(&mut pack);
    let mut s = Session::new().unwrap();
    s.install_pack(&pack).unwrap();
    s.execute(
        "aircraft.load",
        json!({"source":{"kind":"installed","id":"sling2"},"reset":true}),
    )
    .unwrap();
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/navaudio");
    std::fs::create_dir_all(&out).unwrap();
    for page in ["home", "audio"] {
        s.execute(
            "equipment.set",
            json!({"setting":{"kind":"navigator","action":{"kind":"page","page":page}}}),
        )
        .unwrap();
        let image = crate::panel_preview::render_equipment_panel(&mut s, 1728, 968).unwrap();
        image.save(out.join(format!("sling-{page}-panel.png"))).unwrap();
        let panel = s
            .aircraft_configuration()
            .unwrap()
            .panel
            .equipment
            .as_ref()
            .unwrap();
        let nav = panel
            .elements
            .iter()
            .find(|e| matches!(e.element, PanelElement::Navigator { .. }))
            .unwrap();
        let [left, top, scale] = crate::panel_preview::equipment_transform(panel, 1728, 968).unwrap();
        image::imageops::crop_imm(
            &image,
            (left + nav.position[0] * scale).round() as u32,
            (top + nav.position[1] * scale).round() as u32,
            (nav.size[0] * scale).round() as u32,
            (nav.size[1] * scale).round() as u32,
        )
        .to_image()
        .save(out.join(format!("sling-{page}-navigator.png")))
        .unwrap();
    }
    assert_eq!(flightcraft_control::read_pack(&root).unwrap().files, original);
    std::fs::write(out.join("preview.txt"), "Sling pack loaded read-only. The navigator element opts into audio_panel:true in memory only; no installation is added or moved. CPU egui capture at 1728 x 968; synthetic vision is omitted. Audio page reads shared comms state. No platform speech or reception verification.\n").unwrap();
}
