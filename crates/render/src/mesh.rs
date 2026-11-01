//! Original procedural aircraft meshes and flat training field. No external assets.
use crate::Palette;
use glam::Vec3;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    color: [f32; 4],
    kind: f32,
}

#[derive(Default)]
struct Mesh(Vec<Vertex>);

impl Mesh {
    fn triangle(&mut self, a: Vec3, b: Vec3, c: Vec3, color: [f32; 4], kind: f32) {
        let normal = (b - a).cross(c - a).normalize_or_zero().to_array();
        for position in [a, b, c] {
            self.0.push(Vertex {
                position: position.to_array(),
                normal,
                color,
                kind,
            });
        }
    }

    fn quad(&mut self, corners: [Vec3; 4], color: [f32; 4], kind: f32) {
        let [a, b, c, d] = corners;
        self.triangle(a, b, c, color, kind);
        self.triangle(a, c, d, color, kind);
    }

    fn box_body(&mut self, low: [f32; 3], high: [f32; 3], color: [f32; 4]) {
        let [x, y, z] = low;
        let [xx, yy, zz] = high;
        let a = Vec3::new(x, y, z);
        let b = Vec3::new(xx, y, z);
        let c = Vec3::new(xx, yy, z);
        let d = Vec3::new(x, yy, z);
        let e = Vec3::new(x, y, zz);
        let f = Vec3::new(xx, y, zz);
        let g = Vec3::new(xx, yy, zz);
        let h = Vec3::new(x, yy, zz);
        for face in [
            [a, d, c, b],
            [e, f, g, h],
            [a, b, f, e],
            [d, h, g, c],
            [a, e, h, d],
            [b, c, g, f],
        ] {
            self.quad(face, color, 1.0);
        }
    }

    fn ground(&mut self, origin: [f32; 2], size: [f32; 2], height: f32, color: [f32; 4], kind: f32) {
        let [x, z] = origin;
        let [width, length] = size;
        self.quad(
            [
                Vec3::new(x, height, z),
                Vec3::new(x, height, z - length),
                Vec3::new(x + width, height, z - length),
                Vec3::new(x + width, height, z),
            ],
            color,
            kind,
        );
    }
}

pub(crate) struct Geometry {
    pub vertices: Vec<Vertex>,
    pub world_end: u32,
    pub exterior_end: u32,
    pub cockpit_end: u32,
}

pub(crate) fn build(p: Palette, larrow: bool) -> Geometry {
    let mut m = Mesh::default();
    m.ground([-7000.0, 7000.0], [14000.0, 14000.0], 0.0, p.ground, 0.0);
    m.ground([-13.5, 8.0], [27.0, 916.0], 0.008, p.shadow, 3.0);
    m.ground([-11.5, 0.0], [23.0, 900.0], 0.015, p.runway, 3.0);
    for x in [-11.0, 10.8] {
        m.ground([x, -3.0], [0.2, 894.0], 0.025, p.marking, 3.0);
    }
    for i in 0..28 {
        m.ground(
            [-0.22, -35.0 - i as f32 * 30.0],
            [0.44, 15.0],
            0.026,
            p.marking,
            3.0,
        );
    }
    for x in [-9.5, -7.5, -5.5, -3.5, 2.5, 4.5, 6.5, 8.5] {
        m.ground([x, -6.0], [1.0, 20.0], 0.027, p.marking, 3.0);
        m.ground([x, -874.0], [1.0, 20.0], 0.027, p.marking, 3.0);
    }
    // A pair of aiming point blocks, 150 m from either threshold.
    for x in [-8.0, 5.0] {
        for z in [-150.0, -720.0] {
            m.ground([x, z], [3.0, 24.0], 0.027, p.marking, 3.0);
        }
    }
    // Edge markers make perspective legible even while parked.
    for i in 0..31 {
        for x in [-12.8, 12.8] {
            m.ground([x, -(i as f32) * 30.0], [0.5, 0.7], 0.04, p.accent, 3.0);
        }
    }
    let world_vertices = m.0.len() as u32;
    if larrow {
        return build_larrow(m, p, world_vertices);
    }
    // Body x forward, y right, z down. A tapered, six-section fuselage.
    let sections = [
        (-3.6, 0.10, 0.08, 0.02),
        (-2.3, 0.22, 0.20, 0.03),
        (-0.8, 0.54, 0.48, 0.0),
        (0.75, 0.54, 0.48, 0.0),
        (1.8, 0.36, 0.34, 0.02),
        (2.3, 0.17, 0.22, 0.03),
    ];
    for pair in sections.windows(2) {
        if let [a, b] = pair {
            let ring = |&(x, width, height, z): &(f32, f32, f32, f32)| {
                [
                    Vec3::new(x, -width, z - height),
                    Vec3::new(x, width, z - height),
                    Vec3::new(x, width, z + height),
                    Vec3::new(x, -width, z + height),
                ]
            };
            let [a0, a1, a2, a3] = ring(a);
            let [b0, b1, b2, b3] = ring(b);
            for face in [
                [a0, b0, b1, a1],
                [a1, b1, b2, a2],
                [a2, b2, b3, a3],
                [a3, b3, b0, a0],
            ] {
                m.quad(face, p.aircraft, 1.0);
            }
        }
    }
    m.box_body([2.25, -0.18, -0.20], [2.35, 0.18, 0.20], p.accent);
    // High wing, contrasting tips, and tailplane.
    m.box_body([-0.35, -5.6, -0.74], [1.05, 5.6, -0.60], p.aircraft);
    for (a, b) in [(-5.6, -5.15), (5.15, 5.6)] {
        m.box_body([-0.35, a, -0.76], [1.05, b, -0.59], p.accent);
    }
    m.box_body([-3.25, -1.95, -0.10], [-2.45, 1.95, 0.0], p.aircraft);
    m.quad(
        [
            Vec3::new(-3.5, 0.0, -0.1),
            Vec3::new(-3.4, 0.0, -1.5),
            Vec3::new(-2.9, 0.0, -1.5),
            Vec3::new(-2.35, 0.0, -0.1),
        ],
        p.accent,
        1.0,
    );
    // Cabin glazing, clear of the fuselage faces.
    for y in [-0.548, 0.548] {
        m.quad(
            [
                Vec3::new(-0.55, y, -0.42),
                Vec3::new(0.65, y, -0.42),
                Vec3::new(0.65, y, -0.05),
                Vec3::new(-0.55, y, -0.05),
            ],
            p.glass,
            1.0,
        );
    }
    m.box_body([0.77, -0.47, -0.45], [0.79, 0.47, -0.08], p.glass);
    // Wing struts and fixed tricycle gear.
    for y in [-1.25, 1.25] {
        m.box_body([-0.2, y - 0.04, 0.23], [-0.10, y + 0.04, 0.95], p.rubber);
        m.box_body([-0.4, y - 0.11, 0.77], [0.12, y + 0.11, 1.15], p.rubber);
        m.quad(
            [
                Vec3::new(0.1, y.signum() * 0.5, 0.25),
                Vec3::new(0.18, y.signum() * 0.5, 0.25),
                Vec3::new(0.55, y.signum() * 3.5, -0.6),
                Vec3::new(0.47, y.signum() * 3.5, -0.6),
            ],
            p.shadow,
            1.0,
        );
    }
    m.box_body([1.55, -0.035, 0.2], [1.63, 0.035, 0.98], p.rubber);
    m.box_body([1.36, -0.10, 0.79], [1.85, 0.10, 1.15], p.rubber);
    m.box_body([2.38, -0.065, -1.0], [2.43, 0.065, 1.0], p.rubber);
    let exterior_end = m.0.len() as u32;
    // Original open cabin: a shallow bevelled glare shield, with no replica panel layout.
    m.box_body([0.62, -0.54, -0.19], [1.10, 0.54, -0.13], p.glare_shield);
    m.quad(
        [
            Vec3::new(0.62, -0.54, -0.19),
            Vec3::new(0.62, 0.54, -0.19),
            Vec3::new(0.53, 0.51, -0.10),
            Vec3::new(0.53, -0.51, -0.10),
        ],
        p.cockpit_frame,
        1.0,
    );
    // Angled windscreen posts, side sills, and the overhead wing roots.
    for side in [-1.0_f32, 1.0] {
        let y = side * 0.52;
        let a = Vec3::new(0.94, y, -0.18);
        let b = Vec3::new(0.63, y, -0.68);
        let width = Vec3::new(0.025, 0.0, 0.0);
        let depth = Vec3::new(0.0, 0.018, 0.0);
        for offset in [-depth, depth] {
            m.quad(
                [
                    a - width + offset,
                    a + width + offset,
                    b + width + offset,
                    b - width + offset,
                ],
                p.cockpit_frame,
                1.0,
            );
        }
        m.quad(
            [
                a - width - depth,
                a - width + depth,
                b - width + depth,
                b - width - depth,
            ],
            p.cockpit_frame,
            1.0,
        );
        m.box_body(
            [-0.60, y - 0.025, -0.06],
            [0.95, y + 0.025, 0.02],
            p.cockpit_frame,
        );
        let (lo, hi) = if side < 0.0 { (-5.6, -0.46) } else { (0.46, 5.6) };
        m.box_body([-0.35, lo, -0.74], [1.05, hi, -0.60], p.aircraft);
        // A solid rectangular strut from the lower cabin to the high wing.
        let a = Vec3::new(0.10, side * 0.53, 0.25);
        let b = Vec3::new(0.51, side * 3.5, -0.60);
        let x = Vec3::new(0.035, 0.0, 0.0);
        let z = Vec3::new(0.0, 0.0, 0.025);
        for face in [
            [a - x - z, a + x - z, b + x - z, b - x - z],
            [a - x + z, b - x + z, b + x + z, a + x + z],
            [a - x - z, b - x - z, b - x + z, a - x + z],
            [a + x - z, a + x + z, b + x + z, b + x - z],
        ] {
            m.quad(face, p.cockpit_frame, 1.0);
        }
    }
    m.box_body([0.60, -0.54, -0.70], [0.67, 0.54, -0.65], p.cockpit_frame);
    // Low nose visible beyond the shield; the full exterior cabin would obstruct the eye.
    m.box_body([1.10, -0.32, -0.12], [2.3, 0.32, 0.18], p.aircraft);
    m.box_body([2.27, -0.17, -0.18], [2.36, 0.17, 0.18], p.accent);
    let cockpit_end = m.0.len() as u32;
    // One transparent fan, drawn after opaque geometry without depth writes. No animation clock.
    let center = Vec3::new(2.40, 0.0, 0.0);
    for i in 0..48 {
        let edge = |i: u32| {
            let angle = i as f32 * std::f32::consts::TAU / 48.0;
            center + Vec3::new(0.0, angle.cos(), angle.sin())
        };
        m.triangle(center, edge(i), edge(i + 1), p.propeller, 2.0);
    }
    Geometry {
        vertices: m.0,
        world_end: world_vertices,
        exterior_end,
        cockpit_end,
    }
}

/// Narrow single-seat canopy, low wing, and three wheels at the flight-model contacts.
fn build_larrow(mut m: Mesh, p: Palette, world_end: u32) -> Geometry {
    let sections = [
        (-3.25, 0.06, 0.08),
        (-2.0, 0.17, 0.22),
        (-0.65, 0.36, 0.40),
        (0.65, 0.38, 0.40),
        (1.6, 0.28, 0.30),
        (2.0, 0.12, 0.18),
    ];
    for pair in sections.windows(2) {
        if let [start, end] = pair {
            let ring = |&(x, w, h): &(f32, f32, f32)| {
                [
                    Vec3::new(x, -w, -h),
                    Vec3::new(x, w, -h),
                    Vec3::new(x, w, h),
                    Vec3::new(x, -w, h),
                ]
            };
            let [a, b, c, d] = ring(start);
            let [e, f, g, h] = ring(end);
            for face in [[a, e, f, b], [b, f, g, c], [c, g, h, d], [d, h, e, a]] {
                m.quad(face, p.aircraft, 1.0);
            }
        }
    }
    // 8 m span, 1.375 m mean chord; contrasting tips and aileron strips.
    m.box_body([-0.65, -4.0, 0.12], [0.725, 4.0, 0.22], p.aircraft);
    for (lo, hi) in [(-4.0, -3.65), (3.65, 4.0)] {
        m.box_body([-0.65, lo, 0.10], [0.725, hi, 0.23], p.accent);
    }
    for (lo, hi) in [(-3.8, -2.0), (2.0, 3.8)] {
        m.box_body([-0.67, lo, 0.11], [-0.45, hi, 0.23], p.accent);
    }
    m.box_body([-3.05, -1.35, -0.02], [-2.35, 1.35, 0.07], p.aircraft);
    m.quad(
        [
            Vec3::new(-3.2, 0.0, 0.0),
            Vec3::new(-3.0, 0.0, -1.05),
            Vec3::new(-2.65, 0.0, -1.05),
            Vec3::new(-2.25, 0.0, 0.0),
        ],
        p.accent,
        1.0,
    );
    // Faceted bubble over a single central seat, with its own windscreen.
    for side in [-1.0, 1.0] {
        m.quad(
            [
                Vec3::new(-0.95, side * 0.32, -0.35),
                Vec3::new(0.7, side * 0.32, -0.35),
                Vec3::new(0.35, side * 0.22, -0.82),
                Vec3::new(-0.5, side * 0.22, -0.85),
            ],
            p.glass,
            1.0,
        );
    }
    m.quad(
        [
            Vec3::new(-0.5, -0.22, -0.85),
            Vec3::new(0.35, -0.22, -0.82),
            Vec3::new(0.35, 0.22, -0.82),
            Vec3::new(-0.5, 0.22, -0.85),
        ],
        p.glass,
        1.0,
    );
    m.quad(
        [
            Vec3::new(0.7, -0.32, -0.35),
            Vec3::new(0.7, 0.32, -0.35),
            Vec3::new(0.35, 0.22, -0.82),
            Vec3::new(0.35, -0.22, -0.82),
        ],
        p.glass,
        1.0,
    );
    m.box_body([-0.55, -0.19, -0.55], [-0.40, 0.19, -0.27], p.rubber);
    for y in [-1.05_f32, 1.05] {
        m.quad(
            [
                Vec3::new(0.42, y.signum() * 0.3, 0.28),
                Vec3::new(0.62, y.signum() * 0.3, 0.28),
                Vec3::new(0.62, y, 0.82),
                Vec3::new(0.42, y, 0.82),
            ],
            p.cockpit_frame,
            1.0,
        );
        m.box_body([0.35, y - 0.09, 0.64], [0.75, y + 0.09, 1.0], p.rubber);
    }
    m.box_body([-3.12, -0.035, 0.06], [-3.05, 0.035, 0.24], p.cockpit_frame);
    m.box_body([-3.21, -0.055, 0.14], [-2.99, 0.055, 0.32], p.rubber);
    m.box_body([2.0, -0.12, -0.16], [2.12, 0.12, 0.16], p.accent);
    let exterior_end = m.0.len() as u32;
    // Central pilot, unobstructed low wing, and a small glare shield.
    m.box_body([0.40, -0.32, -0.32], [0.8, 0.32, -0.26], p.glare_shield);
    for y in [-0.32, 0.32] {
        m.box_body([-0.9, y - 0.02, -0.32], [0.7, y + 0.02, -0.27], p.cockpit_frame);
        m.quad(
            [
                Vec3::new(0.68, y, -0.32),
                Vec3::new(0.71, y, -0.32),
                Vec3::new(0.37, y * 0.69, -0.82),
                Vec3::new(0.34, y * 0.69, -0.82),
            ],
            p.cockpit_frame,
            1.0,
        );
    }
    for (lo, hi) in [(-4.0, -0.36), (0.36, 4.0)] {
        m.box_body([-0.65, lo, 0.12], [0.725, hi, 0.22], p.aircraft);
    }
    m.box_body([0.8, -0.27, -0.25], [2.0, 0.27, 0.25], p.aircraft);
    let cockpit_end = m.0.len() as u32;
    let center = Vec3::new(2.13, 0.0, 0.0);
    for i in 0..48 {
        let edge = |i: u32| {
            let angle = i as f32 * std::f32::consts::TAU / 48.0;
            center + Vec3::new(0.0, 0.925 * angle.cos(), 0.925 * angle.sin())
        };
        m.triangle(center, edge(i), edge(i + 1), p.propeller, 2.0);
    }
    Geometry {
        vertices: m.0,
        world_end,
        exterior_end,
        cockpit_end,
    }
}

/// Two immutable, nonindexed LOD meshes per tile, within WebGL 2 buffer limits.
/// Skirts hide cracks where neighbours select different LODs. Tile boundaries remain fixed.
pub(crate) struct TerrainMesh {
    pub vertices: Vec<Vertex>,
    pub fine_end: u32,
    pub center: [f32; 3],
}

pub(crate) fn terrain(tile: &flightcraft_world::HeightTile, p: Palette) -> TerrainMesh {
    use flightcraft_world::Surface;
    let [north, east] = tile.origin_m();
    let extent = tile.extent_m();
    let mut m = Mesh::default();
    let mut fine_end = 0;
    // Coarse cells also sample full-resolution tile normals, retaining slope shading.
    for step in [1_u32, 4] {
        let end = tile.size() - 1;
        let point = |i: u32, j: u32| {
            let n = north + f64::from(i) * tile.spacing_m();
            let e = east + f64::from(j) * tile.spacing_m();
            let ground = tile
                .sample(n, e)
                .unwrap_or_else(|| flightcraft_world::ground(n, e));
            (Vec3::new(e as f32, ground.height_msl_m as f32, -n as f32), ground)
        };
        for j in (0..end).step_by(step as usize) {
            for i in (0..end).step_by(step as usize) {
                let ii = (i + step).min(end);
                let jj = (j + step).min(end);
                let corners = [point(i, j), point(ii, j), point(ii, jj), point(i, jj)];
                let n = north + f64::from(i + ii) * 0.5 * tile.spacing_m();
                let e = east + f64::from(j + jj) * 0.5 * tile.spacing_m();
                let surface = tile.sample(n, e).map(|g| g.surface).unwrap_or(Surface::Grass);
                for corner in [0, 1, 2, 0, 2, 3].into_iter().filter_map(|k| corners.get(k)) {
                    let (position, ground) = corner;
                    let [dn, de, down] = ground.normal_ned.map(|v| v as f32);
                    let blend = ((1.0 + down) * 2.0).clamp(0.0, 1.0);
                    let color = match surface {
                        Surface::Water => p.water,
                        Surface::Pavement => p.runway,
                        Surface::Grass => std::array::from_fn(|k| {
                            p.ground.get(k).copied().unwrap_or(1.0) * (1.0 - blend)
                                + p.earth.get(k).copied().unwrap_or(1.0) * blend
                        }),
                    };
                    m.0.push(Vertex {
                        position: position.to_array(),
                        normal: if surface == Surface::Water {
                            [0.0, 1.0, 0.0]
                        } else {
                            [de, -down, -dn]
                        },
                        color,
                        kind: if surface == Surface::Water { 5.0 } else { 4.0 },
                    });
                }
            }
        }
        for i in (0..end).step_by(step as usize) {
            let next = (i + step).min(end);
            for (a, b) in [
                (point(i, 0).0, point(next, 0).0),
                (point(i, end).0, point(next, end).0),
                (point(0, i).0, point(0, next).0),
                (point(end, i).0, point(end, next).0),
            ] {
                m.quad([a, b, b - Vec3::Y * 100.0, a - Vec3::Y * 100.0], p.ground, 4.0);
            }
        }
        if step == 1 {
            fine_end = m.0.len() as u32;
        }
    }
    TerrainMesh {
        vertices: m.0,
        fine_end,
        center: [(east + extent * 0.5) as f32, 0.0, -(north + extent * 0.5) as f32],
    }
}

#[must_use]
pub(crate) fn near_tile(eye: [f32; 4], center: [f32; 3]) -> bool {
    let [x, y, z, _] = eye;
    let [cx, cy, cz] = center;
    (x - cx) * (x - cx) + (y - cy) * (y - cy) + (z - cz) * (z - cz) < 3000.0 * 3000.0
}

#[cfg(test)]
mod terrain_tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use flightcraft_world::{World, WorldChoice};

    fn palette() -> Palette {
        Palette {
            sky_top: [0.0; 4],
            sky_horizon: [0.0; 4],
            ground: [0.3, 0.4, 0.3, 1.0],
            earth: [0.5, 0.4, 0.2, 1.0],
            water: [0.1, 0.3, 0.6, 1.0],
            grid: [0.0; 4],
            runway: [0.0; 4],
            marking: [0.0; 4],
            aircraft: [0.0; 4],
            accent: [0.0; 4],
            glass: [0.0; 4],
            rubber: [0.0; 4],
            shadow: [0.0; 4],
            cockpit_frame: [0.0; 4],
            glare_shield: [0.0; 4],
            propeller: [0.0; 4],
        }
    }

    #[test]
    fn both_aircraft_keep_the_world_and_their_own_cockpit_ranges() {
        let corvid = build(palette(), false);
        let larrow = build(palette(), true);
        assert_eq!(corvid.world_end, larrow.world_end);
        assert_ne!(corvid.exterior_end, larrow.exterior_end);
        assert_ne!(corvid.cockpit_end, larrow.cockpit_end);
        for (a, b) in corvid
            .vertices
            .iter()
            .zip(&larrow.vertices)
            .take(corvid.world_end as usize)
        {
            assert_eq!(a.position, b.position);
            assert_eq!(a.normal, b.normal);
            assert_eq!(a.color, b.color);
            assert_eq!(a.kind, b.kind);
        }
        for geometry in [corvid, larrow] {
            assert!(geometry.world_end < geometry.exterior_end);
            assert!(geometry.exterior_end < geometry.cockpit_end);
            assert!((geometry.cockpit_end as usize) < geometry.vertices.len());
            for (index, vertex) in geometry
                .vertices
                .iter()
                .enumerate()
                .skip(geometry.world_end as usize)
            {
                assert_eq!(
                    vertex.kind,
                    if index < geometry.cockpit_end as usize {
                        1.0
                    } else {
                        2.0
                    }
                );
            }
        }
    }

    #[test]
    fn lods_fit_webgl_buffers_and_water_stays_flat() {
        let world = World::load(WorldChoice::TrainingValley).unwrap();
        let p = palette();
        let mut water_vertices = 0;
        for tile in world.tiles() {
            let mesh = terrain(tile, p);
            assert!(mesh.fine_end as usize > mesh.vertices.len() - mesh.fine_end as usize);
            assert!(mesh.vertices.len() * std::mem::size_of::<Vertex>() < 16 * 1024 * 1024);
            for vertex in mesh.vertices {
                assert!(
                    vertex
                        .position
                        .into_iter()
                        .chain(vertex.normal)
                        .chain(vertex.color)
                        .all(f32::is_finite)
                );
                if vertex.kind == 5.0 {
                    let [_, height, _] = vertex.position;
                    assert_eq!(height, 35.0);
                    assert_eq!(vertex.normal, [0.0, 1.0, 0.0]);
                    assert_eq!(vertex.color, p.water);
                    water_vertices += 1;
                }
            }
            assert!(near_tile(
                [
                    mesh.center.first().copied().unwrap_or(0.0),
                    100.0,
                    mesh.center.get(2).copied().unwrap_or(0.0),
                    0.0
                ],
                mesh.center
            ));
            assert!(!near_tile([100_000.0; 4], mesh.center));
        }
        assert!(water_vertices > 0);
    }
}
