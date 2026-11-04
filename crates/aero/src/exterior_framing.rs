//! Camera framings for the exterior inset, derived from the exterior's `moving_parts` nodes.
//!
//! The inset follows the pilot's hand: move the flaps and it looks at the trailing edge from
//! behind and above; move the ailerons and it looks at the deflecting wing tip; and so on. The
//! positions come from the node transforms of the pack's own model, so any aircraft with an
//! exterior works. All vectors are in the model's body frame (x forward, y right, z down), in
//! metres, relative to the centre of gravity.
use crate::exterior::{ExteriorAsset, ExteriorModel, Motion};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A part of the aircraft the inset can frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FramingTarget {
    /// Three-quarter rear chase of the whole aircraft.
    #[default]
    Default,
    /// Flap trailing edges, from behind and slightly above.
    Flaps,
    /// The deflecting wing tip.
    Ailerons,
    /// Elevator, trim tab, or rudder.
    Tail,
    /// The canopy and cockpit.
    Cockpit,
    /// Wheels, for the parking brake and nosewheel steering.
    Wheels,
}

impl FramingTarget {
    /// The moving-part motions whose nodes this target frames.
    #[must_use]
    pub fn motions(self) -> &'static [Motion] {
        match self {
            Self::Default => &[],
            Self::Flaps => &[Motion::Flaps],
            Self::Ailerons => &[Motion::Aileron],
            Self::Tail => &[Motion::Elevator, Motion::Rudder],
            Self::Cockpit => &[Motion::Slide],
            Self::Wheels => &[Motion::Wheel],
        }
    }
    /// Node names of the exterior's moving parts that this target frames, in file order.
    #[must_use]
    pub fn nodes(self, asset: &ExteriorAsset) -> Vec<String> {
        asset
            .definition
            .moving_parts
            .iter()
            .filter(|p| self.motions().contains(&p.motion))
            .map(|p| p.node.clone())
            .collect()
    }
}

/// An eye position, a point to look at, and a field of view, in the body frame about the CG.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Framing {
    pub eye_m: [f32; 3],
    pub target_m: [f32; 3],
    pub fov_deg: f32,
}

impl Framing {
    /// Linear blend, for easing between framings.
    #[must_use]
    pub fn lerp(self, to: Self, t: f32) -> Self {
        let t = t.clamp(0., 1.);
        let mix = |[ax, ay, az]: [f32; 3], [bx, by, bz]: [f32; 3]| {
            [ax + (bx - ax) * t, ay + (by - ay) * t, az + (bz - az) * t]
        };
        Self {
            eye_m: mix(self.eye_m, to.eye_m),
            target_m: mix(self.target_m, to.target_m),
            fov_deg: self.fov_deg + (to.fov_deg - self.fov_deg) * t,
        }
    }
}

/// Absolute model-frame position of a named node (the translation of its accumulated transform).
#[must_use]
pub fn node_position(model: &ExteriorModel, name: &str) -> Option<[f32; 3]> {
    let index = model.nodes.iter().position(|n| n.name == name)?;
    // Column-major 4x4: the translation is the fourth column of each node transform.
    let mut p = [0_f32; 3];
    let mut at = Some(index);
    let mut depth = 0;
    // Apply each ancestor's affine transform to the point, innermost first.
    while let Some(i) = at {
        let n = model.nodes.get(i)?;
        let m = &n.transform;
        let [c0, c1, c2, c3] = *m;
        let [x, y, z] = p;
        p = [
            c0[0] * x + c1[0] * y + c2[0] * z + c3[0],
            c0[1] * x + c1[1] * y + c2[1] * z + c3[1],
            c0[2] * x + c1[2] * y + c2[2] * z + c3[2],
        ];
        at = n.parent;
        depth += 1;
        if depth > 128 {
            return None;
        }
    }
    Some(p)
}

/// Rotates a body-frame vector into the world frame by a unit attitude quaternion `[w, x, y, z]`.
#[must_use]
pub fn rotate(attitude: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let [w, x, y, z] = attitude;
    let cross = |[ax, ay, az]: [f32; 3], [bx, by, bz]: [f32; 3]| {
        [ay * bz - az * by, az * bx - ax * bz, ax * by - ay * bx]
    };
    let t = cross([x, y, z], v).map(|c| 2. * c);
    let u = cross([x, y, z], t);
    let [vx, vy, vz] = v;
    let [tx, ty, tz] = t;
    let [ux, uy, uz] = u;
    [vx + w * tx + ux, vy + w * ty + uy, vz + w * tz + uz]
}

fn mean(points: &[[f32; 3]]) -> Option<[f32; 3]> {
    if points.is_empty() {
        return None;
    }
    let n = points.len() as f32;
    let total = points
        .iter()
        .fold([0_f32; 3], |[sx, sy, sz], [x, y, z]| [sx + x, sy + y, sz + z]);
    Some(total.map(|v| v / n))
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// The default framing: a three-quarter rear chase, sized from the aircraft's own dimensions.
#[must_use]
pub fn default_framing(asset: &ExteriorAsset) -> Framing {
    let d = &asset.definition;
    Framing {
        eye_m: [-d.length_m * 1.05, d.span_m * 0.62, -d.height_m * 0.95],
        target_m: [-d.length_m * 0.05, 0., 0.],
        fov_deg: 50.,
    }
}

/// The framing for `target`, from the node positions of the aircraft's exterior. `cg_m` is the
/// centre of gravity in pack body coordinates; `None` when the aircraft has no such part.
#[must_use]
pub fn frame(asset: &ExteriorAsset, target: FramingTarget, cg_m: [f32; 3]) -> Option<Framing> {
    if target == FramingTarget::Default {
        return Some(default_framing(asset));
    }
    let model = &asset.lod0;
    let mut points: Vec<[f32; 3]> = target
        .nodes(asset)
        .iter()
        .filter_map(|n| node_position(model, n))
        .map(|p| sub(p, cg_m))
        .collect();
    if target == FramingTarget::Cockpit && points.is_empty() {
        let eye = asset.definition.interior.as_ref()?.pilot_eye_m;
        points.push(sub(eye, cg_m));
    }
    let centre = mean(&points)?;
    let span = asset.definition.span_m;
    let length = asset.definition.length_m;
    // Which side the part is on, so the camera looks at it from outside the aircraft.
    let side = if centre[1] >= 0. { 1. } else { -1. };
    Some(match target {
        // Behind the trailing edge and slightly above, so the flap is seen swinging down.
        FramingTarget::Flaps => Framing {
            eye_m: add(
                centre,
                [
                    -length * 0.42,
                    side * span * 0.16,
                    -asset.definition.height_m * 0.38,
                ],
            ),
            target_m: add(centre, [-0.1, 0., 0.05]),
            fov_deg: 48.,
        },
        // The outboard aileron, from behind and above the wing.
        FramingTarget::Ailerons => {
            let outboard = points
                .iter()
                .copied()
                .max_by(|a, b| a[1].abs().total_cmp(&b[1].abs()))
                .unwrap_or(centre);
            let side = if outboard[1] >= 0. { 1. } else { -1. };
            Framing {
                eye_m: add(
                    outboard,
                    [
                        -length * 0.38,
                        side * span * 0.2,
                        -asset.definition.height_m * 0.4,
                    ],
                ),
                target_m: outboard,
                fov_deg: 46.,
            }
        }
        FramingTarget::Tail => Framing {
            eye_m: add(
                centre,
                [
                    -length * 0.38,
                    side.max(0.4) * span * 0.34,
                    -asset.definition.height_m * 0.2,
                ],
            ),
            target_m: centre,
            fov_deg: 46.,
        },
        FramingTarget::Cockpit => Framing {
            eye_m: add(
                centre,
                [length * 0.12, span * 0.38, -asset.definition.height_m * 0.45],
            ),
            target_m: centre,
            fov_deg: 46.,
        },
        FramingTarget::Wheels => Framing {
            eye_m: add(
                centre,
                [length * 0.28, span * 0.3, -asset.definition.height_m * 0.12],
            ),
            target_m: centre,
            fov_deg: 50.,
        },
        FramingTarget::Default => default_framing(asset),
    })
}
