#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
#[cfg(test)]
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/region_data.rs"));
use super::*;
pub(super) fn session() -> Session {
    let mut s = Session::new().unwrap();
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/terrain/new-york");
    s.install_region(flightcraft_control::read_region(&directory).unwrap())
        .unwrap();
    s.execute(
        "sim.reset",
        json!({"world":"new-york","start":{"runway":{"airport":"1N7","end":"07"}},"paused":true}),
    )
    .unwrap();
    s.execute(
        "map.set",
        json!({"centre":{"latitude_deg":40.80,"longitude_deg":-74.72},"range_nm":24.}),
    )
    .unwrap();
    s
}
pub(super) fn planned_session() -> Session {
    let mut s = session();
    for ident in ["1N7", "SBJ", "KMMU"] {
        let output: flightcraft_control::SearchOutput =
            serde_json::from_value(s.execute("search.query", json!({"text":ident})).unwrap()).unwrap();
        let item = output
            .results
            .iter()
            .find(|r| r.ident == ident || (ident == "KMMU" && r.ident == "MMU"))
            .unwrap_or(&output.results[0]);
        let waypoint = json!({"ident":ident,"kind":if ident=="SBJ"{"navaid"}else{"airport"},"position":item.position.unwrap()});
        s.execute(
            "plan.add",
            json!({"point":{"waypoint":waypoint,"altitude_m":1200.}}),
        )
        .unwrap();
    }
    s
}
#[test]
fn region_view_round_trips_rotations_and_aspect_ratios() {
    require_region_data!();
    let s = session();
    for size in [
        egui::vec2(600., 300.),
        egui::vec2(300., 600.),
        egui::vec2(800., 800.),
    ] {
        let rect = Rect::from_min_size(egui::pos2(19., 23.), size);
        for angle in [0., 0.6, 2.3] {
            let mut v = View::new(&s, s.map_state(), rect).unwrap();
            v.angle = angle;
            for local in [[0., 0.], [10000., -10000.], [-40000., 30000.]] {
                let [n, e] = v.local_at(v.local(local));
                assert!((n - local[0]).abs() < 0.01 && (e - local[1]).abs() < 0.01);
            }
            let p = v.position(rect.center()).unwrap();
            assert!(v.point(p).unwrap().distance(rect.center()) < 0.001);
        }
    }
}
#[test]
fn cards_hit_points_and_route_legs() {
    require_region_data!();
    let mut s = planned_session();
    let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800., 600.));
    let v = View::new(&s, s.map_state(), rect).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install(&ctx);
    let mut features = Vec::new();
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        features = paint_features(
            &ctx.layer_painter(egui::LayerId::background()),
            &v,
            &mut s,
            &MapState::default(),
        );
    });
    let airport = features.iter().find(|f| f.ident == "KMMU").unwrap();
    assert_eq!(hit(&features, airport.pixel).unwrap().kind, "airport");
    let navaid = features.iter().find(|f| f.ident == "SBJ").unwrap();
    assert!(navaid.details.contains("MHz"));
    assert!(hit(&features, egui::pos2(-1000., -1000.)).is_none());
    let points = &s.flight_plan().waypoints;
    let a = v.point(points[0].waypoint.position).unwrap();
    let b = v.point(points[1].waypoint.position).unwrap();
    assert_eq!(route_hit(&s, &v, a), Some(Drag::Waypoint(0)));
    assert_eq!(route_hit(&s, &v, a.lerp(b, 0.5)), Some(Drag::Leg(0)));
}
fn input(ctx: &egui::Context, widget: &mut PlanningMap, s: &mut Session, events: Vec<egui::Event>) {
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800., 600.))),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                widget.map(
                    ui,
                    Rect::from_min_size(Pos2::ZERO, egui::vec2(800., 600.)),
                    s,
                    false,
                );
            });
        },
    );
}
fn drag(ctx: &egui::Context, widget: &mut PlanningMap, s: &mut Session, from: Pos2, to: Pos2) {
    input(
        ctx,
        widget,
        s,
        vec![
            egui::Event::PointerMoved(from),
            egui::Event::PointerButton {
                pos: from,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
        ],
    );
    input(ctx, widget, s, vec![egui::Event::PointerMoved(to)]);
    input(
        ctx,
        widget,
        s,
        vec![egui::Event::PointerButton {
            pos: to,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }],
    );
}
#[test]
fn simulated_drags_move_and_insert_through_commands() {
    require_region_data!();
    let mut s = planned_session();
    let ctx = egui::Context::default();
    crate::theme::install(&ctx);
    let mut widget = PlanningMap {
        edit: true,
        ..Default::default()
    };
    input(&ctx, &mut widget, &mut s, vec![]);
    let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800., 600.));
    let v = View::new(&s, s.map_state(), rect).unwrap();
    let origin = s.flight_plan().waypoints[0].waypoint.position;
    let a = v.point(origin).unwrap();
    let target = a + egui::vec2(30., -20.);
    drag(&ctx, &mut widget, &mut s, a, target);
    assert!(origin.inverse(s.flight_plan().waypoints[0].waypoint.position).0 > 100.);
    assert!(
        s.flight_plan().waypoints[0]
            .waypoint
            .position
            .inverse(v.position(target).unwrap())
            .0
            < 1.
    );
    let a = v.point(s.flight_plan().waypoints[0].waypoint.position).unwrap();
    let b = v.point(s.flight_plan().waypoints[1].waypoint.position).unwrap();
    let middle = a.lerp(b, 0.5);
    drag(&ctx, &mut widget, &mut s, middle, middle + egui::vec2(25., 0.));
    assert_eq!(s.flight_plan().waypoints.len(), 4);
    assert!(widget.error.is_none(), "{:?}", widget.error);
    let pos = v.point(s.flight_plan().waypoints[1].waypoint.position).unwrap();
    input(
        &ctx,
        &mut widget,
        &mut s,
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Secondary,
                pressed: true,
                modifiers: Default::default(),
            },
        ],
    );
    input(
        &ctx,
        &mut widget,
        &mut s,
        vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed: false,
            modifiers: Default::default(),
        }],
    );
    assert_eq!(s.flight_plan().waypoints.len(), 3);
}
#[test]
fn widget_paints_within_its_rectangle() {
    require_region_data!();
    let mut s = planned_session();
    for size in [
        egui::vec2(320., 240.),
        egui::vec2(1000., 546.),
        egui::vec2(1000., 700.),
    ] {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut widget = PlanningMap::default();
        let bounds = Rect::from_min_size(egui::pos2(30., 40.), size);
        for _ in 0..2 {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200., 900.))),
                    ..Default::default()
                },
                |ctx| {
                    let mut ui = egui::Ui::new(
                        ctx.clone(),
                        egui::Id::new("layout"),
                        egui::UiBuilder::new()
                            .max_rect(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200., 900.))),
                    );
                    widget.show(&mut ui, bounds, &mut s);
                },
            );
            assert!(!output.shapes.is_empty());
            for shape in &output.shapes {
                assert!(shape.clip_rect.is_finite());
                assert!(
                    bounds.contains_rect(shape.clip_rect),
                    "clip {:?} exceeds {:?}",
                    shape.clip_rect,
                    bounds
                );
            }
            assert!(widget.error.is_none());
        }
    }
}

#[test]
fn magnetic_orientation_keeps_aircraft_up_and_true_north_up() {
    require_region_data!();
    let mut s = session();
    let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800., 600.));
    let mut state = s.map_state().clone();
    let north = View::new(&s, &state, rect).unwrap();
    let centre = north.position(rect.center()).unwrap();
    let point = north.point(centre.destination(0., 1000.)).unwrap();
    assert!((point.x - rect.center().x).abs() < 0.05 && point.y < rect.center().y);
    for orientation in [MapUp::HeadingUp, MapUp::TrackUp] {
        state.orientation = orientation;
        let view = View::new(&s, &state, rect).unwrap();
        let [n, e, _] = s.sim().state().position_m;
        let heading = view.aircraft_grid_heading_rad;
        let a = view.local([n, e]);
        let b = view.local([n + heading.cos() * 1000., e + heading.sin() * 1000.]);
        assert!((a.x - b.x).abs() < 0.001 && b.y < a.y);
        let start = view.position(a).unwrap();
        let end = view.position(b).unwrap();
        let variation = flightcraft_world::geomag::MagneticModel::wmm2025()
            .unwrap()
            .declination(start.latitude_deg, start.longitude_deg, s.simulation_date())
            .unwrap();
        let expected = (flightcraft_sim::avionics::navigation::inverse_nm(
            [start.latitude_deg, start.longitude_deg],
            [end.latitude_deg, end.longitude_deg],
        )
        .1 - variation)
            .rem_euclid(360.);
        assert!((view.magnetic_bearing_deg.unwrap() - expected).abs() < 0.03);
    }
    let before = View::new(&s, &state, rect).unwrap();
    s.execute("sim.set_date", json!({"date":{"year":2029,"month":1,"day":1}}))
        .unwrap();
    let after = View::new(&s, &state, rect).unwrap();
    assert!((after.magnetic_bearing_deg.unwrap() - before.magnetic_bearing_deg.unwrap()).abs() > 0.01);
    assert!((after.angle - before.angle).sin().abs() < 1e-12);
}
#[test]
fn declutter_preserves_route_fixes_and_pilot_units() {
    require_region_data!();
    let mut s = session();
    let ctx = egui::Context::default();
    crate::theme::install(&ctx);
    let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800., 600.));
    let view = View::new(&s, s.map_state(), rect).unwrap();
    let fix = s
        .sim()
        .world()
        .navdata()
        .unwrap()
        .fixes
        .iter()
        .find(|f| view.point(f.position).is_some_and(|p| rect.contains(p)))
        .unwrap()
        .clone();
    s.execute("plan.add",json!({"point":{"waypoint":{"ident":fix.ident,"kind":"fix","position":fix.position},"altitude_m":1000.}})).unwrap();
    let mut features = Vec::new();
    let state = s.map_state().clone();
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        features = paint_features(
            &ctx.layer_painter(egui::LayerId::background()),
            &view,
            &mut s,
            &state,
        );
    });
    assert_eq!(features.iter().filter(|f| f.kind == "fix").count(), 1);
    assert_eq!(
        features.iter().find(|f| f.kind == "fix").unwrap().ident,
        fix.ident
    );
    assert_eq!(
        units::fuel(0.72 * 3.785411784, crate::instruments::EngineUnits::Us),
        "1.0 US gal"
    );
    assert_eq!(units::fuel(1., crate::instruments::EngineUnits::Si), "1.0 kg");
    assert_eq!(units::speed(51.44444444444445), "100 kt");
}
#[test]
fn terrain_vertices_end_at_grid_boundary_without_unknown_colour_blending() {
    require_region_data!();
    let s = session();
    let ctx = egui::Context::default();
    crate::theme::install(&ctx);
    let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800., 600.));
    let view = View::new(&s, s.map_state(), rect).unwrap();
    let output = ctx.run(egui::RawInput::default(), |ctx| {
        terrain(
            &ctx.layer_painter(egui::LayerId::background()),
            &view,
            &s,
            s.map_state(),
            false,
            false,
        );
    });
    let region = s.sim().world().region().unwrap();
    let [sn, se] = region.southwest_m;
    for shape in output.shapes {
        if let egui::Shape::Mesh(mesh) = shape.shape {
            for vertex in &mesh.vertices {
                let [n, e] = view.local_at(vertex.pos);
                assert!(n >= sn - 0.02 && e >= se - 0.02);
                assert_ne!(vertex.color, theme::BORDER);
            }
        }
    }
}
