//! Replace a baked registration in its existing UV atlas; preserve every other PBR channel.
use flightcraft_aero::exterior::{ExteriorAsset, ExteriorModel, ModelNode, RegistrationSurface};
use glam::{Mat4, Vec2, Vec3};
use std::collections::BTreeMap;
use std::sync::Arc;

fn encode(linear: f32) -> u8 {
    let encoded = if linear <= 0.0031308 {
        12.92 * linear
    } else {
        1.055 * linear.powf(1. / 2.4) - 0.055
    };
    (encoded * 255.).round().clamp(0., 255.) as u8
}

/// Match the actual baked paint. A diffuse bake can differ from the source material's
/// nominal color; repainting that nominal color would leave a visible rectangular patch.
fn baked_backgrounds(
    model: &ExteriorModel,
    node: &ModelNode,
    matrix: Mat4,
    surface: &RegistrationSurface,
) -> BTreeMap<usize, [f32; 3]> {
    let expected = surface.background.map(encode);
    let mut counts: BTreeMap<usize, BTreeMap<[u8; 3], usize>> = BTreeMap::new();
    for triangle in node.vertices.as_chunks::<3>().0 {
        let [a, b, c] = triangle;
        if [a, b, c]
            .iter()
            .all(|v| v.normal.get(1).is_none_or(|y| y.abs() < 0.4))
        {
            continue;
        }
        let Some(index) = model
            .materials
            .get(a.material)
            .and_then(|m| m.images.first())
            .copied()
            .flatten()
        else {
            continue;
        };
        let Some(image) = model.images.get(index) else {
            continue;
        };
        let points = [a, b, c].map(|v| matrix.transform_point3(Vec3::from_array(v.position)));
        for weights in [
            [1. / 3.; 3],
            [0.7, 0.15, 0.15],
            [0.15, 0.7, 0.15],
            [0.15, 0.15, 0.7],
        ] {
            let point: Vec3 = points.iter().zip(weights).map(|(p, w)| *p * w).sum();
            if point.x < surface.x_m[0]
                || point.x > surface.x_m[1]
                || point.z < surface.z_m[0]
                || point.z > surface.z_m[1]
            {
                continue;
            }
            let uv: Vec2 = [a, b, c]
                .iter()
                .zip(weights)
                .map(|(v, w)| Vec2::from_array(v.uv) * w)
                .sum();
            if uv.min_element() < 0. || uv.max_element() > 1. {
                continue;
            }
            let [w, h] = image.size;
            let x = ((uv.x * w as f32) as u32).min(w.saturating_sub(1));
            let y = ((uv.y * h as f32) as u32).min(h.saturating_sub(1));
            let offset = (y as usize * w as usize + x as usize) * 4;
            let Some(pixel) = image.rgba.get(offset..offset + 4) else {
                continue;
            };
            let [r, g, b, _] = pixel else { continue };
            let rgb = [*r, *g, *b];
            // Sample only clean paint close to the authored hint, excluding old dark ink.
            if rgb.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 32) {
                *counts.entry(index).or_default().entry(rgb).or_default() += 1;
            }
        }
    }
    counts
        .into_iter()
        .filter_map(|(index, colors)| {
            let (rgb, _) = colors.into_iter().max_by_key(|(_, count)| *count)?;
            Some((
                index,
                rgb.map(|value| {
                    let encoded = f32::from(value) / 255.;
                    if encoded <= 0.04045 {
                        encoded / 12.92
                    } else {
                        ((encoded + 0.055) / 1.055).powf(2.4)
                    }
                }),
            ))
        })
        .collect()
}

pub(super) fn paint(
    base: &ExteriorAsset,
    model: &ExteriorModel,
    text: &str,
    surface: &RegistrationSurface,
) -> ExteriorModel {
    let mut out = model.clone();
    let [left, right] = surface.x_m;
    let [top, bottom] = surface.z_m;
    let count = text.chars().count().clamp(1, 12) as f32;
    let height = surface
        .height_m
        .min((right - left - 0.04) / ((count * 1.25 - 0.25) * 2. / 3.));
    let width = height * 2. / 3.;
    let total = width * (count * 1.25 - 0.25);
    let center = (left + right) / 2.;
    let ink_top = (top + bottom - height) / 2.;
    let radius = height / 12.;
    let mut segments = Vec::new();
    for (i, c) in text.chars().take(12).enumerate() {
        for path in crate::registration::glyph(c) {
            for pair in path.windows(2) {
                if let Some((a, b)) = pair.first().zip(pair.get(1)) {
                    let point = |p: &Vec2| {
                        Vec2::new(
                            i as f32 * width * 1.25 + p.x * width,
                            (p.y - 0.08) * (height - 2. * radius) / 0.84 + radius,
                        )
                    };
                    segments.push((point(a), point(b)));
                }
            }
        }
    }
    if segments.is_empty() {
        return out;
    }
    let matrices = crate::exterior::transforms(base, model, &Default::default());
    let mut changed: BTreeMap<usize, Vec<u8>> = BTreeMap::new();
    for (index, node) in model
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.name == surface.node)
    {
        let matrix = matrices.get(index).copied().unwrap_or(glam::Mat4::IDENTITY);
        let backgrounds = baked_backgrounds(model, node, matrix, surface);
        for triangle in node.vertices.as_chunks::<3>().0 {
            let [a, b, c] = triangle;
            let points = [a, b, c].map(|v| matrix.transform_point3(Vec3::from_array(v.position)));
            let [pa, pb, pc] = points;
            if points.iter().all(|p| p.x < left)
                || points.iter().all(|p| p.x > right)
                || points.iter().all(|p| p.z < top)
                || points.iter().all(|p| p.z > bottom)
            {
                continue;
            }
            if [a, b, c]
                .iter()
                .all(|v| v.normal.get(1).is_none_or(|y| y.abs() < 0.4))
            {
                continue;
            }
            let Some(image_index) = model
                .materials
                .get(a.material)
                .and_then(|m| m.images.first())
                .copied()
                .flatten()
            else {
                continue;
            };
            let Some(image) = model.images.get(image_index) else {
                continue;
            };
            let background = backgrounds.get(&image_index).unwrap_or(&surface.background);
            let [w, h] = image.size;
            let [ua, ub, uc] = [a, b, c].map(|v| Vec2::from_array(v.uv));
            let determinant = (ub - ua).perp_dot(uc - ua);
            if determinant.abs() < 1e-12
                || [ua, ub, uc]
                    .iter()
                    .any(|uv| uv.min_element() < 0. || uv.max_element() > 1.)
            {
                continue;
            }
            let low = ua.min(ub).min(uc);
            let high = ua.max(ub).max(uc);
            let min_x = (low.x * w as f32).floor().max(0.) as u32;
            let max_x = (high.x * w as f32).ceil().min(w as f32) as u32;
            let min_y = (low.y * h as f32).floor().max(0.) as u32;
            let max_y = (high.y * h as f32).ceil().min(h as f32) as u32;
            let derivative_u = ((pb - pa) * (uc.y - ua.y) - (pc - pa) * (ub.y - ua.y)) / determinant;
            let derivative_v = ((pc - pa) * (ub.x - ua.x) - (pb - pa) * (uc.x - ua.x)) / determinant;
            let aa = (0.5 * (derivative_u.length() / w as f32 + derivative_v.length() / h as f32))
                .clamp(0.0005, 0.006);
            let rgba = changed.entry(image_index).or_insert_with(|| image.rgba.to_vec());
            for y in min_y..max_y {
                for x in min_x..max_x {
                    let uv = Vec2::new((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
                    let beta = (uv - ua).perp_dot(uc - ua) / determinant;
                    let gamma = (ub - ua).perp_dot(uv - ua) / determinant;
                    if beta < 0. || gamma < 0. || beta + gamma > 1. {
                        continue;
                    }
                    let p = pa + (pb - pa) * beta + (pc - pa) * gamma;
                    if p.x < left || p.x > right || p.z < top || p.z > bottom {
                        continue;
                    }
                    let local = Vec2::new(
                        if p.y < 0. {
                            center - p.x + total / 2.
                        } else {
                            p.x - center + total / 2.
                        },
                        p.z - ink_top,
                    );
                    let distance = segments
                        .iter()
                        .map(|(a, b)| {
                            let delta = *b - *a;
                            let t =
                                ((local - *a).dot(delta) / delta.length_squared().max(1e-12)).clamp(0., 1.);
                            local.distance(*a + delta * t)
                        })
                        .fold(f32::INFINITY, f32::min);
                    let coverage = ((radius - distance) / aa + 0.5).clamp(0., 1.);
                    let offset = (y as usize * w as usize + x as usize) * 4;
                    if let Some(pixel) = rgba.get_mut(offset..offset + 4) {
                        for (channel, (background, color)) in
                            pixel.iter_mut().take(3).zip(background.iter().zip(surface.color))
                        {
                            let linear = background * (1. - coverage) + color * coverage;
                            *channel = encode(linear);
                        }
                        if let Some(alpha) = pixel.get_mut(3) {
                            *alpha = 255;
                        }
                    }
                }
            }
        }
    }
    for (index, pixels) in changed {
        if let Some(image) = out.images.get_mut(index) {
            image.rgba = Arc::from(pixels);
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use flightcraft_aero::exterior::{ModelImage, ModelNode, ModelVertex};
    use glam::Mat4;

    #[test]
    fn registration_erases_baked_ink_on_both_sides_at_all_lods_and_restores_original() {
        let builtin = crate::exterior::builtin("corvid").unwrap();
        let mut definition = builtin.definition.clone();
        definition.registration.surface = Some(RegistrationSurface {
            node: "airframe".into(),
            x_m: [-4.68, -3.17],
            z_m: [-0.23, 0.10],
            height_m: 0.30,
            background: [0.86, 0.89, 0.90],
            color: [0.015, 0.038, 0.067],
        });
        let mut vertices = Vec::new();
        for (side, low_u, high_u) in [(-1., 0.05, 0.45), (1., 0.55, 0.95)] {
            let vertex = |x, z, u, v| ModelVertex {
                position: [x, side * 0.2, z],
                normal: [0., side, 0.],
                color: [1.; 4],
                uv: [u, v],
                tangent: [1., 0., 0., 1.],
                material: 0,
            };
            let corners = [
                vertex(-4.68, -0.23, low_u, 0.1),
                vertex(-3.17, -0.23, high_u, 0.1),
                vertex(-3.17, 0.10, high_u, 0.9),
                vertex(-4.68, 0.10, low_u, 0.9),
            ];
            for index in [0, 1, 2, 0, 2, 3] {
                vertices.push(corners[index].clone());
            }
        }
        let mut material = builtin.lod0.materials[0].clone();
        material.images = [Some(0), Some(1), Some(1), Some(1), Some(1)];
        let baked_paint = [232, 235, 237, 255]; // Slightly different from the nominal material.
        let mut pixels = baked_paint.repeat(128 * 64);
        for (lo, hi) in [(22, 40), (86, 104)] {
            for y in 15..49 {
                for x in lo..hi {
                    pixels[(y * 128 + x) * 4..][..4].copy_from_slice(&[170, 0, 170, 255]);
                }
            }
        }
        let original_pixels: Arc<[u8]> = Arc::from(pixels);
        let model = Arc::new(ExteriorModel {
            nodes: vec![ModelNode {
                name: "airframe".into(),
                parent: None,
                transform: Mat4::IDENTITY.to_cols_array_2d(),
                vertices,
            }],
            triangles: 4,
            materials: vec![material],
            images: vec![
                ModelImage {
                    size: [128, 64],
                    rgba: original_pixels.clone(),
                },
                ModelImage {
                    size: [1, 1],
                    rgba: Arc::from([128, 128, 255, 255]),
                },
            ],
        });
        let base = Arc::new(ExteriorAsset {
            definition,
            lod0: model.clone(),
            lod1: model.clone(),
            lod2: Some(model),
        });
        let mut scene = crate::Scene::default();
        scene.set_exterior(base.clone());
        scene.set_registration(Some("N824RX"));
        let painted = scene.exterior.clone().unwrap();
        assert_eq!(painted.definition.registration.text, "N824RX");
        for lod in [&painted.lod0, &painted.lod1, painted.lod2.as_ref().unwrap()] {
            assert_eq!(lod.triangles, 4);
            assert_eq!(lod.nodes[0].vertices.len(), 12);
            assert!(Arc::ptr_eq(&lod.images[1].rgba, &base.lod0.images[1].rgba));
            assert_eq!(
                &lod.images[0].rgba[..4],
                &original_pixels[..4],
                "unrelated texels stay intact"
            );
            for (low_x, high_x) in [(7, 57), (71, 121)] {
                let patch: Vec<_> = (7..57)
                    .flat_map(|y| (low_x..high_x).map(move |x| &lod.images[0].rgba[(y * 128 + x) * 4..][..4]))
                    .collect();
                assert!(
                    patch.iter().all(|p| **p != [170, 0, 170, 255]),
                    "old ink is erased"
                );
                assert!(
                    patch.iter().filter(|p| ***p == baked_paint).count() > patch.len() / 3,
                    "erased ink matches the actual baked paint, with no rectangular color change"
                );
                assert!(
                    patch.iter().any(|p| p[0] < 100),
                    "new ink is visible on each side"
                );
            }
        }
        assert!(Arc::ptr_eq(&base.lod0.images[0].rgba, &original_pixels));
        scene.set_exterior(base.clone());
        scene.set_registration(Some("N824RX"));
        assert!(
            Arc::ptr_eq(scene.exterior.as_ref().unwrap(), &painted),
            "override is cached"
        );
        scene.set_registration(Some("G-ABCD"));
        assert_ne!(
            scene.exterior.as_ref().unwrap().lod0.images[0].rgba,
            painted.lod0.images[0].rgba
        );
        scene.set_registration(None);
        assert!(
            Arc::ptr_eq(scene.exterior.as_ref().unwrap(), &base),
            "reset restores baked N371FC"
        );
    }
}
