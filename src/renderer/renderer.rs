// renderer/renderer.rs
use crate::renderer::{Assets, CameraUniform, Depth, RenderBatcher, Vertex};
use crate::scene::Camera;

use std::{mem, num::NonZeroU64};
use wgpu::util::DeviceExt;
use winit::{dpi::PhysicalSize, window::Window};

const INITIAL_OBJECTS_CAPACITY: u32 = 1024;

pub struct Renderer {
    context: RenderContext,
    pipeline: RenderPipeline,
    objects_buffer: DynamicObjectsBuffer,
    camera_buffer: CameraBuffer,
}

struct RenderContext {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: PhysicalSize<u32>,
    depth: Depth,
}

struct RenderPipeline {
    pipeline: wgpu::RenderPipeline,
    dummy_texture_bind_group: wgpu::BindGroup,
}

struct DynamicObjectsBuffer {
    buffer: wgpu::Buffer,
    capacity: u32,
    bind_group: wgpu::BindGroup,
    bind_layout: wgpu::BindGroupLayout,
    scratch: Vec<crate::renderer::ObjectData>,
}

struct CameraBuffer {
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    bind_layout: wgpu::BindGroupLayout,
}

impl Renderer {
    pub async fn new(window: &Window) -> Self {
        let size = window.inner_size();
        let context = RenderContext::new(window, size).await;
        let camera_buffer = CameraBuffer::new(&context.device);
        let objects_buffer = DynamicObjectsBuffer::new(&context.device, INITIAL_OBJECTS_CAPACITY);
        let pipeline = RenderPipeline::new(&context, &camera_buffer, &objects_buffer);

        Self {
            context,
            pipeline,
            objects_buffer,
            camera_buffer,
        }
    }

    pub fn get_device(&self) -> &wgpu::Device {
        &self.context.device
    }

    pub fn get_queue(&self) -> &wgpu::Queue {
        &self.context.queue
    }

    pub fn resize(&mut self, new_size: PhysicalSize<u32>) {
        self.context.resize(new_size);
    }

    pub fn aspect_ratio(&self) -> f32 {
        self.context.config.width as f32 / self.context.config.height.max(1) as f32
    }

    pub fn set_camera(&self, camera: &Camera, aspect: f32) {
        let vp = camera.view_proj(aspect);
        let uni = CameraUniform {
            view_proj: vp.to_cols_array_2d(),
        };
        self.context
            .queue
            .write_buffer(&self.camera_buffer.buffer, 0, bytemuck::bytes_of(&uni));
    }

    pub fn create_mesh(&self, vertices: &[Vertex], indices: &[u16]) -> crate::renderer::assets::Mesh {
        crate::renderer::assets::Mesh::from_vertices(&self.context.device, vertices, indices)
    }

    pub fn render(
        &mut self,
        assets: &Assets,
        batcher: &RenderBatcher,
    ) -> Result<(), wgpu::SurfaceError> {
        let frame = self.context.surface.get_current_texture()?;
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .context
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Encoder"),
            });

        self.objects_buffer.update(&self.context, batcher)?;

        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("MainPass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.05,
                            g: 0.07,
                            b: 0.10,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.context.depth.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            rpass.set_pipeline(&self.pipeline.pipeline);
            rpass.set_bind_group(0, &self.camera_buffer.bind_group, &[]);
            rpass.set_bind_group(1, &self.objects_buffer.bind_group, &[]);
            rpass.set_bind_group(2, &self.pipeline.dummy_texture_bind_group, &[]);

            let mut object_offset = 0u32;
            for (mesh_handle, instances) in batcher.iter() {
                let Some(mesh) = assets.meshes.get(mesh_handle) else {
                    log::warn!("Skipping batch with invalid mesh handle");
                    object_offset += instances.len() as u32;
                    continue;
                };

                let instance_count = instances.len() as u32;
                rpass.set_vertex_buffer(0, mesh.vertex_buffer().slice(..));
                rpass.set_index_buffer(mesh.index_buffer().slice(..), wgpu::IndexFormat::Uint16);
                rpass.draw_indexed(
                    0..mesh.index_count(),
                    0,
                    object_offset..(object_offset + instance_count),
                );

                object_offset += instance_count;
            }
        }

        self.context.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(())
    }
}

impl RenderContext {
    async fn new(window: &Window, size: PhysicalSize<u32>) -> Self {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let surface = unsafe {
            instance
                .create_surface_unsafe(
                    wgpu::SurfaceTargetUnsafe::from_window(window).expect("surface target"),
                )
                .expect("surface")
        };

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("adapter");

        let adapter_features = adapter.features();
        log::info!("Adapter features: {}", adapter_features);

        let mut required_features = wgpu::Features::empty();
        if adapter_features.contains(
            wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING,
        ) {
            required_features |= wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING
                | wgpu::Features::TEXTURE_BINDING_ARRAY;
            log::info!("Bindless textures supported!");
        } else {
            log::warn!("Bindless textures not supported");
        }

        let limits = wgpu::Limits {
            max_binding_array_elements_per_shader_stage: 256,
            ..wgpu::Limits::default()
        };

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("Device"),
                    required_features,
                    required_limits: limits,
                    experimental_features: wgpu::ExperimentalFeatures::disabled(),
                    memory_hints: wgpu::MemoryHints::Performance,
                    trace: wgpu::Trace::Off,
                },
            )
            .await
            .expect("device");

        let surface_caps = surface.get_capabilities(&adapter);
        let format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let depth = Depth::new(&device, size);

        Self {
            surface,
            device,
            queue,
            config,
            size,
            depth,
        }
    }

    fn resize(&mut self, new_size: PhysicalSize<u32>) {
        if new_size.width == 0 || new_size.height == 0 {
            return;
        }
        self.size = new_size;
        self.config.width = new_size.width;
        self.config.height = new_size.height;
        self.surface.configure(&self.device, &self.config);
        self.depth = Depth::new(&self.device, new_size);
    }
}

impl CameraBuffer {
    fn new(device: &wgpu::Device) -> Self {
        let camera = CameraUniform::new();
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("CameraBuffer"),
            contents: bytemuck::bytes_of(&camera),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("CameraBindLayout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: Some(
                        NonZeroU64::new(mem::size_of::<CameraUniform>() as u64).unwrap(),
                    ),
                },
                count: None,
            }],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("CameraBindGroup"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });

        Self { 
            buffer, 
            bind_group,
            bind_layout,  // Store it
        }
    }
}

impl DynamicObjectsBuffer {
    fn new(device: &wgpu::Device, capacity: u32) -> Self {
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ObjectsBindLayout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let buffer_size =
            (capacity as usize * mem::size_of::<crate::renderer::ObjectData>()) as u64;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ObjectsBuffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ObjectsBindGroup"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });

        Self {
            buffer,
            capacity,
            bind_group,
            bind_layout,
            scratch: Vec::with_capacity(capacity as usize),
        }
    }

    fn update(
        &mut self,
        context: &RenderContext,
        batcher: &RenderBatcher,
    ) -> Result<(), wgpu::SurfaceError> {
        self.scratch.clear();
        for (_, instances) in batcher.iter() {
            self.scratch.extend(instances.iter().map(|inst| {
                crate::renderer::ObjectData::new(
                    inst.transform,
                    inst.material.color_f32(),
                    inst.material.texture_index,
                    inst.material.flags_bits(),
                )
            }));
        }

        let required = self.scratch.len() as u32;
        if required > self.capacity {
            self.grow(context, required);
        }

        if !self.scratch.is_empty() {
            context
                .queue
                .write_buffer(&self.buffer, 0, bytemuck::cast_slice(&self.scratch));
        }

        Ok(())
    }

    fn grow(&mut self, context: &RenderContext, required: u32) {
        let new_capacity = required.max(self.capacity * 2);
        log::info!(
            "Growing objects buffer: {} -> {}",
            self.capacity,
            new_capacity
        );

        let buffer_size =
            (new_capacity as usize * mem::size_of::<crate::renderer::ObjectData>()) as u64;
        self.buffer = context.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ObjectsBuffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        self.bind_group = context
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ObjectsBindGroup"),
                layout: &self.bind_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.buffer.as_entire_binding(),
                }],
            });

        self.capacity = new_capacity;
    }
}

impl RenderPipeline {
    fn new(
        context: &RenderContext,
        camera: &CameraBuffer,
        objects: &DynamicObjectsBuffer,
    ) -> Self {
        let texture_array_bind_layout =
            context
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("TextureArrayBindGroupLayout"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: std::num::NonZero::new(256),
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                    ],
                });

        let dummy_texture_bind_group =
            Self::create_dummy_texture_bind_group(&context.device, &texture_array_bind_layout);

        let shader = context
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("../shader.wgsl").into()),
            });

        let pipeline_layout =
            context
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("PipelineLayout"),
                    bind_group_layouts: &[
                        &camera.bind_layout,
                        &objects.bind_layout,
                        &texture_array_bind_layout,
                    ],
                    push_constant_ranges: &[],
                });

        let pipeline = context
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[Vertex::layout()],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: context.config.format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: Some(wgpu::Face::Back),
                    front_face: wgpu::FrontFace::Ccw,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: context.depth.format,
                    depth_write_enabled: true,
                    depth_compare: wgpu::CompareFunction::LessEqual,
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            });

        Self {
            pipeline,
            dummy_texture_bind_group,
        }
    }

    fn create_dummy_texture_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
    ) -> wgpu::BindGroup {
        let dummy_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("DummyTexture"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        let dummy_view = dummy_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let dummy_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("DummySampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let dummy_views = vec![&dummy_view; 256];
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("DummyTextureBindGroup"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureViewArray(&dummy_views),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&dummy_sampler),
                },
            ],
        })
    }
}