//! The walkaround figure never clips into the aircraft.
//!
//! Every inspection item of the Sling 2 pre-flight is performed in the session; at every animation
//! sample the figure's rendered vertices (the real `flightcraft_render::walkaround` mesh) are
//! tested against the aircraft's keep-out volumes: the pack's plan-view keep-out rectangles,
//! extruded through the height each part occupies, plus the three wheel fairings. Torso, legs,
//! and shoes must stay outside, the cap brim by all but a peek. Arms, hands and the face (skin and
//! sleeve materials) may reach into a volume only as far as a touch target authored inside it
//! requires.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_aero::{
    preflight::Preflight,
    walkaround::{FigureAction, FigureState},
};
use flightcraft_control::{PreflightAdvance, PreflightPerform, Session};
use flightcraft_render::walkaround::primitives;

/// Torso, legs, shoes and cap: no more than a vertex's worth of rounding inside.
const BODY_M: f32 = 0.02;
/// Arms, hands and face: the deepest an authored touch target lies inside a keep-out volume.
const REACH_M: f32 = 0.45;
/// Render material indices of the arm and skin groups (`walkaround.rs`: SKIN, SHIRT).
const LIMB_MATERIALS: [usize; 2] = [0, 11];
/// The cap and shoes (`walkaround.rs`: DARK): the cap brim peeks toward what the figure looks at.
const CAP_M: f32 = 0.15;
const DARK_MATERIAL: usize = 3;
/// Body-frame ground level used by the fixture (z down), matching the pack's wheel contact.
const GROUND_Z: f32 = 0.85;

struct Volume {
    id: String,
    min: [f32; 3],
    max: [f32; 3],
}

/// Heights (body z, down) the keep-out rectangles occupy. The pack gives the plan rectangles;
/// these are the matching vertical extents of the airframe parts the rectangles stand for.
fn band(id: &str) -> [f32; 2] {
    match id {
        // Low wing: root skin at z = 0.27 with the section about 0.07 deep, raised by dihedral.
        "wing_left" | "wing_right" => [0.10, 0.40],
        "tailplane" => [-0.30, 0.30],
        // Fuselage: from above the canopy to the belly. The wheels hang below it; the wheel
        // fairings are separate volumes.
        "fuselage" => [-1.40, 0.45],
        // Propeller disc: the blades' full circle about the shaft.
        _ => [-0.96, 0.84],
    }
}

/// Plan rectangles from the pack, extruded, plus the wheel fairings
/// (`blender-exterior.py`: teardrop pants span x -0.45 .. +0.35 about the axle, 0.135 m half
/// width, and stand from the axle line down to the ground).
fn volumes(def: &Preflight) -> Vec<Volume> {
    let mut out: Vec<Volume> = def
        .keepouts
        .iter()
        .map(|k| {
            let [z0, z1] = band(&k.id);
            Volume {
                id: k.id.clone(),
                min: [k.min_m[0] as f32, k.min_m[1] as f32, z0],
                max: [k.max_m[0] as f32, k.max_m[1] as f32, z1],
            }
        })
        .collect();
    for (id, x, y) in [
        ("wheel_pant_left", -1.959, -0.975),
        ("wheel_pant_right", -1.959, 0.975),
        ("nose_wheel_pant", -0.549, 0.),
    ] {
        out.push(Volume {
            id: id.into(),
            min: [x - 0.45, y - 0.135, 0.45],
            max: [x + 0.35, y + 0.135, GROUND_Z],
        });
    }
    out
}

/// How far a point is inside a volume (0 when outside).
fn depth(v: &Volume, p: [f32; 3]) -> f32 {
    (0..3)
        .map(|i| (p[i] - v.min[i]).min(v.max[i] - p[i]))
        .fold(f32::INFINITY, f32::min)
        .max(0.)
}

fn worst(def: &Preflight, volumes: &[Volume], scene: &flightcraft_aero::walkaround::WalkaroundScene) -> (f32, String) {
    let mut only_figure = scene.clone();
    only_figure.props.clear();
    only_figure.caps.clear();
    only_figure.cues.clear();
    only_figure.fuel_view = None;
    only_figure.dipstick = None;
    let _ = def;
    let mut worst = (0_f32, String::new());
    for primitive in primitives(&only_figure, [0.; 3], 0) {
        let slack = if LIMB_MATERIALS.contains(&primitive.material) {
            REACH_M
        } else if primitive.material == DARK_MATERIAL {
            CAP_M
        } else {
            BODY_M
        };
        for vertex in primitive.vertices {
            for v in volumes {
                let d = depth(v, vertex.position) - slack;
                if d > worst.0 {
                    worst = (d, format!("{} (material {} at {:.2?})", v.id, primitive.material, vertex.position));
                }
            }
        }
    }
    worst
}

fn fixture() -> Preflight {
    serde_json::from_str(include_str!("../../control/tests/fixtures/sling-preflight.json")).unwrap()
}

#[test]
fn no_figure_vertex_enters_a_keep_out_volume_at_any_station_pose() {
    let def = fixture();
    let volumes = volumes(&def);
    let mut session = Session::new().unwrap();
    session
        .begin_preflight_with(
            def.clone(),
            GROUND_Z,
            flightcraft_control::BeginOptions {
                speed: Some(8.),
                ..Default::default()
            },
        )
        .unwrap();
    let mut failures: Vec<String> = Vec::new();
    let mut crouched = 0;
    // The propeller check needs the cabin ignition item done first.
    let mut items: Vec<_> = def.items.iter().collect();
    items.sort_by_key(|i| i.id != "ignition_off");
    for item in items {
        session.run(PreflightPerform { item: item.id.clone() }).unwrap();
        let mut guard = 0;
        let mut clipped = false;
        loop {
            let st = session.run(PreflightAdvance { seconds: 0.03 }).unwrap();
            let figure: &FigureState = &st.scene.figure;
            // Boarding steps through the cabin door by design.
            if !figure.seated && figure.action != FigureAction::Boarding {
                crouched += usize::from(figure.crouch > 0.5);
                let (depth, id) = worst(&def, &volumes, &st.scene);
                if depth > 0. && !clipped {
                    clipped = true;
                    failures.push(format!(
                        "{}: figure {:.3} m inside {id} at ({:.2}, {:.2}) crouch {:.2}",
                        item.id, depth, figure.position_m[0], figure.position_m[1], figure.crouch
                    ));
                }
            }
            if st.queue.is_empty() && st.current_item.is_none() {
                if item.id == "ignition_off" {
                    session.sim_mut().step(240);
                }
                break;
            }
            guard += 1;
            assert!(guard < 4000, "{} never finished", item.id);
        }
    }
    assert!(crouched > 20, "the sweep must include crouched poses ({crouched})");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
