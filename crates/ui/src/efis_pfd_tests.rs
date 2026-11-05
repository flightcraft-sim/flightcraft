//! The landscape touch EFIS primary flight display: element placement against the model's
//! documented arrangement, the data bar, status bar and AFCS box, the touch boxes, the keypads,
//! the nearest tabs and zoom buttons, and the skin's instrument palette and typography.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::efis;
use crate::glass::touch_pfd::{self, Options, Typography};
use crate::{instruments::*, modern::PanelUi, theme};
use eframe::egui::{self, Rect};
use flightcraft_aero::display_model::{DataField, DisplayModel, PfdElement as E, PfdGeometry, PfdMode};
use flightcraft_aero::modern::{MfdPage, NearestPage, PanelElement};
use flightcraft_aero::skin_pack::SkinPackManifest;
use flightcraft_control::touch_efis::{Overlay, ScreenMode};
use flightcraft_control::touch_entry::EntryTarget;
use flightcraft_control::{DisplayAction, EquipmentSetting, GlassState, PackContents, Session, SkinContents};
use flightcraft_sim::equipment::{LateralMode, VerticalMode};
use serde_json::json;

const MODEL: DisplayModel = DisplayModel::LandscapeTouchEfis;

pub(crate) fn session() -> Session {
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

/// A snapshot of a flying aircraft: valid and aligned sensors, a GPS leg, and some wind.
pub(crate) fn flying(s: &Session) -> GlassState {
    let mut g = s.glass_state();
    let a = &mut g.air_data;
    a.powered = true;
    a.attitude_valid = true;
    a.air_data_valid = true;
    a.ahrs_failed = false;
    a.adc_failed = false;
    a.alignment_remaining_s = 0.;
    a.indicated_airspeed_mps = 150. / 1.943_844;
    a.true_airspeed_mps = 164. / 1.943_844;
    a.altitude_msl_m = 5000. * 0.3048;
    a.heading_rad = 265f64.to_radians();
    a.roll_rad = 0.1;
    a.pitch_rad = 0.04;
    a.vertical_speed_mps = 500. * 0.3048 / 60.;
    a.outside_temperature_c = 15.;
    g.settings.selected_altitude_m = 5500. * 0.3048;
    g.settings.heading_bug_rad = 232f64.to_radians();
    g.settings.course_manual = true;
    g.settings.course_rad = 267f64.to_radians();
    g.baro_pa = 29.92 * 3386.389;
    g.ground_speed_mps = 164. / 1.943_844;
    g.ground_track_rad = 265f64.to_radians();
    g.wind_speed_mps = 12. / 1.943_844;
    g.wind_from_rad = 350f64.to_radians();
    g.gps.valid = true;
    g.gps.distance_valid = true;
    g.gps.source = "GPS".into();
    g.gps.destination = Some("KMMU".into());
    g.gps.distance_nm = 11.1;
    g.gps.bearing_rad = 267f64.to_radians();
    g.gps.ete_s = Some(242.);
    g.gps.desired_track_rad = 267f64.to_radians();
    g.gps.cdi_full_scale_nm = 1.25;
    g
}

pub(crate) fn labels(d: &DrawList) -> Vec<(String, [f32; 2], f32, Ink)> {
    d.flat_items()
        .into_iter()
        .filter_map(|i| match i {
            DrawItem::Label { at, text, size, ink } => Some((text.clone(), *at, *size, *ink)),
            _ => None,
        })
        .collect()
}

fn inside([x, y, w, h]: [f64; 4], at: [f32; 2]) -> bool {
    let (x, y, w, h) = (x as f32, y as f32, w as f32, h as f32);
    at[0] >= x - 0.5 && at[0] <= x + w + 0.5 && at[1] >= y - 0.5 && at[1] <= y + h + 0.5
}

fn drawn_in(d: &DrawList, rect: [f64; 4], needle: &str) -> bool {
    labels(d)
        .iter()
        .any(|(t, at, ..)| t.contains(needle) && inside(rect, *at))
}

pub(crate) fn options() -> Options {
    Options {
        typography: Typography::default(),
        reversion: false,
    }
}

#[test]
fn full_pfd_elements_are_drawn_inside_the_rectangles_the_model_documents() {
    let s = session();
    let g = flying(&s);
    let input = InstrumentInput::new(s.sim().systems(), 0.);
    let geometry = MODEL.pfd_geometry(PfdMode::Full);
    let d = touch_pfd::panel(&g, input, &geometry, options());
    assert_eq!(d.size, [1280., 768.]);
    let r = |e| geometry.rect(e);
    // The pointers carry the numbers, centred in their boxes.
    assert!(drawn_in(&d, r(E::AirspeedPointer), "150"));
    assert!(drawn_in(&d, r(E::AltitudePointer), "5,000") || drawn_in(&d, r(E::AltitudePointer), "5000"));
    // Boxes around the tapes.
    assert!(drawn_in(&d, r(E::TrueAirspeedBox), "164KT"));
    assert!(drawn_in(&d, r(E::GroundSpeedBox), "164KT"));
    assert!(
        drawn_in(&d, r(E::SelectedAltitudeBox), "5,500FT")
            || drawn_in(&d, r(E::SelectedAltitudeBox), "5500FT")
    );
    assert!(drawn_in(&d, r(E::BaroBox), "29.92IN"));
    assert!(drawn_in(&d, r(E::SelectedHeadingBox), "HDG 232"));
    assert!(drawn_in(&d, r(E::SelectedCourseBox), "CRS 267"));
    assert!(drawn_in(&d, r(E::HeadingBox), "265"));
    assert!(drawn_in(&d, r(E::WindBox), "12KT"));
    assert!(drawn_in(&d, r(E::WindBox), "350"));
    // Tape numbers sit on their tapes; vertical speed digits beside the scale.
    let speed = r(E::AirspeedTape);
    assert!(
        labels(&d)
            .iter()
            .any(|(t, at, ..)| t == "160" && inside(speed, *at)),
        "airspeed scale numbers"
    );
    let alt = r(E::AltitudeTape);
    assert!(
        labels(&d)
            .iter()
            .any(|(t, at, ..)| (t == "5,100" || t == "5100") && inside(alt, *at)),
        "altitude scale numbers"
    );
    let vsi = r(E::VerticalSpeedScale);
    assert!(drawn_in(&d, vsi, "2") && drawn_in(&d, vsi, ".5") && drawn_in(&d, vsi, "1"));
    // Every label stays on the canvas.
    for (text, at, ..) in labels(&d) {
        assert!(
            (0. ..=1280.).contains(&at[0]) && (0. ..=768.).contains(&at[1]),
            "{text} at {at:?}"
        );
    }
}

#[test]
fn compass_rose_is_centred_where_the_model_puts_it_with_cardinals_every_ninety_degrees() {
    let s = session();
    let g = flying(&s);
    let input = InstrumentInput::new(s.sim().systems(), 0.);
    let geometry = MODEL.pfd_geometry(PfdMode::Full);
    let d = touch_pfd::panel(&g, input, &geometry, options());
    let [cx, cy] = geometry.hsi_center.map(|v| v as f32);
    let r = geometry.hsi_radius as f32;
    // Heading 265: west is almost straight up, the labels all lie on the rose's ring.
    let rose: Vec<_> = labels(&d)
        .into_iter()
        .filter(|(t, ..)| ["N", "E", "S", "W", "30", "33", "24", "21"].contains(&t.as_str()))
        .collect();
    assert!(rose.len() >= 8, "{rose:?}");
    for (text, at, ..) in &rose {
        let dist = libm::hypot(f64::from(at[0] - cx), f64::from(at[1] - cy)) as f32;
        assert!((dist - (r - 30.)).abs() < 1.5, "{text} at radius {dist}");
    }
    let w = rose.iter().find(|(t, ..)| t == "W").unwrap();
    assert!(
        (w.1[0] - cx).abs() < 20. && w.1[1] < cy,
        "west is at the top at heading 265"
    );
    // The aircraft symbol (white lines) is at the centre.
    assert!(d.flat_items().iter().any(|i| matches!(i,
        DrawItem::Line { a, b, ink: Ink::White, .. } if (a[0] - cx).abs() < 30. && (b[0] - cx).abs() < 30. && (a[1] - cy).abs() < 30.)));
}

#[test]
fn guidance_colours_follow_the_navigation_source() {
    let s = session();
    let mut g = flying(&s);
    let input = InstrumentInput::new(s.sim().systems(), 0.);
    let geometry = MODEL.pfd_geometry(PfdMode::Full);
    let course_ink = |g: &GlassState| {
        let d = touch_pfd::panel(g, input, &geometry, options());
        labels(&d)
            .into_iter()
            .find(|(t, ..)| t.starts_with("CRS"))
            .map(|(.., ink)| ink)
            .unwrap()
    };
    assert_eq!(course_ink(&g), Ink::Magenta, "GPS guidance is magenta");
    g.gps.source = "NAV1".into();
    assert_eq!(course_ink(&g), Ink::Green, "a VOR or localizer is green");
    // The selected heading is always cyan.
    let d = touch_pfd::panel(&g, input, &geometry, options());
    assert!(
        labels(&d)
            .iter()
            .any(|(t, _, _, ink)| t.starts_with("HDG") && *ink == Ink::Cyan)
    );
    // Trend vectors are magenta: accelerating and climbing.
    g.air_data.airspeed_trend_mps = 3.;
    g.air_data.altitude_trend_m = 150.;
    let d = touch_pfd::panel(&g, input, &geometry, options());
    let trends = d
        .flat_items()
        .iter()
        .filter(
            |i| matches!(i, DrawItem::Line { ink: Ink::Magenta, width, .. } if (*width - 4.).abs() < 0.01),
        )
        .count();
    assert_eq!(trends, 2, "one airspeed and one altitude trend vector");
}

fn autopilot_state(s: &Session) -> flightcraft_sim::equipment::EquipmentState {
    s.sim().installed_autopilot().unwrap().clone()
}

#[test]
fn the_afcs_box_shows_the_autopilot_modes_in_the_models_box() {
    let s = session();
    let g = flying(&s);
    let geometry = MODEL.pfd_geometry(PfdMode::Full);
    let box_rect = geometry.rect(E::AfcsBox);
    let mut ap = autopilot_state(&s);
    // Nothing engaged: an empty box.
    let input = InstrumentInput::new(s.sim().systems(), 0.).with_autopilot(Some(&ap));
    let d = touch_pfd::panel(&g, input, &geometry, options());
    assert!(!drawn_in(&d, box_rect, "AP"));
    ap.engaged = true;
    ap.flight_director = true;
    ap.yaw_damper = true;
    ap.active_lateral = Some(LateralMode::Hdg);
    ap.armed_lateral = Some(LateralMode::Nav);
    ap.active_vertical = Some(VerticalMode::Alt);
    ap.captured_altitude_m = 5000. * 0.3048;
    ap.alts_armed = true;
    let input = InstrumentInput::new(s.sim().systems(), 0.).with_autopilot(Some(&ap));
    let d = touch_pfd::panel(&g, input, &geometry, options());
    let cell = |text: &str| {
        labels(&d)
            .into_iter()
            .find(|(t, at, ..)| t == text && inside(box_rect, *at))
            .unwrap_or_else(|| panic!("{text} in the AFCS box"))
    };
    let (_, ap_at, _, ap_ink) = cell("AP");
    let (_, yd_at, ..) = cell("YD");
    let (_, hdg_at, _, hdg_ink) = cell("HDG");
    let (_, alt_at, _, alt_ink) = cell("ALT");
    let (_, gps_at, _, gps_ink) = cell("GPS");
    let (_, alts_at, _, alts_ink) = cell("ALTS");
    // Active modes are green, armed modes white.
    assert_eq!((ap_ink, hdg_ink, alt_ink), (Ink::Green, Ink::Green, Ink::Green));
    assert_eq!((gps_ink, alts_ink), (Ink::White, Ink::White));
    // Autopilot and yaw damper far left, lateral centre, vertical right.
    assert!(ap_at[0] < yd_at[0] && yd_at[0] < gps_at[0] && gps_at[0] < hdg_at[0]);
    assert!(hdg_at[0] < alt_at[0] && alt_at[0] < alts_at[0]);
    assert!(drawn_in(&d, box_rect, "5000ft"));
    // A disconnect flashes the annunciation yellow, then it disappears with the flash.
    ap.engaged = false;
    ap.ap_flash = true;
    ap.ap_flash_visible = true;
    ap.active_lateral = None;
    ap.active_vertical = None;
    let input = InstrumentInput::new(s.sim().systems(), 0.).with_autopilot(Some(&ap));
    let d = touch_pfd::panel(&g, input, &geometry, options());
    assert!(
        labels(&d)
            .iter()
            .any(|(t, _, _, ink)| t == "AP" && *ink == Ink::Yellow)
    );
}

#[test]
fn flight_director_bars_are_hollow_by_hand_solid_with_the_autopilot_and_gone_in_reversion() {
    let s = session();
    let g = flying(&s);
    let geometry = MODEL.pfd_geometry(PfdMode::Full);
    let mut ap = autopilot_state(&s);
    ap.flight_director = true;
    ap.command_pitch_rad = g.air_data.pitch_rad + 0.03;
    let bars = |ap: &flightcraft_sim::equipment::EquipmentState, reversion: bool| {
        let input = InstrumentInput::new(s.sim().systems(), 0.).with_autopilot(Some(ap));
        let d = touch_pfd::panel(
            &g,
            input,
            &geometry,
            Options {
                typography: Typography::default(),
                reversion,
            },
        );
        let items = d.flat_items();
        let solid = items
            .iter()
            .filter(|i| matches!(i, DrawItem::Polygon { ink: Ink::Magenta, points } if points.len() == 4))
            .count();
        let hollow = items
            .iter()
            .filter(|i| matches!(i, DrawItem::Line { ink: Ink::Magenta, width, .. } if (*width - 2.).abs() < 0.01))
            .count();
        (solid, hollow)
    };
    // Hand flying with the director alone: outlines, no fills.
    assert_eq!(bars(&ap, false), (0, 8));
    // With the autopilot engaged the bars are solid.
    ap.engaged = true;
    assert_eq!(bars(&ap, false), (2, 0));
    // The director is unavailable in reversion.
    assert_eq!(bars(&ap, true), (0, 0));
    // Director off: no bars.
    ap.flight_director = false;
    assert_eq!(bars(&ap, false), (0, 0));
}

#[test]
fn bars_run_com_fields_in_order_and_knob_hints_across_the_bottom() {
    let s = session();
    let g = flying(&s);
    let input = InstrumentInput::new(s.sim().systems(), 0.);
    for mode in [PfdMode::Full, PfdMode::Split] {
        let geometry = MODEL.pfd_geometry(mode);
        let d = touch_pfd::bars(
            &g,
            input,
            &geometry,
            Typography::default(),
            [("Heading", "Altitude"), ("Course", "Baro")],
        );
        let all = labels(&d);
        // Field names above their values, in the model's order and at its x positions.
        let names: Vec<_> = geometry
            .data_fields
            .iter()
            .map(|(f, x)| (f.label(), *x))
            .collect();
        for (name, x) in &names {
            let (_, at, ..) = all
                .iter()
                .find(|(t, ..)| t == name)
                .unwrap_or_else(|| panic!("{name} in {mode:?}"));
            assert!((f64::from(at[0]) - x).abs() < 0.5);
            assert!(at[1] < 30.);
        }
        // The first field is the waypoint, with its value in magenta below the name.
        let waypoint = all.iter().find(|(t, ..)| t == "KMMU").unwrap();
        assert_eq!(waypoint.3, Ink::Magenta);
        assert!(waypoint.1[1] > 30.);
        // COM frequencies in the left box.
        let com = geometry.rect(E::ComBox);
        for text in ["COM 1", "STBY"] {
            assert!(
                all.iter().any(|(t, at, ..)| t == text && inside(com, *at)),
                "{text}"
            );
        }
        // Knob hints in the status bar: the left knob's at the left, the right knob's at the right.
        let bar = geometry.rect(E::StatusBar);
        let hint = |t: &str| {
            all.iter()
                .find(|(l, ..)| l.contains(t))
                .unwrap_or_else(|| panic!("{t}"))
        };
        assert!(inside(bar, hint("Heading").1) && hint("Heading").1[0] < 200.);
        assert!(inside(bar, hint("Baro").1) && hint("Baro").1[0] > 1000.);
        // OAT in the middle.
        assert!(
            all.iter()
                .any(|(t, at, ..)| t.starts_with("OAT") && inside(bar, *at))
        );
    }
    let split = MODEL.pfd_geometry(PfdMode::Split);
    let d = touch_pfd::bars(&g, input, &split, Typography::default(), [("", ""), ("", "")]);
    assert!(
        labels(&d)
            .iter()
            .any(|(t, at, ..)| t == "1200" || t.len() == 4 && inside(split.rect(E::TransponderBox), *at))
    );
    // Unused: keep the field list type visible to the compiler as used.
    let _ = DataField::Waypoint;
}

#[test]
fn the_vertical_data_bar_values_follow_the_gps_state() {
    let s = session();
    let mut g = flying(&s);
    let input = InstrumentInput::new(s.sim().systems(), 0.);
    let geometry = MODEL.pfd_geometry(PfdMode::Full);
    let d = touch_pfd::bars(&g, input, &geometry, Typography::default(), [("", ""), ("", "")]);
    let all = labels(&d);
    for expected in ["267°", "11.1NM", "00:04", "164KT", "265°"] {
        assert!(all.iter().any(|(t, ..)| t == expected), "{expected} in {all:?}");
    }
    // Minimum safe altitude and vertical speed required are not modelled: dashes, never a guess.
    assert!(all.iter().filter(|(t, ..)| t == "-----").count() == 2);
    // With no valid leg the bearing and distance read as blanks.
    g.gps.valid = false;
    g.gps.distance_valid = false;
    g.gps.destination = None;
    let d = touch_pfd::bars(&g, input, &geometry, Typography::default(), [("", ""), ("", "")]);
    let all = labels(&d);
    assert!(all.iter().any(|(t, ..)| t == "_____") && all.iter().any(|(t, ..)| t == "__._NM"));
}

#[test]
fn a_skin_palette_and_typography_reach_the_instrument_faces_and_fall_back_when_absent() {
    let manifest = |instruments: serde_json::Value| SkinContents {
        manifest: SkinPackManifest::from_json(
            &json!({"kind":"skin_pack","format_version":1,"id":"painted","version":"1",
                "display_name":"Painted","licence":"Apache-2.0","display_model":"landscape_touch_efis",
                "instruments":instruments})
            .to_string(),
        )
        .unwrap(),
        assets: Default::default(),
    };
    // No skin, or a skin that says nothing about instruments: the built-in colours and sizes.
    let (colors, typography) = touch_pfd::face_style(None);
    assert!(colors.is_empty() && typography == Typography::default());
    let plain = manifest(json!({}));
    let (colors, typography) = touch_pfd::face_style(Some(&plain));
    assert!(colors.is_empty() && typography == Typography::default());
    // A skin sets some colours and one size: the rest stay built-in.
    let painted = manifest(json!({
        "palette":{"magenta":"#ff44cc","cyan":"#00ffff","tape_fill":"#102030","sky":"#2060c0"},
        "typography":{"tape_text":1.25}}));
    let (colors, typography) = touch_pfd::face_style(Some(&painted));
    let get = |ink| colors.iter().find(|(i, _)| *i == ink).map(|(_, c)| *c);
    assert_eq!(get(Ink::Magenta), Some(egui::Color32::from_rgb(0xff, 0x44, 0xcc)));
    assert_eq!(get(Ink::Cyan), Some(egui::Color32::from_rgb(0, 255, 255)));
    assert_eq!(get(Ink::Sky), Some(egui::Color32::from_rgb(0x20, 0x60, 0xc0)));
    assert!(
        get(Ink::TapeFill).is_some_and(|c| c.a() < 255),
        "tapes stay translucent"
    );
    assert_eq!(get(Ink::Green), None, "unset colours stay built-in");
    assert_eq!(typography.tape, 1.25);
    assert_eq!(
        (typography.boxes, typography.data_bar, typography.annunciation),
        (1., 1., 1.)
    );
    // Typography scales the type: the airspeed number on the tape grows with it.
    let s = session();
    let g = flying(&s);
    let input = InstrumentInput::new(s.sim().systems(), 0.);
    let geometry = MODEL.pfd_geometry(PfdMode::Full);
    let size = |typography| {
        let d = touch_pfd::panel(
            &g,
            input,
            &geometry,
            Options {
                typography,
                reversion: false,
            },
        );
        labels(&d)
            .into_iter()
            .find(|(t, at, ..)| t == "150" && inside(geometry.rect(E::AirspeedPointer), *at))
            .unwrap()
            .2
    };
    let big = size(typography);
    let normal = size(Typography::default());
    assert!((big / normal - 1.25).abs() < 1e-4, "{big} vs {normal}");
    // The palette reaches the painted shapes: a label in the magenta role takes the skin's colour.
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let paint = |rgb: &[(Ink, egui::Color32)]| {
        let mut d = DrawList {
            size: [100., 100.],
            items: Vec::new(),
            indicated: 0.,
            valid: true,
        };
        d.items.push(DrawItem::Label {
            at: [50., 50.],
            text: "X".into(),
            size: 20.,
            ink: Ink::Magenta,
        });
        let out = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let area = Rect::from_min_size(egui::pos2(0., 0.), egui::vec2(100., 100.));
                crate::glass::paint_fill_with(ui, area, d.clone(), rgb);
            });
        });
        out.shapes
            .into_iter()
            .find_map(|s| match s.shape {
                egui::Shape::Text(t) => t.galley.job.sections.first().map(|sec| sec.format.color),
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(paint(&[]), theme::FACE_MAGENTA);
    assert_eq!(paint(&colors), egui::Color32::from_rgb(0xff, 0x44, 0xcc));
}

// ---- Screen layout and touch boxes ----

pub(crate) fn isolated(s: &Session, id: &str) -> flightcraft_aero::modern::EquipmentPanel {
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
        .retain(|e| e.id == id && matches!(e.element, PanelElement::TouchDisplay { .. }));
    for e in &mut p.elements {
        e.position = [0., 0.];
        e.size = [w, h];
    }
    p
}

pub(crate) fn frame(
    ctx: &egui::Context,
    view: &mut PanelUi,
    panel: &flightcraft_aero::modern::EquipmentPanel,
    s: &Session,
    size: egui::Vec2,
    events: Vec<egui::Event>,
    time: f64,
) {
    let state = s.glass_state();
    let _ = ctx.run(
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
                        InstrumentInput::new(s.sim().systems(), 0.).with_glass(&state),
                        &Registry::default(),
                    );
                });
        },
    );
}

pub(crate) fn tap(
    ctx: &egui::Context,
    view: &mut PanelUi,
    panel: &flightcraft_aero::modern::EquipmentPanel,
    s: &Session,
    size: egui::Vec2,
    at: egui::Pos2,
    t: &mut f64,
) {
    let button = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    frame(ctx, view, panel, s, size, vec![egui::Event::PointerMoved(at)], *t);
    frame(ctx, view, panel, s, size, vec![button(true)], *t + 0.1);
    frame(ctx, view, panel, s, size, vec![button(false)], *t + 0.2);
    *t += 1.;
}

pub(crate) fn actions(view: &mut PanelUi) -> Vec<DisplayAction> {
    std::mem::take(&mut view.commands)
        .into_iter()
        .filter_map(|c| match c {
            EquipmentSetting::Display { action, .. } => Some(action),
            _ => None,
        })
        .collect()
}

pub(crate) fn screen_of(panel: &flightcraft_aero::modern::EquipmentPanel, size: egui::Vec2) -> Rect {
    let cell = crate::panel_preview::equipment_rect(panel, Rect::from_min_size(egui::Pos2::ZERO, size));
    efis::screen_rect(efis::bezel_rect(cell, MODEL), MODEL)
}

fn set(s: &mut Session, id: &str, action: serde_json::Value) {
    s.execute(
        "equipment.set",
        json!({"setting":{"kind":"display","id":id,"action":action}}),
    )
    .unwrap();
}

#[test]
fn the_data_bar_user_field_shows_dashes_the_clock_or_the_flight_timer() {
    use flightcraft_control::{ClockReadout, DataBarField};
    let s = session();
    let input = InstrumentInput::new(s.sim().systems(), 0.);
    let geometry = MODEL.pfd_geometry(PfdMode::Full);
    let bar = geometry.rect(E::DataBar);
    let slot_x = geometry
        .data_fields
        .iter()
        .find(|(f, _)| *f == DataField::SafeAltitude)
        .unwrap()
        .1 as f32;
    let draw =
        |g: &GlassState| touch_pfd::bars(g, input, &geometry, Typography::default(), [("", ""), ("", "")]);
    let in_slot = |d: &DrawList, text: &str| {
        labels(d)
            .into_iter()
            .find(|(t, at, ..)| t == text && (at[0] - slot_x).abs() < 1. && at[1] < bar[3] as f32)
    };
    let mut g = flying(&s);
    g.clock = ClockReadout {
        seconds: 12. * 3600. + 34. * 60. + 56.,
        label: "LCL".into(),
        timer_s: 754.,
        timer_running: true,
        timer_expired: false,
        timer_down: false,
    };
    // Default: the minimum safe altitude slot, dashes.
    let d = draw(&g);
    assert!(in_slot(&d, "MSA").is_some() && in_slot(&d, "-----").is_some());
    // Clock: its label and HH:MM:SS.
    g.settings.data_bar_field = DataBarField::Clock;
    let d = draw(&g);
    assert!(in_slot(&d, "LCL").is_some() && in_slot(&d, "12:34:56").is_some());
    assert!(in_slot(&d, "MSA").is_none());
    // Timer: TMR and MM:SS, magenta while counting, amber once a count-down has expired.
    g.settings.data_bar_field = DataBarField::Timer;
    let d = draw(&g);
    assert_eq!(in_slot(&d, "12:34").unwrap().3, Ink::Magenta);
    assert!(in_slot(&d, "TMR").is_some());
    g.clock.timer_down = true;
    g.clock.timer_expired = true;
    g.clock.timer_s = 0.;
    let d = draw(&g);
    assert_eq!(in_slot(&d, "00:00").unwrap().3, Ink::Yellow);
    // An hour or more reads H:MM:SS.
    g.clock.timer_s = 3725.;
    g.clock.timer_expired = false;
    assert!(in_slot(&draw(&g), "1:02:05").is_some());
}

#[test]
fn touching_the_user_field_cycles_it_and_the_timer_half_starts_and_stops_the_timer() {
    use flightcraft_control::{DataBarField, GlassSetting, TimerAction};
    let mut s = session();
    let panel = isolated(&s, "left_display");
    let size = egui::vec2(1088., 780.);
    let screen = screen_of(&panel, size);
    let geometry = MODEL.pfd_geometry(PfdMode::Full);
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let mut view = PanelUi::default();
    frame(&ctx, &mut view, &panel, &s, size, Vec::new(), 0.0);
    let fx = geometry
        .data_fields
        .iter()
        .find(|(f, _)| *f == DataField::SafeAltitude)
        .unwrap()
        .1;
    let mut t = 1.;
    let glass = |view: &mut PanelUi| std::mem::take(&mut view.avionics);
    // Left of the field: the choice advances.
    let at = efis::native_rect(screen, [fx - 30., 10., 20., 40.]).center();
    tap(&ctx, &mut view, &panel, &s, size, at, &mut t);
    let sent = glass(&mut view);
    assert!(
        sent.iter().any(|a| matches!(
            a,
            flightcraft_control::AvionicsSetting::Glass {
                setting: GlassSetting::DataBarField {
                    field: DataBarField::Clock
                }
            }
        )),
        "{sent:?}"
    );
    // Nothing to start or stop until the timer is the field shown.
    let at = efis::native_rect(screen, [fx + 20., 10., 20., 40.]).center();
    tap(&ctx, &mut view, &panel, &s, size, at, &mut t);
    assert!(!glass(&mut view).iter().any(|a| matches!(
        a,
        flightcraft_control::AvionicsSetting::Glass {
            setting: GlassSetting::Timer { .. }
        }
    )));
    // With the timer shown, its right half toggles it.
    s.set_glass(GlassSetting::DataBarField {
        field: DataBarField::Timer,
    })
    .unwrap();
    frame(&ctx, &mut view, &panel, &s, size, Vec::new(), t);
    glass(&mut view);
    tap(&ctx, &mut view, &panel, &s, size, at, &mut t);
    assert!(glass(&mut view).iter().any(|a| matches!(
        a,
        flightcraft_control::AvionicsSetting::Glass {
            setting: GlassSetting::Timer {
                operation: TimerAction::Toggle {}
            }
        }
    )));
}

#[test]
fn minimums_bug_box_and_annunciation_follow_the_altitude() {
    let s = session();
    let input = InstrumentInput::new(s.sim().systems(), 0.);
    let geometry = MODEL.pfd_geometry(PfdMode::Full);
    let tape = geometry.rect(E::AltitudeTape);
    let draw = |altitude_ft: f64, minimums_ft: Option<f64>| {
        let mut g = flying(&s);
        g.air_data.altitude_msl_m = altitude_ft * 0.3048;
        g.settings.minimums_m = minimums_ft.map(|f| f * 0.3048);
        touch_pfd::panel(&g, input, &geometry, options())
    };
    let amber_items = |d: &DrawList| {
        d.flat_items()
            .into_iter()
            .filter(|i| matches!(i, DrawItem::Polygon { ink: Ink::Yellow, .. }))
            .count()
    };
    // Not set: a dashed box and no annunciation.
    let d = draw(1500., None);
    assert!(drawn_in(&d, geometry.rect(E::MinimumsBox), "MIN ----"));
    assert!(!drawn_in(&d, tape, "MINIMUMS"));
    // Set and 500 ft above: the box and the bug, no annunciation.
    let d = draw(1500., Some(1000.));
    assert!(
        drawn_in(&d, geometry.rect(E::MinimumsBox), "MIN 1,000")
            || drawn_in(&d, geometry.rect(E::MinimumsBox), "MIN 1000")
    );
    assert!(!drawn_in(&d, tape, "MINIMUMS"));
    assert!(amber_items(&d) >= 1, "the minimums bug is amber");
    // Within 100 ft above, then at or below: the annunciation, amber, in the tape.
    for (altitude, ink) in [(1080., Ink::Yellow), (1000., Ink::BoxFill), (940., Ink::BoxFill)] {
        let d = draw(altitude, Some(1000.));
        let text = labels(&d)
            .into_iter()
            .find(|(t, at, ..)| t == "MINIMUMS" && inside(tape, *at))
            .unwrap_or_else(|| panic!("no annunciation at {altitude} ft"));
        assert_eq!(text.3, ink, "{altitude}");
    }
}

#[test]
fn the_pfd_boxes_are_touch_targets_at_the_models_positions() {
    let s = session();
    let panel = isolated(&s, "left_display");
    let size = egui::vec2(1088., 780.);
    let screen = screen_of(&panel, size);
    let geometry = MODEL.pfd_geometry(PfdMode::Full);
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let mut view = PanelUi::default();
    frame(&ctx, &mut view, &panel, &s, size, Vec::new(), 0.0);
    let mut t = 1.;
    for (element, want) in [
        (E::SelectedAltitudeBox, Overlay::Keypad(EntryTarget::Altitude)),
        (E::BaroBox, Overlay::Keypad(EntryTarget::Baro)),
        (E::MinimumsBox, Overlay::Keypad(EntryTarget::Minimums)),
        (E::SelectedHeadingBox, Overlay::Keypad(EntryTarget::Heading)),
        (E::SelectedCourseBox, Overlay::Keypad(EntryTarget::Course)),
    ] {
        let at = efis::native_rect(screen, geometry.screen_rect(element)).center();
        tap(&ctx, &mut view, &panel, &s, size, at, &mut t);
        let got = actions(&mut view);
        assert!(
            got.iter()
                .any(|a| matches!(a, DisplayAction::OpenOverlay { overlay } if *overlay == want)),
            "{element:?}: {got:?}"
        );
    }
    // The radio box: the active half transfers, the standby half opens the COM window.
    let com = efis::native_rect(screen, geometry.screen_rect(E::ComBox));
    tap(
        &ctx,
        &mut view,
        &panel,
        &s,
        size,
        egui::pos2(com.left() + com.width() * 0.25, com.center().y),
        &mut t,
    );
    assert!(actions(&mut view).iter().any(|a| matches!(
        a,
        DisplayAction::Transfer {
            band: flightcraft_control::RadioBand::Com
        }
    )));
    tap(
        &ctx,
        &mut view,
        &panel,
        &s,
        size,
        egui::pos2(com.left() + com.width() * 0.75, com.center().y),
        &mut t,
    );
    assert!(actions(&mut view).iter().any(|a| matches!(
        a,
        DisplayAction::OpenOverlay {
            overlay: Overlay::ComWindow
        }
    )));
    // The split button sits at the right end of the data bar.
    let split = efis::native_rect(screen, geometry.screen_rect(E::SplitButton));
    assert!(
        split.right() > screen.right() - screen.width() * 0.06
            && split.bottom() < screen.top() + screen.height() * 0.08
    );
    tap(&ctx, &mut view, &panel, &s, size, split.center(), &mut t);
    assert!(
        actions(&mut view)
            .iter()
            .any(|a| matches!(a, DisplayAction::ToggleSplit {}))
    );
}

#[test]
fn split_screen_regions_follow_the_figure_and_stay_between_the_bars() {
    let screen = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280., 768.));
    let content = efis::content_rect(screen);
    assert!((content.top() - 60.).abs() < 0.5 && (content.bottom() - 744.).abs() < 0.5);
    assert_eq!(efis::mfd_rect(screen, ScreenMode::FullPfd), None);
    assert_eq!(efis::mfd_rect(screen, ScreenMode::FullMfd), Some(content));
    let pane = efis::mfd_rect(screen, ScreenMode::Split).unwrap();
    assert!(
        (pane.left() / 1280. - 0.563).abs() < 0.002,
        "pane starts at 56.3 percent"
    );
    assert_eq!((pane.top(), pane.bottom()), (content.top(), content.bottom()));
    // The page bar is the pane's bottom strip, above the status bar; nearest tabs are its top.
    let bar = efis::page_bar(screen, ScreenMode::Split).unwrap();
    assert_eq!(bar.bottom(), pane.bottom());
    let tabs = efis::nearest_tabs(screen, ScreenMode::Split).unwrap();
    assert_eq!(tabs.top(), pane.top());
    let area = efis::pane_content(screen, ScreenMode::Split, MfdPage::Nearest).unwrap();
    assert_eq!((area.top(), area.bottom()), (tabs.bottom(), bar.top()));
    let map = efis::pane_content(screen, ScreenMode::Split, MfdPage::Map).unwrap();
    assert_eq!((map.top(), map.bottom()), (pane.top(), bar.top()));
}

#[test]
fn the_keyboard_types_into_a_keypad_and_enter_sends_only_a_valid_entry() {
    let mut s = session();
    let panel = isolated(&s, "left_display");
    let size = egui::vec2(1088., 780.);
    set(
        &mut s,
        "left_display",
        json!({"kind":"open_overlay","overlay":{"keypad":"altitude"}}),
    );
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let mut view = PanelUi::default();
    let key = |key, pressed| egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Default::default(),
    };
    frame(&ctx, &mut view, &panel, &s, size, Vec::new(), 0.0);
    // Letters are ignored on a numeric pad; "99" is too low an altitude, so Enter sends nothing.
    frame(
        &ctx,
        &mut view,
        &panel,
        &s,
        size,
        vec![egui::Event::Text("x99".into())],
        0.1,
    );
    frame(
        &ctx,
        &mut view,
        &panel,
        &s,
        size,
        vec![key(egui::Key::Enter, true)],
        0.2,
    );
    assert!(
        !actions(&mut view)
            .iter()
            .any(|a| matches!(a, DisplayAction::Enter { .. })),
        "99 ft is not a valid altitude"
    );
    // Backspace and retype: 3500, then Enter.
    frame(
        &ctx,
        &mut view,
        &panel,
        &s,
        size,
        vec![key(egui::Key::Backspace, true)],
        0.3,
    );
    frame(
        &ctx,
        &mut view,
        &panel,
        &s,
        size,
        vec![key(egui::Key::Backspace, true)],
        0.4,
    );
    frame(
        &ctx,
        &mut view,
        &panel,
        &s,
        size,
        vec![egui::Event::Text("3500".into())],
        0.5,
    );
    frame(
        &ctx,
        &mut view,
        &panel,
        &s,
        size,
        vec![key(egui::Key::Enter, true)],
        0.6,
    );
    let got = actions(&mut view);
    assert!(
        got.iter().any(|a| matches!(a,
            DisplayAction::Enter { target: EntryTarget::Altitude, text } if text == "3500")),
        "{got:?}"
    );
}

#[test]
fn every_keypad_and_window_offers_its_keys_and_shortcuts() {
    let mut s = session();
    let panel = isolated(&s, "left_display");
    let size = egui::vec2(1088., 780.);
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let overlays = [
        (
            json!({"keypad":"baro"}),
            vec![".", "ENTER", "Set To Standard", "Set For Field"],
        ),
        (
            json!({"keypad":"heading"}),
            vec!["9", "BKSP", "CLR", "Set To Current"],
        ),
        (json!({"keypad":"altitude"}), vec!["0", "Set To Current"]),
        (json!({"keypad":"course"}), vec!["5", "Set To Current"]),
        (json!("com_window"), vec!["COM1", "XFER", "."]),
        (json!("nav_window"), vec!["NAV1", "XFER"]),
        (
            json!("transponder_window"),
            vec!["TRANSPONDER", "STBY", "ALT", "ON"],
        ),
    ];
    for (overlay, want) in overlays {
        set(
            &mut s,
            "left_display",
            json!({"kind":"open_overlay","overlay":overlay}),
        );
        let mut view = PanelUi::default();
        let state = s.glass_state();
        let out = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
                time: Some(0.),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        view.show(
                            ui,
                            &panel,
                            InstrumentInput::new(s.sim().systems(), 0.).with_glass(&state),
                            &Registry::default(),
                        );
                    });
            },
        );
        let text: Vec<String> = out
            .shapes
            .into_iter()
            .filter_map(|s| match s.shape {
                egui::Shape::Text(t) => Some(t.galley.job.text.clone()),
                _ => None,
            })
            .collect();
        for w in want {
            assert!(text.iter().any(|t| t == w), "{overlay}: {w} in {text:?}");
        }
    }
    // The waypoint pad has the alphabet.
    set(&mut s, "left_display", json!({"kind":"close_overlay"}));
    s.execute(
        "equipment.set",
        json!({"setting":{"kind":"display","id":"left_display","action":{"kind":"key","key":"direct"}}}),
    )
    .unwrap();
    let mut view = PanelUi::default();
    let state = s.glass_state();
    let out = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
            time: Some(0.),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    view.show(
                        ui,
                        &panel,
                        InstrumentInput::new(s.sim().systems(), 0.).with_glass(&state),
                        &Registry::default(),
                    );
                });
        },
    );
    let letters: Vec<String> = out
        .shapes
        .into_iter()
        .filter_map(|s| match s.shape {
            egui::Shape::Text(t) => Some(t.galley.job.text.clone()),
            _ => None,
        })
        .collect();
    for c in ["A", "M", "Z", "0", "9"] {
        assert!(letters.iter().any(|t| t == c), "{c}");
    }
}

#[test]
fn nearest_tabs_and_map_zoom_buttons_send_their_actions() {
    let mut s = session();
    let panel = isolated(&s, "left_display");
    let size = egui::vec2(1088., 780.);
    let screen = screen_of(&panel, size);
    set(&mut s, "left_display", json!({"kind":"layout","layout":"mfd"}));
    set(
        &mut s,
        "left_display",
        json!({"kind":"mfd_page","page":"nearest"}),
    );
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let mut view = PanelUi::default();
    frame(&ctx, &mut view, &panel, &s, size, Vec::new(), 0.0);
    let mut t = 1.;
    let tabs = efis::nearest_tabs(screen, ScreenMode::FullMfd).unwrap();
    let width = tabs.width() / 5.;
    for (i, page) in NearestPage::ALL.iter().enumerate() {
        let at = egui::pos2(tabs.left() + width * (i as f32 + 0.5), tabs.center().y);
        tap(&ctx, &mut view, &panel, &s, size, at, &mut t);
        assert!(
            actions(&mut view)
                .iter()
                .any(|a| matches!(a, DisplayAction::NearestPage { page: p } if p == page)),
            "{page:?}"
        );
    }
    // Back on the map page the zoom buttons sit at the right edge of the pane.
    set(&mut s, "left_display", json!({"kind":"mfd_page","page":"map"}));
    frame(&ctx, &mut view, &panel, &s, size, Vec::new(), 20.0);
    t = 21.;
    let area = efis::pane_content(screen, ScreenMode::FullMfd, MfdPage::Map).unwrap();
    let side = screen.height() * 0.075;
    let map_right = area.left() + area.width() * 0.64;
    let plus = egui::pos2(map_right - side * 0.5 - 6., area.top() + 8. + side * 0.5);
    let minus = egui::pos2(plus.x, plus.y + side + 4.);
    tap(&ctx, &mut view, &panel, &s, size, plus, &mut t);
    assert!(
        actions(&mut view)
            .iter()
            .any(|a| matches!(a, DisplayAction::MapZoom { steps: 1 }))
    );
    tap(&ctx, &mut view, &panel, &s, size, minus, &mut t);
    assert!(
        actions(&mut view)
            .iter()
            .any(|a| matches!(a, DisplayAction::MapZoom { steps: -1 }))
    );
    // The page name opens the Select Page menu.
    let bar = efis::page_bar(screen, ScreenMode::FullMfd).unwrap();
    tap(
        &ctx,
        &mut view,
        &panel,
        &s,
        size,
        egui::pos2(bar.left() + bar.width() * 0.1, bar.center().y),
        &mut t,
    );
    assert!(actions(&mut view).iter().any(|a| matches!(
        a,
        DisplayAction::OpenOverlay {
            overlay: Overlay::SelectPage
        }
    )));
}

#[test]
fn nearest_tables_show_their_lists_and_the_empty_text() {
    let s = session();
    let mut g = flying(&s);
    let table = |g: &GlassState, page| {
        let mut d = DrawList {
            size: [500., 380.],
            items: Vec::new(),
            indicated: 0.,
            valid: true,
        };
        crate::glass::nearest_table_for_tests(&mut d, g, page);
        labels(&d).into_iter().map(|(t, ..)| t).collect::<Vec<_>>()
    };
    let empty = table(&g, NearestPage::Vors);
    assert!(empty.iter().any(|t| t == "None Within 200 NM"), "{empty:?}");
    assert!(
        table(&g, NearestPage::Airspace)
            .iter()
            .any(|t| t == "No Immediate Airspace")
    );
    use flightcraft_control::touch_nearest::{NearestAirspace, NearestFacility};
    let facility = |ident: &str, nm, khz| NearestFacility {
        ident: ident.into(),
        name: ident.into(),
        bearing_rad: 1.0,
        distance_nm: nm,
        frequency_khz: khz,
        detail: "VOR".into(),
    };
    g.nearest_vors = vec![facility("JFK", 12.0, Some(115_900.))];
    g.nearest_ndbs = vec![facility("AB", 3.0, Some(350.))];
    g.nearest_intersections = vec![facility("ROBER", 8.0, None)];
    g.nearest_airspace = vec![
        NearestAirspace {
            name: "NEW YORK".into(),
            class: Some("B".into()),
            entry_s: 0,
        },
        NearestAirspace {
            name: "TETERBORO".into(),
            class: Some("D".into()),
            entry_s: 150,
        },
    ];
    let vors = table(&g, NearestPage::Vors);
    assert!(
        vors.iter().any(|t| t.starts_with("JFK") && t.ends_with("115.90")),
        "{vors:?}"
    );
    assert!(
        table(&g, NearestPage::Ndbs)
            .iter()
            .any(|t| t.starts_with("AB") && t.ends_with("350"))
    );
    assert!(
        table(&g, NearestPage::Intersections)
            .iter()
            .any(|t| t.starts_with("ROBER"))
    );
    let air = table(&g, NearestPage::Airspace);
    assert!(air.iter().any(|t| t == "NEW YORK Class B Inside"), "{air:?}");
    assert!(air.iter().any(|t| t == "TETERBORO Class D Entry 2:30"), "{air:?}");
    // The page names are the titles.
    assert!(vors.iter().any(|t| t == "NEAREST VORS"));
    let _: PfdGeometry = MODEL.pfd_geometry(PfdMode::Full);
}

/// Close-ups of the Sling pack's left PFD (full) and right display (split), at 1920 x 1080 with the
/// public skin. Set FLIGHTCRAFT_LAYOUT_PACK to the pack and FLIGHTCRAFT_SKINS_DIR to the skins.
#[test]
#[ignore = "writes close-up previews under out/layout/after"]
fn render_efis_closeups() {
    let Some(pack) = std::env::var_os("FLIGHTCRAFT_LAYOUT_PACK") else {
        return;
    };
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/layout/after");
    std::fs::create_dir_all(&out).unwrap();
    let mut s = crate::panel_fidelity_tests::geographic_session();
    s.install_pack(&flightcraft_control::read_pack(std::path::Path::new(&pack)).unwrap())
        .unwrap();
    s.execute(
        "aircraft.load",
        json!({"source":{"kind":"installed","id":"sling2"},"reset":true}),
    )
    .unwrap();
    s.execute("sim.reset", json!({"world":"panel-preview","start":{"kind":"air","height_m":1200.,"airspeed_mps":45.,"heading_rad":0.5}})).unwrap();
    s.execute(
        "assist.set",
        json!({"altitude_m":1200.,"airspeed_mps":45.,"heading_rad":0.5}),
    )
    .unwrap();
    s.execute("sim.step", json!({"steps":7200})).unwrap();
    s.execute("assist.off", json!({})).unwrap();
    if let Some(dir) = std::env::var_os("FLIGHTCRAFT_SKINS_DIR") {
        assert!(s.load_skins_directory(std::path::Path::new(&dir)).is_empty());
    }
    let ap = |s: &mut Session, setting: serde_json::Value| {
        s.execute(
            "avionics.set",
            json!({"setting":{"kind":"autopilot","setting":setting}}),
        )
        .unwrap();
    };
    ap(&mut s, json!({"kind":"flight_director","on":true}));
    ap(&mut s, json!({"kind":"engage","on":true}));
    ap(&mut s, json!({"kind":"lateral","mode":"hdg"}));
    ap(&mut s, json!({"kind":"vertical","mode":"alt"}));
    s.execute(
        "avionics.set",
        json!({"setting":{"kind":"heading_bug","degrees":55.0}}),
    )
    .unwrap();
    s.execute("sim.step", json!({"steps":240})).unwrap();
    // Make the right display a split screen on the map page.
    set(&mut s, "right_display", json!({"kind":"layout","layout":"split"}));
    set(&mut s, "right_display", json!({"kind":"mfd_page","page":"map"}));
    let config = s.aircraft_configuration().unwrap().clone();
    let owned_skins: Vec<SkinContents> = s.skins().cloned().collect();
    let skins: Vec<&SkinContents> = owned_skins.iter().collect();
    let state = s.glass_state();
    for (id, name) in [
        ("left_display", "sling-left-pfd-full.png"),
        ("right_display", "sling-right-split.png"),
    ] {
        let mut panel_config = config.panel.clone();
        panel_config.equipment = Some(isolated(&s, id));
        let mut panel = InstrumentPanel::default();
        let image = crate::panel_preview::rasterize(1920, 1080, |ui| {
            panel
                .show(
                    ui,
                    &panel_config,
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
        })
        .unwrap();
        image.save(out.join(name)).unwrap();
    }
    // The same PFD with a seeded alert (the ballistic chute pin pulled), flaps at 20 degrees, and some trim:
    // the CAS window and the flap and trim indicators.
    s.sim_mut()
        .set_systems(flightcraft_sim::systems::SystemSetting::Equipment {
            setting: flightcraft_sim::modern::ModernSetting::ParachutePin { inserted: false },
        })
        .unwrap();
    s.sim_mut()
        .set_systems(flightcraft_sim::systems::SystemSetting::Flaps { value: 2. / 3. })
        .unwrap();
    s.sim_mut()
        .set_systems(flightcraft_sim::systems::SystemSetting::Trim { value: 0.12 })
        .unwrap();
    s.execute("sim.step", json!({"steps":4})).unwrap();
    let state = s.glass_state();
    let mut panel_config = config.panel.clone();
    panel_config.equipment = Some(isolated(&s, "left_display"));
    let mut panel = InstrumentPanel::default();
    let image = crate::panel_preview::rasterize(1920, 1080, |ui| {
        panel
            .show(
                ui,
                &panel_config,
                InstrumentInput::new(s.sim().systems(), 0.)
                    .with_pilot(s.sim().controls())
                    .with_aircraft(&config)
                    .with_skins(&skins)
                    .with_glass(&state)
                    .with_autopilot(s.sim().installed_autopilot()),
                s.flight_generation(),
            )
            .unwrap();
        panel.show_shared_hosts(ui, &mut s);
    })
    .unwrap();
    image.save(out.join("sling-left-pfd-cas.png")).unwrap();
}
