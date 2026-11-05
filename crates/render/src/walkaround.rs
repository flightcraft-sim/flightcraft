//! The pre-flight figure and its props, drawn with the exterior PBR pipeline.
//!
//! Everything here is generated in code: a stylised low-poly mannequin (capsule limbs, a sphere
//! head, a cap, a hi-vis vest, boots) posed by two-bone inverse kinematics from the plain
//! [`FigureState`] the session supplies, plus covers, chocks, ropes, filler caps, finding cues, and
//! the fuel and dipstick close-ups. No third-party or scanned assets. Positions are body
//! coordinates (x forward, y right, z down) before the centre-of-gravity offset.
use crate::exterior::{AlphaMode, PbrMaterial, PbrPrimitive, PbrVertex};
use flightcraft_aero::preflight::{DefectKind, PropKind};
use flightcraft_aero::walkaround::{FigureState, FuelView, WalkaroundScene};
use glam::Vec3;

pub const MATERIAL_COUNT: usize = 12;
const SKIN: usize = 0;
const VEST: usize = 1;
const TROUSERS: usize = 2;
const DARK: usize = 3;
const RED_FABRIC: usize = 4;
const CHOCK: usize = 5;
const ROPE: usize = 6;
const FUEL: usize = 7;
const GLASS: usize = 8;
const MARKER: usize = 9;
const METAL: usize = 10;
const SHIRT: usize = 11;

/// Figure proportions, metres. A 1.75 m adult.
const THIGH: f32 = 0.44;
const SHIN: f32 = 0.44;
const UPPER_ARM: f32 = 0.30;
const FOREARM: f32 = 0.29;
const HIP_HEIGHT: f32 = 0.92;
const TORSO: f32 = 0.50;
/// Most the torso leans beyond the commanded lean to bring a hand to its target, radians.
const MAX_REACH_LEAN: f32 = 0.35;

fn material(base: [f32; 4], metallic: f32, roughness: f32) -> PbrMaterial {
    PbrMaterial {
        base_color: base,
        metallic,
        roughness,
        ..Default::default()
    }
}

/// The materials the walkaround appends after a model's own.
#[must_use]
pub fn materials() -> Vec<PbrMaterial> {
    let mut v = vec![
        material([0.74, 0.60, 0.50, 1.], 0., 0.75),   // skin
        material([1.0, 0.45, 0.04, 1.], 0., 0.7),     // hi-vis vest
        material([0.22, 0.31, 0.50, 1.], 0., 0.85),   // trousers
        material([0.06, 0.06, 0.07, 1.], 0., 0.6),    // boots, cap, caps
        material([0.80, 0.04, 0.03, 1.], 0., 0.8),    // red cover and streamer
        material([0.95, 0.72, 0.05, 1.], 0., 0.9),    // chock rubber
        material([0.62, 0.50, 0.32, 1.], 0., 0.95),   // rope
        material([0.95, 0.62, 0.10, 0.85], 0., 0.15), // fuel
        material([0.78, 0.90, 1.0, 1.], 0., 0.2),     // tank frame
        PbrMaterial {
            emissive: [1.4, 0.35, 0.05],
            ..material([0.9, 0.25, 0.05, 1.], 0., 0.5)
        }, // finding marker
        material([0.72, 0.73, 0.76, 1.], 0.9, 0.3),   // dipstick
        material([0.18, 0.30, 0.42, 1.], 0., 0.85),   // shirt
    ];
    if let Some(m) = v.get_mut(FUEL) {
        m.alpha_mode = AlphaMode::Blend;
        m.double_sided = true;
    }
    v
}

#[derive(Default)]
struct Builder {
    groups: Vec<Vec<PbrVertex>>,
    cg: Vec3,
}

impl Builder {
    fn new(cg: Vec3) -> Self {
        Self {
            groups: (0..MATERIAL_COUNT).map(|_| Vec::new()).collect(),
            cg,
        }
    }

    fn tri_n(&mut self, m: usize, p: [Vec3; 3], n: [Vec3; 3]) {
        let Some(group) = self.groups.get_mut(m) else {
            return;
        };
        for (p, n) in p.into_iter().zip(n) {
            let reference = if n.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
            let t = n.cross(reference).normalize_or_zero();
            group.push(PbrVertex {
                position: (p - self.cg).to_array(),
                normal: n.to_array(),
                tangent: [t.x, t.y, t.z, 1.],
                uv: [0., 0.],
                color: [1.; 4],
                emissive_scale: 1.,
            });
        }
    }

    /// A triangle whose normal faces away from `inside`, whatever its winding.
    fn tri_out(&mut self, m: usize, a: Vec3, b: Vec3, c: Vec3, inside: Vec3) {
        let mut n = (b - a).cross(c - a).normalize_or_zero();
        if n.dot((a + b + c) / 3. - inside) < 0. {
            n = -n;
        }
        self.tri_n(m, [a, b, c], [n; 3]);
    }

    fn quad_out(&mut self, m: usize, a: Vec3, b: Vec3, c: Vec3, d: Vec3, inside: Vec3) {
        self.tri_out(m, a, b, c, inside);
        self.tri_out(m, a, c, d, inside);
    }

    /// A tapered tube with smooth sides and flat end caps.
    fn tube(&mut self, m: usize, a: Vec3, b: Vec3, ra: f32, rb: f32, sides: usize) {
        let axis = (b - a).normalize_or_zero();
        if axis == Vec3::ZERO {
            return;
        }
        let u = axis
            .cross(if axis.x.abs() < 0.9 { Vec3::X } else { Vec3::Y })
            .normalize();
        let v = axis.cross(u);
        let ring = |i: usize| {
            let t = i as f32 / sides as f32 * std::f32::consts::TAU;
            u * t.cos() + v * t.sin()
        };
        for i in 0..sides {
            let (d0, d1) = (ring(i), ring(i + 1));
            let (a0, a1, b0, b1) = (a + d0 * ra, a + d1 * ra, b + d0 * rb, b + d1 * rb);
            self.tri_n(m, [a0, b0, b1], [d0, d0, d1]);
            self.tri_n(m, [a0, b1, a1], [d0, d1, d1]);
            self.tri_n(m, [a, a1, a0], [-axis; 3]);
            self.tri_n(m, [b, b0, b1], [axis; 3]);
        }
    }

    fn sphere(&mut self, m: usize, c: Vec3, r: f32, rings: usize, sides: usize) {
        let p = |ring: usize, side: usize| {
            let phi = ring as f32 / rings as f32 * std::f32::consts::PI;
            let th = side as f32 / sides as f32 * std::f32::consts::TAU;
            Vec3::new(phi.sin() * th.cos(), phi.sin() * th.sin(), phi.cos())
        };
        for ring in 0..rings {
            for side in 0..sides {
                let (a, b, c2, d) = (
                    p(ring, side),
                    p(ring + 1, side),
                    p(ring + 1, side + 1),
                    p(ring, side + 1),
                );
                self.tri_n(m, [c + a * r, c + b * r, c + c2 * r], [a, b, c2]);
                self.tri_n(m, [c + a * r, c + c2 * r, c + d * r], [a, c2, d]);
            }
        }
    }

    /// A box with edges along `x`, `y`, `z` (unit, orthogonal) around `c`, half sizes `h`.
    fn cuboid(&mut self, m: usize, c: Vec3, axes: [Vec3; 3], h: Vec3) {
        let [x, y, z] = axes;
        let corner = |sx: f32, sy: f32, sz: f32| c + x * (h.x * sx) + y * (h.y * sy) + z * (h.z * sz);
        let faces = [
            ([1., -1., -1.], [1., 1., -1.], [1., 1., 1.], [1., -1., 1.]),
            ([-1., -1., 1.], [-1., 1., 1.], [-1., 1., -1.], [-1., -1., -1.]),
            ([-1., 1., -1.], [-1., 1., 1.], [1., 1., 1.], [1., 1., -1.]),
            ([-1., -1., 1.], [-1., -1., -1.], [1., -1., -1.], [1., -1., 1.]),
            ([-1., -1., 1.], [1., -1., 1.], [1., 1., 1.], [-1., 1., 1.]),
            ([-1., 1., -1.], [1., 1., -1.], [1., -1., -1.], [-1., -1., -1.]),
        ];
        for (a, b, c2, d) in faces {
            self.quad_out(
                m,
                corner(a[0], a[1], a[2]),
                corner(b[0], b[1], b[2]),
                corner(c2[0], c2[1], c2[2]),
                corner(d[0], d[1], d[2]),
                c,
            );
        }
    }

    fn finish(self, base: usize) -> Vec<PbrPrimitive> {
        self.groups
            .into_iter()
            .enumerate()
            .filter(|(_, v)| !v.is_empty())
            .map(|(m, vertices)| PbrPrimitive {
                material: base + m,
                vertices,
            })
            .collect()
    }
}

fn v(p: [f32; 3]) -> Vec3 {
    Vec3::from_array(p)
}

/// Two-bone IK: the middle joint for a root, a target (clamped to reach), and a pole direction.
fn ik(root: Vec3, target: Vec3, l1: f32, l2: f32, pole: Vec3) -> (Vec3, Vec3) {
    let to = target - root;
    let d = to.length().clamp(0.05, (l1 + l2) * 0.999);
    let dir = to.normalize_or_zero();
    let end = root + dir * d;
    let a = (l1 * l1 - l2 * l2 + d * d) / (2. * d);
    let h = (l1 * l1 - a * a).max(0.).sqrt();
    let perp = (pole - dir * pole.dot(dir)).normalize_or_zero();
    (root + dir * a + perp * h, end)
}

const UP: Vec3 = Vec3::NEG_Z;

fn figure(b: &mut Builder, f: &FigureState, ground_z: f32) {
    let h = f.heading_rad;
    let fwd = Vec3::new(h.cos(), h.sin(), 0.);
    let right = Vec3::new(-h.sin(), h.cos(), 0.);
    let pos = v(f.position_m);
    let crouch = f.crouch.clamp(0., 1.);
    let pelvis = if f.seated {
        pos + UP * 0.10
    } else {
        let hip = HIP_HEIGHT * (1. - 0.45 * crouch);
        Vec3::new(pos.x, pos.y, ground_z) + UP * hip - fwd * (0.05 * crouch)
    };
    // Torso leans forward about the pelvis, as far as a hand target needs to be within reach.
    let base_lean = f.lean_rad + crouch * 0.45 + if f.seated { 0.05 } else { 0. };
    let spine_for = |lean: f32| (UP * lean.cos() + fwd * lean.sin()).normalize();
    let reach_gap = |lean: f32| -> f32 {
        let chest = pelvis + spine_for(lean) * TORSO;
        [(-1_f32, f.left_hand_m), (1., f.right_hand_m)]
            .into_iter()
            .filter_map(|(side, hand)| {
                hand.map(|h| {
                    (v(h) - (chest + right * (0.195 * side))).length() - (UPPER_ARM + FOREARM) * 0.93
                })
            })
            .fold(0., f32::max)
    };
    // Leaning further to bring a hand to its target is bounded: past this the arm simply stops
    // at full reach. The session keeps the torso clear of the airframe by choosing `lean_rad`;
    // an unbounded search would lean the body straight through the structure to touch it.
    let mut lean = base_lean;
    if !f.seated {
        while lean < (base_lean + MAX_REACH_LEAN).min(1.2) && reach_gap(lean) > 0. {
            lean += 0.05;
        }
    }
    let spine = spine_for(lean);
    let chest = pelvis + spine * TORSO;
    let neck = pelvis + spine * (TORSO + 0.10);
    let head_dir = match f.gaze_m {
        Some(g) => {
            let to = (v(g) - neck).normalize_or_zero();
            (UP * 0.8 + to * 0.6).normalize()
        }
        None => (UP * 0.9 + fwd * 0.3).normalize(),
    };
    let head = neck + head_dir * 0.13;
    let phase = f.walk_phase;
    let walking = f.walking && !f.seated;

    // Legs.
    for side in [-1_f32, 1.] {
        let hip = pelvis + right * (0.095 * side);
        let (foot, knee_pole) = if f.seated {
            let knee = hip + fwd * 0.42;
            let foot = Vec3::new(knee.x + fwd.x * 0.06, knee.y + fwd.y * 0.06, knee.z + SHIN * 0.98);
            (foot + right * (0.02 * side), fwd)
        } else if walking {
            let s = (phase + if side > 0. { std::f32::consts::PI } else { 0. }).sin();
            let c = (phase + if side > 0. { std::f32::consts::PI } else { 0. }).cos();
            let lift = c.max(0.) * 0.09;
            (
                Vec3::new(pos.x, pos.y, ground_z) + right * (0.10 * side) + fwd * (0.30 * s) + UP * lift,
                fwd,
            )
        } else {
            (
                Vec3::new(pos.x, pos.y, ground_z)
                    + right * ((0.10 + 0.05 * crouch) * side)
                    + fwd * (0.05 * crouch * side),
                fwd,
            )
        };
        let (knee, ankle) = ik(hip, foot, THIGH, SHIN, knee_pole);
        b.tube(TROUSERS, hip, knee, 0.075, 0.062, 8);
        b.tube(TROUSERS, knee, ankle, 0.060, 0.045, 8);
        b.sphere(TROUSERS, knee, 0.063, 4, 8);
        let toe = ankle + fwd * 0.16;
        let heel = ankle - fwd * 0.05;
        let axes = [fwd, right, UP];
        let mid = (toe + heel) * 0.5 + UP * -0.035;
        b.cuboid(DARK, mid, axes, Vec3::new(0.115, 0.05, 0.04));
    }

    // Torso (vest over shirt) and shoulders.
    b.tube(
        TROUSERS,
        pelvis - spine * 0.02,
        pelvis + spine * 0.12,
        0.155,
        0.16,
        10,
    );
    b.tube(VEST, pelvis + spine * 0.10, chest, 0.16, 0.185, 10);
    b.sphere(VEST, chest, 0.185, 4, 10);
    let shoulder_c = chest - spine * 0.04;
    // Head, neck, cap.
    b.tube(SKIN, chest, neck, 0.05, 0.045, 8);
    b.sphere(SKIN, head, 0.098, 6, 10);
    let cap_axes = [fwd, right, UP];
    b.sphere(DARK, head + head_dir * 0.025, 0.104, 4, 10);
    b.cuboid(
        DARK,
        head + fwd * 0.10 + head_dir * 0.02,
        cap_axes,
        Vec3::new(0.05, 0.075, 0.008),
    );

    // Arms.
    for side in [-1_f32, 1.] {
        let shoulder = shoulder_c + right * (0.195 * side);
        let swing = if walking {
            0.20 * (phase + if side > 0. { 0. } else { std::f32::consts::PI }).sin()
        } else {
            0.
        };
        let target = match (if side > 0. { f.right_hand_m } else { f.left_hand_m }, f.seated) {
            (Some(t), _) => v(t),
            (None, true) => pelvis + fwd * 0.35 + right * (0.20 * side) + UP * 0.05,
            (None, false) => shoulder - UP * 0.58 + right * (0.03 * side) + fwd * swing + (spine - UP) * 0.1,
        };
        let pole = (-fwd * 0.4 - UP * 0.2 + right * side * 0.6).normalize();
        let (elbow, hand) = ik(shoulder, target, UPPER_ARM, FOREARM, pole);
        b.sphere(SHIRT, shoulder, 0.062, 4, 8);
        b.tube(SHIRT, shoulder, elbow, 0.052, 0.045, 8);
        b.tube(SKIN, elbow, hand, 0.042, 0.034, 8);
        b.sphere(SKIN, hand, 0.045, 4, 8);
    }
}

fn marker(b: &mut Builder, at: Vec3, kind: DefectKind) {
    // A ring of bright bars around the finding, so it reads from the inset camera.
    let n = 10;
    for i in 0..n {
        let t0 = i as f32 / n as f32 * std::f32::consts::TAU;
        let t1 = (i + 1) as f32 / n as f32 * std::f32::consts::TAU;
        let a = at + Vec3::new(0., t0.cos(), t0.sin()) * 0.11;
        let c = at + Vec3::new(0., t1.cos(), t1.sin()) * 0.11;
        b.tube(MARKER, a, c, 0.006, 0.006, 4);
        let a = at + Vec3::new(t0.cos(), t0.sin(), 0.) * 0.11;
        let c = at + Vec3::new(t1.cos(), t1.sin(), 0.) * 0.11;
        b.tube(MARKER, a, c, 0.006, 0.006, 4);
    }
    match kind {
        DefectKind::WheelFairingCrack => {
            let zig = [
                Vec3::new(0., -0.07, -0.02),
                Vec3::new(0., -0.03, 0.02),
                Vec3::new(0., 0.01, -0.02),
                Vec3::new(0., 0.05, 0.03),
            ];
            for w in zig.windows(2) {
                if let [p, q] = w {
                    b.tube(DARK, at + *p, at + *q, 0.004, 0.004, 4);
                }
            }
        }
        DefectKind::DamagedTyre => {
            b.cuboid(
                DARK,
                at,
                [Vec3::X, Vec3::Y, Vec3::Z],
                Vec3::new(0.012, 0.03, 0.05),
            );
        }
        _ => b.sphere(MARKER, at, 0.012, 3, 6),
    }
}

fn prop(b: &mut Builder, kind: PropKind, at: Vec3, yaw: f32, ground_z: f32) {
    let fwd = Vec3::new(yaw.cos(), yaw.sin(), 0.);
    let right = Vec3::new(-yaw.sin(), yaw.cos(), 0.);
    match kind {
        PropKind::PitotCover => {
            b.tube(RED_FABRIC, at - fwd * 0.11, at + fwd * 0.11, 0.032, 0.026, 8);
            // The remove-before-flight streamer hangs from the back of the cover.
            let top = at - fwd * 0.10;
            b.cuboid(
                RED_FABRIC,
                top - UP * 0.12,
                [fwd, right, UP],
                Vec3::new(0.004, 0.03, 0.12),
            );
        }
        PropKind::Chock => {
            // A wedge: the long slope rises towards the wheel.
            let (l, w, hgt) = (0.15, 0.07, 0.11);
            let base = Vec3::new(at.x, at.y, ground_z);
            let p = |dx: f32, dy: f32, up: f32| base + fwd * dx + right * dy + UP * up;
            let inside = p(l * 0.3, 0., hgt * 0.3);
            b.quad_out(
                CHOCK,
                p(-l, -w, 0.),
                p(l, -w, 0.),
                p(l, w, 0.),
                p(-l, w, 0.),
                inside,
            );
            b.quad_out(
                CHOCK,
                p(-l, -w, 0.),
                p(-l, w, 0.),
                p(l, w, hgt),
                p(l, -w, hgt),
                inside,
            );
            b.quad_out(
                CHOCK,
                p(l, -w, 0.),
                p(l, -w, hgt),
                p(l, w, hgt),
                p(l, w, 0.),
                inside,
            );
            b.tri_out(CHOCK, p(-l, -w, 0.), p(l, -w, hgt), p(l, -w, 0.), inside);
            b.tri_out(CHOCK, p(-l, w, 0.), p(l, w, 0.), p(l, w, hgt), inside);
        }
        PropKind::TieDownRope => {
            let ground = Vec3::new(at.x + 0.18, at.y, ground_z);
            let top = Vec3::new(at.x, at.y, at.z - 0.35);
            b.tube(ROPE, top, ground, 0.009, 0.009, 5);
            b.sphere(ROPE, top, 0.02, 3, 6);
            b.tube(METAL, ground, ground + UP * 0.07, 0.012, 0.012, 6);
        }
    }
}

fn fuel_overlay(b: &mut Builder, view: &FuelView, side: f32) {
    // A cutaway tank hovering beside the open filler: its bottom plate, the fuel on it, and a
    // mark at the level that just covers the whole bottom.
    let cap = v(view.cap_m);
    let c = cap + Vec3::new(0., 0.42 * side, -0.30);
    let (lx, ly, hz) = (0.17, 0.10, 0.12);
    let bottom = c + Vec3::Z * hz;
    let axes = [Vec3::X, Vec3::Y, UP];
    b.cuboid(METAL, bottom + Vec3::Z * 0.004, axes, Vec3::new(lx, ly, 0.004));
    let film = 0.010;
    let full = hz * 2. * 0.9;
    let level = if view.litres < view.covers_bottom_l {
        film * (view.litres / view.covers_bottom_l).max(0.)
    } else {
        film + (full - film)
            * ((view.litres - view.covers_bottom_l) / (view.capacity_l - view.covers_bottom_l).max(1.))
                .clamp(0., 1.)
    };
    // Below the cover level fuel only wets part of the bottom.
    let coverage = if view.litres < view.covers_bottom_l {
        (view.litres / view.covers_bottom_l).sqrt().clamp(0.05, 1.)
    } else {
        1.
    };
    if level > 0.0005 {
        let h = (level * 0.5).max(0.001);
        b.cuboid(
            FUEL,
            bottom + UP * h - Vec3::X * (lx * (1. - coverage)),
            axes,
            Vec3::new(lx * coverage, ly * coverage.sqrt(), h),
        );
    }
    // Corner posts and top frame outline the tank.
    let top = |sx: f32, sy: f32| bottom + Vec3::new(lx * sx, ly * sy, 0.) + UP * (hz * 2.);
    let foot = |sx: f32, sy: f32| bottom + Vec3::new(lx * sx, ly * sy, 0.);
    let corners = [(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)];
    for ((sx, sy), (nx, ny)) in corners.into_iter().zip(corners.into_iter().cycle().skip(1)) {
        b.tube(GLASS, foot(sx, sy), top(sx, sy), 0.004, 0.004, 4);
        b.tube(GLASS, top(sx, sy), top(nx, ny), 0.003, 0.003, 4);
    }
    // The 30-litre line: the level at which fuel just covers the whole bottom.
    let line = bottom + UP * film;
    for sy in [-1_f32, 1.] {
        b.tube(
            MARKER,
            line + Vec3::new(-lx - 0.02, ly * sy, 0.),
            line + Vec3::new(lx + 0.02, ly * sy, 0.),
            0.004,
            0.004,
            4,
        );
    }
}

fn dipstick(b: &mut Builder, at: Vec3, level: f32, side: f32) {
    // Magnified 3x beside the hand: the stick, its MIN and MAX marks, and the oil wetting it.
    let s = 3.;
    let top = at + Vec3::new(0., 0.30 * side, -0.20);
    let len = 0.25 * s;
    let bottom = top - UP * len;
    b.tube(METAL, top, bottom, 0.006 * s, 0.006 * s, 6);
    let along = |t: f32| bottom + UP * (t * len);
    let (min_t, max_t) = (0.22, 0.42);
    let wet = (min_t + level * (max_t - min_t)).clamp(0.02, 0.9);
    b.tube(FUEL, along(0.0), along(wet), 0.0075 * s, 0.0075 * s, 6);
    for (t, m) in [(min_t, RED_FABRIC), (max_t, DARK)] {
        let p = along(t);
        b.tube(
            m,
            p - Vec3::Y * 0.03,
            p + Vec3::Y * 0.03,
            0.0035 * s,
            0.0035 * s,
            4,
        );
    }
}

fn cap(b: &mut Builder, at: Vec3, open: bool) {
    if open {
        b.tube(DARK, at, at + UP * 0.002, 0.045, 0.045, 10);
        // The cap, set beside the filler.
        let rest = at + Vec3::new(-0.16, 0., -0.012);
        b.tube(DARK, rest, rest + UP * 0.02, 0.05, 0.047, 10);
    } else {
        b.tube(DARK, at, at + UP * 0.02, 0.052, 0.05, 10);
        b.tube(METAL, at + UP * 0.02, at + UP * 0.024, 0.036, 0.036, 10);
    }
}

/// PBR primitives for the walkaround, with material indices offset by `base`.
#[must_use]
pub fn primitives(scene: &WalkaroundScene, cg: [f32; 3], base: usize) -> Vec<PbrPrimitive> {
    let mut b = Builder::new(Vec3::from_array(cg));
    let ground = scene.ground_z_m;
    figure(&mut b, &scene.figure, ground);
    for p in &scene.props {
        prop(&mut b, p.kind, v(p.position_m), p.yaw_rad, ground);
    }
    for c in &scene.caps {
        cap(&mut b, v(c.position_m), c.open);
    }
    for c in &scene.cues {
        marker(&mut b, v(c.position_m), c.kind);
    }
    if let Some(view) = &scene.fuel_view {
        let side = if view.cap_m[1] >= 0. { 1. } else { -1. };
        fuel_overlay(&mut b, view, side);
    }
    if let Some(d) = &scene.dipstick {
        let side = if d.position_m[1] >= 0. { 1. } else { -1. };
        dipstick(&mut b, v(d.position_m), d.level, side);
    }
    b.finish(base)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use flightcraft_aero::walkaround::FigureAction;

    fn triangles(prims: &[PbrPrimitive]) -> usize {
        prims.iter().map(|p| p.vertices.len() / 3).sum()
    }

    #[test]
    fn the_standing_figure_is_a_small_well_formed_mesh() {
        let scene = WalkaroundScene {
            figure: FigureState {
                position_m: [-3., -1., 0.85],
                ..Default::default()
            },
            ground_z_m: 0.85,
            ..Default::default()
        };
        let prims = primitives(&scene, [0.; 3], 20);
        let t = triangles(&prims);
        assert!((300..5000).contains(&t), "{t} triangles");
        assert!(
            prims
                .iter()
                .all(|p| p.material >= 20 && p.material < 20 + MATERIAL_COUNT)
        );
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for p in prims.iter().flat_map(|p| &p.vertices) {
            let q = v(p.position);
            assert!(q.is_finite() && v(p.normal).is_finite());
            lo = lo.min(q);
            hi = hi.max(q);
        }
        let height = hi.z.max(0.85) - lo.z;
        assert!((1.6..1.9).contains(&height), "figure is {height} m tall");
    }

    #[test]
    fn crouching_lowers_the_figure_and_hands_reach_their_targets() {
        let mk = |crouch: f32, hand: Option<[f32; 3]>| WalkaroundScene {
            figure: FigureState {
                position_m: [-2.78, -1.3, 0.85],
                crouch,
                right_hand_m: hand,
                action: FigureAction::Crouching,
                ..Default::default()
            },
            ground_z_m: 0.85,
            ..Default::default()
        };
        let top = |s: &WalkaroundScene| {
            primitives(s, [0.; 3], 0)
                .iter()
                .flat_map(|p| &p.vertices)
                .map(|v| v.position[2])
                .fold(f32::MAX, f32::min)
        };
        assert!(top(&mk(0.85, None)) > top(&mk(0., None)) + 0.3);
        // The hand lands on a reachable target.
        let target = [-2.3, -1.0, 0.6];
        let prims = primitives(&mk(0.85, Some(target)), [0.; 3], 0);
        let skin = prims.iter().find(|p| p.material == SKIN).unwrap();
        let near = skin
            .vertices
            .iter()
            .map(|p| (v(p.position) - v(target)).length())
            .fold(f32::MAX, f32::min);
        assert!(near < 0.12, "{near}");
    }

    #[test]
    fn a_hand_held_surface_moves_only_its_own_node_and_the_prop_follows_the_hand() {
        use crate::exterior::{Pose, builtin, pbr_materials, pbr_primitives, transforms};
        use flightcraft_aero::exterior::Motion;
        let asset = builtin("corvid").unwrap();
        let ailerons: Vec<_> = asset
            .definition
            .moving_parts
            .iter()
            .filter(|p| p.motion == Motion::Aileron)
            .map(|p| p.node.clone())
            .collect();
        assert!(ailerons.len() >= 2);
        let neutral = transforms(&asset, &asset.lod0, &Pose::default());
        let mut pose = Pose::default();
        pose.walkaround = Some(std::sync::Arc::new(WalkaroundScene {
            surfaces: vec![flightcraft_aero::walkaround::SurfaceOverride {
                node: ailerons[0].clone(),
                value: 0.8,
            }],
            propeller_phase: Some(1.0),
            ..Default::default()
        }));
        let held = transforms(&asset, &asset.lod0, &pose);
        let index = |name: &str| asset.lod0.nodes.iter().position(|n| n.name == name).unwrap();
        assert_ne!(
            neutral[index(&ailerons[0])],
            held[index(&ailerons[0])],
            "the held aileron moves"
        );
        assert_eq!(
            neutral[index(&ailerons[1])],
            held[index(&ailerons[1])],
            "the other aileron stays"
        );
        let prop = asset
            .definition
            .moving_parts
            .iter()
            .find(|p| p.motion == Motion::Propeller)
            .unwrap();
        assert_ne!(
            neutral[index(&prop.node)],
            held[index(&prop.node)],
            "the hand turns the propeller"
        );
        // The scene's materials follow the model's, and its triangles are appended after them.
        let materials = pbr_materials(&asset.lod0);
        assert_eq!(materials.len(), asset.lod0.materials.len() + MATERIAL_COUNT);
        let base = pbr_primitives(&asset, &asset.lod0, &Pose::default());
        let with = pbr_primitives(&asset, &asset.lod0, &pose);
        assert!(with.len() > base.len(), "a scene always draws its figure");
        assert!(with.iter().all(|p| p.material < materials.len()));
        assert!(
            base.iter().all(|p| p.material < asset.lod0.materials.len()),
            "no scene, no extra geometry"
        );
    }
}
