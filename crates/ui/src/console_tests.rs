//! The Sling 2's centre console as a drawn region of the panel: where the POH puts the throttle,
//! hand brake, park-brake valve, and sticks (POH 7.2, 7.4, 7.9, 7.15), that the console window
//! draws and fits them on a desktop and a phone, and that the dash itself is untouched.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::panel_art::{console_panel, dash_bounds, installation_rect, outline_points};
use eframe::egui::{self, Rect};
use flightcraft_aero::modern::{EquipmentPanel, Outline};

fn panel() -> EquipmentPanel {
    crate::layout_tests::sling_panel().equipment.unwrap()
}

fn inside_outline(outline: &Outline, p: egui::Pos2) -> bool {
    let pts = outline_points(outline);
    let mut inside = false;
    let mut j = pts.len() - 1;
    for i in 0..pts.len() {
        let (xi, yi) = (pts[i][0] as f32, pts[i][1] as f32);
        let (xj, yj) = (pts[j][0] as f32, pts[j][1] as f32);
        if (yi > p.y) != (yj > p.y) && p.x < (xj - xi) * (p.y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn corners(r: Rect) -> [egui::Pos2; 4] {
    [r.left_top(), r.right_top(), r.left_bottom(), r.right_bottom()]
}

#[test]
fn the_levers_stand_on_the_drawn_console_and_the_sticks_on_the_floor_in_the_pohs_arrangement() {
    let p = panel();
    let shell = p.shell.as_ref().unwrap();
    let console = shell.centre_console.as_ref().unwrap();
    let floor = shell.floor.as_ref().expect("the pack draws the cabin floor");
    let rect = |id: &str| installation_rect(p.elements.iter().find(|e| e.id == id).unwrap());
    // Throttle, hand brake, and park-brake valve are on the console top (POH 7.4, 7.9, 7.15).
    for id in ["throttle", "brake_lever", "park_brake"] {
        for c in corners(rect(id)) {
            assert!(inside_outline(console, c), "{id} corner {c:?} is on the console");
        }
    }
    // The sticks stand on the floor beside it, not on it (POH 7.2: dual control sticks), one
    // in front of each seat.
    for id in ["pilot_stick", "copilot_stick"] {
        for c in corners(rect(id)) {
            assert!(inside_outline(floor, c), "{id} is on the floor");
            assert!(!inside_outline(console, c), "{id} is outside the console");
        }
    }
    let (pilot, passenger) = (rect("pilot_stick"), rect("copilot_stick"));
    let console_mid = rect("throttle").union(rect("park_brake")).center().x;
    assert!(pilot.center().x < console_mid && passenger.center().x > console_mid);
    // The pilot sits on the left: the throttle is under the pilot's left hand, ahead of the
    // brake lever, and the park valve is the furthest forward of the three.
    assert!(rect("throttle").center().x < rect("brake_lever").center().x);
    assert!(rect("park_brake").top() < rect("brake_lever").top());
    // Everything is below the fuel selector at the front of the console (POH 7.2) and none of it
    // crosses the dash outline.
    let selector = rect("fuel_selector");
    let dash = dash_bounds(&p).unwrap();
    for id in [
        "throttle",
        "brake_lever",
        "park_brake",
        "pilot_stick",
        "copilot_stick",
    ] {
        assert!(
            rect(id).top() >= selector.bottom(),
            "{id} is aft of the fuel selector"
        );
        assert!(!dash.intersects(rect(id)), "{id} is below the dash");
    }
    // The levers are tall enough to read: the throttle has travel (a slot), not a button.
    assert!(rect("throttle").height() > 2. * rect("throttle").width());
    // The master switch and flap selector stay on the instrument panel itself (POH 7.10 items 1, 20).
    for id in ["master_switch", "flap_selector"] {
        assert!(dash.contains_rect(rect(id)), "{id} is on the dash");
    }
}

#[test]
fn the_lower_console_window_keeps_the_console_artwork_and_stays_on_screen_at_desktop_and_phone_sizes() {
    let p = panel();
    let dash = dash_bounds(&p).unwrap();
    let c = console_panel(&p, dash);
    // The window host draws the console and the floor, rebased to its own origin.
    let shell = c.shell.as_ref().expect("the console keeps its artwork");
    assert!(shell.floor.is_some() && shell.centre_console.is_none());
    let bounds = outline_points(shell.floor.as_ref().unwrap())
        .iter()
        .fold(Rect::NOTHING, |r, [x, y]| {
            r.union(Rect::from_min_max(
                egui::pos2(*x as f32, *y as f32),
                egui::pos2(*x as f32, *y as f32),
            ))
        });
    assert!(
        Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(c.size[0] as f32, c.size[1] as f32))
            .expand(1.)
            .contains_rect(bounds),
        "the floor lies inside the console host: {bounds:?}"
    );
    for e in &c.elements {
        assert!(
            Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(c.size[0] as f32, c.size[1] as f32))
                .expand(0.1)
                .contains_rect(installation_rect(e)),
            "{} inside the console host",
            e.id
        );
    }
    for screen in [
        egui::vec2(1440., 900.),
        egui::vec2(390., 844.),
        egui::vec2(844., 390.),
    ] {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut size = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, screen)),
                ..Default::default()
            },
            |ctx| {
                size = Some(crate::modern::console_window_size(ctx, &c));
            },
        );
        let size = size.unwrap();
        let bounds = crate::theme::window_bounds(&ctx);
        assert!(
            size.x <= bounds.width() && size.y <= bounds.height() - 40.,
            "{screen:?}: window {size:?} fits {bounds:?}"
        );
        // Each control keeps a finger-sized target at that scale (about 24 points).
        let scale = size.x / c.size[0] as f32;
        for e in &c.elements {
            let r = installation_rect(e);
            assert!(
                r.width() * scale >= 20. && r.height() * scale >= 20.,
                "{screen:?}: {} is {:.0} x {:.0} points",
                e.id,
                r.width() * scale,
                r.height() * scale
            );
        }
    }
}
