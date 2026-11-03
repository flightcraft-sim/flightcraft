//! Opt-in GPU smoke test for the shared app and its real paint callback, without a window server.
#![cfg(not(target_arch = "wasm32"))]

use eframe::{App, egui};
use std::sync::{Arc, mpsc};

#[test]
#[ignore = "requires a Vulkan software or hardware adapter"]
fn app_renders_with_downlevel_limits() -> TestResult {
    pollster::block_on(render(Default::default()))
}

#[test]
#[ignore = "requires a Vulkan software or hardware adapter"]
fn region_renders_at_elevation_with_active_attribution() -> TestResult {
    // Synthetic source data only; exercise the same portable manifest loader as the browser.
    let mut region = flightcraft_control::Region::from_json(
        r#"{
        "id":"synthetic-field", "name":"Synthetic elevation field",
        "origin":{"latitude_deg":37.5,"longitude_deg":-122.5,"field_elevation_m":750.0,
            "runway_heading_deg":75.0,"runway_length_m":900.0,"runway_width_m":23.0},
        "southwest_m":[-2000.0,-2000.0],"tiles_north":2,"tiles_east":2,
        "spacing_m":31.25,"tile_size":65,"runway_blend_m":200.0,
        "source":"SRTM","licence":"public domain",
        "attribution":"NASA Shuttle Radar Topography Mission, public domain; distributed by USGS"
    }"#,
    )?;
    let mut tiles = Vec::new();
    for e in 0..2 {
        for n in 0..2 {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(b"FCHT");
            bytes.extend_from_slice(&1_u16.to_le_bytes());
            bytes.extend_from_slice(&0_u16.to_le_bytes());
            for coordinate in [-2000.0 + 2000.0 * f64::from(n), -2000.0 + 2000.0 * f64::from(e)] {
                bytes.extend_from_slice(&coordinate.to_le_bytes());
            }
            bytes.extend_from_slice(&31.25_f32.to_le_bytes());
            bytes.extend_from_slice(&65_u32.to_le_bytes());
            for _ in 0..65 * 65 {
                bytes.extend_from_slice(&750_f32.to_le_bytes());
            }
            bytes.extend(vec![0; 64 * 64]);
            region.tiles.push(format!("{:03}.fcht", tiles.len()));
            tiles.push(bytes);
        }
    }
    let world = flightcraft_control::region_world(region, tiles)?;
    let options = flightcraft_ui::AppOptions {
        regions: flightcraft_ui::RegionOptions {
            fetched: vec![world],
            ..Default::default()
        },
        script: Some(flightcraft_ui::script::Script::parse(
            r#"{"at_s":0,"command":"sim.reset","input":{"world":"synthetic-field","start":{"kind":"runway"},"paused":true}}"#,
        )?),
        ..Default::default()
    };
    pollster::block_on(render(options))
}

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

async fn render(options: flightcraft_ui::AppOptions) -> TestResult {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..Default::default()
    });
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .await?;
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            required_limits: wgpu::Limits::downlevel_webgl2_defaults(),
            ..Default::default()
        })
        .await?;
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let renderer = egui_wgpu::Renderer::new(&device, format, Default::default());
    let state = egui_wgpu::RenderState {
        adapter,
        available_adapters: Vec::new(),
        device,
        queue,
        target_format: format,
        renderer: Arc::new(egui::mutex::RwLock::new(renderer)),
    };
    let ctx = egui::Context::default();
    let mut cc = eframe::CreationContext::_new_kittest(ctx.clone());
    cc.wgpu_render_state = Some(state.clone());
    let mut app = flightcraft_ui::FlightCraftApp::new(&cc, options)?;
    let mut frame = eframe::Frame::_new_kittest();
    let width = 1280;
    let height = 800;
    let texture = state.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("app smoke test"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let screen = egui_wgpu::ScreenDescriptor {
        size_in_pixels: [width, height],
        pixels_per_point: 1.0,
    };
    // Warm layout, fonts, and the simulation for the same duration as the desktop capture.
    for n in 0..120 {
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width as f32, height as f32),
                )),
                time: Some(f64::from(n) / 60.0),
                focused: true,
                ..Default::default()
            },
            |ctx| app.update(ctx, &mut frame),
        );
        let jobs = ctx.tessellate(output.shapes, output.pixels_per_point);
        let mut renderer = state.renderer.write();
        for (id, delta) in &output.textures_delta.set {
            renderer.update_texture(&state.device, &state.queue, *id, delta);
        }
        let mut encoder = state.device.create_command_encoder(&Default::default());
        let commands = renderer.update_buffers(&state.device, &state.queue, &mut encoder, &jobs, &screen);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            renderer.render(&mut pass, &jobs, &screen);
        }
        state.queue.submit(commands.into_iter().chain([encoder.finish()]));
        for id in &output.textures_delta.free {
            renderer.free_texture(id);
        }
    }
    let buffer = state.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("smoke test readback"),
        size: u64::from(width * height * 4),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = state.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    state.queue.submit([encoder.finish()]);
    let (tx, rx) = mpsc::channel();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    state.device.poll(wgpu::PollType::wait_indefinitely())?;
    rx.recv()??;
    let bytes = buffer.slice(..).get_mapped_range();
    let colors: std::collections::BTreeSet<_> = bytes.as_chunks::<4>().0.iter().map(|v| v.to_vec()).collect();
    assert!(
        colors.len() > 100,
        "scene and instruments should contain varied pixels"
    );
    if let Ok(path) = std::env::var("FLIGHTCRAFT_PREVIEW") {
        image::save_buffer(&path, &bytes, width, height, image::ColorType::Rgba8)?;
        eprintln!("Offscreen app preview: {path}");
    }
    drop(bytes);
    buffer.unmap();
    assert!(
        state.device.pop_error_scope().await.is_none(),
        "GPU validation failed"
    );
    Ok(())
}
