//! G3X Touch features a student meets on the landscape touch EFIS: the trim and flap position
//! indicators, the CAS window, and touch flight-plan editing. Placement and behaviour follow the
//! pilot's guide (190-01754-00 Rev. Y, pages 81, 106-107, 192-193).
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::efis_pfd_tests::{flying, labels, options, session};
use crate::glass::touch_pfd;
use crate::instruments::*;
use eframe::egui::{self, Rect};
use flightcraft_aero::display_model::{DisplayModel, PfdElement as E, PfdMode};
use flightcraft_control::touch_fpl::FplEdit;
use flightcraft_control::{DisplayAction, EquipmentSetting, Session};
use flightcraft_sim::modern::ModernSetting;
use flightcraft_sim::systems::SystemSetting;
use serde_json::json;

const MODEL: DisplayModel = DisplayModel::LandscapeTouchEfis;

fn inside([x, y, w, h]: [f64; 4], at: [f32; 2]) -> bool {
    let (x, y, w, h) = (x as f32, y as f32, w as f32, h as f32);
    at[0] >= x - 0.5 && at[0] <= x + w + 0.5 && at[1] >= y - 0.5 && at[1] <= y + h + 0.5
}

/// Draw the full PFD for the session's present state, with the pilot's controls supplied.
fn draw(s: &Session, time_s: f64, with_pilot: bool) -> DrawList {
    let g = flying(s);
    let mut input = InstrumentInput::new(s.sim().systems(), time_s);
    if with_pilot {
        input = input.with_pilot(s.sim().controls());
    }
    touch_pfd::panel(&g, input, &MODEL.pfd_geometry(PfdMode::Full), options())
}

/// The vertical position of the white pointer triangle inside an indicator, if drawn.
fn pointer_y(d: &DrawList, rect: [f64; 4]) -> Option<f32> {
    d.flat_items().into_iter().find_map(|i| match i {
        DrawItem::Polygon {
            points,
            ink: Ink::White,
        } if points.len() == 3 && inside(rect, points[0]) => Some(points[0][1]),
        _ => None,
    })
}

fn set(s: &mut Session, setting: SystemSetting) {
    s.sim_mut().set_systems(setting).unwrap();
}

#[test]
fn the_flap_and_trim_indicators_follow_the_simulation() {
    let mut s = session();
    let g = MODEL.pfd_geometry(PfdMode::Full);
    let (flap, trim) = (g.rect(E::FlapIndicator), g.rect(E::TrimIndicator));
    // A host that supplies no pilot controls draws neither indicator.
    let none = draw(&s, 0., false);
    assert!(pointer_y(&none, flap).is_none() && pointer_y(&none, trim).is_none());
    // Flaps: the pointer moves down the scale as the flaps extend, 0 at the top.
    let mut flap_ys = Vec::new();
    for position in [0., 1. / 3., 2. / 3., 1.] {
        set(&mut s, SystemSetting::Flaps { value: position });
        let d = draw(&s, 0., true);
        flap_ys.push(pointer_y(&d, flap).expect("flap pointer"));
        let l = labels(&d);
        assert!(l.iter().any(|(t, at, ..)| t == "0" && inside(flap, *at)));
    }
    assert!(flap_ys.windows(2).all(|w| w[1] > w[0] + 5.), "{flap_ys:?}");
    let f = |v: f64| v as f32;
    assert!(flap_ys[0] < f(flap[1] + flap[3] * 0.3) && flap_ys[3] > f(flap[1] + flap[3] * 0.7));
    // Elevator trim: DN at the top, UP at the bottom, neutral in the middle.
    let mut trim_ys = Vec::new();
    for value in [-0.3, 0., 0.3] {
        set(&mut s, SystemSetting::Trim { value });
        let d = draw(&s, 0., true);
        trim_ys.push(pointer_y(&d, trim).expect("trim pointer"));
        let l = labels(&d);
        for word in ["DN", "UP"] {
            assert!(
                l.iter().any(|(t, at, ..)| t == word && inside(trim, *at)),
                "{word}"
            );
        }
    }
    assert!(
        trim_ys[0] + 5. < trim_ys[1] && trim_ys[1] + 5. < trim_ys[2],
        "{trim_ys:?}"
    );
    let middle = (trim[1] + trim[3] * 0.5) as f32;
    assert!(
        (trim_ys[1] - middle).abs() < 4.,
        "neutral is mid-scale: {trim_ys:?}"
    );
    // The flap indicator sits left of the trim indicator, as in the guide's figure.
    assert!(flap[0] + flap[2] <= trim[0] + 0.5);
}

fn door(s: &mut Session, open: bool) {
    s.sim_mut()
        .set_systems(SystemSetting::Equipment {
            setting: ModernSetting::Door { open },
        })
        .unwrap();
    s.execute("sim.step", json!({"steps": 4})).unwrap();
}

#[test]
fn the_cas_window_shows_a_seeded_alert_in_its_colour_flashes_until_acknowledged() {
    let mut s = session();
    let g = MODEL.pfd_geometry(PfdMode::Full);
    let cas = g.rect(E::CasWindow);
    let shown = |d: &DrawList| {
        labels(d)
            .into_iter()
            .filter(|(_, at, ..)| inside(cas, *at))
            .map(|(t, _, _, ink)| (t, ink))
            .collect::<Vec<_>>()
    };
    // Cold and quiet: nothing in the window except what the cold aircraft already annunciates.
    let quiet = shown(&draw(&s, 0., true));
    assert!(!quiet.iter().any(|(t, _)| t == "DOOR OPEN"), "{quiet:?}");
    // A seeded failure: the door is not latched.
    door(&mut s, true);
    let lit = shown(&draw(&s, 0., true));
    assert!(
        lit.iter().any(|(t, ink)| t == "DOOR OPEN" && *ink == Ink::Yellow),
        "a caution is yellow: {lit:?}"
    );
    // Warnings come before cautions, and are red.
    let order: Vec<_> = lit.iter().map(|(_, ink)| *ink).collect();
    let first_caution = order.iter().position(|i| *i == Ink::Yellow).unwrap();
    assert!(order[..first_caution].iter().all(|i| *i == Ink::Red), "{order:?}");
    // Unacknowledged text flashes: absent in the second half of each half-second.
    let dark = shown(&draw(&s, 0.5, true));
    assert!(
        !dark.iter().any(|(t, _)| t == "DOOR OPEN"),
        "flash off phase: {dark:?}"
    );
    // Touching the window (or BACK) acknowledges: the text is then steady in both phases.
    s.execute(
        "equipment.set",
        json!({"setting":{"kind":"display","id":"left_display","action":{"kind":"acknowledge_cas"}}}),
    )
    .unwrap();
    for t in [0., 0.5] {
        let steady = shown(&draw(&s, t, true));
        assert!(
            steady.iter().any(|(t, _)| t == "DOOR OPEN"),
            "steady at {t}: {steady:?}"
        );
    }
    // The alert clears when its cause does.
    door(&mut s, false);
    assert!(!shown(&draw(&s, 0., true)).iter().any(|(t, _)| t == "DOOR OPEN"));
}

#[test]
fn the_cas_window_lists_autopilot_failures_and_marks_overflow() {
    use flightcraft_control::touch_cas::{self, CAS_WINDOW_LINES};
    use flightcraft_sim::equipment::EquipmentFailure as F;
    let w = touch_cas::window(None, &[F::RollServo, F::PitchServo, F::Trim, F::RollSensor]);
    assert_eq!(w.messages.len(), 4);
    assert!(w.overflow() && w.visible().len() == CAS_WINDOW_LINES);
}

fn run_page(
    ctx: &egui::Context,
    shared: &crate::efis::Shared,
    events: Vec<egui::Event>,
    time: f64,
) -> crate::efis::Output {
    let style = crate::efis::Style::built_in();
    let mut out = crate::efis::Output::default();
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 480.))),
            events,
            time: Some(time),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                crate::efis::flight_plan_page(
                    ui,
                    Rect::from_min_size(egui::pos2(10., 10.), egui::vec2(600., 360.)),
                    "right_display",
                    &style,
                    shared,
                    &mut out,
                );
            });
        },
    );
    out
}

fn shared(rows: &[&str], cursor: usize, active: usize) -> crate::efis::Shared {
    crate::efis::Shared {
        backup: false,
        traffic_shown: true,
        track_up: false,
        pressure_hpa: false,
        range_nm: 20.,
        com_active_khz: 121_500,
        com_standby_khz: 122_800,
        nav_active_khz: None,
        nav_standby_khz: None,
        xpdr_code: "1200".into(),
        xpdr_mode: flightcraft_sim::avionics::glass::TransponderMode::Alt,
        cas_active: false,
        cas_overflow: false,
        cas: Default::default(),
        ident_remaining_s: 0.,
        user_field: flightcraft_control::DataBarField::SafeAltitude,
        g_meter: false,
        pitch_offset_deg: 0.,
        afcs: None,
        bearing: [false, false],
        vspeeds: Default::default(),
        pack_vspeeds_kt: [Some(58.), Some(62.), Some(70.), Some(65.), Some(60.)],
        fpl: crate::efis::FplView {
            rows: rows
                .iter()
                .enumerate()
                .map(|(i, ident)| crate::efis::FplRow {
                    ident: (*ident).to_owned(),
                    bearing_deg: 90. + i as f32,
                    distance_nm: 10. * (i + 1) as f32,
                    constraint: (i == 1).then(|| "3000FT A".to_owned()),
                })
                .collect(),
            cursor,
            active,
        },
    }
}

fn click(
    ctx: &egui::Context,
    shared: &crate::efis::Shared,
    at: egui::Pos2,
    t: &mut f64,
) -> Vec<DisplayAction> {
    let button = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    run_page(ctx, shared, vec![egui::Event::PointerMoved(at)], *t);
    run_page(ctx, shared, vec![button(true)], *t + 0.1);
    let out = run_page(ctx, shared, vec![button(false)], *t + 0.2);
    *t += 1.;
    out.equipment
        .into_iter()
        .filter_map(|e| match e {
            EquipmentSetting::Display { action, .. } => Some(action),
            _ => None,
        })
        .collect()
}

#[test]
fn the_fpl_page_selects_a_waypoint_and_offers_activate_insert_and_remove() {
    let ctx = egui::Context::default();
    crate::theme::install(&ctx);
    let sh = shared(&["1N7", "KMMU", "KTEB"], 1, 0);
    let mut t = 0.;
    run_page(&ctx, &sh, Vec::new(), t);
    // Layout: rows from y = 10 + 8.5% of 360 = 40.6, 30.6 high; buttons along the bottom.
    let row = |i: usize| egui::pos2(300., 10. + 360. * 0.085 + 360. * 0.085 * (i as f32 + 0.5));
    // Two rows of four buttons: list edits, then the altitude constraint of the selected waypoint.
    let button = |i: usize| {
        egui::pos2(
            10. + 600. * ((i % 4) as f32 + 0.5) / 4.,
            10. + 360. - 360. * 0.22 + 360. * 0.11 * ((i / 4) as f32 + 0.5),
        )
    };
    assert!(click(&ctx, &sh, row(2), &mut t).iter().any(|a| matches!(
        a,
        DisplayAction::FlightPlan {
            edit: FplEdit::Select { index: 2 }
        }
    )));
    let want = [
        (0, "activate"),
        (1, "insert before"),
        (2, "insert after"),
        (3, "remove"),
        (4, "altitude at"),
        (5, "altitude at or above"),
        (6, "altitude at or below"),
        (7, "altitude clear"),
    ];
    for (i, what) in want {
        let got = click(&ctx, &sh, button(i), &mut t);
        let ok = got.iter().any(|a| match (i, a) {
            (
                0,
                DisplayAction::FlightPlan {
                    edit: FplEdit::Activate {},
                },
            ) => true,
            (
                3,
                DisplayAction::FlightPlan {
                    edit: FplEdit::Remove {},
                },
            ) => true,
            (1, DisplayAction::OpenOverlay { overlay }) => {
                *overlay
                    == flightcraft_control::touch_efis::Overlay::Keypad(
                        flightcraft_control::touch_entry::EntryTarget::FplBefore,
                    )
            }
            (2, DisplayAction::OpenOverlay { overlay }) => {
                *overlay
                    == flightcraft_control::touch_efis::Overlay::Keypad(
                        flightcraft_control::touch_entry::EntryTarget::FplAfter,
                    )
            }
            (4..=6, DisplayAction::OpenOverlay { overlay }) => {
                use flightcraft_control::touch_entry::EntryTarget as T;
                *overlay
                    == flightcraft_control::touch_efis::Overlay::Keypad(match i {
                        4 => T::FplAltitudeAt,
                        5 => T::FplAltitudeAbove,
                        _ => T::FplAltitudeBelow,
                    })
            }
            (
                7,
                DisplayAction::FlightPlan {
                    edit: FplEdit::Altitude { constraint: None },
                },
            ) => true,
            _ => false,
        });
        assert!(ok, "{what}: {got:?}");
    }
    // An empty plan offers only Insert After.
    let empty = shared(&[], 0, 0);
    run_page(&ctx, &empty, Vec::new(), t);
    assert!(click(&ctx, &empty, button(0), &mut t).is_empty());
    assert!(click(&ctx, &empty, button(3), &mut t).is_empty());
    assert!(click(&ctx, &empty, button(5), &mut t).is_empty());
    assert!(!click(&ctx, &empty, button(2), &mut t).is_empty());
}

#[test]
fn flight_plan_edits_from_the_page_reach_the_existing_plan_through_the_session() {
    // The actions the page sends are the typed ones the session applies (see the control tests
    // for the list arithmetic): a Select sent to a plan with too few waypoints is refused.
    let mut s = session();
    let r = s.execute(
        "equipment.set",
        json!({"setting":{"kind":"display","id":"right_display","action":
            {"kind":"flight_plan","edit":{"kind":"select","index":3}}}}),
    );
    assert!(r.is_err());
}

#[test]
fn touching_the_cas_window_sends_the_acknowledge_action_only_while_an_alert_is_active() {
    use crate::efis_pfd_tests::{actions, frame, isolated, screen_of, tap};
    let mut s = session();
    let panel = isolated(&s, "left_display");
    let size = egui::vec2(1088., 780.);
    let screen = screen_of(&panel, size);
    let at = crate::efis::native_rect(
        screen,
        MODEL.pfd_geometry(PfdMode::Full).screen_rect(E::CasWindow),
    )
    .center();
    let ctx = egui::Context::default();
    crate::theme::install(&ctx);
    let mut view = crate::modern::PanelUi::default();
    frame(&ctx, &mut view, &panel, &s, size, Vec::new(), 0.0);
    let mut t = 1.;
    tap(&ctx, &mut view, &panel, &s, size, at, &mut t);
    assert!(
        !actions(&mut view)
            .iter()
            .any(|a| matches!(a, DisplayAction::AcknowledgeCas {}))
    );
    door(&mut s, true);
    tap(&ctx, &mut view, &panel, &s, size, at, &mut t);
    assert!(
        actions(&mut view)
            .iter()
            .any(|a| matches!(a, DisplayAction::AcknowledgeCas {}))
    );
}

#[test]
fn the_cas_page_lists_newest_first_and_a_touched_unacknowledged_row_sends_cas_acknowledge() {
    use flightcraft_control::GlassSetting;
    use flightcraft_control::touch_cas::{CasMessage, CasSeverity, CasWindow};
    let message = |text: &str, severity, acknowledged, sequence| CasMessage {
        severity,
        text: text.into(),
        acknowledged,
        sequence,
        annunciation: Some(0),
    };
    let mut sh = shared(&[], 0, 0);
    sh.cas = CasWindow {
        messages: vec![
            message("OLD CAUTION", CasSeverity::Caution, true, 1),
            message("NEW CAUTION", CasSeverity::Caution, false, 2),
            message("WARN", CasSeverity::Warning, false, 3),
        ],
        unacknowledged: true,
    };
    // Warnings first, then newest first within a severity.
    let order: Vec<_> = sh.cas.list().into_iter().map(|m| m.text).collect();
    assert_eq!(order, ["WARN", "NEW CAUTION", "OLD CAUTION"]);
    let ctx = egui::Context::default();
    crate::theme::install(&ctx);
    let style = crate::efis::Style::built_in();
    let area = Rect::from_min_size(egui::pos2(10., 10.), egui::vec2(600., 360.));
    let run = |events: Vec<egui::Event>, time: f64| {
        let mut out = crate::efis::Output::default();
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 480.))),
                events,
                time: Some(time),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| crate::efis::cas_page(ui, area, &style, &sh, &mut out));
            },
        );
        out
    };
    let tap = |row: usize, time: f64| {
        let at = egui::pos2(300., 10. + 360. * 0.085 + 360. * 0.085 * (row as f32 + 0.5));
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        run(vec![egui::Event::PointerMoved(at)], time);
        run(vec![button(true)], time + 0.1);
        run(vec![button(false)], time + 0.2).glass
    };
    run(Vec::new(), 0.);
    assert!(matches!(
        tap(0, 1.).as_slice(),
        [GlassSetting::CasAcknowledge { index: 0 }]
    ));
    assert!(matches!(
        tap(1, 3.).as_slice(),
        [GlassSetting::CasAcknowledge { index: 1 }]
    ));
    assert!(tap(2, 5.).is_empty(), "an acknowledged row sends nothing");
}

#[test]
fn the_status_bar_shows_ident_only_while_the_ident_flag_is_up() {
    assert_eq!(crate::glass::xpdr_label("4321", 12.), "4321 IDENT");
    assert_eq!(crate::glass::xpdr_label("4321", 0.), "4321");
}

#[test]
fn the_keyboard_ident_action_runs_the_ident_setting() {
    use crate::keymap::Action;
    assert!(Action::GLASS.contains(&Action::GlassIdent));
    let s = flightcraft_control::Session::new().unwrap();
    assert!(matches!(
        crate::glass::shortcut(Action::GlassIdent, &s.glass_state()),
        Some(flightcraft_control::GlassSetting::Ident {})
    ));
}
