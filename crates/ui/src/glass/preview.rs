//! Captures the actual glass egui meshes and font atlas without a graphics device.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use super::*;
use eframe::egui::{self, ColorImage, Pos2, Rect, TextureId, epaint::Primitive};
use std::collections::BTreeMap;
#[test]
#[ignore = "manual glass display preview"]
fn render_glass_preview() {
    let mut s = tests::flight();
    s.execute(
        "avionics.set",
        serde_json::json!({"setting":{"kind":"heading_bug","degrees":57.}}),
    )
    .unwrap();
    let state = s.glass_state();
    for id in ["glass_pfd", "glass_mfd"] {
        capture(&s, &state, id, 1000, if id == "glass_pfd" { 880 } else { 760 })
            .save(std::env::temp_dir().join(format!("flightcraft-{id}.png")))
            .unwrap();
    }
}
#[test]
#[ignore = "manual integrated autopilot PFD preview"]
fn render_autopilot_glass_preview() {
    let mut s = tests::flight();
    s.execute("assist.off", serde_json::json!({})).unwrap();
    for setting in [
        serde_json::json!({"kind":"heading_bug","degrees":57.}),
        serde_json::json!({"kind":"autopilot","setting":{"kind":"engage","on":true}}),
        serde_json::json!({"kind":"autopilot","setting":{"kind":"lateral","mode":"hdg"}}),
        serde_json::json!({"kind":"autopilot","setting":{"kind":"vertical","mode":"alt"}}),
        serde_json::json!({"kind":"autopilot","setting":{"kind":"altitude","altitude_m":1493.52}}),
    ] {
        s.execute("avionics.set", serde_json::json!({"setting":setting}))
            .unwrap();
    }
    s.run(flightcraft_control::Step { steps: 90 }).unwrap();
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/autopilot");
    std::fs::create_dir_all(&directory).unwrap();
    capture(&s, &s.glass_state(), "glass_pfd", 1000, 880)
        .save(directory.join("glass-pfd.png"))
        .unwrap();
}

fn capture(
    session: &flightcraft_control::Session,
    state: &GlassState,
    id: &str,
    width: u32,
    height: u32,
) -> image::RgbaImage {
    let ctx = egui::Context::default();
    crate::theme::install(&ctx);
    ctx.set_pixels_per_point(1.5);
    let mut textures = BTreeMap::<TextureId, ColorImage>::new();
    let mut output = None;
    for _ in 0..2 {
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
                        use crate::instruments::Skin;
                        let mut display = create(id).unwrap();
                        let input =
                            InstrumentInput::new(session.sim().systems(), session.sim().state().time_s)
                                .with_glass(state)
                                .with_autopilot(session.sim().installed_autopilot())
                                .with_aircraft(session.aircraft_configuration().unwrap());
                        let draw = display.update(input);
                        let rect = crate::instruments::fit_size(ui.max_rect(), draw.size);
                        crate::instruments::scaled(ui.painter(), rect, draw.size, |p| {
                            crate::instruments::SkinPack {
                                id: "original".into(),
                                colors: Vec::new(),
                                face: Vec::new(),
                            }
                            .paint(p, &draw)
                        });
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
