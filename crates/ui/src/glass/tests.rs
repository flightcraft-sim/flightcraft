#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
#[cfg(test)]
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/region_data.rs"));
use super::*;
use eframe::egui::{self, Align2, FontId, Pos2, Rect};
use flightcraft_control::{GlassFailure, GlassSetting, Session};
pub(super) fn flight() -> Session {
    let mut s = Session::new().unwrap();
    s.execute(
        "aircraft.load",
        serde_json::json!({"source":{"kind":"installed","id":"corvid-glass"},"reset":true}),
    )
    .unwrap();
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/terrain/new-york");
    s.install_region(flightcraft_control::read_region(&directory).unwrap())
        .unwrap();
    s.execute("sim.reset",serde_json::json!({"world":"new-york","start":{"air":{"airport":"EWR","distance_m":10000.,"bearing_deg":75.,"height_m":909.,"airspeed_mps":50.,"heading_rad":0.}}})).unwrap();
    s.execute(
        "autopilot.set",
        serde_json::json!({"altitude_m":914.4,"heading_rad":0.,"airspeed_mps":50.}),
    )
    .unwrap();
    s.run(flightcraft_control::Step { steps: 5401 }).unwrap();
    s.set_glass(GlassSetting::DirectTo {
        airport: "KMMU".into(),
    })
    .unwrap();
    s
}
#[test]
fn tape_ticks_trends_and_bugs_share_scale() {
    assert_eq!(tape_y(100., 90., 128., 2.), 108.);
    assert_eq!(tape_y(3000., 2500., 128., 0.2), 28.);
    assert_eq!(tape_y(90., 90., 128., 2.), 128.);
}
#[test]
fn failure_flags_and_reversion_strip_are_visible() {
    require_region_data!();
    let mut s = flight();
    s.set_glass(GlassSetting::Failure {
        target: GlassFailure::Mfd,
        failed: true,
    })
    .unwrap();
    let g = s.glass_state();
    let input = InstrumentInput::new(s.sim().systems(), s.sim().state().time_s)
        .with_glass(&g)
        .with_aircraft(s.aircraft_configuration().unwrap());
    let d = pfd::draw(&g, input, false);
    assert!(
        d.items
            .iter()
            .any(|i| matches!(i,DrawItem::Label{text,..} if text.starts_with("RPM ")))
    );
    s.set_glass(GlassSetting::Failure {
        target: GlassFailure::Ahrs,
        failed: true,
    })
    .unwrap();
    let g = s.glass_state();
    let d = pfd::draw(
        &g,
        InstrumentInput::new(s.sim().systems(), 0.).with_glass(&g),
        false,
    );
    for expected in ["AHRS FAIL", "HDG FAIL"] {
        assert!(
            d.flat_items()
                .iter()
                .any(|i| matches!(i,DrawItem::Label{text,..} if text==expected))
        );
    }
}
#[test]
fn labels_fit_canvas_without_overlap_at_both_window_sizes() {
    require_region_data!();
    let s = flight();
    let mut state = s.glass_state();
    let ctx = egui::Context::default();
    crate::theme::install(&ctx);
    let _ = ctx.run(Default::default(), |ctx| {
        for (width, height) in [(1280., 800.), (1000., 640.)] {
            for compact in [false, true] {
                for reversion in [false, true] {
                    state.reversion = reversion;
                    for page in [
                        flightcraft_control::GlassPage::Map,
                        flightcraft_control::GlassPage::Nearest,
                        flightcraft_control::GlassPage::FlightPlan,
                    ] {
                        state.settings.page = page;
                        for id in ["glass_pfd", "glass_mfd"] {
                            let input = InstrumentInput::new(s.sim().systems(), 0.)
                                .with_glass(&state)
                                .with_aircraft(s.aircraft_configuration().unwrap());
                            let draw = create(id).unwrap().update(input);
                            let cell = Rect::from_min_size(
                                Pos2::ZERO,
                                egui::vec2(
                                    width * 0.42,
                                    crate::instruments::panel_height(height, compact) - 80.,
                                ),
                            );
                            let fit = crate::instruments::fit_size(cell, draw.size);
                            let [canvas_width, _] = draw.size;
                            let factor = fit.width() / canvas_width;
                            let mut labels = Vec::new();
                            for item in draw.flat_items() {
                                if let DrawItem::Label { at, text, size, .. } = item {
                                    let galley = ctx.fonts_mut(|fonts| {
                                        fonts.layout_no_wrap(
                                            text.clone(),
                                            FontId::monospace(*size * factor),
                                            crate::theme::TEXT,
                                        )
                                    });
                                    let rect = Align2::CENTER_CENTER.anchor_rect(Rect::from_min_size(
                                        fit.min + egui::vec2(at[0], at[1]) * factor,
                                        galley.size(),
                                    ));
                                    assert!(
                                        fit.expand(0.5).contains_rect(rect),
                                        "{id} {text}: {rect:?} outside {fit:?}"
                                    );
                                    for (other, other_rect) in &labels {
                                        assert!(
                                            !rect.shrink(0.1).intersects(*other_rect),
                                            "{id} {text} overlaps {other} (reversion {reversion})"
                                        );
                                    }
                                    labels.push((text, rect));
                                }
                            }
                        }
                    }
                }
            }
        }
    });
}

#[test]
fn display_units_zero_and_requested_pfd_readouts() {
    require_region_data!();
    let s = flight();
    let state = s.glass_state();
    for units in [
        crate::instruments::EngineUnits::Us,
        crate::instruments::EngineUnits::Si,
    ] {
        let mut input = InstrumentInput::new(s.sim().systems(), 0.).with_glass(&state);
        input.options.engine = units;
        for id in ["glass_pfd", "glass_mfd"] {
            let d = create(id).unwrap().update(input);
            let labels = d
                .items
                .iter()
                .filter_map(|i| {
                    if let DrawItem::Label { text, .. } = i {
                        Some(text.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            assert!(labels.iter().all(|s| !s.contains("-0 ") && !s.contains("-0.0")));
            if id == "glass_pfd" {
                assert!(labels.contains(&"XPDR 1200 ALT"));
                assert!(
                    labels
                        .iter()
                        .any(|t| t.contains(if units == crate::instruments::EngineUnits::Us {
                            "°F"
                        } else {
                            "°C"
                        }))
                );
            } else {
                for expected in if units == crate::instruments::EngineUnits::Us {
                    ["gal/h", "psi", "°F", "US gal"]
                } else {
                    ["kg/h", "kPa", "°C", "kg"]
                } {
                    assert!(labels.contains(&expected), "{expected}");
                }
                assert!(
                    d.items
                        .iter()
                        .any(|i| matches!(i,DrawItem::Mesh {vertices,..} if vertices.len()>16000))
                );
            }
        }
    }
}

fn local_flight() -> Session {
    let mut s = crate::panel_fidelity_tests::geographic_session();
    s.execute(
        "aircraft.load",
        serde_json::json!({"source":{"kind":"installed","id":"corvid-glass"},"reset":true}),
    )
    .unwrap();
    s.execute("sim.reset",serde_json::json!({"world":"panel-preview","start":{"kind":"air","height_m":1000.,"airspeed_mps":50.,"heading_rad":0.}})).unwrap();
    s.execute(
        "assist.set",
        serde_json::json!({"altitude_m":1000.,"heading_rad":0.,"airspeed_mps":50.}),
    )
    .unwrap();
    s.run(flightcraft_control::Step { steps: 5401 }).unwrap();
    s
}
#[test]
fn synthetic_vision_is_inhibited_without_valid_sensors_and_on_standby() {
    let session = local_flight();
    let mut state = session.glass_state();
    state.settings.synthetic_vision = true;
    let draw = |state: &GlassState, standby| {
        pfd::draw(
            state,
            InstrumentInput::new(session.sim().systems(), 0.)
                .with_glass(state)
                .with_vision(session.sim().world(), session.sim().state()),
            standby,
        )
    };
    let has_pass = |draw: DrawList| {
        draw.items.iter().any(|i| {
            matches!(
                i,
                DrawItem::SyntheticVision {
                    min: [0., 0.],
                    max: [500., 440.],
                    ..
                }
            )
        })
    };
    assert!(has_pass(draw(&state, false)));
    assert!(!has_pass(draw(&state, true)));
    state.air_data.air_data_valid = false;
    assert!(!has_pass(draw(&state, false)));
    state.air_data.air_data_valid = true;
    state.air_data.attitude_valid = false;
    assert!(!has_pass(draw(&state, false)));
    state.air_data.attitude_valid = true;
    state.pfd_failed = true;
    assert!(!has_pass(draw(&state, false)));
    state.pfd_failed = false;
    state.settings.synthetic_vision = false;
    assert!(!has_pass(draw(&state, false)));
}

#[test]
fn pfd_horizon_ladder_and_fixed_aircraft_symbol_share_the_camera_projection() {
    let session = local_flight();
    let mut state = session.glass_state();
    state.settings.synthetic_vision = true;
    state.air_data.roll_rad = 0.;
    for degrees in [-10_f64, 0., 10.] {
        state.air_data.pitch_rad = degrees.to_radians();
        let draw = pfd::draw(
            &state,
            InstrumentInput::new(session.sim().systems(), 0.)
                .with_glass(&state)
                .with_vision(session.sim().world(), session.sim().state()),
            false,
        );
        let expected = 125. + (2.5 * 180. / std::f64::consts::PI * degrees.to_radians().tan()) as f32;
        assert!(draw.items.iter().any(|item| matches!(item,
            DrawItem::Line { a:[215.,y], b:[285.,yy], ink:Ink::Text, width:2. }
            if (*y-expected).abs() < 0.001 && (*yy-expected).abs() < 0.001)));
        assert!(draw.items.iter().any(|item| matches!(
            item,
            DrawItem::Circle {
                center: [250., 125.],
                radius: 2.,
                ink: Ink::Needle
            }
        )));
    }
}
