use crate::renderer::PipelineBuilder;

use super::resources::BloomMip;
use super::{begin_color_pass, fullscreen_vertex, ColorAttachment};

pub struct BloomStage {
    prefilter_layout: wgpu::BindGroupLayout,
    prefilter_pipeline: wgpu::RenderPipeline,
    downsample_layout: wgpu::BindGroupLayout,
    downsample_pipeline: wgpu::RenderPipeline,
    upsample_layout: wgpu::BindGroupLayout,
    upsample_pipeline: wgpu::RenderPipeline,
    prefilter_bind_group: Option<wgpu::BindGroup>,
    downsample_passes: Vec<BloomDownsamplePass>,
    upsample_passes: Vec<BloomUpsamplePass>,
}

pub struct BloomTargets<'a> {
    pub down_chain: &'a [BloomMip],
    pub up_chain: &'a [BloomMip],
}

pub struct BloomBindGroupInputs<'a> {
    pub scene_view: &'a wgpu::TextureView,
    pub sampler_linear: &'a wgpu::Sampler,
    pub targets: BloomTargets<'a>,
}

struct BloomDownsamplePass {
    target_index: usize,
    bind_group: wgpu::BindGroup,
}

struct BloomUpsamplePass {
    target_index: usize,
    bind_group: wgpu::BindGroup,
}

impl BloomStage {
    pub fn new(
        device: &wgpu::Device,
        uniform_layout: &wgpu::BindGroupLayout,
        shader: &wgpu::ShaderModule,
    ) -> Self {
        let prefilter_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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

        let prefilter_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("BloomPrefilterPipelineLayout"),
                bind_group_layouts: &[uniform_layout, &prefilter_layout],
                push_constant_ranges: &[],
            });

        let prefilter_pipeline = PipelineBuilder::new(device, &prefilter_pipeline_layout, shader)
            .with_label("BloomPrefilterPipeline")
            .with_vertex_entry("vs_fullscreen")
            .with_fragment_entry("fs_bloom_prefilter")
            .with_color_target(super::BLOOM_FORMAT, Some(wgpu::BlendState::REPLACE))
            .with_vertex_state(fullscreen_vertex(shader))
            .with_no_culling()
            .build();

        let downsample_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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

        let downsample_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("BloomDownsamplePipelineLayout"),
                bind_group_layouts: &[uniform_layout, &downsample_layout],
                push_constant_ranges: &[],
            });

        let downsample_pipeline = PipelineBuilder::new(device, &downsample_pipeline_layout, shader)
            .with_label("BloomDownsamplePipeline")
            .with_vertex_entry("vs_fullscreen")
            .with_fragment_entry("fs_bloom_downsample")
            .with_color_target(super::BLOOM_FORMAT, Some(wgpu::BlendState::REPLACE))
            .with_vertex_state(fullscreen_vertex(shader))
            .with_no_culling()
            .build();

        let upsample_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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

        let upsample_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("BloomUpsamplePipelineLayout"),
                bind_group_layouts: &[uniform_layout, &upsample_layout],
                push_constant_ranges: &[],
            });

        let upsample_pipeline = PipelineBuilder::new(device, &upsample_pipeline_layout, shader)
            .with_label("BloomUpsamplePipeline")
            .with_vertex_entry("vs_fullscreen")
            .with_fragment_entry("fs_bloom_upsample")
            .with_color_target(super::BLOOM_FORMAT, Some(wgpu::BlendState::REPLACE))
            .with_vertex_state(fullscreen_vertex(shader))
            .with_no_culling()
            .build();

        Self {
            prefilter_layout,
            prefilter_pipeline,
            downsample_layout,
            downsample_pipeline,
            upsample_layout,
            upsample_pipeline,
            prefilter_bind_group: None,
            downsample_passes: Vec::new(),
            upsample_passes: Vec::new(),
        }
    }

    pub fn invalidate(&mut self) {
        self.prefilter_bind_group = None;
        self.downsample_passes.clear();
        self.upsample_passes.clear();
    }

    pub fn ensure_bind_groups(&mut self, device: &wgpu::Device, inputs: BloomBindGroupInputs<'_>) {
        self.prefilter_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("BloomPrefilterBindGroup"),
            layout: &self.prefilter_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 20,
                    resource: wgpu::BindingResource::TextureView(inputs.scene_view),
                },
                wgpu::BindGroupEntry {
                    binding: 21,
                    resource: wgpu::BindingResource::Sampler(inputs.sampler_linear),
                },
            ],
        }));

        self.downsample_passes = inputs
            .targets
            .down_chain
            .iter()
            .enumerate()
            .skip(1)
            .map(|(level, _)| BloomDownsamplePass {
                target_index: level,
                bind_group: device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(&format!("BloomDownsampleBindGroup{level}")),
                    layout: &self.downsample_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 30,
                            resource: wgpu::BindingResource::TextureView(
                                &inputs.targets.down_chain[level - 1].view,
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 31,
                            resource: wgpu::BindingResource::Sampler(inputs.sampler_linear),
                        },
                    ],
                }),
            })
            .collect();

        self.upsample_passes.clear();
        for level in (1..inputs.targets.up_chain.len()).rev() {
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("BloomUpsampleBindGroup{level}")),
                layout: &self.upsample_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 40,
                        resource: wgpu::BindingResource::TextureView(
                            &inputs.targets.up_chain[level].view,
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 41,
                        resource: wgpu::BindingResource::TextureView(
                            &inputs.targets.down_chain[level - 1].view,
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 42,
                        resource: wgpu::BindingResource::Sampler(inputs.sampler_linear),
                    },
                ],
            });
            self.upsample_passes.push(BloomUpsamplePass {
                target_index: level - 1,
                bind_group,
            });
        }
    }

    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        uniform_bind_group: &wgpu::BindGroup,
        targets: BloomTargets<'_>,
    ) {
        let prefilter_bind_group = self
            .prefilter_bind_group
            .as_ref()
            .expect("Bloom prefilter bind group not initialized");

        {
            let mut pass = begin_color_pass(
                encoder,
                "BloomPrefilter",
                ColorAttachment::single(&targets.down_chain[0].view),
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                None,
            );
            pass.set_pipeline(&self.prefilter_pipeline);
            pass.set_bind_group(0, uniform_bind_group, &[]);
            pass.set_bind_group(1, prefilter_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        for pass_info in &self.downsample_passes {
            let mut pass = begin_color_pass(
                encoder,
                "BloomDownsample",
                ColorAttachment::single(&targets.down_chain[pass_info.target_index].view),
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                None,
            );
            pass.set_pipeline(&self.downsample_pipeline);
            pass.set_bind_group(0, uniform_bind_group, &[]);
            pass.set_bind_group(1, &pass_info.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        if let (Some(last_down), Some(last_up)) =
            (targets.down_chain.last(), targets.up_chain.last())
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

        for pass_info in &self.upsample_passes {
            let mut pass = begin_color_pass(
                encoder,
                "BloomUpsample",
                ColorAttachment::single(&targets.up_chain[pass_info.target_index].view),
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                None,
            );
            pass.set_pipeline(&self.upsample_pipeline);
            pass.set_bind_group(0, uniform_bind_group, &[]);
            pass.set_bind_group(1, &pass_info.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    pub fn clear(&self, encoder: &mut wgpu::CommandEncoder, targets: BloomTargets<'_>) {
        for mip in targets.up_chain {
            let _pass = begin_color_pass(
                encoder,
                "BloomDisabledClear",
                ColorAttachment::single(&mip.view),
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                None,
            );
        }
    }
}
