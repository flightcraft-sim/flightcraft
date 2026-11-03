use super::{DecodedTile, Selection, Vertex};
use std::{collections::BTreeMap, sync::Arc};
use wgpu::util::DeviceExt;
pub(crate) struct Pipeline {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}
struct Tile {
    vertices: wgpu::Buffer,
    count: u32,
    bind: wgpu::BindGroup,
    payload: Arc<DecodedTile>,
}
#[derive(Default)]
pub(crate) struct Cache {
    tiles: BTreeMap<usize, Tile>,
    selected: Vec<usize>,
}
impl Pipeline {
    pub(crate) fn new(device: &wgpu::Device, uniform: &wgpu::BindGroupLayout) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("3D Tiles material"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("3D Tiles scenery shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../scenery.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("3D Tiles pipeline"),
            bind_group_layouts: &[uniform, &layout],
            push_constant_ranges: &[],
        });
        let attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x2,3=>Float32x4];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("3D Tiles opaque glTF"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24Plus,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: Default::default(),
                bias: wgpu::DepthBiasState {
                    constant: -2,
                    slope_scale: -1.,
                    clamp: 0.,
                },
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        Self { pipeline, layout }
    }
    pub(crate) fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        selection: &Selection,
        cache: &mut Cache,
        uploads: usize,
    ) {
        // Retain previous coverage until every replacement is uploaded. CPU readiness alone
        // cannot retire a parent when GPU uploads are spread over several frames.
        cache.tiles.retain(|id, tile| {
            !tile.payload.expired()
                && selection
                    .live_tiles
                    .iter()
                    .any(|(live_id, payload)| live_id == id && Arc::ptr_eq(payload, &tile.payload))
        });
        cache.selected.retain(|id| cache.tiles.contains_key(id));
        let mut uploaded = 0;
        let mut candidates = Vec::new();
        if cache.selected.is_empty()
            && let Some(fallback) = &selection.fallback
        {
            candidates.push(fallback.clone());
        }
        candidates.extend(selection.tiles.iter().cloned());
        for (id, payload) in &candidates {
            if payload.expired() || cache.tiles.contains_key(id) || uploaded >= uploads {
                continue;
            }
            let Some(base) = payload.mips.first() else {
                continue;
            };
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("3D Tiles WebP mip pyramid"),
                size: wgpu::Extent3d {
                    width: base.width(),
                    height: base.height(),
                    depth_or_array_layers: 1,
                },
                mip_level_count: payload.mips.len() as u32,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            for (level, mip) in payload.mips.iter().enumerate() {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: level as u32,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    mip.as_raw(),
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(mip.width() * 4),
                        rows_per_image: Some(mip.height()),
                    },
                    wgpu::Extent3d {
                        width: mip.width(),
                        height: mip.height(),
                        depth_or_array_layers: 1,
                    },
                );
            }
            let view = texture.create_view(&Default::default());
            let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("3D Tiles trilinear sampler"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            });
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("3D Tiles material"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            });
            let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("3D Tiles geometry"),
                contents: bytemuck::cast_slice(&payload.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
            cache.tiles.insert(
                *id,
                Tile {
                    vertices,
                    count: payload.vertices.len() as u32,
                    bind,
                    payload: payload.clone(),
                },
            );
            uploaded += 1;
        }
        if cache.selected.is_empty()
            && let Some((id, _)) = &selection.fallback
            && cache.tiles.contains_key(id)
        {
            cache.selected.push(*id);
        }
        if selection.tiles.iter().all(|(id, _)| cache.tiles.contains_key(id)) {
            cache.selected = selection.tiles.iter().map(|(id, _)| *id).collect();
            cache.tiles.retain(|id, _| cache.selected.contains(id));
        }
    }
    pub(crate) fn draw(&self, pass: &mut wgpu::RenderPass<'_>, cache: &Cache) {
        pass.set_pipeline(&self.pipeline);
        for id in &cache.selected {
            if let Some(tile) = cache.tiles.get(id).filter(|tile| !tile.payload.expired()) {
                pass.set_bind_group(1, &tile.bind, &[]);
                pass.set_vertex_buffer(0, tile.vertices.slice(..));
                pass.draw(0..tile.count, 0..1);
            }
        }
    }
}

impl Cache {
    pub(crate) fn terrain_bounds(&self) -> [f32; 4] {
        let mut result: Option<[f32; 4]> = None;
        for id in &self.selected {
            if let Some([a, b, c, d]) = self.tiles.get(id).and_then(|t| t.payload.terrain_bounds) {
                let bounds = result.get_or_insert([a, b, c, d]);
                let [minx, minz, maxx, maxz] = bounds;
                *minx = minx.min(a);
                *minz = minz.min(b);
                *maxx = maxx.max(c);
                *maxz = maxz.max(d);
            }
        }
        result.unwrap_or([0.; 4])
    }
}
