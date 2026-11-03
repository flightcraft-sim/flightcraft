//! Original stroke-and-arc block lettering. No fonts or external assets.
//! The pack's baked registration supplies both surface patches, color, and letter height.
use flightcraft_aero::exterior::{ExteriorAsset, ExteriorModel, ModelVertex};
use glam::{Vec2, Vec3};
use std::sync::Arc;

pub(crate) struct Override {
    pub base: Arc<ExteriorAsset>,
    pub text: String,
    pub painted: Arc<ExteriorAsset>,
}

pub(crate) fn paint(base: &ExteriorAsset, text: &str) -> Arc<ExteriorAsset> {
    let mut definition = base.definition.clone();
    definition.registration.text = text.into();
    let paint = |model: &ExteriorModel| {
        if let Some(surface) = &base.definition.registration.surface {
            crate::registration_atlas::paint(base, model, text, surface)
        } else {
            paint_model(model, &base.definition.registration.node, text)
        }
    };
    Arc::new(ExteriorAsset {
        definition,
        lod0: Arc::new(paint(&base.lod0)),
        lod1: Arc::new(paint(&base.lod1)),
        lod2: base.lod2.as_ref().map(|model| Arc::new(paint(model))),
    })
}

fn paint_model(model: &ExteriorModel, name: &str, text: &str) -> ExteriorModel {
    let mut out = model.clone();
    for node in out.nodes.iter_mut().filter(|n| n.name == name) {
        let mut lettering = Vec::new();
        for sign in [-1., 1.] {
            let patch: Vec<_> = node
                .vertices
                .iter()
                .filter(|v| {
                    v.position.get(1).is_some_and(|y| y * sign > 0.)
                        && v.normal.get(1).is_some_and(|y| y.abs() > 0.1)
                })
                .collect();
            let Some(sample) = patch.first() else { continue };
            let (low, high) = patch.iter().fold(
                (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
                |(lo, hi), v| {
                    let p = Vec3::from_array(v.position);
                    (lo.min(p), hi.max(p))
                },
            );
            let height = high.z - low.z;
            let count = text.chars().count().max(1) as f32;
            let width = (height * 0.6).min((high.x - low.x) / (count * 1.25 - 0.25));
            if height <= 0. || width <= 0. {
                continue;
            }
            let total = width * (count * 1.25 - 0.25);
            let center = (low.x + high.x) * 0.5;
            let surface = |p: Vec2| {
                let x = center + sign * (p.x - total * 0.5);
                let z = low.z + p.y;
                let nearest = patch
                    .iter()
                    .min_by(|a, b| {
                        let distance = |v: &&ModelVertex| {
                            (v.position.first().copied().unwrap_or(0.) - x).powi(2)
                                + (v.position.get(2).copied().unwrap_or(0.) - z).powi(2)
                        };
                        distance(a).total_cmp(&distance(b))
                    })
                    .copied()
                    .unwrap_or(sample);
                let [vx, vy, vz] = nearest.position;
                let [nx, ny, nz] = nearest.normal;
                let y = vy - (nx * (x - vx) + nz * (z - vz)) / ny + sign * 0.001;
                ModelVertex {
                    position: [x, y, z],
                    normal: nearest.normal,
                    color: sample.color,
                    uv: nearest.uv,
                    tangent: nearest.tangent,
                    material: sample.material,
                }
            };
            let mut flat = Vec::new();
            for (i, c) in text.chars().take(12).enumerate() {
                for path in glyph(c) {
                    for pair in path.windows(2) {
                        let Some((a, b)) = pair.first().zip(pair.get(1)) else {
                            continue;
                        };
                        let a = Vec2::new(i as f32 * width * 1.25 + a.x * width, a.y * height);
                        let b = Vec2::new(i as f32 * width * 1.25 + b.x * width, b.y * height);
                        let delta = b - a;
                        let perpendicular = Vec2::new(-delta.y, delta.x).normalize_or_zero() * width * 0.075;
                        // Long planar quads would cut through the curved fuselage. Sample
                        // short strips against the baked surface, including straight stems.
                        let steps = (delta.length() / 0.012).ceil().clamp(1., 64.) as u32;
                        for step in 0..steps {
                            let low = a + delta * (step as f32 / steps as f32);
                            let high = a + delta * ((step + 1) as f32 / steps as f32);
                            let corners = [
                                low - perpendicular,
                                high - perpendicular,
                                high + perpendicular,
                                low + perpendicular,
                            ];
                            for index in [0, 1, 2, 0, 2, 3] {
                                if let Some(p) = corners.get(index) {
                                    flat.push(*p);
                                }
                            }
                        }
                    }
                }
            }
            // Fit the ink's actual bounds to the baked letter height, including stroke width.
            let top = flat.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
            let bottom = flat.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max);
            if bottom > top {
                lettering.extend(
                    flat.into_iter()
                        .map(|p| surface(Vec2::new(p.x, (p.y - top) * height / (bottom - top)))),
                );
            }
        }
        // Keep the same bounded GPU allocation used by loaded packs.
        if !lettering.is_empty() && lettering.len() <= 12_000 {
            node.vertices = lettering;
        }
    }
    let vertices = out.nodes.iter().map(|n| n.vertices.len()).sum::<usize>();
    if vertices > flightcraft_aero::exterior::MAX_VERTICES {
        return model.clone();
    }
    out.triangles = vertices / 3;
    out
}

fn line(points: &[[f32; 2]]) -> Vec<Vec2> {
    points.iter().copied().map(Vec2::from_array).collect()
}
fn arc(center: [f32; 2], radius: [f32; 2], start: f32, end: f32) -> Vec<Vec2> {
    (0..=12)
        .map(|i| {
            let angle = (start + (end - start) * i as f32 / 12.).to_radians();
            Vec2::from_array(center) + Vec2::new(angle.cos(), angle.sin()) * Vec2::from_array(radius)
        })
        .collect()
}

/// Coordinates run left-to-right and top-to-bottom, with inset strokes at 0.08 and 0.92.
pub(super) fn glyph(c: char) -> Vec<Vec<Vec2>> {
    let mut paths = Vec::new();
    let strokes: &[&[[f32; 2]]] = match c {
        'A' => &[
            &[[0.08, 0.92], [0.5, 0.08], [0.92, 0.92]],
            &[[0.25, 0.58], [0.75, 0.58]],
        ],
        'B' => &[
            &[[0.08, 0.92], [0.08, 0.08], [0.5, 0.08]],
            &[[0.08, 0.5], [0.5, 0.5]],
            &[[0.08, 0.92], [0.5, 0.92]],
        ],
        'D' => &[
            &[[0.08, 0.92], [0.08, 0.08], [0.4, 0.08]],
            &[[0.08, 0.92], [0.4, 0.92]],
        ],
        'E' => &[
            &[[0.92, 0.08], [0.08, 0.08], [0.08, 0.92], [0.92, 0.92]],
            &[[0.08, 0.5], [0.75, 0.5]],
        ],
        'F' => &[
            &[[0.92, 0.08], [0.08, 0.08], [0.08, 0.92]],
            &[[0.08, 0.5], [0.75, 0.5]],
        ],
        'G' => &[&[[0.92, 0.5], [0.55, 0.5]], &[[0.92, 0.5], [0.92, 0.82]]],
        'H' => &[
            &[[0.08, 0.08], [0.08, 0.92]],
            &[[0.92, 0.08], [0.92, 0.92]],
            &[[0.08, 0.5], [0.92, 0.5]],
        ],
        'I' => &[
            &[[0.08, 0.08], [0.92, 0.08]],
            &[[0.5, 0.08], [0.5, 0.92]],
            &[[0.08, 0.92], [0.92, 0.92]],
        ],
        'J' => &[&[[0.08, 0.08], [0.92, 0.08], [0.92, 0.7]]],
        'K' => &[
            &[[0.08, 0.08], [0.08, 0.92]],
            &[[0.92, 0.08], [0.08, 0.5], [0.92, 0.92]],
        ],
        'L' => &[&[[0.08, 0.08], [0.08, 0.92], [0.92, 0.92]]],
        'M' => &[&[
            [0.08, 0.92],
            [0.08, 0.08],
            [0.5, 0.55],
            [0.92, 0.08],
            [0.92, 0.92],
        ]],
        'N' => &[&[[0.08, 0.92], [0.08, 0.08], [0.92, 0.92], [0.92, 0.08]]],
        'P' | 'R' => &[
            &[[0.08, 0.92], [0.08, 0.08], [0.5, 0.08]],
            &[[0.08, 0.5], [0.5, 0.5]],
        ],
        'Q' => &[&[[0.55, 0.65], [0.98, 0.98]]],
        'T' => &[&[[0.08, 0.08], [0.92, 0.08]], &[[0.5, 0.08], [0.5, 0.92]]],
        'U' => &[&[[0.08, 0.08], [0.08, 0.7]], &[[0.92, 0.08], [0.92, 0.7]]],
        'V' => &[&[[0.08, 0.08], [0.5, 0.92], [0.92, 0.08]]],
        'W' => &[&[
            [0.08, 0.08],
            [0.25, 0.92],
            [0.5, 0.48],
            [0.75, 0.92],
            [0.92, 0.08],
        ]],
        'X' => &[&[[0.08, 0.08], [0.92, 0.92]], &[[0.08, 0.92], [0.92, 0.08]]],
        'Y' => &[
            &[[0.08, 0.08], [0.5, 0.5], [0.92, 0.08]],
            &[[0.5, 0.5], [0.5, 0.92]],
        ],
        'Z' => &[&[[0.08, 0.08], [0.92, 0.08], [0.08, 0.92], [0.92, 0.92]]],
        '2' => &[&[[0.92, 0.29], [0.08, 0.92], [0.92, 0.92]]],
        '0' => &[&[[0.25, 0.8], [0.75, 0.2]]],
        '1' => &[
            &[[0.2, 0.25], [0.5, 0.08], [0.5, 0.92]],
            &[[0.15, 0.92], [0.85, 0.92]],
        ],
        '4' => &[&[[0.7, 0.92], [0.7, 0.08], [0.08, 0.65], [0.92, 0.65]]],
        '5' => &[&[[0.92, 0.08], [0.08, 0.08], [0.08, 0.5], [0.5, 0.5]]],
        '7' => &[&[[0.08, 0.08], [0.92, 0.08], [0.3, 0.92]]],
        '-' => &[&[[0.2, 0.5], [0.8, 0.5]]],
        _ => &[],
    };
    paths.extend(strokes.iter().map(|p| line(p)));
    match c {
        'B' => {
            paths.push(arc([0.5, 0.29], [0.42, 0.21], -90., 90.));
            paths.push(arc([0.5, 0.71], [0.42, 0.21], -90., 90.));
        }
        'C' | 'G' => paths.push(arc([0.5, 0.5], [0.42, 0.42], 40., 320.)),
        'D' => paths.push(arc([0.4, 0.5], [0.52, 0.42], -90., 90.)),
        'J' | 'U' => paths.push(arc([0.5, 0.7], [0.42, 0.22], 0., 180.)),
        'O' | 'Q' | '0' => paths.push(arc([0.5, 0.5], [0.42, 0.42], 0., 360.)),
        'P' | 'R' => {
            paths.push(arc([0.5, 0.29], [0.42, 0.21], -90., 90.));
            if c == 'R' {
                paths.push(line(&[[0.45, 0.5], [0.92, 0.92]]));
            }
        }
        'S' => {
            paths.push(arc([0.5, 0.29], [0.42, 0.21], -30., -270.));
            paths.push(arc([0.5, 0.71], [0.42, 0.21], -90., 150.));
        }
        '2' => paths.push(arc([0.5, 0.29], [0.42, 0.21], 180., 360.)),
        '3' => {
            paths.push(arc([0.4, 0.29], [0.52, 0.21], -130., 90.));
            paths.push(arc([0.4, 0.71], [0.52, 0.21], -90., 130.));
        }
        '5' => paths.push(arc([0.5, 0.71], [0.42, 0.21], -90., 160.)),
        '6' => {
            paths.push(arc([0.5, 0.71], [0.42, 0.21], 0., 360.));
            paths.push(arc([0.5, 0.5], [0.42, 0.42], 180., 300.));
        }
        '8' => {
            paths.push(arc([0.5, 0.29], [0.42, 0.21], 0., 360.));
            paths.push(arc([0.5, 0.71], [0.42, 0.21], 0., 360.));
        }
        '9' => {
            paths.push(arc([0.5, 0.29], [0.42, 0.21], 0., 360.));
            paths.push(arc([0.5, 0.5], [0.42, 0.42], 0., 120.));
        }
        _ => {}
    }
    paths
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn every_original_glyph_is_distinct_and_bounded() {
        let mut previous = Vec::new();
        for c in "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-".chars() {
            let paths = glyph(c);
            assert!(!paths.is_empty(), "{c}");
            assert!(
                paths
                    .iter()
                    .flatten()
                    .all(|p| p.is_finite() && p.min_element() >= 0. && p.max_element() <= 1.)
            );
            assert!(
                !previous.contains(&paths),
                "glyph {c} duplicates an earlier character"
            );
            previous.push(paths);
        }
    }

    #[test]
    fn registration_replaces_only_its_node_at_both_lods_and_restores_original() {
        for name in ["corvid", "corvid-standby", "corvid-glass", "larrow"] {
            let asset = crate::exterior::builtin(name).unwrap();
            let painted = paint(&asset, "G-IO12345678");
            for (base, new) in [(&asset.lod0, &painted.lod0), (&asset.lod1, &painted.lod1)] {
                assert!(new.triangles * 3 <= flightcraft_aero::exterior::MAX_VERTICES);
                for (before, after) in base.nodes.iter().zip(&new.nodes) {
                    assert_eq!(before.transform, after.transform);
                    if before.name == asset.definition.registration.node {
                        for sign in [-1., 1.] {
                            let patch: Vec<_> = after
                                .vertices
                                .iter()
                                .filter(|v| v.position[1] * sign > 0. && v.normal[1].abs() > 0.1)
                                .collect();
                            assert!(patch.len() > 100);
                            assert!(patch.iter().all(|v| v.position.iter().all(|x| x.is_finite())));
                            assert!(patch.iter().all(|v| v.color == before.vertices[0].color));
                            let bounds = |vertices: Vec<&ModelVertex>| {
                                let top = vertices
                                    .iter()
                                    .map(|v| v.position[2])
                                    .fold(f32::INFINITY, f32::min);
                                let bottom = vertices
                                    .iter()
                                    .map(|v| v.position[2])
                                    .fold(f32::NEG_INFINITY, f32::max);
                                (top, bottom)
                            };
                            let (top, bottom) = bounds(patch);
                            let (old_top, old_bottom) = bounds(
                                before
                                    .vertices
                                    .iter()
                                    .filter(|v| v.position[1] * sign > 0. && v.normal[1].abs() > 0.1)
                                    .collect(),
                            );
                            assert!(
                                (top - old_top).abs() < 0.0001 && (bottom - old_bottom).abs() < 0.0001,
                                "pack letter height is preserved"
                            );
                        }
                    } else {
                        assert_eq!(before.vertices.len(), after.vertices.len());
                        assert!(
                            before
                                .vertices
                                .iter()
                                .zip(&after.vertices)
                                .all(|(a, b)| a.position == b.position && a.color == b.color)
                        );
                    }
                }
                let pose = crate::exterior::Pose::default();
                assert!(!crate::exterior::frame(&painted, &pose, 200.).opaque.is_empty());
            }
            let mut scene = crate::Scene::default();
            scene.set_aircraft(name);
            assert!(scene.exterior.is_some());
            scene.set_exterior(asset.clone());
            scene.set_registration(Some("N371FC"));
            let first = scene.exterior.clone().unwrap();
            scene.set_exterior(asset.clone());
            scene.set_registration(Some("N371FC"));
            assert!(
                Arc::ptr_eq(&first, scene.exterior.as_ref().unwrap()),
                "same override is cached"
            );
            scene.set_registration(None);
            assert!(
                Arc::ptr_eq(&asset, scene.exterior.as_ref().unwrap()),
                "default preserves baked geometry"
            );
        }
    }
}
