use crate::renderer::PipelineBuilder;

use super::{begin_color_pass, fullscreen_vertex, ColorAttachment};

pub struct SsaoStage {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    blur_layout: wgpu::BindGroupLayout,
    blur_horizontal_pipeline: wgpu::RenderPipeline,
    blur_vertical_pipeline: wgpu::RenderPipeline,
    bind_group: Option<wgpu::BindGroup>,
    blur_horizontal_bind_group: Option<wgpu::BindGroup>,
    blur_vertical_bind_group: Option<wgpu::BindGroup>,
}

pub struct SsaoTargets<'a> {
    pub output: &'a wgpu::TextureView,
    pub ping: &'a wgpu::TextureView,
}

pub struct SsaoBindGroupInputs<'a> {
    pub depth_view: &'a wgpu::TextureView,
    pub noise_view: &'a wgpu::TextureView,
    pub sampler_noise: &'a wgpu::Sampler,
    pub sampler_linear: &'a wgpu::Sampler,
    pub normal_view: &'a wgpu::TextureView,
    pub position_view: &'a wgpu::TextureView,
    pub targets: SsaoTargets<'a>,
}

impl SsaoStage {
    pub fn new(
        device: &wgpu::Device,
        uniform_layout: &wgpu::BindGroupLayout,
        shader: &wgpu::ShaderModule,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("SsaoPipelineLayout"),
            bind_group_layouts: &[uniform_layout, &layout],
            push_constant_ranges: &[],
        });

        let pipeline = PipelineBuilder::new(device, &pipeline_layout, shader)
            .with_label("SsaoPipeline")
            .with_vertex_entry("vs_fullscreen")
            .with_fragment_entry("fs_ssao")
            .with_color_target(wgpu::TextureFormat::R8Unorm, None)
            .with_vertex_state(fullscreen_vertex(shader))
            .with_no_culling()
            .build();

        let blur_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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

        let blur_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("SsaoBlurPipelineLayout"),
            bind_group_layouts: &[uniform_layout, &blur_layout],
            push_constant_ranges: &[],
        });

        let blur_horizontal_pipeline = PipelineBuilder::new(device, &blur_pipeline_layout, shader)
            .with_label("SsaoBlurHorizontalPipeline")
            .with_vertex_entry("vs_fullscreen")
            .with_fragment_entry("fs_ssao_blur_horizontal")
            .with_color_target(wgpu::TextureFormat::R8Unorm, None)
            .with_vertex_state(fullscreen_vertex(shader))
            .with_no_culling()
            .build();

        let blur_vertical_pipeline = PipelineBuilder::new(device, &blur_pipeline_layout, shader)
            .with_label("SsaoBlurVerticalPipeline")
            .with_vertex_entry("vs_fullscreen")
            .with_fragment_entry("fs_ssao_blur_vertical")
            .with_color_target(wgpu::TextureFormat::R8Unorm, None)
            .with_vertex_state(fullscreen_vertex(shader))
            .with_no_culling()
            .build();

        Self {
            layout,
            pipeline,
            blur_layout,
            blur_horizontal_pipeline,
            blur_vertical_pipeline,
            bind_group: None,
            blur_horizontal_bind_group: None,
            blur_vertical_bind_group: None,
        }
    }

    pub fn invalidate(&mut self) {
        self.bind_group = None;
        self.blur_horizontal_bind_group = None;
        self.blur_vertical_bind_group = None;
    }

    pub fn ensure_bind_groups(&mut self, device: &wgpu::Device, inputs: SsaoBindGroupInputs<'_>) {
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("SsaoBindGroup"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(inputs.depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(inputs.noise_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(inputs.sampler_noise),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(inputs.normal_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(inputs.position_view),
                },
            ],
        }));

        self.blur_horizontal_bind_group =
            Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("SsaoBlurHorizontalBindGroup"),
                layout: &self.blur_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 60,
                        resource: wgpu::BindingResource::TextureView(inputs.targets.output),
                    },
                    wgpu::BindGroupEntry {
                        binding: 61,
                        resource: wgpu::BindingResource::Sampler(inputs.sampler_linear),
                    },
                ],
            }));

        self.blur_vertical_bind_group =
            Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("SsaoBlurVerticalBindGroup"),
                layout: &self.blur_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 60,
                        resource: wgpu::BindingResource::TextureView(inputs.targets.ping),
                    },
                    wgpu::BindGroupEntry {
                        binding: 61,
                        resource: wgpu::BindingResource::Sampler(inputs.sampler_linear),
                    },
                ],
            }));
    }

    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        uniform_bind_group: &wgpu::BindGroup,
        targets: SsaoTargets<'_>,
    ) {
        let ssao_bind_group = self
            .bind_group
            .as_ref()
            .expect("SSAO bind group not initialized");

        {
            let mut pass = begin_color_pass(
                encoder,
                "SsaoPass",
                ColorAttachment::single(targets.output),
                wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                None,
            );
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, uniform_bind_group, &[]);
            pass.set_bind_group(1, ssao_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        let blur_horizontal = self
            .blur_horizontal_bind_group
            .as_ref()
            .expect("SSAO horizontal blur bind group not initialized");
        let blur_vertical = self
            .blur_vertical_bind_group
            .as_ref()
            .expect("SSAO vertical blur bind group not initialized");

        {
            let mut pass = begin_color_pass(
                encoder,
                "SsaoBlurHorizontal",
                ColorAttachment::single(targets.ping),
                wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                None,
            );
            pass.set_pipeline(&self.blur_horizontal_pipeline);
            pass.set_bind_group(0, uniform_bind_group, &[]);
            pass.set_bind_group(1, blur_horizontal, &[]);
            pass.draw(0..3, 0..1);
        }

        {
            let mut pass = begin_color_pass(
                encoder,
                "SsaoBlurVertical",
                ColorAttachment::single(targets.output),
                wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                None,
            );
            pass.set_pipeline(&self.blur_vertical_pipeline);
            pass.set_bind_group(0, uniform_bind_group, &[]);
            pass.set_bind_group(1, blur_vertical, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    pub fn clear(&self, encoder: &mut wgpu::CommandEncoder, targets: SsaoTargets<'_>) {
        {
            let _pass = begin_color_pass(
                encoder,
                "SsaoClear",
                ColorAttachment::single(targets.output),
                wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                None,
            );
        }

        {
            let _pass = begin_color_pass(
                encoder,
                "SsaoPingClear",
                ColorAttachment::single(targets.ping),
                wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                None,
            );
        }
    }
}
