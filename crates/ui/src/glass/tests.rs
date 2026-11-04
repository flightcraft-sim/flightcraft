#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
#[cfg(test)]
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/region_data.rs"));
use super::*;
use eframe::egui::{self, Align2, FontId, Pos2, Rect};
use flightcraft_control::{GlassFailure, GlassSetting, Session};
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/atc_fixture.rs"));
/// A glass aircraft 4 NM west of Allentown on the committed flat ATC fixture (needs no tile data).
pub(super) fn fixture_flight() -> Session {
    let mut s = Session::new().unwrap();
    s.execute(
        "aircraft.load",
        serde_json::json!({"source":{"kind":"installed","id":"corvid-glass"},"reset":true}),
    )
    .unwrap();
    s.install_region(atc_world()).unwrap();
    s.execute("sim.reset",serde_json::json!({"world":"atc-fixture","start":{"air":{"airport":"KABE","distance_m":7000.,"bearing_deg":240.,"height_m":900.,"airspeed_mps":50.,"heading_rad":0.}}})).unwrap();
    s.execute(
        "autopilot.set",
        serde_json::json!({"altitude_m":914.4,"heading_rad":0.,"airspeed_mps":50.}),
    )
    .unwrap();
    s.execute(
        "atc.configure",
        serde_json::json!({"airport":"KABE","destination":"KMMU"}),
    )
    .unwrap();
    s
}
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

#[test]
fn synthetic_vision_on_1n7_runway_keeps_terrain_without_awareness_bands() {
    require_region_data!();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/terrain/eastern-pa");
    let mut s = Session::new().unwrap();
    s.install_region(flightcraft_control::read_region(&root).unwrap())
        .unwrap();
    s.execute(
        "aircraft.load",
        serde_json::json!({"source":{"kind":"installed","id":"corvid-glass"}}),
    )
    .unwrap();
    s.execute("sim.reset",serde_json::json!({"world":"eastern-pa","start":{"kind":"airport_runway","airport":"1N7","end":"07"}})).unwrap();
    let mut state = s.glass_state();
    state.settings.synthetic_vision = true;
    let draw = super::pfd::draw(
        &state,
        InstrumentInput::new(s.sim().systems(), 0.)
            .with_glass(&state)
            .with_vision(s.sim().world(), s.sim().state()),
        false,
    );
    let views: Vec<_> = draw
        .items
        .iter()
        .filter_map(|item| {
            if let DrawItem::SyntheticVision { view, .. } = item {
                Some(view)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(views.len(), 1, "SVT should still show normal terrain");
    assert!(
        !views[0].terrain_awareness_enabled(),
        "runway SVT must have neither caution nor warning shading"
    );
}

#[test]
fn manifold_pressure_uses_inches_of_mercury_and_oil_pressure_retains_psi() {
    let value = |label: &str| flightcraft_control::EngineValue {
        label: label.into(),
        value: 37.25,
        unit: "kPa".into(),
        min: 0.,
        max: 120.,
        normal: [20., 105.],
    };
    assert_eq!(
        super::engine_display(&value("MANIFOLD"), crate::instruments::EngineUnits::Us),
        ("11.0".into(), "inHg".into())
    );
    assert_eq!(
        super::engine_display(&value("OIL P"), crate::instruments::EngineUnits::Us).1,
        "psi"
    );
    assert_eq!(
        super::engine_display(&value("MANIFOLD"), crate::instruments::EngineUnits::Si).1,
        "kPa"
    );
}

#[test]
fn engine_strip_captions_do_not_overlap_each_other_or_round_gauges() {
    let session = crate::panel_fidelity_tests::geographic_session();
    let mut state = session.glass_state();
    state.engine.push(flightcraft_control::EngineValue {
        label: "MANIFOLD".into(),
        value: 37.25,
        unit: "kPa".into(),
        min: 0.,
        max: 120.,
        normal: [20., 105.],
    });
    for size in [egui::vec2(110., 380.), egui::vec2(63., 216.)] {
        for units in [
            crate::instruments::EngineUnits::Us,
            crate::instruments::EngineUnits::Si,
        ] {
            let ctx = egui::Context::default();
            crate::theme::install(&ctx);
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ctx, |ui| {
                            let mut input = InstrumentInput::new(session.sim().systems(), 0.);
                            input.options.engine = units;
                            super::touch_engine(
                                ui,
                                ui.max_rect(),
                                &state,
                                input,
                                flightcraft_aero::modern::EngineStripStyle::DialsAndBars,
                            );
                        });
                },
            );
            fn collect(shape: &egui::Shape, text: &mut Vec<(String, Rect)>, dials: &mut Vec<Rect>) {
                match shape {
                    egui::Shape::Text(t) => text.push((
                        t.galley.text().into(),
                        Rect::from_min_size(t.pos, t.galley.size()),
                    )),
                    egui::Shape::Circle(c) => dials.push(Rect::from_center_size(
                        c.center,
                        egui::Vec2::splat(c.radius * 2. + 2.),
                    )),
                    egui::Shape::Vec(shapes) => {
                        for s in shapes {
                            collect(s, text, dials);
                        }
                    }
                    _ => {}
                }
            }
            let mut text = Vec::new();
            let mut dials = Vec::new();
            for shape in &output.shapes {
                collect(&shape.shape, &mut text, &mut dials);
            }
            assert!(text.iter().any(|(t, _)| t.starts_with("MANIFOLD ")));
            assert!(dials.len() >= 2);
            for (i, (a, rect)) in text.iter().enumerate() {
                assert!(
                    Rect::from_min_size(Pos2::ZERO, size)
                        .expand(0.1)
                        .contains_rect(*rect),
                    "clipped {a}: {rect:?}"
                );
                for dial in &dials {
                    assert!(!rect.intersects(*dial), "{a} overlaps gauge: {rect:?} {dial:?}");
                }
                for (b, other) in text.iter().skip(i + 1) {
                    assert!(!rect.intersects(*other), "{a} overlaps {b}");
                }
            }
        }
    }
}

fn labels(d: &DrawList) -> Vec<String> {
    d.items
        .iter()
        .filter_map(|i| match i {
            DrawItem::Label { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}
fn target(address: u32, class: flightcraft_control::TrafficClass, x: f64) -> flightcraft_control::MapTraffic {
    flightcraft_control::MapTraffic {
        callsign: format!("Cessna {address}"),
        registration: format!("N{address}"),
        position: [x, 0.4],
        relative_altitude_m: 500. * 0.3048,
        address,
        class,
        chart_track_rad: 0.5,
        vertical_rate_fpm: 800.,
        ground_speed_kt: 100.,
        range_nm: 3.,
        highlighted: false,
    }
}

#[test]
fn mfd_traffic_symbols_follow_class_and_annunciation_follows_alert() {
    use flightcraft_control::TrafficClass::{Alert, Other, Proximate};
    let mut s = Session::new().unwrap();
    s.execute(
        "aircraft.load",
        serde_json::json!({"source":{"kind":"installed","id":"corvid-glass"},"reset":true}),
    )
    .unwrap();
    let mut g = s.glass_state();
    g.traffic_installed = true;
    let polygons = |d: &DrawList, ink: Ink| {
        d.items
            .iter()
            .filter(|i| matches!(i, DrawItem::Polygon { ink: i2, .. } if *i2 == ink))
            .count()
    };
    let base = mfd::draw(&g, Default::default());
    // Other traffic is a hollow arrow, so it adds lines but no filled arrow.
    g.traffic = vec![target(1, Other, 0.3)];
    let other = mfd::draw(&g, Default::default());
    assert_eq!(polygons(&other, Ink::Text), polygons(&base, Ink::Text));
    assert!(other.items.len() > base.items.len() + 4);
    // Proximate traffic is a filled arrow in the text ink.
    g.traffic = vec![target(1, Proximate, 0.3)];
    let proximate = mfd::draw(&g, Default::default());
    assert_eq!(polygons(&proximate, Ink::Text), polygons(&base, Ink::Text) + 1);
    // An alert is amber with a filled circle, and names itself.
    g.traffic = vec![target(1, Alert, 0.3)];
    let alert = mfd::draw(&g, Default::default());
    assert!(alert.items.iter().any(|i| matches!(
        i,
        DrawItem::Circle {
            ink: Ink::Warning,
            ..
        }
    )));
    assert!(labels(&alert).contains(&"N1".to_string()));
    // Relative altitude tag in hundreds of feet with a sign, and a climb arrow above 500 fpm.
    assert!(labels(&proximate).contains(&"+05".to_string()));
    let climbing = alert.items.len();
    g.traffic[0].vertical_rate_fpm = 0.;
    assert!(mfd::draw(&g, Default::default()).items.len() < climbing);
    // The footer shows the filter, and TFC OFF when hidden.
    assert!(labels(&alert).iter().any(|l| l.contains("TFC NORMAL")));
    g.settings.traffic_shown = false;
    assert!(
        labels(&mfd::draw(&g, Default::default()))
            .iter()
            .any(|l| l.contains("TFC OFF"))
    );
    // Without a receiver, no TFC text.
    g.traffic_installed = false;
    assert!(
        !labels(&mfd::draw(&g, Default::default()))
            .iter()
            .any(|l| l.contains("TFC"))
    );
    // Highlighted traffic gets the magenta ring and its registration.
    g.traffic_installed = true;
    g.settings.traffic_shown = true;
    g.traffic = vec![target(7, Other, 0.3)];
    g.traffic[0].highlighted = true;
    let named = mfd::draw(&g, Default::default());
    assert!(
        named
            .items
            .iter()
            .any(|i| matches!(i, DrawItem::Line { ink: Ink::Gps, .. }))
    );
    assert!(labels(&named).contains(&"N7".to_string()));
    // The PFD annunciates TRAFFIC only for an alert.
    let annunciated = |g: &GlassState| {
        let input = InstrumentInput::new(s.sim().systems(), s.sim().state().time_s)
            .with_glass(g)
            .with_aircraft(s.aircraft_configuration().unwrap());
        labels(&pfd::draw(g, input, false)).contains(&"TRAFFIC".to_string())
    };
    assert!(!annunciated(&g));
    g.traffic_alert = true;
    assert!(annunciated(&g));
}

#[test]
fn live_fleet_traffic_reaches_the_glass_state_and_obeys_the_filter() {
    let mut s = fixture_flight();
    assert!(s.glass_state().traffic_installed);
    assert!(s.glass_state().traffic.is_empty());
    s.execute(
        "traffic.density",
        serde_json::json!({"density":"busy","airport":"KABE"}),
    )
    .unwrap();
    s.run(flightcraft_control::Step { steps: 120 * 5 }).unwrap();
    s.set_glass(GlassSetting::MapRange { range_nm: 40. }).unwrap();
    s.set_glass(GlassSetting::TrafficFilter {
        filter: flightcraft_control::TrafficFilter::Unrestricted,
    })
    .unwrap();
    let all = s.glass_state().traffic.len();
    assert!(all > 0, "busy traffic around the field must reach the MFD");
    s.set_glass(GlassSetting::TrafficFilter {
        filter: flightcraft_control::TrafficFilter::Normal,
    })
    .unwrap();
    let state = s.glass_state();
    assert!(state.traffic.len() <= all);
    assert!(
        state
            .traffic
            .iter()
            .all(|t| t.relative_altitude_m / 0.3048 <= 2700.
                || t.class == flightcraft_control::TrafficClass::Alert)
    );
    s.set_glass(GlassSetting::Traffic { shown: false }).unwrap();
    let hidden = s.glass_state();
    assert!(hidden.traffic.is_empty());
    assert_eq!(
        hidden.traffic_alert, state.traffic_alert,
        "alerts ignore the map toggle"
    );
}
