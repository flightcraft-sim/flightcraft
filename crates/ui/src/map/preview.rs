//! Rasterise the actual map widget, including the font atlas and on-demand chart textures.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/region_data.rs"));
use super::*;
use eframe::egui::{ColorImage, TextureId, epaint::Primitive};
use std::collections::BTreeMap;
#[test]
#[ignore = "manual planning map previews"]
fn render_planning_map_previews() {
    require_region_data!();
    let mut s = tests::planned_session();
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/map");
    std::fs::create_dir_all(&directory).unwrap();
    for (base, name) in [("aero", "aero"), ("sectional", "sectional"), ("tac", "tac")] {
        s.execute("map.set", json!({"base_map":base})).unwrap();
        for (width, height, suffix) in [
            (1200, 1000, ""),
            (1000, 640, "-compact"),
            (1000, 546, "-compact-content"),
        ] {
            capture(&mut s, width, height)
                .save(directory.join(format!("{name}{suffix}.png")))
                .unwrap();
        }
    }
}
fn capture(s: &mut Session, width: u32, height: u32) -> image::RgbaImage {
    let ctx = egui::Context::default();
    crate::theme::install(&ctx);
    let mut widget = PlanningMap::with_root(Some(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/terrain")
            .to_string_lossy()
            .into(),
    ));
    let mut textures = BTreeMap::<TextureId, ColorImage>::new();
    let mut output = None;
    for _ in 0..40 {
        let frame = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    Pos2::ZERO,
                    egui::vec2(width as f32, height as f32),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::new())
                    .show(ctx, |ui| {
                        widget.show(ui, ui.max_rect(), s);
                    });
            },
        );
        for (id, delta) in &frame.textures_delta.set {
            let egui::ImageData::Color(data) = &delta.image;
            if let Some([x, y]) = delta.pos {
                let texture = textures.get_mut(id).unwrap();
                let [tw, _] = texture.size;
                let [dw, dh] = data.size;
                for dy in 0..dh {
                    for dx in 0..dw {
                        texture.pixels[(y + dy) * tw + x + dx] = data.pixels[dy * dw + dx];
                    }
                }
            } else {
                textures.insert(*id, (**data).clone());
            }
        }
        for id in &frame.textures_delta.free {
            textures.remove(id);
        }
        output = Some(frame);
    }
    let output = output.unwrap();
    let scale = output.pixels_per_point;
    let (pixel_width, pixel_height) = ((width as f32 * scale) as u32, (height as f32 * scale) as u32);
    let mut image = image::RgbaImage::from_pixel(
        pixel_width,
        pixel_height,
        image::Rgba(crate::theme::PANEL.to_array()),
    );
    let cross = |a: egui::Vec2, b: egui::Vec2| a.x * b.y - a.y * b.x;
    for clipped in ctx.tessellate(output.shapes, scale) {
        let Primitive::Mesh(mesh) = clipped.primitive else {
            continue;
        };
        let texture = textures.get(&mesh.texture_id).unwrap();
        let [tw, th] = texture.size;
        let clip = Rect::from_min_max(clipped.clip_rect.min * scale, clipped.clip_rect.max * scale);
        for indices in mesh.indices.chunks_exact(3) {
            let [mut a, mut b, mut c] = [
                mesh.vertices[indices[0] as usize],
                mesh.vertices[indices[1] as usize],
                mesh.vertices[indices[2] as usize],
            ];
            a.pos *= scale;
            b.pos *= scale;
            c.pos *= scale;
            let denom = cross(b.pos - a.pos, c.pos - a.pos);
            if denom.abs() < 1e-6 {
                continue;
            }
            let bounds =
                Rect::from_min_max(a.pos.min(b.pos).min(c.pos), a.pos.max(b.pos).max(c.pos)).intersect(clip);
            for y in (bounds.min.y.max(0.) as u32)..(bounds.max.y.ceil() as u32).min(pixel_height) {
                for x in (bounds.min.x.max(0.) as u32)..(bounds.max.x.ceil() as u32).min(pixel_width) {
                    let p = egui::pos2(x as f32 + 0.5, y as f32 + 0.5);
                    let u = cross(b.pos - p, c.pos - p) / denom;
                    let v = cross(c.pos - p, a.pos - p) / denom;
                    let w = 1. - u - v;
                    if u < 0. || v < 0. || w < 0. {
                        continue;
                    }
                    let uv = a.uv.to_vec2() * u + b.uv.to_vec2() * v + c.uv.to_vec2() * w;
                    let tx = (uv.x * tw as f32 - 0.5).clamp(0., (tw - 1) as f32);
                    let ty = (uv.y * th as f32 - 0.5).clamp(0., (th - 1) as f32);
                    let (ix, iy) = (tx as usize, ty as usize);
                    let mut tex = [0_f32; 4];
                    for (dx, dy, weight) in [
                        (0, 0, (1. - tx.fract()) * (1. - ty.fract())),
                        (1, 0, tx.fract() * (1. - ty.fract())),
                        (0, 1, (1. - tx.fract()) * ty.fract()),
                        (1, 1, tx.fract() * ty.fract()),
                    ] {
                        let p = texture.pixels[(iy + dy).min(th - 1) * tw + (ix + dx).min(tw - 1)].to_array();
                        for i in 0..4 {
                            tex[i] += f32::from(p[i]) * weight;
                        }
                    }
                    let (ca, cb, cc) = (a.color.to_array(), b.color.to_array(), c.color.to_array());
                    let mut src = [0_f32; 4];
                    for i in 0..4 {
                        src[i] = (f32::from(ca[i]) * u + f32::from(cb[i]) * v + f32::from(cc[i]) * w)
                            * tex[i]
                            / 255.;
                    }
                    let dst = image.get_pixel_mut(x, y);
                    for i in 0..3 {
                        dst.0[i] =
                            (src[i] + f32::from(dst.0[i]) * (1. - src[3] / 255.)).clamp(0., 255.) as u8;
                    }
                    dst.0[3] = 255;
                }
            }
        }
    }
    image::imageops::resize(&image, width, height, image::imageops::FilterType::Lanczos3)
}
