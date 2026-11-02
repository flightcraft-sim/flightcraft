//! Manual, headless egui panel capture. The same meshes and font atlas as the application.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
#[cfg(test)]
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/region_data.rs"));
use super::*;
use eframe::egui::{ColorImage, TextureId, epaint::Primitive};
use flightcraft_control::Session;

#[test]
#[ignore = "manual panel preview"]
fn render_panel_preview() {
    require_region_data!();
    for compact in [false, true] {
        let mut panels = Vec::new();
        for (width, height) in [(1280_u32, 800.), (1000, 640.)] {
            for name in ["corvid", "larrow"] {
                panels.push(capture(name, width, panel_height(height, compact) as u32));
            }
        }
        let total_height = panels.iter().map(image::RgbaImage::height).sum();
        let mut combined =
            image::RgbaImage::from_pixel(1280, total_height, image::Rgba(t::BACKGROUND.to_array()));
        let mut top = 0;
        for panel in panels {
            for (x, y, p) in panel.enumerate_pixels() {
                combined.put_pixel(x, top + y, *p);
            }
            top += panel.height();
        }
        let filename = if compact {
            "flightcraft-panels-compact.png"
        } else {
            "flightcraft-panels.png"
        };
        combined.save(std::env::temp_dir().join(filename)).unwrap();
    }
}

#[test]
#[ignore = "manual standby panel preview"]
fn render_standby_preview() {
    require_region_data!();
    let mut panels = Vec::new();
    for compact in [false, true] {
        for (width, height) in [(1280, 800.), (1000, 640.)] {
            panels.push(capture(
                "corvid-standby",
                width,
                panel_height(height, compact) as u32,
            ));
        }
    }
    let mut combined = image::RgbaImage::from_pixel(
        1280,
        panels.iter().map(image::RgbaImage::height).sum(),
        image::Rgba(t::BACKGROUND.to_array()),
    );
    let mut top = 0;
    for panel in panels {
        for (x, y, p) in panel.enumerate_pixels() {
            combined.put_pixel(x, top + y, *p);
        }
        top += panel.height();
    }
    combined
        .save(std::env::temp_dir().join("flightcraft-standby-pair.png"))
        .unwrap();
}

fn capture(name: &str, width: u32, height: u32) -> image::RgbaImage {
    let mut session = Session::new().unwrap();
    session
        .execute(
            "aircraft.load",
            serde_json::json!({"source":{"kind":"installed","id":name},"reset":true}),
        )
        .unwrap();
    if name == "corvid-standby" || name == "corvid-glass" {
        let directory =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/terrain/new-york");
        session
            .install_region(flightcraft_control::read_region(&directory).unwrap())
            .unwrap();
        session.execute("sim.reset",serde_json::json!({"world":"new-york","start":{"kind":"air","height_m":1200.,"airspeed_mps":45.,"heading_rad":1.}})).unwrap();
        session
            .execute(
                "autopilot.set",
                serde_json::json!({"altitude_m":1200.,"airspeed_mps":45.,"heading_rad":1.}),
            )
            .unwrap();
        session
            .execute("sim.step", serde_json::json!({"steps":5500}))
            .unwrap();
        session
            .execute(
                "avionics.set",
                if name=="corvid-glass" {serde_json::json!({"setting":{"kind":"glass","setting":{"kind":"direct_to","airport":"KMMU"}}})}else{serde_json::json!({"setting":{"kind":"direct_to","ident":"KMMU"}})},
            )
            .unwrap();
    }
    let glass = session.glass_state();
    let configuration = session.aircraft_configuration().unwrap();
    let panel = &configuration.panel;
    let mut runtime = InstrumentPanel::default();
    rasterize(width, height, |ui| {
        runtime
            .show(
                ui,
                panel,
                InstrumentInput::new(session.sim().systems(), 0.)
                    .with_aircraft(configuration)
                    .with_glass(&glass)
                    .with_autopilot(session.sim().installed_autopilot()),
                0,
            )
            .unwrap();
    })
}

fn rasterize(width: u32, height: u32, mut paint: impl FnMut(&mut egui::Ui)) -> image::RgbaImage {
    let ctx = egui::Context::default();
    t::install(&ctx);
    ctx.set_pixels_per_point(2.);
    let mut output = None;
    // A second frame lets layout settle and includes the real font atlas updates.
    let mut textures = BTreeMap::<TextureId, ColorImage>::new();
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
                    .frame(
                        egui::Frame::new()
                            .fill(t::PANEL)
                            .inner_margin(egui::Margin::symmetric(12, 5)),
                    )
                    .show(ctx, |ui| {
                        paint(ui);
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
    let mut image = image::RgbaImage::from_pixel(pixel_width, pixel_height, image::Rgba(t::PANEL.to_array()));
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

#[test]
#[ignore = "manual autopilot equipment preview"]
fn render_autopilot_previews() {
    use crate::autopilot;
    use flightcraft_sim::equipment::{LateralMode, PanelSetting, VerticalMode};
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/autopilot");
    std::fs::create_dir_all(&directory).unwrap();
    for name in ["leveler", "legacy", "digital"] {
        let mut session = Session::new().unwrap();
        let load = session.aircraft_configuration().unwrap().load.clone();
        session.execute("aircraft.load",serde_json::json!({"variant":"corvid-standard","panel":format!("corvid-{name}"),"load":load,"reset":true})).unwrap();
        session.execute("sim.reset",serde_json::json!({"start":{"kind":"air","height_m":1493.52,"airspeed_mps":40.0,"heading_rad":0.0}})).unwrap();
        session
            .sim_mut()
            .set_autopilot_panel(PanelSetting::Engage { on: true })
            .unwrap();
        if name != "leveler" {
            session
                .sim_mut()
                .set_autopilot_panel(PanelSetting::Lateral {
                    mode: LateralMode::Hdg,
                })
                .unwrap();
            session
                .sim_mut()
                .set_autopilot_panel(PanelSetting::Vertical {
                    mode: VerticalMode::Alt,
                })
                .unwrap();
            session
                .sim_mut()
                .set_autopilot_panel(PanelSetting::Altitude { altitude_m: 1493.52 })
                .unwrap();
        }
        session.sim_mut().step(1);
        let state = session.sim().installed_autopilot().unwrap();
        let mut controller = autopilot::Controller::default();
        let image = rasterize(1000, 180, |ui| {
            controller.show(ui, state, session.sim().systems().controls.heading_bug_rad, false);
        });
        image.save(directory.join(format!("{name}.png"))).unwrap();
    }
}

#[test]
#[ignore = "manual integrated glass panel preview"]
fn render_glass_panel_preview() {
    require_region_data!();
    let mut panels = Vec::new();
    for compact in [false, true] {
        for (width, height) in [(1280, 800.), (1000, 640.)] {
            panels.push(capture(
                "corvid-glass",
                width,
                panel_height(height, compact) as u32,
            ));
        }
    }
    let mut combined = image::RgbaImage::from_pixel(
        1280,
        panels.iter().map(image::RgbaImage::height).sum(),
        image::Rgba(t::BACKGROUND.to_array()),
    );
    let mut top = 0;
    for panel in panels {
        for (x, y, p) in panel.enumerate_pixels() {
            combined.put_pixel(x, top + y, *p);
        }
        top += panel.height();
    }
    combined
        .save(std::env::temp_dir().join("flightcraft-glass-panel.png"))
        .unwrap();
}

#[test]
#[ignore = "manual touch equipment previews"]
fn render_equipment_previews() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/equipment");
    std::fs::create_dir_all(&directory).unwrap();
    let pack_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/packs/equipment-demo");
    let mut session = Session::new().unwrap();
    session
        .install_pack(&flightcraft_control::read_pack(&pack_path).unwrap())
        .unwrap();
    session
        .execute(
            "aircraft.load",
            serde_json::json!({"source":{"kind":"installed","id":"equipment-demonstrator"},"reset":true}),
        )
        .unwrap();
    let terrain = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/terrain/new-york");
    let regional = terrain.join("000.fcht").exists();
    if regional {
        session
            .install_region(flightcraft_control::read_region(&terrain).unwrap())
            .unwrap();
    }
    session.execute("sim.reset",serde_json::json!({"world":if regional { "new-york" } else { "flat" },"start":{"kind":"air","height_m":1200.,"airspeed_mps":45.,"heading_rad":1.}})).unwrap();
    session
        .execute(
            "assist.set",
            serde_json::json!({"altitude_m":1200.,"airspeed_mps":45.,"heading_rad":1.}),
        )
        .unwrap();
    session
        .execute("sim.step", serde_json::json!({"steps":5500}))
        .unwrap();
    if regional {
        session
        .execute(
            "avionics.set",
            serde_json::json!({"setting":{"kind":"glass","setting":{"kind":"direct_to","airport":"KMMU"}}}),
        )
        .unwrap();
    }
    // Acknowledgement latches remain lit after a transient condition clears.
    session
        .execute(
            "equipment.set",
            serde_json::json!({"setting":{"kind":"systems","setting":{"kind":"door","open":true}}}),
        )
        .unwrap();
    session.execute("equipment.set",serde_json::json!({"setting":{"kind":"systems","setting":{"kind":"light","light":"taxi","on":true}}})).unwrap();
    session
        .execute(
            "systems.set",
            serde_json::json!({"setting":{"kind":"load_multiplier","load":"taxi","value":8.}}),
        )
        .unwrap();
    session
        .execute("sim.step", serde_json::json!({"steps":120}))
        .unwrap();
    session
        .execute(
            "systems.set",
            serde_json::json!({"setting":{"kind":"fuel_selector","position":"off"}}),
        )
        .unwrap();
    session
        .execute("sim.step", serde_json::json!({"steps":1}))
        .unwrap();
    session
        .execute(
            "systems.set",
            serde_json::json!({"setting":{"kind":"fuel_selector","position":"left"}}),
        )
        .unwrap();
    session
        .execute(
            "equipment.set",
            serde_json::json!({"setting":{"kind":"systems","setting":{"kind":"key","position":"start"}}}),
        )
        .unwrap();
    session
        .execute("sim.step", serde_json::json!({"steps":250}))
        .unwrap();
    session
        .execute(
            "equipment.set",
            serde_json::json!({"setting":{"kind":"systems","setting":{"kind":"key","position":"on"}}}),
        )
        .unwrap();
    let configuration = session.aircraft_configuration().unwrap().clone();
    for (suffix, width, height) in [("", 1600_u32, 900_u32), ("-compact", 1100, 620)] {
        let state = session.glass_state();
        let mut runtime = InstrumentPanel::default();
        let image = rasterize(width, height, |ui| {
            runtime
                .show(
                    ui,
                    &configuration.panel,
                    InstrumentInput::new(session.sim().systems(), session.sim().readout().time_s)
                        .with_aircraft(&configuration)
                        .with_glass(&state)
                        .with_autopilot(session.sim().installed_autopilot()),
                    session.flight_generation(),
                )
                .unwrap();
            runtime.show_shared_hosts(ui, &mut session);
        });
        image.save(directory.join(format!("panel{suffix}.png"))).unwrap();
        let panel = configuration.panel.equipment.as_ref().unwrap();
        let [pw, ph] = panel.size;
        let scale = ((width as f64 - 24.) / pw).min((height as f64 - 10.) / ph);
        let (left, top) = (
            (width as f64 - pw * scale) * 0.5,
            (height as f64 - ph * scale) * 0.5,
        );
        for (name, id) in [("navigator", "navigator"), ("display", "left_display")] {
            let e = panel.elements.iter().find(|e| e.id == id).unwrap();
            let [x, y] = e.position;
            let [w, h] = e.size;
            image::imageops::crop_imm(
                &image,
                (left + x * scale).round() as u32,
                (top + y * scale).round() as u32,
                (w * scale).round() as u32,
                (h * scale).round() as u32,
            )
            .to_image()
            .save(directory.join(format!("{name}{suffix}.png")))
            .unwrap();
        }
        if std::env::var_os("FLIGHTCRAFT_EXTENDED_PANEL_PREVIEWS").is_some() {
            // Also capture full-height split composition, the procedures page, and the navigator map.
            session.execute("equipment.set",serde_json::json!({"setting":{"kind":"display","id":"left_display","action":{"kind":"layout","layout":"pfd_map"}}})).unwrap();
            for (page, name) in [("procedures", "procedures"), ("map", "navigator-map")] {
                session
                .execute(
                    "equipment.set",
                    serde_json::json!({"setting":{"kind":"navigator","action":{"kind":"page","page":page}}}),
                )
                .unwrap();
                let state = session.glass_state();
                let image = rasterize(width, height, |ui| {
                    runtime
                        .show(
                            ui,
                            &configuration.panel,
                            InstrumentInput::new(session.sim().systems(), 0.)
                                .with_aircraft(&configuration)
                                .with_glass(&state)
                                .with_autopilot(session.sim().installed_autopilot()),
                            session.flight_generation(),
                        )
                        .unwrap();
                    runtime.show_shared_hosts(ui, &mut session);
                });
                image
                    .save(directory.join(format!("panel-{name}{suffix}.png")))
                    .unwrap();
            }
            session.execute("equipment.set",serde_json::json!({"setting":{"kind":"display","id":"left_display","action":{"kind":"layout","layout":"full_pfd"}}})).unwrap();
            session
            .execute(
                "equipment.set",
                serde_json::json!({"setting":{"kind":"navigator","action":{"kind":"page","page":"home"}}}),
            )
            .unwrap();
        }
    }
}
