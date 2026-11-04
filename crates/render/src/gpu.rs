use crate::{Palette, mesh};
use eframe::egui;
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use wgpu::util::DeviceExt;

const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Uniform {
    pub view_projection: [[f32; 4]; 4],
    pub aircraft: [[f32; 4]; 4],
    pub world_from_clip: [[f32; 4]; 4],
    pub sky_top: [f32; 4],
    pub sky_horizon: [f32; 4],
    pub ground: [f32; 4],
    pub dry_grass: [f32; 4],
    pub rock: [f32; 4],
    pub forest: [f32; 4],
    pub shoreline: [f32; 4],
    pub water: [f32; 4],
    pub grid: [f32; 4],
    pub eye: [f32; 4],
    pub vision: [f32; 4],
    pub caution: [f32; 4],
    pub warning: [f32; 4],
    pub atmosphere: [f32; 4],
    pub fog_color: [f32; 4],
    pub clouds: [[f32; 4]; 3],
    pub light_red: [f32; 4],
    pub viewport: [f32; 4],
    pub landing_light: [f32; 4],
    pub scenery_bounds: [f32; 4],
}

pub(crate) struct SceneCallback {
    pub resources: std::sync::Arc<std::sync::Mutex<Option<ViewGpu>>>,
    pub world: flightcraft_world::World,
    pub uniform: Uniform,
    pub size: [f32; 2],
    pub show_aircraft: bool,
    pub show_airspace: bool,
    pub show_lights: bool,
    pub aircraft_shadow: bool,
    pub larrow: bool,
    pub sling: bool,
    pub exterior: Option<std::sync::Arc<flightcraft_aero::exterior::ExteriorAsset>>,
    pub exterior_pose: crate::exterior::Pose,
    pub scenery: crate::scenery::Selection,
    pub scenery_quality: crate::scenery::Quality,
    pub lighting: crate::lighting::Frame,
    pub exterior_environment: Option<std::sync::Arc<crate::exterior::ExteriorEnvironment>>,
}

struct ExteriorGpu {
    pose: Option<crate::exterior::Pose>,
    eye: Option<[f32; 3]>,
    pbr: crate::exterior::PbrRenderer,
    asset: Option<std::sync::Arc<flightcraft_aero::exterior::ExteriorAsset>>,
    lod: Option<std::sync::Arc<flightcraft_aero::exterior::ExteriorModel>>,
    ready: bool,
    vertices: wgpu::Buffer,
    opaque_count: u32,
    count: u32,
}

pub(crate) struct ViewGpu {
    scenery: crate::scenery::gpu::Cache,
    exterior: Option<ExteriorGpu>,
    lighting: crate::lighting::GpuLighting,
    shadows: crate::shadows::State,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
    targets: Targets,
}

struct Targets {
    size: [u32; 2],
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
    bind: wgpu::BindGroup,
}

struct TerrainBuffer {
    vertices: wgpu::Buffer,
    count: u32,
    fine_end: u32,
    center: [f32; 3],
    half_extent_m: f32,
}

struct DrawMesh {
    vertices: wgpu::Buffer,
    vertex_count: u32,
    world_count: u32,
    exterior_end: u32,
    cockpit_end: u32,
}

struct Gpu {
    empty: wgpu::BindGroup,
    scenery: crate::scenery::gpu::Pipeline,
    shadows: crate::shadows::Pipelines,
    vision_lighting: crate::lighting::GpuLighting,
    terrain: Vec<TerrainBuffer>,
    airspace: Option<(wgpu::Buffer, u32)>,
    terrain_fingerprint: Option<u64>,
    palette: Palette,
    sky: wgpu::RenderPipeline,
    scene: wgpu::RenderPipeline,
    transparent: wgpu::RenderPipeline,
    blit: wgpu::RenderPipeline,
    meshes: [DrawMesh; 3],
    uniform_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    vision_targets: Targets,
    vision_uniform: wgpu::Buffer,
    vision_bind: wgpu::BindGroup,
    vision_schedule: VisionSchedule,
    vision_runways: Option<(wgpu::Buffer, u32)>,
    runway_lights: Option<(wgpu::Buffer, u32)>,
}

/// Install resources in egui's callback store; there is one scene per viewport.
pub fn initialize(state: &egui_wgpu::RenderState, palette: Palette) {
    state.renderer.write().callback_resources.insert(Gpu::new(
        &state.device,
        state.target_format,
        palette,
        if state
            .adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::ANISOTROPIC_FILTERING)
        {
            4
        } else {
            1
        },
    ));
}

/// Register the shared terrain target as an egui image, avoiding a separate composite pipeline
/// switch for each display. Call after `initialize` for the context that hosts the displays.
pub fn initialize_synthetic_vision(state: &egui_wgpu::RenderState, context: &egui::Context) {
    let mut renderer = state.renderer.write();
    let Some(gpu) = renderer.callback_resources.get::<Gpu>() else {
        return;
    };
    let view = gpu.vision_targets.color.clone();
    let texture = renderer.register_native_texture(&state.device, &view, wgpu::FilterMode::Linear);
    crate::vision::bind_texture(context, texture);
}

fn targets(device: &wgpu::Device, size: [u32; 2], layout: &wgpu::BindGroupLayout) -> Targets {
    let [width, height] = size;
    let descriptor = |format, usage| wgpu::TextureDescriptor {
        label: Some("flight scene target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    };
    let color = device
        .create_texture(&descriptor(
            COLOR_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        ))
        .create_view(&Default::default());
    let depth = device
        .create_texture(&descriptor(DEPTH_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT))
        .create_view(&Default::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("scene sampler"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("scene image"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&color),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });
    Targets {
        size,
        color,
        depth,
        bind,
    }
}

impl Gpu {
    fn new(device: &wgpu::Device, output: wgpu::TextureFormat, palette: Palette, anisotropy: u16) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flight scene WGSL"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}",
                    include_str!("lighting.wgsl"),
                    include_str!("scene.wgsl")
                )
                .into(),
            ),
        });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene uniform layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Uniform>() as u64),
                },
                count: None,
            }],
        });
        let lighting_layout = crate::lighting::layout(device);
        let empty = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("unused scene bindings"),
            entries: &[],
        });
        let scene_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene pipeline layout"),
            bind_group_layouts: &[&uniform_layout, &empty, &empty, &lighting_layout],
            push_constant_ranges: &[],
        });
        let vertex_attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32, 4 => Float32];
        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<mesh::Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &vertex_attributes,
        };
        let pipeline = |label,
                        vertex,
                        fragment,
                        buffers: &[wgpu::VertexBufferLayout<'_>],
                        sky: bool,
                        transparent: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&scene_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vertex),
                    compilation_options: Default::default(),
                    buffers,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: COLOR_FORMAT,
                        blend: transparent.then_some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: !sky && !transparent,
                    depth_compare: if sky {
                        wgpu::CompareFunction::Always
                    } else {
                        wgpu::CompareFunction::LessEqual
                    },
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        };
        let sky = pipeline("sky gradient", "sky_vertex", "sky_fragment", &[], true, false);
        let scene = pipeline(
            "terrain and aircraft",
            "scene_vertex",
            "scene_fragment",
            std::slice::from_ref(&vertex_layout),
            false,
            false,
        );
        let transparent = pipeline(
            "propeller transparency",
            "scene_vertex",
            "scene_fragment",
            &[vertex_layout],
            false,
            true,
        );
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene texture layout"),
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
        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene composite WGSL"),
            source: wgpu::ShaderSource::Wgsl(include_str!("composite.wgsl").into()),
        });
        let blit_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("composite layout"),
            bind_group_layouts: &[&texture_layout],
            push_constant_ranges: &[],
        });
        let blit = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scene composite"),
            layout: Some(&blit_layout),
            vertex: wgpu::VertexState {
                module: &blit_shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &blit_shader,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: output,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let meshes = [0, 1, 2].map(|kind| {
            let geometry = mesh::build(palette, kind == 1);
            let geometry = if kind == 2 {
                mesh::sling_geometry(geometry, palette)
            } else {
                geometry
            };
            let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("original field and aircraft mesh"),
                contents: bytemuck::cast_slice(&geometry.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
            DrawMesh {
                vertices,
                vertex_count: geometry.vertices.len() as u32,
                world_count: geometry.world_end,
                exterior_end: geometry.exterior_end,
                cockpit_end: geometry.cockpit_end,
            }
        });
        let vision_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("synthetic vision uniforms"),
            size: std::mem::size_of::<Uniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let vision_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("synthetic vision uniform bind"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: vision_uniform.as_entire_binding(),
            }],
        });
        let vision_targets = targets(device, [750, 285], &texture_layout);
        let scenery = crate::scenery::gpu::Pipeline::new(device, &uniform_layout, anisotropy);
        Self {
            empty: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("unused scene slots"),
                layout: &empty,
                entries: &[],
            }),
            shadows: crate::shadows::Pipelines::new(device),
            vision_lighting: crate::lighting::GpuLighting::new(device, crate::scenery::Quality::Low),
            scenery,
            sky,
            scene,
            transparent,
            blit,
            meshes,
            terrain: Vec::new(),
            airspace: None,
            terrain_fingerprint: None,
            palette,
            uniform_layout,
            texture_layout,
            vision_targets,
            vision_uniform,
            vision_bind,
            vision_schedule: VisionSchedule::default(),
            vision_runways: None,
            runway_lights: None,
        }
    }
}

impl Gpu {
    fn update_world(&mut self, device: &wgpu::Device, world: &flightcraft_world::World) {
        if self.terrain_fingerprint != Some(world.fingerprint()) {
            self.meshes = [0, 1, 2].map(|kind| {
                let geometry = mesh::build_for_world(self.palette, kind == 1, world);
                let geometry = if kind == 2 {
                    mesh::sling_geometry(geometry, self.palette)
                } else {
                    geometry
                };
                DrawMesh {
                    vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("field and aircraft mesh"),
                        contents: bytemuck::cast_slice(&geometry.vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
                    vertex_count: geometry.vertices.len() as u32,
                    world_count: geometry.world_end,
                    exterior_end: geometry.exterior_end,
                    cockpit_end: geometry.cockpit_end,
                }
            });
            self.terrain = world
                .tiles()
                .iter()
                .map(|tile| {
                    let geometry = mesh::terrain(tile, world, self.palette);
                    TerrainBuffer {
                        vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("cached terrain LODs"),
                            contents: bytemuck::cast_slice(&geometry.vertices),
                            usage: wgpu::BufferUsages::VERTEX,
                        }),
                        count: geometry.vertices.len() as u32,
                        fine_end: geometry.fine_end,
                        center: geometry.center,
                        half_extent_m: tile.extent_m() as f32 * 0.5,
                    }
                })
                .collect();
            let vertices = mesh::airspace(world, self.palette);
            self.airspace = if vertices.is_empty() {
                None
            } else {
                Some((
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("airspace curtains and outlines"),
                        contents: bytemuck::cast_slice(&vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
                    vertices.len() as u32,
                ))
            };
            let lights = mesh::runway_lights(world, self.palette);
            self.runway_lights = (!lights.is_empty()).then(|| {
                (
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("runway and approach light impostors"),
                        contents: bytemuck::cast_slice(&lights),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
                    lights.len() as u32,
                )
            });
            let outlines = mesh::vision_runways(world, self.palette);
            self.vision_runways = (!outlines.is_empty()).then(|| {
                (
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("synthetic runway outlines"),
                        contents: bytemuck::cast_slice(&outlines),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
                    outlines.len() as u32,
                )
            });
            self.terrain_fingerprint = Some(world.fingerprint());
        }
    }
}

impl CallbackTrait for SceneCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(gpu) = resources.get_mut::<Gpu>() else {
            return Vec::new();
        };
        gpu.update_world(device, &self.world);
        let max_dimension = device.limits().max_texture_dimension_2d.min(4096) as f32;
        let size = self
            .size
            .map(|v| (v * screen.pixels_per_point).round().clamp(1.0, max_dimension) as u32);
        let Ok(mut resources) = self.resources.lock() else {
            return Vec::new();
        };
        let view = resources.get_or_insert_with(|| {
            let uniform = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("screen uniforms"),
                size: std::mem::size_of::<Uniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("screen uniforms"),
                layout: &gpu.uniform_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                }],
            });
            ViewGpu {
                scenery: Default::default(),
                lighting: crate::lighting::GpuLighting::new(device, self.scenery_quality),
                shadows: gpu.shadows.state(device),
                exterior: None,
                uniform,
                bind,
                targets: targets(device, size, &gpu.texture_layout),
            }
        });
        if view.targets.size != size {
            view.targets = targets(device, size, &gpu.texture_layout);
        }
        let atlas_size = if self.scenery_quality == crate::scenery::Quality::Low {
            1024
        } else {
            2048
        };
        if view.lighting.size != atlas_size {
            view.lighting = crate::lighting::GpuLighting::new(device, self.scenery_quality);
        }
        let mut physical = self.lighting;
        let forward = (glam::Mat4::from_cols_array_2d(&self.uniform.world_from_clip)
            * glam::Vec4::new(0., 0., 1., 1.))
        .truncate()
        .normalize_or_zero();
        physical.cascades(forward, self.scenery_quality);
        physical.clouds = self.uniform.clouds;
        if self.uniform.atmosphere[2] > 0.5 || self.uniform.atmosphere[3] > 0.5 {
            physical.splits[3] = 0.;
            physical.radiance[3] = 2.;
        }
        if physical.sun[1] <= 0. {
            physical.splits[3] = 0.;
        }
        physical.settings[3] = 1. / view.lighting.size as f32;
        view.lighting.update(queue, &physical);
        if (self.show_aircraft || physical.splits[3] > 0.)
            && let Some(asset) = &self.exterior
        {
            let eye = glam::Vec3::from_slice(&self.uniform.eye);
            let center = glam::Mat4::from_cols_array_2d(&self.uniform.aircraft)
                .w_axis
                .truncate();
            let mut pose = self.exterior_pose.clone();
            if !self.show_lights {
                pose.lights = [false; 4];
                for powered in pose.powered_loads.values_mut() {
                    *powered = false;
                }
            }
            let exterior = view.exterior.get_or_insert_with(|| ExteriorGpu {
                pose: None,
                eye: None,
                pbr: crate::exterior::PbrRenderer::new(device),
                asset: None,
                lod: None,
                ready: false,
                vertices: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("animated aircraft exterior"),
                    size: (flightcraft_aero::exterior::MAX_VERTICES * std::mem::size_of::<mesh::Vertex>())
                        as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                opaque_count: 0,
                count: 0,
            });
            let lod = crate::exterior::model_at_distance(asset, eye.distance(center));
            let changed = exterior
                .asset
                .as_ref()
                .is_none_or(|a| !std::sync::Arc::ptr_eq(a, asset))
                || exterior
                    .lod
                    .as_ref()
                    .is_none_or(|m| !std::sync::Arc::ptr_eq(m, lod));
            let geometry_changed =
                changed || exterior.pose.as_ref() != Some(&pose) || exterior.eye != Some(eye.to_array());
            if geometry_changed {
                let frame = crate::exterior::frame(asset, &pose, eye.distance(center));
                exterior.opaque_count = frame.opaque.len() as u32;
                let mut vertices = frame.opaque;
                vertices.extend(frame.transparent);
                exterior.count = vertices.len() as u32;
                queue.write_buffer(&exterior.vertices, 0, bytemuck::cast_slice(&vertices));
            }
            let result = (|| -> Result<(), String> {
                if changed {
                    exterior.pbr.set_materials_with_limit(
                        device,
                        queue,
                        &crate::exterior::pbr_materials(lod),
                        if std::sync::Arc::ptr_eq(lod, &asset.lod0) {
                            4096
                        } else if std::sync::Arc::ptr_eq(lod, &asset.lod1) {
                            1024
                        } else {
                            512
                        },
                    )?;
                    exterior.asset = Some(asset.clone());
                    exterior.lod = Some(lod.clone());
                }
                let mut lighting = self
                    .exterior_environment
                    .as_ref()
                    .map_or_else(crate::exterior::PbrFrame::default, |environment| {
                        environment.frame
                    });
                lighting.view_projection = self.uniform.view_projection;
                lighting.world_from_body = self.uniform.aircraft;
                lighting.eye = eye.extend(1.).to_array();
                if self.exterior_environment.is_none() {
                    lighting.sun_direction = physical.sun;
                    lighting.sun_radiance = physical.radiance;
                    lighting.sun_direction[3] = physical.settings[2];
                    lighting.sky_zenith = physical.sky(glam::Vec3::Y).extend(1.).to_array();
                    lighting.sky_horizon = physical.sky(glam::Vec3::Z).extend(1.).to_array();
                    lighting.environment[2] = physical.settings[0];
                }
                let probe = self
                    .exterior_environment
                    .as_ref()
                    .and_then(|e| e.probe.as_deref())
                    .or(Some(&view.lighting.probe))
                    .map(|p| crate::exterior::SkyProbe {
                        view: &p.view,
                        mip_count: p.mip_count,
                    });
                exterior.pbr.set_scene_lighting(&view.lighting.bind);
                if geometry_changed {
                    exterior.pbr.prepare(
                        device,
                        queue,
                        lighting,
                        &crate::exterior::pbr_primitives(asset, lod, &pose),
                        probe,
                    )
                } else {
                    exterior.pbr.update_frame(device, queue, lighting, probe)
                }
            })();
            if result.is_ok() {
                exterior.pose = Some(pose);
                exterior.eye = Some(eye.to_array());
            }
            exterior.ready = result.is_ok();
        }
        gpu.scenery.prepare(
            device,
            queue,
            &self.scenery,
            &mut view.scenery,
            self.scenery_quality.uploads_per_frame(),
        );
        let mut uniform = self.uniform;
        uniform.scenery_bounds = view.scenery.terrain_bounds();
        if uniform.atmosphere[3] < 0.5 {
            uniform.fog_color = physical.display(physical.sky(glam::Vec3::new(0., 0.05, 1.).normalize()));
        }
        queue.write_buffer(&view.uniform, 0, bytemuck::bytes_of(&uniform));
        // Each cascade uses the same selected payloads and the current animated aircraft.
        for (index, (buffer, bind)) in view
            .shadows
            .cascades
            .iter()
            .enumerate()
            .take(physical.splits[3] as usize)
        {
            let Some(matrix) = physical.shadow.get(index) else {
                continue;
            };
            queue.write_buffer(
                buffer,
                0,
                bytemuck::bytes_of(&crate::shadows::Uniform {
                    matrix: *matrix,
                    aircraft: uniform.aircraft,
                    bounds: uniform.scenery_bounds,
                }),
            );
            let mut depth = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sun shadow cascade"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &view.lighting.atlas,
                    depth_ops: Some(wgpu::Operations {
                        load: if index == 0 {
                            wgpu::LoadOp::Clear(1.)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            let side = view.lighting.size / 2;
            depth.set_viewport(
                (index as u32 % 2 * side) as f32,
                (index as u32 / 2 * side) as f32,
                side as f32,
                side as f32,
                0.,
                1.,
            );
            depth.set_scissor_rect(index as u32 % 2 * side, index as u32 / 2 * side, side, side);
            depth.set_bind_group(0, bind, &[]);
            depth.set_pipeline(&gpu.shadows.mesh);
            for tile in &gpu.terrain {
                depth.set_vertex_buffer(0, tile.vertices.slice(..));
                depth.draw(tile.fine_end..tile.count, 0..1);
            }
            let mesh = if self.sling {
                gpu.meshes.get(2)
            } else if self.larrow {
                gpu.meshes.get(1)
            } else {
                gpu.meshes.first()
            };
            if let Some(mesh) = mesh {
                depth.set_vertex_buffer(0, mesh.vertices.slice(..));
                depth.draw(0..mesh.world_count, 0..1);
            }
            if self.aircraft_shadow
                && let Some(exterior) = &view.exterior
            {
                depth.set_vertex_buffer(0, exterior.vertices.slice(..));
                depth.draw(0..exterior.opaque_count, 0..1);
            }
            gpu.scenery
                .draw_shadow(&mut depth, &view.scenery, &gpu.shadows.instances);
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("flight scene"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view.targets.color,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &view.targets.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_bind_group(0, &view.bind, &[]);
        pass.set_bind_group(3, &view.lighting.bind, &[]);
        pass.set_bind_group(1, &gpu.empty, &[]);
        pass.set_bind_group(2, &gpu.empty, &[]);
        pass.set_pipeline(&gpu.sky);
        pass.draw(0..3, 0..1);
        pass.set_bind_group(1, &gpu.empty, &[]);
        pass.set_bind_group(2, &gpu.empty, &[]);
        pass.set_pipeline(&gpu.scene);
        if !self.world.tiles().is_empty() {
            for tile in &gpu.terrain {
                pass.set_vertex_buffer(0, tile.vertices.slice(..));
                let near = if self.world.region().is_some() {
                    mesh::near_region_tile(self.uniform.eye, tile.center, tile.half_extent_m)
                } else {
                    mesh::near_tile(self.uniform.eye, tile.center)
                };
                let range = if near {
                    0..tile.fine_end
                } else {
                    tile.fine_end..tile.count
                };
                pass.draw(range, 0..1);
            }
        }
        gpu.scenery.draw(&mut pass, &view.scenery);
        pass.set_bind_group(1, &gpu.empty, &[]);
        pass.set_bind_group(2, &gpu.empty, &[]);
        pass.set_pipeline(&gpu.scene);
        let [corvid, larrow, sling] = &gpu.meshes;
        let mesh = if self.sling {
            sling
        } else if self.larrow {
            larrow
        } else {
            corvid
        };
        pass.set_vertex_buffer(0, mesh.vertices.slice(..));
        pass.draw(0..mesh.world_count, 0..1);
        if self.show_aircraft && self.uniform.atmosphere[3] < 0.5 {
            if self.exterior.is_some()
                && let Some(exterior) = &view.exterior
            {
                if exterior.ready {
                    exterior.pbr.draw_opaque(&mut pass);
                    pass.set_bind_group(0, &view.bind, &[]);
                } else {
                    pass.set_vertex_buffer(0, exterior.vertices.slice(..));
                    pass.draw(0..exterior.opaque_count, 0..1);
                    pass.set_bind_group(1, &gpu.empty, &[]);
                    pass.set_bind_group(2, &gpu.empty, &[]);
                    pass.set_pipeline(&gpu.transparent);
                    pass.draw(exterior.opaque_count..exterior.count, 0..1);
                }
                pass.set_bind_group(1, &gpu.empty, &[]);
                pass.set_bind_group(2, &gpu.empty, &[]);
                pass.set_pipeline(&gpu.scene);
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            } else {
                pass.draw(mesh.world_count..mesh.exterior_end, 0..1);
            }
        } else {
            pass.draw(mesh.exterior_end..mesh.cockpit_end, 0..1);
        }
        if (!self.show_aircraft || self.exterior.is_none())
            && self.uniform.eye.get(3).is_some_and(|power| *power > 0.01)
        {
            pass.set_bind_group(1, &gpu.empty, &[]);
            pass.set_bind_group(2, &gpu.empty, &[]);
            pass.set_pipeline(&gpu.transparent);
            pass.draw(mesh.cockpit_end..mesh.vertex_count, 0..1);
        }
        if self.show_lights
            && let Some((vertices, count)) = &gpu.runway_lights
        {
            pass.set_bind_group(1, &gpu.empty, &[]);
            pass.set_bind_group(2, &gpu.empty, &[]);
            pass.set_pipeline(&gpu.scene);
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.draw(0..*count, 0..1);
        }
        if self.show_airspace
            && let Some((vertices, count)) = &gpu.airspace
        {
            pass.set_bind_group(1, &gpu.empty, &[]);
            pass.set_bind_group(2, &gpu.empty, &[]);
            pass.set_pipeline(&gpu.transparent);
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.draw(0..*count, 0..1);
        }
        if self.show_aircraft
            && self.uniform.atmosphere[3] < 0.5
            && let Some(exterior) = &view.exterior
            && exterior.ready
        {
            exterior.pbr.draw_transparent(&mut pass);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        if let Some(gpu) = resources.get::<Gpu>()
            && let Ok(resources) = self.resources.lock()
            && let Some(view) = resources.as_ref()
        {
            pass.set_pipeline(&gpu.blit);
            pass.set_bind_group(0, &view.targets.bind, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn scenery_shader_validates_and_translates_to_webgl2() -> Result<(), Box<dyn std::error::Error>> {
        use naga::{back::glsl, valid};
        let module = naga::front::wgsl::parse_str(&format!(
            "{}\n{}",
            include_str!("lighting.wgsl"),
            include_str!("scenery.wgsl")
        ))?;
        let info = valid::Validator::new(valid::ValidationFlags::all(), valid::Capabilities::empty())
            .validate(&module)?;
        let options = glsl::Options {
            version: glsl::Version::Embedded {
                version: 300,
                is_webgl: true,
            },
            ..Default::default()
        };
        for entry in &module.entry_points {
            let pipeline = glsl::PipelineOptions {
                shader_stage: entry.stage,
                entry_point: entry.name.clone(),
                multiview: None,
            };
            let mut output = String::new();
            glsl::Writer::new(
                &mut output,
                &module,
                &info,
                &options,
                &pipeline,
                Default::default(),
            )?
            .write()?;
            assert!(output.starts_with("#version 300 es"));
        }
        let size = module.types.iter().find_map(|(_, ty)| match &ty.inner {
            naga::TypeInner::Struct { span, .. } if ty.name.as_deref() == Some("Uniforms") => {
                Some(*span as usize)
            }
            _ => None,
        });
        assert_eq!(size, Some(std::mem::size_of::<super::Uniform>()));
        Ok(())
    }
    #[test]
    fn scene_shader_validates_and_translates_to_webgl2() -> Result<(), Box<dyn std::error::Error>> {
        use naga::{back::glsl, valid};
        let module = naga::front::wgsl::parse_str(&format!(
            "{}\n{}",
            include_str!("lighting.wgsl"),
            include_str!("scene.wgsl")
        ))?;
        let info = valid::Validator::new(valid::ValidationFlags::all(), valid::Capabilities::empty())
            .validate(&module)?;
        let options = glsl::Options {
            version: glsl::Version::Embedded {
                version: 300,
                is_webgl: true,
            },
            ..Default::default()
        };
        for entry in &module.entry_points {
            let pipeline = glsl::PipelineOptions {
                shader_stage: entry.stage,
                entry_point: entry.name.clone(),
                multiview: None,
            };
            let mut output = String::new();
            glsl::Writer::new(
                &mut output,
                &module,
                &info,
                &options,
                &pipeline,
                Default::default(),
            )?
            .write()?;
            assert!(output.starts_with("#version 300 es"));
        }
        // The host buffer and WGSL must agree, including the newly added terrain tokens.
        let uniforms = module
            .types
            .iter()
            .find_map(|(_, ty)| match &ty.inner {
                naga::TypeInner::Struct { members, span } if ty.name.as_deref() == Some("Uniforms") => {
                    Some((members, *span))
                }
                _ => None,
            })
            .ok_or("missing scene uniforms")?;
        assert_eq!(uniforms.1 as usize, std::mem::size_of::<super::Uniform>());
        assert_eq!(
            uniforms
                .0
                .iter()
                .find(|m| m.name.as_deref() == Some("eye"))
                .map(|m| m.offset as usize),
            Some(std::mem::offset_of!(super::Uniform, eye))
        );
        Ok(())
    }
}

/// Counts real offscreen terrain passes, independently of the number of display composites.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyntheticVisionStats {
    pub terrain_passes: u64,
}
pub fn synthetic_vision_stats(state: &egui_wgpu::RenderState) -> SyntheticVisionStats {
    state
        .renderer
        .read()
        .callback_resources
        .get::<Gpu>()
        .map_or(SyntheticVisionStats::default(), |gpu| SyntheticVisionStats {
            terrain_passes: gpu.vision_schedule.passes,
        })
}
#[derive(Default)]
struct VisionSchedule {
    last: Option<(u64, f64, u64)>,
    passes: u64,
}
impl VisionSchedule {
    fn request(&mut self, frame: u64, time: f64, world: u64) -> bool {
        if !time.is_finite() {
            return false;
        }
        if self.last.is_some_and(|(last_frame, last_time, last_world)| {
            world == last_world
                && (frame == last_frame || (time >= last_time && time - last_time < 1. / 30. - 1e-6))
        }) {
            return false;
        }
        self.last = Some((frame, time, world));
        self.passes = self.passes.saturating_add(1);
        true
    }
}
pub(crate) struct VisionCallback {
    pub composite: bool,
    pub view: crate::SyntheticVision,
    pub frame: u64,
    pub time: f64,
}
impl CallbackTrait for VisionCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(gpu) = resources.get_mut::<Gpu>() else {
            return Vec::new();
        };
        if !gpu
            .vision_schedule
            .request(self.frame, self.time, self.view.world.fingerprint())
        {
            return Vec::new();
        }
        gpu.update_world(device, &self.view.world);
        let uniform = self.view.uniform(gpu.palette);
        queue.write_buffer(&gpu.vision_uniform, 0, bytemuck::bytes_of(&uniform));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("shared synthetic vision terrain"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &gpu.vision_targets.color,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &gpu.vision_targets.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_bind_group(0, &gpu.vision_bind, &[]);
        pass.set_bind_group(3, &gpu.vision_lighting.bind, &[]);
        pass.set_bind_group(1, &gpu.empty, &[]);
        pass.set_bind_group(2, &gpu.empty, &[]);
        pass.set_pipeline(&gpu.sky);
        pass.draw(0..3, 0..1);
        pass.set_bind_group(1, &gpu.empty, &[]);
        pass.set_bind_group(2, &gpu.empty, &[]);
        pass.set_pipeline(&gpu.scene);
        for tile in &gpu.terrain {
            if !self.view.tile_in_range(tile.center, tile.half_extent_m) {
                continue;
            }
            pass.set_vertex_buffer(0, tile.vertices.slice(..));
            let near = mesh::near_region_tile(uniform.eye, tile.center, tile.half_extent_m);
            pass.draw(
                if near {
                    0..tile.fine_end
                } else {
                    tile.fine_end..tile.count
                },
                0..1,
            );
        }
        if self.view.world.tiles().is_empty() {
            let [mesh, _, _] = &gpu.meshes;
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.draw(0..6.min(mesh.world_count), 0..1);
        }
        if let Some((outlines, count)) = &gpu.vision_runways {
            pass.set_vertex_buffer(0, outlines.slice(..));
            pass.draw(0..*count, 0..1);
        }
        Vec::new()
    }
    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        if self.composite
            && let Some(gpu) = resources.get::<Gpu>()
        {
            pass.set_pipeline(&gpu.blit);
            pass.set_bind_group(0, &gpu.vision_targets.bind, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}
#[cfg(test)]
mod vision_schedule_tests {
    use super::*;
    #[test]
    fn zero_displays_skip_pass_and_two_displays_share_thirty_hz_at_120_hz() {
        let mut schedule = VisionSchedule::default();
        assert_eq!(schedule.passes, 0);
        for frame in 0..120 {
            for _ in 0..2 {
                schedule.request(frame, frame as f64 / 120., 1);
            }
        }
        assert_eq!(schedule.passes, 30);
        assert!(schedule.request(120, 1., 2));
        assert!(!schedule.request(120, 1., 2));
        assert!(schedule.request(121, 0., 2));
    }
}
