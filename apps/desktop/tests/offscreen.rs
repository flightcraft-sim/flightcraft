//! Opt-in GPU smoke test for the shared app and its real paint callback, without a window server.
#![cfg(not(target_arch = "wasm32"))]

use eframe::{App, egui};
use std::sync::{Arc, mpsc};

#[test]
#[ignore = "requires a Vulkan software or hardware adapter"]
fn app_renders_with_downlevel_limits() -> TestResult {
    pollster::block_on(render())
}

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

async fn render() -> TestResult {
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
    let mut app = flightcraft_ui::FlightCraftApp::new(&cc, Default::default())?;
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
    let colors: std::collections::BTreeSet<_> = bytes.chunks_exact(4).map(|v| v.to_vec()).collect();
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
