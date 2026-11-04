//! CPU rasterization of real egui panel meshes and the live font atlas for local comparisons.
use crate::instruments::{InstrumentInput, InstrumentPanel};
use crate::theme as t;
use eframe::egui::{self, ColorImage, Pos2, Rect, TextureId, epaint::Primitive};
use flightcraft_control::Session;
use image::{Rgba, RgbaImage};
use std::collections::BTreeMap;

/// The same complete-panel fit used by equipment artwork, controls, maps, and comparison crops.
pub(crate) fn equipment_rect(panel: &flightcraft_aero::modern::EquipmentPanel, available: Rect) -> Rect {
    crate::instruments::panel_fit(available, panel.size.map(|v| v as f32))
}
/// Transform the authored full canvas so the chosen dash viewport fits the host.
pub(crate) fn dash_rect(panel: &flightcraft_aero::modern::EquipmentPanel, available: Rect) -> Rect {
    let Some(dash) = crate::panel_art::dash_bounds(panel) else {
        return equipment_rect(panel, available);
    };
    let fit = crate::instruments::fit_size(available, [dash.width(), dash.height()]);
    let scale = fit.width() / dash.width();
    Rect::from_min_size(
        fit.min - dash.min.to_vec2() * scale,
        egui::vec2(panel.size[0] as f32, panel.size[1] as f32) * scale,
    )
}
/// Pixel-space [left, top, scale] for crops of a full-window equipment capture.
pub fn equipment_transform(
    panel: &flightcraft_aero::modern::EquipmentPanel,
    width: u32,
    height: u32,
) -> Result<[f64; 3], String> {
    panel.validate().map_err(|e| e.to_string())?;
    if width == 0 || height == 0 {
        return Err("preview dimensions must be positive".into());
    }
    let rect = equipment_rect(
        panel,
        Rect::from_min_size(Pos2::ZERO, egui::vec2(width as f32, height as f32)),
    );
    Ok([
        f64::from(rect.left()),
        f64::from(rect.top()),
        f64::from(rect.width()) / panel.size.first().copied().unwrap_or(1.),
    ])
}

/// Capture installed equipment without advancing simulation time. Bounded to 16 megapixels.
pub fn render_equipment_panel(session: &mut Session, width: u32, height: u32) -> Result<RgbaImage, String> {
    if width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || u64::from(width) * u64::from(height) > 16_777_216
    {
        return Err("preview dimensions exceed limits".into());
    }
    let configuration = session
        .aircraft_configuration()
        .ok_or("no aircraft selected")?
        .clone();
    if configuration.panel.equipment.is_none() {
        return Err("selected panel has no equipment".into());
    }
    let glass = session.glass_state();
    let registration = session.pilot_callsign().registration;
    let mut panel = InstrumentPanel::default();
    let mut error = None;
    let image = rasterize(width, height, |ui| {
        if let Err(e) = panel.show(
            ui,
            &configuration.panel,
            InstrumentInput::new(session.sim().systems(), session.sim().readout().time_s)
                .with_aircraft(&configuration)
                .with_registration(&registration)
                .with_glass(&glass)
                .with_autopilot(session.sim().installed_autopilot()),
            session.flight_generation(),
        ) {
            error = Some(e);
        }
        panel.show_shared_hosts(ui, session);
    })?;
    if let Some(e) = error {
        return Err(e);
    }
    Ok(image)
}
pub(crate) fn rasterize(
    width: u32,
    height: u32,
    mut paint: impl FnMut(&mut egui::Ui),
) -> Result<RgbaImage, String> {
    let ctx = egui::Context::default();
    t::install(&ctx);
    ctx.set_pixels_per_point(1.);
    let mut textures = BTreeMap::<TextureId, ColorImage>::new();
    let mut output = None;
    for _ in 0..3 {
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
                    .frame(egui::Frame::new().fill(t::BACKGROUND).inner_margin(0.))
                    .show(ctx, |ui| paint(ui));
            },
        );
        for (id, delta) in &frame.textures_delta.set {
            let egui::ImageData::Color(data) = &delta.image;
            if let Some([x, y]) = delta.pos {
                let dst = textures.get_mut(id).ok_or("texture update without allocation")?;
                let [tw, _] = dst.size;
                let [dw, dh] = data.size;
                for row in 0..dh {
                    for col in 0..dw {
                        if let (Some(target), Some(source)) = (
                            dst.pixels.get_mut((y + row) * tw + x + col),
                            data.pixels.get(row * dw + col),
                        ) {
                            *target = *source;
                        } else {
                            return Err("invalid texture update".into());
                        }
                    }
                }
            } else {
                textures.insert(*id, (**data).clone());
            }
        }
        output = Some(frame);
    }
    let output = output.ok_or("missing frame")?;
    let mut image = RgbaImage::from_pixel(width, height, Rgba(t::BACKGROUND.to_array()));
    let cross = |a: egui::Vec2, b: egui::Vec2| a.x * b.y - a.y * b.x;
    for clipped in ctx.tessellate(output.shapes, 1.) {
        let Primitive::Mesh(mesh) = clipped.primitive else {
            continue;
        };
        let texture = textures.get(&mesh.texture_id).ok_or("missing mesh texture")?;
        let [tw, th] = texture.size;
        if tw == 0 || th == 0 {
            continue;
        }
        for triangle in mesh.indices.as_chunks::<3>().0.iter() {
            let [ai, bi, ci] = triangle;
            let (Some(a), Some(b), Some(c)) = (
                mesh.vertices.get(*ai as usize),
                mesh.vertices.get(*bi as usize),
                mesh.vertices.get(*ci as usize),
            ) else {
                return Err("invalid triangle".into());
            };
            let denom = cross(b.pos - a.pos, c.pos - a.pos);
            if denom.abs() < 1e-6 {
                continue;
            }
            let bounds = Rect::from_min_max(a.pos.min(b.pos).min(c.pos), a.pos.max(b.pos).max(c.pos))
                .intersect(clipped.clip_rect)
                .intersect(Rect::from_min_size(
                    Pos2::ZERO,
                    egui::vec2(width as f32, height as f32),
                ));
            for y in (bounds.min.y.max(0.).floor() as u32)..(bounds.max.y.ceil() as u32).min(height) {
                for x in (bounds.min.x.max(0.).floor() as u32)..(bounds.max.x.ceil() as u32).min(width) {
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
                    let mut tex = [0_f32; 4];
                    for (dx, dy, weight) in [
                        (0, 0, (1. - tx.fract()) * (1. - ty.fract())),
                        (1, 0, tx.fract() * (1. - ty.fract())),
                        (0, 1, (1. - tx.fract()) * ty.fract()),
                        (1, 1, tx.fract() * ty.fract()),
                    ] {
                        let pixel = texture
                            .pixels
                            .get((ty as usize + dy).min(th - 1) * tw + (tx as usize + dx).min(tw - 1))
                            .ok_or("invalid atlas coordinates")?
                            .to_array();
                        for (value, channel) in tex.iter_mut().zip(pixel) {
                            *value += f32::from(channel) * weight;
                        }
                    }
                    let src: Vec<_> = a
                        .color
                        .to_array()
                        .into_iter()
                        .zip(b.color.to_array())
                        .zip(c.color.to_array())
                        .zip(tex)
                        .map(|(((a, b), c), t)| {
                            (f32::from(a) * u + f32::from(b) * v + f32::from(c) * w) * t / 255.
                        })
                        .collect();
                    let alpha = src.get(3).copied().unwrap_or(0.) / 255.;
                    if let Some(dst) = image.get_pixel_mut_checked(x, y) {
                        for (target, source) in dst.0.iter_mut().take(3).zip(src) {
                            *target = (source + f32::from(*target) * (1. - alpha)).clamp(0., 255.) as u8;
                        }
                    }
                }
            }
        }
    }
    Ok(image)
}
