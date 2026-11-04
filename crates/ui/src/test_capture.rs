//! Shared offscreen GPU harness for renderer pixel tests and previews.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use eframe::egui::{self, Rect};
use std::sync::{Arc, mpsc};
pub(crate) struct Capture {
    pub(crate) state: egui_wgpu::RenderState,
    context: egui::Context,
    texture: wgpu::Texture,
    size: [u32; 2],
    pub(crate) frame: u32,
}
impl Capture {
    pub(crate) async fn new(size: [u32; 2]) -> Self {
        let backends = if std::env::var("FLIGHTCRAFT_TEST_BACKEND").as_deref() == Ok("gl") {
            wgpu::Backends::GL
        } else {
            wgpu::Backends::VULKAN
        };
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends,
            ..Default::default()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_limits: wgpu::Limits::downlevel_webgl2_defaults(),
                ..Default::default()
            })
            .await
            .unwrap();
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
        flightcraft_render::initialize(&state, crate::theme::scene());
        let context = egui::Context::default();
        crate::theme::install(&context);
        flightcraft_render::initialize_synthetic_vision(&state, &context);
        let [width, height] = size;
        let texture = state.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("PFD preview"),
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
        Self {
            state,
            context,
            texture,
            size,
            frame: 0,
        }
    }
    pub(crate) fn resize(&mut self, size: [u32; 2]) {
        self.size = size;
        let [width, height] = size;
        self.texture = self.state.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("PFD preview"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.state.target_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
    }
    pub(crate) fn draw(&mut self, mut paint: impl FnMut(&mut egui::Ui)) {
        let [width, height] = self.size;
        let output = self.context.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width as f32, height as f32),
                )),
                time: Some(f64::from(self.frame) / 120.),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::new())
                    .show(ctx, |ui| paint(ui));
            },
        );
        self.frame += 1;
        let jobs = self.context.tessellate(output.shapes, output.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: self.size,
            pixels_per_point: output.pixels_per_point,
        };
        let view = self.texture.create_view(&Default::default());
        let mut renderer = self.state.renderer.write();
        for (id, delta) in &output.textures_delta.set {
            renderer.update_texture(&self.state.device, &self.state.queue, *id, delta);
        }
        let mut encoder = self.state.device.create_command_encoder(&Default::default());
        let commands = renderer.update_buffers(
            &self.state.device,
            &self.state.queue,
            &mut encoder,
            &jobs,
            &screen,
        );
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
        self.state
            .queue
            .submit(commands.into_iter().chain([encoder.finish()]));
        for id in &output.textures_delta.free {
            renderer.free_texture(id);
        }
        self.state
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
    }
    pub(crate) fn image(&self) -> image::RgbaImage {
        let [width, height] = self.size;
        let stride = (width * 4).div_ceil(256) * 256;
        let buffer = self.state.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("PFD readback"),
            size: u64::from(stride * height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.state.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.state.queue.submit([encoder.finish()]);
        let (tx, rx) = mpsc::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            tx.send(r).unwrap();
        });
        self.state
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        rx.recv().unwrap().unwrap();
        let bytes = buffer.slice(..).get_mapped_range();
        let pixels = bytes
            .chunks_exact(stride as usize)
            .flat_map(|row| row[..(width * 4) as usize].iter().copied())
            .collect();
        let image = image::RgbaImage::from_raw(width, height, pixels).unwrap();
        drop(bytes);
        buffer.unmap();
        image
    }
    pub(crate) async fn validate(&self) {
        let error = self.state.device.pop_error_scope().await;
        assert!(error.is_none(), "offscreen GPU validation: {error:?}");
        self.state.device.push_error_scope(wgpu::ErrorFilter::Validation);
    }
}
