//! Depth-only cascades share the actual selected terrain/object and animated aircraft geometry.
use wgpu::util::DeviceExt;
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Uniform {
    pub matrix: [[f32; 4]; 4],
    pub aircraft: [[f32; 4]; 4],
    pub bounds: [f32; 4],
}
pub(crate) struct State {
    pub cascades: Vec<(wgpu::Buffer, wgpu::BindGroup)>,
}
pub(crate) struct Pipelines {
    pub mesh: wgpu::RenderPipeline,
    pub instances: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}
impl Pipelines {
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow cascade view"),
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
        let material = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("alpha-cutout shadow material"),
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
            label: Some("sun depth casters"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shadows.wgsl").into()),
        });
        let mesh_attributes =
            wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x4,3=>Float32,4=>Float32];
        let tile_attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x2,3=>Float32x4];
        let instance_attributes =
            wgpu::vertex_attr_array![4=>Float32x4,5=>Float32x4,6=>Float32x4,7=>Float32x4];
        let mesh_buffers = [wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<crate::mesh::Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &mesh_attributes,
        }];
        let instance_buffers = [
            wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<crate::scenery::Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &tile_attributes,
            },
            wgpu::VertexBufferLayout {
                array_stride: 64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &instance_attributes,
            },
        ];
        let single_layout = [&layout];
        let instance_layout = [&layout, &material];
        let make = |instanced: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("cascaded sunlight depth"),
                layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("sun depth"),
                    bind_group_layouts: if instanced {
                        &instance_layout
                    } else {
                        &single_layout
                    },
                    push_constant_ranges: &[],
                })),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(if instanced {
                        "instance_vertex"
                    } else {
                        "mesh_vertex"
                    }),
                    compilation_options: Default::default(),
                    buffers: if instanced {
                        &instance_buffers
                    } else {
                        &mesh_buffers
                    },
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(if instanced {
                        "instance_fragment"
                    } else {
                        "mesh_fragment"
                    }),
                    compilation_options: Default::default(),
                    targets: &[],
                }),
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: true,
                    depth_compare: wgpu::CompareFunction::LessEqual,
                    stencil: Default::default(),
                    bias: wgpu::DepthBiasState {
                        constant: 1,
                        slope_scale: 1.5,
                        clamp: 0.,
                    },
                }),
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        };
        let mesh = make(false);
        let instances = make(true);
        Self {
            mesh,
            instances,
            layout,
        }
    }
    pub fn state(&self, device: &wgpu::Device) -> State {
        let cascades = (0..3)
            .map(|_| {
                let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("cascade matrix"),
                    contents: bytemuck::bytes_of(&Uniform {
                        matrix: glam::Mat4::IDENTITY.to_cols_array_2d(),
                        aircraft: glam::Mat4::IDENTITY.to_cols_array_2d(),
                        bounds: [0.; 4],
                    }),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                });
                let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("cascade matrix"),
                    layout: &self.layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    }],
                });
                (buffer, bind)
            })
            .collect();
        State { cascades }
    }
}
