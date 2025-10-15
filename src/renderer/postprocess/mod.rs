use crate::environment::ColorGrading;
use crate::renderer::{PipelineBuilder, RenderRegion, ShaderBuilder};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec2, Vec3};

const NOISE_TEXTURE_SIZE: u32 = 4;
const BLOOM_MIP_COUNT: usize = 5;
const BLOOM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const SSAO_NOISE_DATA: [f32; (NOISE_TEXTURE_SIZE * NOISE_TEXTURE_SIZE * 4) as usize] = [
    -0.6401949,
    -0.76821256,
    0.0,
    0.0,
    0.98767775,
    0.1565012,
    0.0,
    0.0,
    -0.1566164,
    0.9876595,
    0.0,
    0.0,
    0.1675282,
    0.98586726,
    0.0,
    0.0,
    -0.08490153,
    -0.9963893,
    0.0,
    0.0,
    -0.44445047,
    -0.89580345,
    0.0,
    0.0,
    0.77917,
    -0.62681264,
    0.0,
    0.0,
    0.85447717,
    0.519489,
    0.0,
    0.0,
    -0.88205993,
    0.471_137_3,
    0.0,
    0.0,
    0.98252517,
    0.18612963,
    0.0,
    0.0,
    0.19578062,
    0.98064774,
    0.0,
    0.0,
    -0.99943393,
    -0.03364192,
    0.0,
    0.0,
    0.9861326,
    0.165959,
    0.0,
    0.0,
    0.3159545,
    0.94877434,
    0.0,
    0.0,
    -0.5883725,
    -0.80859,
    0.0,
    0.0,
    -0.96039623,
    -0.278638,
    0.0,
    0.0,
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PostProcessEffects {
    pub ssao: bool,
    pub bloom: bool,
    pub fxaa: bool,
    pub ssao_settings: SsaoSettings,
    pub bloom_settings: BloomSettings,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SsaoSettings {
    pub radius: f32,
    pub bias: f32,
    pub intensity: f32,
    pub power: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BloomSettings {
    pub threshold: f32,
    pub knee: f32,
    pub scatter: f32,
}

impl Default for PostProcessEffects {
    fn default() -> Self {
        Self {
            ssao: true,
            bloom: true,
            fxaa: true,
            ssao_settings: SsaoSettings::default(),
            bloom_settings: BloomSettings::default(),
        }
    }
}

impl Default for SsaoSettings {
    fn default() -> Self {
        Self {
            radius: 0.2,
            bias: 0.05,
            intensity: 0.75,
            power: 1.25,
        }
    }
}

impl Default for BloomSettings {
    fn default() -> Self {
        Self {
            threshold: 0.8,
            knee: 0.4,
            scatter: 0.95,
        }
    }
}

impl PostProcessEffects {
    fn uniform_components(self) -> [f32; 4] {
        [
            if self.ssao { 1.0 } else { 0.0 },
            if self.bloom { 1.0 } else { 0.0 },
            if self.fxaa { 1.0 } else { 0.0 },
            0.0,
        ]
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PostProcessCamera {
    pub proj: Mat4,
    pub view_proj: Mat4,
    pub view_proj_inv: Mat4,
    pub position: Vec3,
    pub near: f32,
    pub far: f32,
}

pub struct PostProcess {
    scene_source: TextureBundle,
    scene: TextureBundle,
    scene_msaa: Option<MsaaTarget>,
    ssao: TextureBundle,
    ssao_ping: TextureBundle,
    bloom_down_chain: Vec<BloomMip>,
    bloom_up_chain: Vec<BloomMip>,
    sampler_linear: wgpu::Sampler,
    sampler_noise: wgpu::Sampler,
    _noise_texture: wgpu::Texture,
    noise_view: wgpu::TextureView,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    color_grading_layout: wgpu::BindGroupLayout,
    color_grading_pipeline: wgpu::RenderPipeline,
    depth_resolve_layout: Option<wgpu::BindGroupLayout>,
    depth_resolve_pipeline: Option<wgpu::RenderPipeline>,
    depth_resolve_bind_group: Option<wgpu::BindGroup>,
    ssao_layout: wgpu::BindGroupLayout,
    ssao_pipeline: wgpu::RenderPipeline,
    ssao_blur_layout: wgpu::BindGroupLayout,
    ssao_blur_horizontal_pipeline: wgpu::RenderPipeline,
    ssao_blur_vertical_pipeline: wgpu::RenderPipeline,
    bloom_prefilter_layout: wgpu::BindGroupLayout,
    bloom_prefilter_pipeline: wgpu::RenderPipeline,
    bloom_downsample_layout: wgpu::BindGroupLayout,
    bloom_downsample_pipeline: wgpu::RenderPipeline,
    bloom_upsample_layout: wgpu::BindGroupLayout,
    bloom_upsample_pipeline: wgpu::RenderPipeline,
    composite_layout: wgpu::BindGroupLayout,
    composite_pipeline: wgpu::RenderPipeline,
    size: wgpu::Extent3d,
    scene_format: wgpu::TextureFormat,
    effects: PostProcessEffects,
    ssao_bind_group: Option<wgpu::BindGroup>,
    ssao_blur_horizontal_bind_group: Option<wgpu::BindGroup>,
    ssao_blur_vertical_bind_group: Option<wgpu::BindGroup>,
    bloom_prefilter_bind_group: Option<wgpu::BindGroup>,
    bloom_downsample_passes: Vec<BloomDownsamplePass>,
    bloom_upsample_passes: Vec<BloomUpsamplePass>,
    composite_bind_group: Option<wgpu::BindGroup>,
    color_grading_bind_group: Option<wgpu::BindGroup>,
    resolved_depth: Option<TextureBundle>,
    cached_depth_view: Option<wgpu::TextureView>,
    bind_groups_dirty: bool,
    last_proj: Mat4,
    last_view_proj: Mat4,
    last_view_proj_inv: Mat4,
    last_camera_position: Vec3,
    last_near: f32,
    last_far: f32,
    viewport_resolution: Vec2,
    viewport_offset: Vec2,
    viewport_scale: Vec2,
    sample_count: u32,
    color_grading: ColorGrading,
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
        let scene_format = config.format.remove_srgb_suffix();

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

        let sampler_noise = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("PostProcessNoiseSampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let (scene_source, scene, scene_msaa) =
            Self::create_scene_targets(device, &size, scene_format, sample_count);
        let ssao = TextureBundle::ssao(device, &size);
        let ssao_ping = TextureBundle::ssao(device, &size);
        let (bloom_down_chain, bloom_up_chain) = Self::create_bloom_chain(device, &size);

        let resolved_depth = if sample_count > 1 {
            Some(TextureBundle::depth(device, &size, "ResolvedDepth"))
        } else {
            None
        };

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

        let postprocess_source =
            ShaderBuilder::new().build(include_str!("../../shader/postprocess.wgsl"));

        let postprocess_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("PostProcessShader"),
            source: wgpu::ShaderSource::Wgsl(postprocess_source.into()),
        });

        let fullscreen_vertex = wgpu::VertexState {
            module: &postprocess_shader,
            entry_point: Some("vs_fullscreen"),
            buffers: &[],
            compilation_options: Default::default(),
        };

        let color_grading_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("ColorGradingLayout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 10,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 11,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let color_grading_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("ColorGradingPipelineLayout"),
                bind_group_layouts: &[&uniform_layout, &color_grading_layout],
                push_constant_ranges: &[],
            });

        let color_grading_pipeline =
            PipelineBuilder::new(device, &color_grading_pipeline_layout, &postprocess_shader)
                .with_label("ColorGradingPipeline")
                .with_vertex_entry("vs_fullscreen")
                .with_fragment_entry("fs_color_adjust")
                .with_color_target(scene_format, Some(wgpu::BlendState::REPLACE))
                .with_vertex_state(fullscreen_vertex.clone())
                .with_no_culling()
                .build();

        let (depth_resolve_layout, depth_resolve_pipeline) = if sample_count > 1 {
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("DepthResolveLayout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: true,
                    },
                    count: None,
                }],
            });
            let depth_resolve_source =
                ShaderBuilder::new().build(include_str!("../../shader/depth_resolve.wgsl"));

            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("DepthResolveShader"),
                source: wgpu::ShaderSource::Wgsl(depth_resolve_source.into()),
            });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("DepthResolvePipelineLayout"),
                bind_group_layouts: &[&uniform_layout, &layout],
                push_constant_ranges: &[],
            });
            let pipeline = PipelineBuilder::new(device, &pipeline_layout, &shader)
                .with_label("DepthResolvePipeline")
                .with_vertex_entry("vs_fullscreen")
                .with_fragment_entry("fs_resolve_depth")
                .with_depth_stencil(
                    wgpu::TextureFormat::Depth32Float,
                    true,
                    wgpu::CompareFunction::Always,
                )
                .with_vertex_state(fullscreen_vertex.clone())
                .with_no_culling()
                .build();
            (Some(layout), Some(pipeline))
        } else {
            (None, None)
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

        let ssao_pipeline =
            PipelineBuilder::new(device, &ssao_pipeline_layout, &postprocess_shader)
                .with_label("SsaoPipeline")
                .with_vertex_entry("vs_fullscreen")
                .with_fragment_entry("fs_ssao")
                .with_color_target(wgpu::TextureFormat::R8Unorm, None)
                .with_vertex_state(fullscreen_vertex.clone())
                .with_no_culling()
                .build();

        let ssao_blur_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("SsaoBlurLayout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 60,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 61,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let ssao_blur_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("SsaoBlurPipelineLayout"),
                bind_group_layouts: &[&uniform_layout, &ssao_blur_layout],
                push_constant_ranges: &[],
            });

        let ssao_blur_horizontal_pipeline =
            PipelineBuilder::new(device, &ssao_blur_pipeline_layout, &postprocess_shader)
                .with_label("SsaoBlurHorizontalPipeline")
                .with_vertex_entry("vs_fullscreen")
                .with_fragment_entry("fs_ssao_blur_horizontal")
                .with_color_target(wgpu::TextureFormat::R8Unorm, None)
                .with_vertex_state(fullscreen_vertex.clone())
                .with_no_culling()
                .build();

        let ssao_blur_vertical_pipeline =
            PipelineBuilder::new(device, &ssao_blur_pipeline_layout, &postprocess_shader)
                .with_label("SsaoBlurVerticalPipeline")
                .with_vertex_entry("vs_fullscreen")
                .with_fragment_entry("fs_ssao_blur_vertical")
                .with_color_target(wgpu::TextureFormat::R8Unorm, None)
                .with_vertex_state(fullscreen_vertex.clone())
                .with_no_culling()
                .build();

        // Bloom prefilter pipeline
        let bloom_prefilter_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("BloomPrefilterLayout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 20,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 21,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let bloom_prefilter_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("BloomPrefilterPipelineLayout"),
                bind_group_layouts: &[&uniform_layout, &bloom_prefilter_layout],
                push_constant_ranges: &[],
            });

        let bloom_prefilter_pipeline = PipelineBuilder::new(
            device,
            &bloom_prefilter_pipeline_layout,
            &postprocess_shader,
        )
        .with_label("BloomPrefilterPipeline")
        .with_vertex_entry("vs_fullscreen")
        .with_fragment_entry("fs_bloom_prefilter")
        .with_color_target(BLOOM_FORMAT, Some(wgpu::BlendState::REPLACE))
        .with_vertex_state(fullscreen_vertex.clone())
        .with_no_culling()
        .build();

        let bloom_downsample_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("BloomDownsampleLayout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 30,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 31,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let bloom_downsample_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("BloomDownsamplePipelineLayout"),
                bind_group_layouts: &[&uniform_layout, &bloom_downsample_layout],
                push_constant_ranges: &[],
            });

        let bloom_downsample_pipeline = PipelineBuilder::new(
            device,
            &bloom_downsample_pipeline_layout,
            &postprocess_shader,
        )
        .with_label("BloomDownsamplePipeline")
        .with_vertex_entry("vs_fullscreen")
        .with_fragment_entry("fs_bloom_downsample")
        .with_color_target(BLOOM_FORMAT, Some(wgpu::BlendState::REPLACE))
        .with_vertex_state(fullscreen_vertex.clone())
        .with_no_culling()
        .build();

        let bloom_upsample_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("BloomUpsampleLayout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 40,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 41,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 42,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let bloom_upsample_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("BloomUpsamplePipelineLayout"),
                bind_group_layouts: &[&uniform_layout, &bloom_upsample_layout],
                push_constant_ranges: &[],
            });

        let bloom_upsample_pipeline =
            PipelineBuilder::new(device, &bloom_upsample_pipeline_layout, &postprocess_shader)
                .with_label("BloomUpsamplePipeline")
                .with_vertex_entry("vs_fullscreen")
                .with_fragment_entry("fs_bloom_upsample")
                .with_color_target(BLOOM_FORMAT, Some(wgpu::BlendState::REPLACE))
                .with_vertex_state(fullscreen_vertex.clone())
                .with_no_culling()
                .build();

        // Composite pipeline
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("CompositeLayout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 50,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 51,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 52,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 53,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let composite_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("CompositePipelineLayout"),
                bind_group_layouts: &[&uniform_layout, &composite_layout],
                push_constant_ranges: &[],
            });

        let composite_pipeline =
            PipelineBuilder::new(device, &composite_pipeline_layout, &postprocess_shader)
                .with_label("CompositePipeline")
                .with_vertex_entry("vs_fullscreen")
                .with_fragment_entry("fs_composite")
                .with_color_target(config.format, Some(wgpu::BlendState::REPLACE))
                .with_vertex_state(fullscreen_vertex.clone())
                .with_no_culling()
                .build();

        let post = Self {
            scene_source,
            scene,
            scene_msaa,
            ssao,
            ssao_ping,
            bloom_down_chain,
            bloom_up_chain,
            sampler_linear,
            sampler_noise,
            _noise_texture: noise_texture,
            noise_view,
            uniform_buffer,
            uniform_bind_group,
            color_grading_layout,
            color_grading_pipeline,
            depth_resolve_layout,
            depth_resolve_pipeline,
            depth_resolve_bind_group: None,
            ssao_layout,
            ssao_pipeline,
            ssao_blur_layout,
            ssao_blur_horizontal_pipeline,
            ssao_blur_vertical_pipeline,
            bloom_prefilter_layout,
            bloom_prefilter_pipeline,
            bloom_downsample_layout,
            bloom_downsample_pipeline,
            bloom_upsample_layout,
            bloom_upsample_pipeline,
            composite_layout,
            composite_pipeline,
            size,
            scene_format,
            effects: PostProcessEffects::default(),
            ssao_bind_group: None,
            ssao_blur_horizontal_bind_group: None,
            ssao_blur_vertical_bind_group: None,
            bloom_prefilter_bind_group: None,
            bloom_downsample_passes: Vec::new(),
            bloom_upsample_passes: Vec::new(),
            composite_bind_group: None,
            color_grading_bind_group: None,
            resolved_depth,
            cached_depth_view: None,
            bind_groups_dirty: true,
            last_proj: Mat4::IDENTITY,
            last_view_proj: Mat4::IDENTITY,
            last_view_proj_inv: Mat4::IDENTITY,
            last_camera_position: Vec3::ZERO,
            last_near: 0.01,
            last_far: 100.0,
            viewport_resolution: Vec2::new(size.width as f32, size.height as f32),
            viewport_offset: Vec2::ZERO,
            viewport_scale: Vec2::ONE,
            sample_count,
            color_grading: ColorGrading::default(),
        };

        let initial_uniform = PostProcessUniform::new(
            post.last_proj,
            post.last_proj.inverse(),
            post.last_view_proj,
            post.last_view_proj_inv,
            post.last_camera_position,
            post.viewport_resolution,
            post.viewport_offset,
            post.viewport_scale,
            post.last_near,
            post.last_far,
            post.effects,
            post.color_grading,
            post.sample_count,
        );
        queue.write_buffer(
            &post.uniform_buffer,
            0,
            bytemuck::bytes_of(&initial_uniform),
        );

        post
    }

    pub fn resize(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let (scene_source, scene, scene_msaa) =
            Self::create_scene_targets(device, &self.size, self.scene_format, self.sample_count);
        self.scene_source = scene_source;
        self.scene = scene;
        self.scene_msaa = scene_msaa;
        self.ssao = TextureBundle::ssao(device, &self.size);
        self.ssao_ping = TextureBundle::ssao(device, &self.size);
        self.resolved_depth = if self.sample_count > 1 {
            Some(TextureBundle::depth(device, &self.size, "ResolvedDepth"))
        } else {
            None
        };
        let (down_chain, up_chain) = Self::create_bloom_chain(device, &self.size);
        self.bloom_down_chain = down_chain;
        self.bloom_up_chain = up_chain;
        self.viewport_resolution = Vec2::new(width as f32, height as f32);
        self.viewport_offset = Vec2::ZERO;
        self.viewport_scale = Vec2::ONE;
        self.mark_bind_groups_dirty();
        self.upload_uniform(queue);
    }

    pub fn update_viewport(&mut self, queue: &wgpu::Queue, region: Option<RenderRegion>) {
        let full_width = self.size.width.max(1) as f32;
        let full_height = self.size.height.max(1) as f32;

        let (resolution, offset, scale) = if let Some(region) = region {
            let region_width = region.width().max(1) as f32;
            let region_height = region.height().max(1) as f32;
            let offset = Vec2::new(
                region.x() as f32 / full_width,
                region.y() as f32 / full_height,
            );
            let scale = Vec2::new(region_width / full_width, region_height / full_height);
            (Vec2::new(region_width, region_height), offset, scale)
        } else {
            (Vec2::new(full_width, full_height), Vec2::ZERO, Vec2::ONE)
        };

        if self.viewport_resolution != resolution
            || self.viewport_offset != offset
            || self.viewport_scale != scale
        {
            self.viewport_resolution = resolution;
            self.viewport_offset = offset;
            self.viewport_scale = scale;
            self.upload_uniform(queue);
        }
    }

    pub fn update_camera(&mut self, queue: &wgpu::Queue, camera: PostProcessCamera) {
        self.last_proj = camera.proj;
        self.last_view_proj = camera.view_proj;
        self.last_view_proj_inv = camera.view_proj_inv;
        self.last_camera_position = camera.position;
        self.last_near = camera.near;
        self.last_far = camera.far;
        self.upload_uniform(queue);
    }

    pub fn scene_color_views(&self) -> (&wgpu::TextureView, Option<&wgpu::TextureView>) {
        match self.scene_msaa.as_ref() {
            Some(msaa) => (&msaa.view, Some(&self.scene_source.view)),
            None => (&self.scene_source.view, None),
        }
    }

    pub fn scene_view(&self) -> &wgpu::TextureView {
        &self.scene.view
    }

    pub fn ssao_texture(&self) -> &wgpu::TextureView {
        &self.ssao.view
    }

    pub fn bloom_texture(&self) -> &wgpu::TextureView {
        &self.bloom_up_chain[0].view
    }

    pub fn set_depth_view(&mut self, depth_view: &wgpu::TextureView) {
        self.cached_depth_view = Some(depth_view.clone());
        self.mark_bind_groups_dirty();
    }

    pub fn set_effects(&mut self, queue: &wgpu::Queue, effects: PostProcessEffects) {
        if self.effects != effects {
            self.effects = effects;
            self.upload_uniform(queue);
        }
    }

    pub fn effects(&self) -> PostProcessEffects {
        self.effects
    }

    pub fn set_color_grading(&mut self, queue: &wgpu::Queue, grading: ColorGrading) {
        if self.color_grading != grading {
            self.color_grading = grading;
            self.upload_uniform(queue);
        }
    }

    pub fn execute(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        device: &wgpu::Device,
        target: &wgpu::TextureView,
        region: Option<RenderRegion>,
    ) {
        self.ensure_cached_bind_groups(device);

        if let Some(color_group) = self.color_grading_bind_group.as_ref() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ColorGradingPass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.scene.view,
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
            pass.set_pipeline(&self.color_grading_pipeline);
            pass.set_bind_group(0, &self.uniform_bind_group, &[]);
            pass.set_bind_group(1, color_group, &[]);
            pass.draw(0..3, 0..1);
        }

        if self.effects.ssao {
            if let (Some(pipeline), Some(bind_group), Some(resolved)) = (
                self.depth_resolve_pipeline.as_ref(),
                self.depth_resolve_bind_group.as_ref(),
                self.resolved_depth.as_ref(),
            ) {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("DepthResolvePass"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &resolved.view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                pass.set_bind_group(1, bind_group, &[]);
                pass.draw(0..3, 0..1);
            }

            let ssao_bind_group = self
                .ssao_bind_group
                .as_ref()
                .expect("SSAO bind group not initialized");
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
                pass.set_bind_group(1, ssao_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }

            let blur_horizontal = self
                .ssao_blur_horizontal_bind_group
                .as_ref()
                .expect("SSAO horizontal blur bind group not initialized");
            let blur_vertical = self
                .ssao_blur_vertical_bind_group
                .as_ref()
                .expect("SSAO vertical blur bind group not initialized");

            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("SsaoBlurHorizontal"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.ssao_ping.view,
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
                pass.set_pipeline(&self.ssao_blur_horizontal_pipeline);
                pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                pass.set_bind_group(1, blur_horizontal, &[]);
                pass.draw(0..3, 0..1);
            }

            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("SsaoBlurVertical"),
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
                pass.set_pipeline(&self.ssao_blur_vertical_pipeline);
                pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                pass.set_bind_group(1, blur_vertical, &[]);
                pass.draw(0..3, 0..1);
            }
        } else {
            let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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
            let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("SsaoPingClear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.ssao_ping.view,
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
        }

        if self.effects.bloom {
            let bloom_prefilter = self
                .bloom_prefilter_bind_group
                .as_ref()
                .expect("Bloom prefilter bind group not initialized");

            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("BloomPrefilter"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.bloom_down_chain[0].view,
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
                pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                pass.set_bind_group(1, bloom_prefilter, &[]);
                pass.draw(0..3, 0..1);
            }

            for pass_info in &self.bloom_downsample_passes {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("BloomDownsample"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.bloom_down_chain[pass_info.target_index].view,
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
                pass.set_pipeline(&self.bloom_downsample_pipeline);
                pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                pass.set_bind_group(1, &pass_info.bind_group, &[]);
                pass.draw(0..3, 0..1);
            }

            if let (Some(last_down), Some(last_up)) =
                (self.bloom_down_chain.last(), self.bloom_up_chain.last())
            {
                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: last_down.texture(),
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: last_up.texture(),
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    last_down.extent(),
                );
            }

            for pass_info in &self.bloom_upsample_passes {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("BloomUpsample"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.bloom_up_chain[pass_info.target_index].view,
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
                pass.set_pipeline(&self.bloom_upsample_pipeline);
                pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                pass.set_bind_group(1, &pass_info.bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
        } else {
            for mip in &self.bloom_up_chain {
                let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("BloomDisabledClear"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &mip.view,
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
            }
        }

        let composite_bind_group = self
            .composite_bind_group
            .as_ref()
            .expect("Composite bind group not initialized");

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
            if let Some(region) = region {
                region.apply_to_pass(&mut pass);
            }
            pass.set_pipeline(&self.composite_pipeline);
            pass.set_bind_group(0, &self.uniform_bind_group, &[]);
            pass.set_bind_group(1, composite_bind_group, &[]);
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
            self.last_view_proj,
            self.last_view_proj_inv,
            self.last_camera_position,
            self.viewport_resolution,
            self.viewport_offset,
            self.viewport_scale,
            self.last_near,
            self.last_far,
            self.effects,
            self.color_grading,
            self.sample_count,
        );
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
    }

    fn create_bloom_chain(
        device: &wgpu::Device,
        size: &wgpu::Extent3d,
    ) -> (Vec<BloomMip>, Vec<BloomMip>) {
        let mut down_chain = Vec::with_capacity(BLOOM_MIP_COUNT);
        let mut up_chain = Vec::with_capacity(BLOOM_MIP_COUNT);
        let mut width = (size.width.max(2) / 2).max(1);
        let mut height = (size.height.max(2) / 2).max(1);

        for level in 0..BLOOM_MIP_COUNT {
            let mip_size = wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            };
            down_chain.push(BloomMip::new(
                device,
                mip_size,
                &format!("BloomDown{level}"),
            ));
            up_chain.push(BloomMip::new(device, mip_size, &format!("BloomUp{level}")));
            width = (width / 2).max(1);
            height = (height / 2).max(1);
        }

        (down_chain, up_chain)
    }

    fn mark_bind_groups_dirty(&mut self) {
        self.depth_resolve_bind_group = None;
        self.ssao_bind_group = None;
        self.ssao_blur_horizontal_bind_group = None;
        self.ssao_blur_vertical_bind_group = None;
        self.bloom_prefilter_bind_group = None;
        self.bloom_downsample_passes.clear();
        self.bloom_upsample_passes.clear();
        self.composite_bind_group = None;
        self.color_grading_bind_group = None;
        self.bind_groups_dirty = true;
    }

    fn ensure_cached_bind_groups(&mut self, device: &wgpu::Device) {
        if !self.bind_groups_dirty {
            return;
        }

        let depth_view = self
            .cached_depth_view
            .as_ref()
            .expect("Depth view must be set before executing post process");

        if let (Some(layout), Some(resolved)) = (
            self.depth_resolve_layout.as_ref(),
            self.resolved_depth.as_ref(),
        ) {
            self.depth_resolve_bind_group =
                Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("DepthResolveBindGroup"),
                    layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(depth_view),
                    }],
                }));
            self.ssao_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("SsaoBindGroup"),
                layout: &self.ssao_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 10,
                        resource: wgpu::BindingResource::TextureView(&resolved.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 11,
                        resource: wgpu::BindingResource::TextureView(&self.noise_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler_noise),
                    },
                ],
            }));
        } else {
            self.depth_resolve_bind_group = None;
            self.ssao_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
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
                        resource: wgpu::BindingResource::Sampler(&self.sampler_noise),
                    },
                ],
            }));
        }

        self.ssao_blur_horizontal_bind_group =
            Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("SsaoBlurHorizontalBindGroup"),
                layout: &self.ssao_blur_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 60,
                        resource: wgpu::BindingResource::TextureView(&self.ssao.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 61,
                        resource: wgpu::BindingResource::Sampler(&self.sampler_linear),
                    },
                ],
            }));
        self.ssao_blur_vertical_bind_group =
            Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("SsaoBlurVerticalBindGroup"),
                layout: &self.ssao_blur_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 60,
                        resource: wgpu::BindingResource::TextureView(&self.ssao_ping.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 61,
                        resource: wgpu::BindingResource::Sampler(&self.sampler_linear),
                    },
                ],
            }));

        self.color_grading_bind_group =
            Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ColorGradingBindGroup"),
                layout: &self.color_grading_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 10,
                        resource: wgpu::BindingResource::TextureView(&self.scene_source.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 11,
                        resource: wgpu::BindingResource::Sampler(&self.sampler_linear),
                    },
                ],
            }));

        self.bloom_prefilter_bind_group =
            Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("BloomPrefilterBindGroup"),
                layout: &self.bloom_prefilter_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 20,
                        resource: wgpu::BindingResource::TextureView(&self.scene.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 21,
                        resource: wgpu::BindingResource::Sampler(&self.sampler_linear),
                    },
                ],
            }));

        self.bloom_downsample_passes = self
            .bloom_down_chain
            .iter()
            .enumerate()
            .skip(1)
            .map(|(level, _)| BloomDownsamplePass {
                target_index: level,
                bind_group: device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(&format!("BloomDownsampleBindGroup{level}")),
                    layout: &self.bloom_downsample_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 30,
                            resource: wgpu::BindingResource::TextureView(
                                &self.bloom_down_chain[level - 1].view,
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 31,
                            resource: wgpu::BindingResource::Sampler(&self.sampler_linear),
                        },
                    ],
                }),
            })
            .collect();

        self.bloom_upsample_passes.clear();
        for level in (1..self.bloom_up_chain.len()).rev() {
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("BloomUpsampleBindGroup{level}")),
                layout: &self.bloom_upsample_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 40,
                        resource: wgpu::BindingResource::TextureView(
                            &self.bloom_up_chain[level].view,
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 41,
                        resource: wgpu::BindingResource::TextureView(
                            &self.bloom_down_chain[level - 1].view,
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 42,
                        resource: wgpu::BindingResource::Sampler(&self.sampler_linear),
                    },
                ],
            });
            self.bloom_upsample_passes.push(BloomUpsamplePass {
                target_index: level - 1,
                bind_group,
            });
        }

        let composite_depth_view: &wgpu::TextureView =
            if let Some(resolved) = self.resolved_depth.as_ref() {
                &resolved.view
            } else {
                depth_view
            };

        self.composite_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("CompositeBindGroup"),
            layout: &self.composite_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(composite_depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 50,
                    resource: wgpu::BindingResource::TextureView(&self.scene.view),
                },
                wgpu::BindGroupEntry {
                    binding: 51,
                    resource: wgpu::BindingResource::TextureView(&self.ssao.view),
                },
                wgpu::BindGroupEntry {
                    binding: 52,
                    resource: wgpu::BindingResource::TextureView(&self.bloom_up_chain[0].view),
                },
                wgpu::BindGroupEntry {
                    binding: 53,
                    resource: wgpu::BindingResource::Sampler(&self.sampler_linear),
                },
            ],
        }));

        self.bind_groups_dirty = false;
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
    ) -> (TextureBundle, TextureBundle, Option<MsaaTarget>) {
        let source = TextureBundle::color(device, size, format, "SceneColorSource");
        let target = TextureBundle::color(device, size, format, "SceneColor");
        let msaa = if sample_count > 1 {
            Some(MsaaTarget::new(
                device,
                size,
                format,
                sample_count,
                "SceneColorSourceMsaa",
            ))
        } else {
            None
        };

        (source, target, msaa)
    }
}

// align(16) keeps the uniform buffer size matching WGSL std140 padding rules.
#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PostProcessUniform {
    view_proj: [[f32; 4]; 4],
    view_proj_inv: [[f32; 4]; 4],
    proj: [[f32; 4]; 4],
    proj_inv: [[f32; 4]; 4],
    camera_position: [f32; 4],
    resolution: [f32; 2],
    viewport_offset: [f32; 2],
    viewport_scale: [f32; 2],
    radius_bias: [f32; 2],
    intensity_power: [f32; 2],
    noise_scale: [f32; 2],
    near_far: [f32; 2],
    // Ensure `effects` starts on a 16-byte boundary to match WGSL uniform layout.
    _padding0: [f32; 2],
    color_adjust: [f32; 4],
    bloom_params: [f32; 4],
    effects: [f32; 4],
}

impl PostProcessUniform {
    #[allow(clippy::too_many_arguments)]
    fn new(
        proj: Mat4,
        proj_inv: Mat4,
        view_proj: Mat4,
        view_proj_inv: Mat4,
        camera_position: Vec3,
        viewport_resolution: Vec2,
        viewport_offset: Vec2,
        viewport_scale: Vec2,
        near: f32,
        far: f32,
        effects: PostProcessEffects,
        grading: ColorGrading,
        sample_count: u32,
    ) -> Self {
        let ssao = effects.ssao_settings;
        let bloom = effects.bloom_settings;
        let resolution_width = viewport_resolution.x.max(1.0);
        let resolution_height = viewport_resolution.y.max(1.0);
        let noise_scale = [
            resolution_width / NOISE_TEXTURE_SIZE as f32,
            resolution_height / NOISE_TEXTURE_SIZE as f32,
        ];
        let mut effects_arr = effects.uniform_components();
        // Store sample_count in w component so the depth resolve pass can iterate samples.
        effects_arr[3] = sample_count as f32;
        Self {
            view_proj: view_proj.to_cols_array_2d(),
            view_proj_inv: view_proj_inv.to_cols_array_2d(),
            proj: proj.to_cols_array_2d(),
            proj_inv: proj_inv.to_cols_array_2d(),
            camera_position: [camera_position.x, camera_position.y, camera_position.z, 1.0],
            resolution: [resolution_width, resolution_height],
            viewport_offset: [viewport_offset.x, viewport_offset.y],
            viewport_scale: [viewport_scale.x, viewport_scale.y],
            radius_bias: [ssao.radius, ssao.bias],
            intensity_power: [ssao.intensity, ssao.power],
            noise_scale,
            near_far: [near, far],
            _padding0: [0.0, 0.0],
            color_adjust: [
                grading.exposure(),
                grading.saturation(),
                grading.contrast(),
                0.0,
            ],
            bloom_params: [bloom.threshold, bloom.knee, bloom.scatter, 0.0],
            effects: effects_arr,
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
        }
    }

    fn depth(device: &wgpu::Device, size: &wgpu::Extent3d, label: &str) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: *size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            _texture: texture,
            view,
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
        }
    }
}

struct BloomMip {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: wgpu::Extent3d,
}

impl BloomMip {
    fn new(device: &wgpu::Device, size: wgpu::Extent3d, label: &str) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: BLOOM_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            size,
        }
    }

    fn extent(&self) -> wgpu::Extent3d {
        self.size
    }

    fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }
}

struct BloomDownsamplePass {
    target_index: usize,
    bind_group: wgpu::BindGroup,
}

struct BloomUpsamplePass {
    target_index: usize,
    bind_group: wgpu::BindGroup,
}
