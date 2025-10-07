use bytemuck::{Pod, Zeroable};
use glam::Mat4;

const NOISE_TEXTURE_SIZE: u32 = 4;
const SSAO_NOISE_DATA: [f32; (NOISE_TEXTURE_SIZE * NOISE_TEXTURE_SIZE * 4) as usize] = [
    0.5381, 0.1856, 0.0, 0.0, 0.1379, 0.2486, 0.0, 0.0, 0.3371, 0.5679, 0.0, 0.0, -0.6999, -0.0451,
    0.0, 0.0, 0.0689, -0.1598, 0.0, 0.0, 0.0560, 0.0069, 0.0, 0.0, -0.0146, 0.1402, 0.0, 0.0,
    0.0100, -0.1924, 0.0, 0.0, -0.3577, -0.5301, 0.0, 0.0, -0.3169, 0.1063, 0.0, 0.0, 0.0103,
    -0.5869, 0.0, 0.0, -0.0897, -0.4940, 0.0, 0.0, 0.7119, -0.0154, 0.0, 0.0, -0.0533, 0.0596, 0.0,
    0.0, 0.0352, -0.0631, 0.0, 0.0, -0.4776, 0.2847, 0.0, 0.0,
];

pub struct PostProcess {
    scene: TextureBundle,
    scene_msaa: Option<MsaaTarget>,
    ssao: TextureBundle,
    bloom_ping: TextureBundle,
    bloom_pong: TextureBundle,
    sampler_linear: wgpu::Sampler,
    sampler_clamp: wgpu::Sampler,
    _noise_texture: wgpu::Texture,
    noise_view: wgpu::TextureView,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    ssao_layout: wgpu::BindGroupLayout,
    ssao_pipeline: wgpu::RenderPipeline,
    bloom_prefilter_layout: wgpu::BindGroupLayout,
    bloom_prefilter_pipeline: wgpu::RenderPipeline,
    bloom_blur_layout: wgpu::BindGroupLayout,
    bloom_blur_horizontal: wgpu::RenderPipeline,
    bloom_blur_vertical: wgpu::RenderPipeline,
    composite_layout: wgpu::BindGroupLayout,
    composite_pipeline: wgpu::RenderPipeline,
    size: wgpu::Extent3d,
    last_proj: Mat4,
    last_near: f32,
    last_far: f32,
    sample_count: u32,
}

impl PostProcess {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        config: &wgpu::SurfaceConfiguration,
        sample_count: u32,
    ) -> Self {
        let size = wgpu::Extent3d {
            width: config.width.max(1),
            height: config.height.max(1),
            depth_or_array_layers: 1,
        };

        let sampler_linear = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("PostProcessLinearSampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let sampler_clamp = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("PostProcessClampSampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let (scene, scene_msaa) =
            Self::create_scene_targets(device, &size, config.format, sample_count);
        let ssao = TextureBundle::ssao(device, &size);
        let bloom_ping = TextureBundle::bloom(device, &size, "BloomPing");
        let bloom_pong = TextureBundle::bloom(device, &size, "BloomPong");

        let depth_multisampled = sample_count > 1;

        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("PostProcessUniformLayout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: Some(
                        wgpu::BufferSize::new(std::mem::size_of::<PostProcessUniform>() as u64)
                            .expect("post process uniform must have non-zero size"),
                    ),
                },
                count: None,
            }],
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("PostProcessUniformBuffer"),
            size: std::mem::size_of::<PostProcessUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("PostProcessUniformBindGroup"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let noise_texture = Self::create_noise_texture(device, queue);
        let noise_view = noise_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let postprocess_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("PostProcessShader"),
            source: if depth_multisampled {
                wgpu::ShaderSource::Wgsl(include_str!("../../shader/postprocess_msaa.wgsl").into())
            } else {
                wgpu::ShaderSource::Wgsl(include_str!("../../shader/postprocess.wgsl").into())
            },
        });

        let fullscreen_vertex = wgpu::VertexState {
            module: &postprocess_shader,
            entry_point: Some("vs_fullscreen"),
            buffers: &[],
            compilation_options: Default::default(),
        };

        // SSAO pipeline setup
        let ssao_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("SsaoInputLayout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: depth_multisampled,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let ssao_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("SsaoPipelineLayout"),
            bind_group_layouts: &[&uniform_layout, &ssao_layout],
            push_constant_ranges: &[],
        });

        let ssao_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("SsaoPipeline"),
            layout: Some(&ssao_pipeline_layout),
            vertex: fullscreen_vertex.clone(),
            fragment: Some(wgpu::FragmentState {
                module: &postprocess_shader,
                entry_point: Some("fs_ssao"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::R8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // Bloom prefilter pipeline
        let bloom_prefilter_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("BloomPrefilterLayout"),
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

        let bloom_prefilter_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("BloomPrefilterPipelineLayout"),
                bind_group_layouts: &[&bloom_prefilter_layout],
                push_constant_ranges: &[],
            });

        let bloom_prefilter_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("BloomPrefilterPipeline"),
                layout: Some(&bloom_prefilter_pipeline_layout),
                vertex: fullscreen_vertex.clone(),
                fragment: Some(wgpu::FragmentState {
                    module: &postprocess_shader,
                    entry_point: Some("fs_bloom_prefilter"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: bloom_ping.format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            });

        // Bloom blur pipeline (horizontal & vertical)
        let bloom_blur_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("BloomBlurLayout"),
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

        let bloom_blur_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("BloomBlurPipelineLayout"),
                bind_group_layouts: &[&bloom_blur_layout],
                push_constant_ranges: &[],
            });

        let bloom_blur_horizontal =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("BloomBlurHorizontal"),
                layout: Some(&bloom_blur_pipeline_layout),
                vertex: fullscreen_vertex.clone(),
                fragment: Some(wgpu::FragmentState {
                    module: &postprocess_shader,
                    entry_point: Some("fs_bloom_blur_horizontal"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: bloom_pong.format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            });

        let bloom_blur_vertical = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("BloomBlurVertical"),
            layout: Some(&bloom_blur_pipeline_layout),
            vertex: fullscreen_vertex.clone(),
            fragment: Some(wgpu::FragmentState {
                module: &postprocess_shader,
                entry_point: Some("fs_bloom_blur_vertical"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: bloom_ping.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // Composite pipeline
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("CompositeLayout"),
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
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let composite_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("CompositePipelineLayout"),
                bind_group_layouts: &[&composite_layout, &uniform_layout],
                push_constant_ranges: &[],
            });

        let composite_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("CompositePipeline"),
            layout: Some(&composite_pipeline_layout),
            vertex: fullscreen_vertex,
            fragment: Some(wgpu::FragmentState {
                module: &postprocess_shader,
                entry_point: Some("fs_composite"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let post = Self {
            scene,
            scene_msaa,
            ssao,
            bloom_ping,
            bloom_pong,
            sampler_linear,
            sampler_clamp,
            _noise_texture: noise_texture,
            noise_view,
            uniform_buffer,
            uniform_bind_group,
            ssao_layout,
            ssao_pipeline,
            bloom_prefilter_layout,
            bloom_prefilter_pipeline,
            bloom_blur_layout,
            bloom_blur_horizontal,
            bloom_blur_vertical,
            composite_layout,
            composite_pipeline,
            size,
            last_proj: Mat4::IDENTITY,
            last_near: 0.01,
            last_far: 100.0,
            sample_count,
        };

        let initial_uniform = PostProcessUniform::new(
            post.last_proj,
            post.last_proj.inverse(),
            post.size.width as f32,
            post.size.height as f32,
            post.last_near,
            post.last_far,
        );
        queue.write_buffer(
            &post.uniform_buffer,
            0,
            bytemuck::bytes_of(&initial_uniform),
        );

        post
    }

    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) {
        if width == 0 || height == 0 {
            return;
        }
        self.size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let (scene, scene_msaa) =
            Self::create_scene_targets(device, &self.size, format, self.sample_count);
        self.scene = scene;
        self.scene_msaa = scene_msaa;
        self.ssao = TextureBundle::ssao(device, &self.size);
        self.bloom_ping = TextureBundle::bloom(device, &self.size, "BloomPing");
        self.bloom_pong = TextureBundle::bloom(device, &self.size, "BloomPong");
        self.upload_uniform(queue);
    }

    pub fn update_camera(&mut self, queue: &wgpu::Queue, proj: Mat4, near: f32, far: f32) {
        self.last_proj = proj;
        self.last_near = near;
        self.last_far = far;
        self.upload_uniform(queue);
    }

    pub fn scene_color_views(&self) -> (&wgpu::TextureView, Option<&wgpu::TextureView>) {
        match self.scene_msaa.as_ref() {
            Some(msaa) => (&msaa.view, Some(&self.scene.view)),
            None => (&self.scene.view, None),
        }
    }

    pub fn scene_view(&self) -> &wgpu::TextureView {
        &self.scene.view
    }

    pub fn ssao_texture(&self) -> &wgpu::TextureView {
        &self.ssao.view
    }

    pub fn bloom_texture(&self) -> &wgpu::TextureView {
        &self.bloom_ping.view
    }

    pub fn execute(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        device: &wgpu::Device,
        depth_view: &wgpu::TextureView,
        target: &wgpu::TextureView,
    ) {
        // SSAO
        let ssao_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("SsaoBindGroup"),
            layout: &self.ssao_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.noise_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler_clamp),
                },
            ],
        });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("SsaoPass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.ssao.view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.ssao_pipeline);
            pass.set_bind_group(0, &self.uniform_bind_group, &[]);
            pass.set_bind_group(1, &ssao_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        let bloom_prefilter_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("BloomPrefilterBindGroup"),
            layout: &self.bloom_prefilter_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.scene.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler_linear),
                },
            ],
        });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("BloomPrefilter"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.bloom_ping.view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.bloom_prefilter_pipeline);
            pass.set_bind_group(0, &bloom_prefilter_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        let horizontal_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("BloomHorizontalBindGroup"),
            layout: &self.bloom_blur_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.bloom_ping.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler_linear),
                },
            ],
        });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("BloomBlurHorizontal"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.bloom_pong.view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.bloom_blur_horizontal);
            pass.set_bind_group(0, &horizontal_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        let vertical_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("BloomVerticalBindGroup"),
            layout: &self.bloom_blur_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.bloom_pong.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler_linear),
                },
            ],
        });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("BloomBlurVertical"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.bloom_ping.view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.bloom_blur_vertical);
            pass.set_bind_group(0, &vertical_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        let composite_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("CompositeBindGroup"),
            layout: &self.composite_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.scene.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.ssao.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.bloom_ping.view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler_linear),
                },
            ],
        });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("CompositePass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.composite_pipeline);
            pass.set_bind_group(0, &composite_bind_group, &[]);
            pass.set_bind_group(1, &self.uniform_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

impl PostProcess {
    fn upload_uniform(&self, queue: &wgpu::Queue) {
        let proj_inv = self.last_proj.inverse();
        let uniform = PostProcessUniform::new(
            self.last_proj,
            proj_inv,
            self.size.width as f32,
            self.size.height as f32,
            self.last_near,
            self.last_far,
        );
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
    }

    fn create_noise_texture(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Texture {
        let data_bytes = bytemuck::cast_slice(&SSAO_NOISE_DATA);

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("SsaoNoiseTexture"),
            size: wgpu::Extent3d {
                width: NOISE_TEXTURE_SIZE,
                height: NOISE_TEXTURE_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data_bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some((4 * std::mem::size_of::<f32>()) as u32 * NOISE_TEXTURE_SIZE),
                rows_per_image: Some(NOISE_TEXTURE_SIZE),
            },
            wgpu::Extent3d {
                width: NOISE_TEXTURE_SIZE,
                height: NOISE_TEXTURE_SIZE,
                depth_or_array_layers: 1,
            },
        );

        texture
    }

    fn create_scene_targets(
        device: &wgpu::Device,
        size: &wgpu::Extent3d,
        format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> (TextureBundle, Option<MsaaTarget>) {
        let resolved = TextureBundle::color(device, size, format, "SceneColor");
        let msaa = if sample_count > 1 {
            Some(MsaaTarget::new(
                device,
                size,
                format,
                sample_count,
                "SceneColorMsaa",
            ))
        } else {
            None
        };

        (resolved, msaa)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PostProcessUniform {
    proj: [[f32; 4]; 4],
    proj_inv: [[f32; 4]; 4],
    resolution: [f32; 2],
    radius_bias: [f32; 2],
    intensity_power: [f32; 2],
    noise_scale: [f32; 2],
    near_far: [f32; 2],
    _padding: [f32; 2],
}

impl PostProcessUniform {
    fn new(proj: Mat4, proj_inv: Mat4, width: f32, height: f32, near: f32, far: f32) -> Self {
        let radius = 5.5f32;
        let bias = 0.025f32;
        let intensity = 3.5f32;
        let power = 1.5f32;
        let noise_scale = [
            width / NOISE_TEXTURE_SIZE as f32,
            height / NOISE_TEXTURE_SIZE as f32,
        ];
        Self {
            proj: proj.to_cols_array_2d(),
            proj_inv: proj_inv.to_cols_array_2d(),
            resolution: [width, height],
            radius_bias: [radius, bias],
            intensity_power: [intensity, power],
            noise_scale,
            near_far: [near, far],
            _padding: [0.0, 0.0],
        }
    }
}

struct MsaaTarget {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl MsaaTarget {
    fn new(
        device: &wgpu::Device,
        size: &wgpu::Extent3d,
        format: wgpu::TextureFormat,
        sample_count: u32,
        label: &str,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: *size,
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            _texture: texture,
            view,
        }
    }
}

#[derive(Clone)]
struct TextureBundle {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    format: wgpu::TextureFormat,
}

impl TextureBundle {
    fn color(
        device: &wgpu::Device,
        size: &wgpu::Extent3d,
        format: wgpu::TextureFormat,
        label: &str,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: *size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            _texture: texture,
            view,
            format,
        }
    }

    fn ssao(device: &wgpu::Device, size: &wgpu::Extent3d) -> Self {
        let format = wgpu::TextureFormat::R8Unorm;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("SsaoTexture"),
            size: *size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            _texture: texture,
            view,
            format,
        }
    }

    fn bloom(device: &wgpu::Device, size: &wgpu::Extent3d, label: &str) -> Self {
        let format = wgpu::TextureFormat::Rgba16Float;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: *size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            _texture: texture,
            view,
            format,
        }
    }
}
