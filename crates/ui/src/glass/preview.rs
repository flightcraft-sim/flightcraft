//! Captures the actual glass egui meshes and font atlas without a graphics device.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
#[cfg(test)]
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/region_data.rs"));
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
#[ignore = "manual traffic MFD preview"]
fn render_traffic_glass_preview() {
    let mut s = tests::fixture_flight();
    s.execute(
        "traffic.density",
        serde_json::json!({"density":"busy","airport":"KABE"}),
    )
    .unwrap();
    s.run(flightcraft_control::Step { steps: 120 * 20 }).unwrap();
    for setting in [
        flightcraft_control::GlassSetting::MapRange { range_nm: 15. },
        flightcraft_control::GlassSetting::TrafficFilter {
            filter: flightcraft_control::TrafficFilter::Unrestricted,
        },
    ] {
        s.set_glass(setting).unwrap();
    }
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/traffic");
    std::fs::create_dir_all(&directory).unwrap();
    let live = s.glass_state();
    eprintln!(
        "{} targets on the MFD, alert {}",
        live.traffic.len(),
        live.traffic_alert
    );
    capture(&s, &live, "glass_mfd", 1000, 760)
        .save(directory.join("mfd-traffic.png"))
        .unwrap();
    // The same display with the nearest target moved inside the alert volume, to show the alert
    // symbology and the PFD TRAFFIC annunciation. Synthetic: it changes only this snapshot.
    let mut alert = live.clone();
    if let Some(t) = alert.traffic.first_mut() {
        t.class = flightcraft_control::TrafficClass::Alert;
        t.highlighted = true;
    }
    alert.traffic_alert = true;
    capture(&s, &alert, "glass_mfd", 1000, 760)
        .save(directory.join("mfd-traffic-synthetic-alert.png"))
        .unwrap();
    capture(&s, &alert, "glass_pfd", 1000, 880)
        .save(directory.join("pfd-traffic-synthetic-alert.png"))
        .unwrap();
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
                                rgb: Vec::new(),
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
        for indices in mesh.indices.as_chunks::<3>().0.iter() {
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

#[test]
#[ignore = "manual KMMU ILS 23 captured APR/GS PFD preview"]
fn render_procedure_ils_preview() {
    require_region_data!();
    use flightcraft_control::{Session, Step, read_region};
    use serde_json::json;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let world = read_region(&root.join("assets/terrain/new-york")).unwrap();
    let loc = world
        .procedures()
        .unwrap()
        .localizers
        .iter()
        .find(|l| l.airport == "KMMU" && l.runway == "RW23")
        .unwrap();
    let inbound = loc.course_magnetic_deg + loc.magnetic_variation_deg;
    let start = loc.gs_position.unwrap().destination(inbound + 180., 9. * 1852.);
    let r = world.region().unwrap();
    let projection =
        flightcraft_world::geo::LocalProjection::new(r.origin.latitude_deg, r.origin.longitude_deg).unwrap();
    let [n, e] = projection
        .project(start.latitude_deg, start.longitude_deg)
        .unwrap();
    let convergence = flightcraft_sim::avionics::navigation::inverse(
        [start.latitude_deg, start.longitude_deg],
        projection.unproject(n + 10., e).unwrap(),
    )
    .1;
    let heading = (inbound.to_radians() - convergence).rem_euclid(core::f64::consts::TAU);
    let altitude = loc.gs_elevation_m.unwrap()
        + (9. * 1852. - 46. * 40.) * libm::tan(loc.gs_angle_deg.unwrap().to_radians());
    let heading = flightcraft_sim::magnetic::reference_at(&world, [n, e, -altitude], Default::default())
        .unwrap()
        .grid_to_magnetic(heading);
    let height = altitude - world.ground(n, e).height_msl_m;
    let mut session = Session::new().unwrap();
    session
        .execute(
            "aircraft.load",
            json!({"source":{"kind":"installed","id":"corvid-glass"},"reset":true}),
        )
        .unwrap();
    session.install_region(world).unwrap();
    session.execute("sim.reset",json!({"world":"new-york","start":{"kind":"geographic_air","position":start,"height_agl_m":height,"airspeed_mps":40.,"heading_rad":heading}})).unwrap();
    session
        .execute(
            "assist.set",
            json!({"altitude_m":altitude,"heading_rad":heading,"airspeed_mps":40.}),
        )
        .unwrap();
    session.run(Step { steps: 5520 }).unwrap();
    session.execute("assist.off", json!({})).unwrap();
    for setting in [
        json!({"kind":"load_procedure","airport":"KMMU","procedure":"I23-Y","vectors_to_final":true}),
        json!({"kind":"vectors_to_final"}),
        json!({"kind":"cdi_source","source":"nav1"}),
        json!({"kind":"autopilot","setting":{"kind":"engage","on":true}}),
        json!({"kind":"autopilot","setting":{"kind":"vertical","mode":"alt"}}),
        json!({"kind":"autopilot","setting":{"kind":"lateral","mode":"apr"}}),
    ] {
        session
            .execute("avionics.set", json!({"setting":setting}))
            .unwrap();
    }
    session.run(Step { steps: 3600 }).unwrap();
    let ap = session.sim().installed_autopilot().unwrap();
    assert_eq!(
        ap.active_lateral,
        Some(flightcraft_sim::equipment::LateralMode::Apr)
    );
    assert_eq!(
        ap.active_vertical,
        Some(flightcraft_sim::equipment::VerticalMode::Gs)
    );
    let directory = root.join("out/procedures");
    std::fs::create_dir_all(&directory).unwrap();
    capture(&session, &session.glass_state(), "glass_pfd", 1000, 880)
        .save(directory.join("kmmu-ils23-pfd.png"))
        .unwrap();
}
