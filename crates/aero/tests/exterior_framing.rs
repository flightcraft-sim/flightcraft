#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_aero::exterior::{Exterior, ExteriorAsset};
use flightcraft_aero::exterior_framing::*;
use std::sync::Arc;

fn corvid() -> ExteriorAsset {
    let definition: Exterior =
        serde_json::from_str(include_str!("../../../aircraft/corvid/exterior.json")).unwrap();
    let lod0 = flightcraft_aero::exterior::load_glb(
        include_bytes!("../../../aircraft/corvid/exterior.glb"),
        &definition,
    )
    .unwrap();
    ExteriorAsset {
        lod0: Arc::new(lod0.clone()),
        lod1: Arc::new(lod0),
        definition,
        lod2: None,
    }
}

#[test]
fn flap_framing_looks_at_the_trailing_edge_from_behind_and_above() {
    let asset = corvid();
    let nodes = FramingTarget::Flaps.nodes(&asset);
    assert_eq!(nodes, ["flaps_left", "flaps_right"]);
    let positions: Vec<_> = nodes
        .iter()
        .map(|n| node_position(&asset.lod0, n).unwrap())
        .collect();
    let f = frame(&asset, FramingTarget::Flaps, [0.; 3]).unwrap();
    let mean_x = (positions[0][0] + positions[1][0]) / 2.;
    assert!(
        f.eye_m[0] < mean_x - 1.0,
        "eye behind the flaps: {f:?} {positions:?}"
    );
    assert!(f.eye_m[2] < positions[0][2], "eye above the flaps (z is down)");
    assert!((f.target_m[0] - mean_x).abs() < 0.3);
}

#[test]
fn other_targets_frame_their_own_parts() {
    let asset = corvid();
    let flaps = frame(&asset, FramingTarget::Flaps, [0.; 3]).unwrap();
    let ail = frame(&asset, FramingTarget::Ailerons, [0.; 3]).unwrap();
    assert!(
        ail.target_m[1].abs() > flaps.target_m[1].abs(),
        "wing tip is outboard of the flap"
    );
    let tail = frame(&asset, FramingTarget::Tail, [0.; 3]).unwrap();
    assert!(tail.target_m[0] < flaps.target_m[0], "tail is aft of the flaps");
    let wheels = frame(&asset, FramingTarget::Wheels, [0.; 3]).unwrap();
    assert!(wheels.target_m[2] > 0., "wheels are below the CG");
    let default = frame(&asset, FramingTarget::Default, [0.; 3]).unwrap();
    assert!(
        default.eye_m[0] < 0. && default.eye_m[2] < 0.,
        "three-quarter rear, above"
    );
}

#[test]
fn centre_of_gravity_shifts_every_framing_and_framings_ease() {
    let asset = corvid();
    let a = frame(&asset, FramingTarget::Flaps, [0.; 3]).unwrap();
    let b = frame(&asset, FramingTarget::Flaps, [0.5, 0., 0.]).unwrap();
    assert!((a.target_m[0] - b.target_m[0] - 0.5).abs() < 1e-4);
    let mid = a.lerp(b, 0.5);
    assert!((mid.eye_m[0] - (a.eye_m[0] + b.eye_m[0]) / 2.).abs() < 1e-5);
    assert_eq!(a.lerp(b, 0.), a);
}
