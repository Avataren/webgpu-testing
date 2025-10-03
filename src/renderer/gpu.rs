// renderer/gpu.rs (Complete Updated Version)
use crate::{
    renderer::{self, Assets, CameraUniform, RenderBatcher, Vertex},
    Depth,
};

use std::{mem, num::NonZeroU64};
use wgpu::util::DeviceExt;
use winit::{dpi::PhysicalSize, window::Window};

const INITIAL_OBJECTS_CAPACITY: u32 = 1024;

pub struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: PhysicalSize<u32>,
    depth: Depth,

    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,

    objects_buf: wgpu::Buffer,
    objects_capacity: u32,
    objects_bind_group: wgpu::BindGroup,
    objects_bind_layout: wgpu::BindGroupLayout,
    objects_scratch: Vec<renderer::ObjectData>,

    camera_buf: wgpu::Buffer,
}

impl Gpu {
    pub async fn new(window: &Window) -> Self {
        let size = window.inner_size();
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

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            })
            .await
            .expect("device");

        let surface_caps = surface.get_capabilities(&adapter);
        let format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let alpha_mode = surface_caps
            .alpha_modes
            .iter()
            .copied()
            .find(|m| *m == wgpu::CompositeAlphaMode::PreMultiplied)
            .unwrap_or(surface_caps.alpha_modes[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let depth = Depth::new(&device, size);

        let camera = CameraUniform::new();
        let camera_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("CameraBuffer"),
            contents: bytemuck::bytes_of(&camera),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("BindLayout"),
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
            label: Some("BindGroup"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buf.as_entire_binding(),
            }],
        });

        let objects_bind_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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

        let objects_capacity: u32 = INITIAL_OBJECTS_CAPACITY;
        let objects_buf_size = (objects_capacity as usize
            * std::mem::size_of::<renderer::ObjectData>())
            as wgpu::BufferAddress;

        let objects_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ObjectsBuffer"),
            size: objects_buf_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let objects_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ObjectsBindGroup"),
            layout: &objects_bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: objects_buf.as_entire_binding(),
            }],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shader.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("PipelineLayout"),
            bind_group_layouts: &[&bind_layout, &objects_bind_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
                    format: config.format,
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
                unclipped_depth: false,
                conservative: false,
                strip_index_format: None,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth.format,
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
            surface,
            device,
            queue,
            config,
            size,
            depth,
            pipeline,
            bind_group,
            objects_buf,
            objects_capacity,
            objects_bind_group,
            objects_bind_layout,
            objects_scratch: Vec::with_capacity(INITIAL_OBJECTS_CAPACITY as usize),
            camera_buf,
        }
    }

    pub fn get_device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn get_config(&self) -> &wgpu::SurfaceConfiguration {
        &self.config
    }

    pub fn resize(&mut self, new_size: PhysicalSize<u32>) {
        if new_size.width == 0 || new_size.height == 0 {
            return;
        }
        self.size = new_size;
        self.config.width = new_size.width;
        self.config.height = new_size.height;
        self.surface.configure(&self.device, &self.config);
        self.depth = Depth::new(&self.device, new_size);
    }

    pub fn set_view_proj(&self, vp: glam::Mat4) {
        let uni = renderer::CameraUniform {
            view_proj: vp.to_cols_array_2d(),
        };
        self.queue
            .write_buffer(&self.camera_buf, 0, bytemuck::bytes_of(&uni));
    }

    /// Render using the new batching system
    pub fn render_batched(
        &mut self,
        assets: &Assets,
        batcher: &RenderBatcher,
    ) -> Result<(), wgpu::SurfaceError> {
        let frame = self.surface.get_current_texture()?;
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Encoder"),
            });

        // Collect all transforms first to write them in one go
        self.objects_scratch.clear();
        for (_, _, transforms) in batcher.iter() {
            self.objects_scratch
                .extend(transforms.iter().map(|&m| renderer::ObjectData::from(m)));
        }

        // Ensure buffer is large enough
        let required = self.objects_scratch.len() as u32;
        if required > self.objects_capacity {
            let new_capacity = required.max(self.objects_capacity * 2);
            log::info!(
                "Growing objects buffer: {} -> {}",
                self.objects_capacity,
                new_capacity
            );

            let buf_size = (new_capacity as usize * std::mem::size_of::<renderer::ObjectData>())
                as wgpu::BufferAddress;

            self.objects_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ObjectsBuffer"),
                size: buf_size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            self.objects_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ObjectsBindGroup"),
                layout: &self.objects_bind_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.objects_buf.as_entire_binding(),
                }],
            });

            self.objects_capacity = new_capacity;
        }

        // Write all transforms
        if !self.objects_scratch.is_empty() {
            self.queue.write_buffer(
                &self.objects_buf,
                0,
                bytemuck::cast_slice(&self.objects_scratch),
            );
        }

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
                    view: &self.depth.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            rpass.set_pipeline(&self.pipeline);
            rpass.set_bind_group(0, &self.bind_group, &[]);
            rpass.set_bind_group(1, &self.objects_bind_group, &[]);

            // Track offset into the objects buffer
            let mut object_offset = 0u32;

            // Draw each batch
            for (mesh_handle, _material, transforms) in batcher.iter() {
                let Some(mesh) = assets.meshes.get(mesh_handle) else {
                    log::warn!("Skipping batch with invalid mesh handle");
                    object_offset += transforms.len() as u32;
                    continue;
                };

                let instance_count = transforms.len() as u32;
                
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

        self.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(())
    }
}