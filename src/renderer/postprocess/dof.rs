use crate::renderer::PipelineBuilder;

use super::pipelines::{sampler_entry, texture_entry};
use super::{begin_color_pass, fullscreen_vertex, ColorAttachment, DOF_FORMAT};

/// Depth of field at half resolution: a prefilter that stores colour and circle of
/// confusion, then a bokeh gather the composite pass blends in by blur size.
pub struct DofStage {
    prefilter_layout: wgpu::BindGroupLayout,
    prefilter_pipeline: wgpu::RenderPipeline,
    blur_layout: wgpu::BindGroupLayout,
    blur_pipeline: wgpu::RenderPipeline,
    prefilter_bind_group: Option<wgpu::BindGroup>,
    blur_bind_group: Option<wgpu::BindGroup>,
}

impl DofStage {
    pub fn new(
        device: &wgpu::Device,
        uniform_layout: &wgpu::BindGroupLayout,
        shader: &wgpu::ShaderModule,
    ) -> Self {
        let unfilterable = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let prefilter_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("DofPrefilterLayout"),
            entries: &[unfilterable(120), unfilterable(121)],
        });
        let blur_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("DofBlurLayout"),
            entries: &[texture_entry(130), sampler_entry(131)],
        });
        let make_pipeline = |label: &str, entry: &str, layout: &wgpu::BindGroupLayout| {
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(uniform_layout), Some(layout)],
                immediate_size: 0,
            });
            PipelineBuilder::new(device, &pipeline_layout, shader)
                .with_label(label)
                .with_vertex_entry("vs_fullscreen")
                .with_fragment_entry(entry)
                .with_color_target(DOF_FORMAT, Some(wgpu::BlendState::REPLACE))
                .with_vertex_state(fullscreen_vertex(shader))
                .with_no_culling()
                .build()
        };
        let prefilter_pipeline = make_pipeline(
            "DofPrefilterPipeline",
            "fs_dof_prefilter",
            &prefilter_layout,
        );
        let blur_pipeline = make_pipeline("DofBlurPipeline", "fs_dof_blur", &blur_layout);
        Self {
            prefilter_layout,
            prefilter_pipeline,
            blur_layout,
            blur_pipeline,
            prefilter_bind_group: None,
            blur_bind_group: None,
        }
    }

    pub fn invalidate(&mut self) {
        self.prefilter_bind_group = None;
        self.blur_bind_group = None;
    }

    pub fn ensure_bind_groups(
        &mut self,
        device: &wgpu::Device,
        scene: &wgpu::TextureView,
        position: &wgpu::TextureView,
        half: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
    ) {
        self.prefilter_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("DofPrefilterBindGroup"),
            layout: &self.prefilter_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 120,
                    resource: wgpu::BindingResource::TextureView(scene),
                },
                wgpu::BindGroupEntry {
                    binding: 121,
                    resource: wgpu::BindingResource::TextureView(position),
                },
            ],
        }));
        self.blur_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("DofBlurBindGroup"),
            layout: &self.blur_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 130,
                    resource: wgpu::BindingResource::TextureView(half),
                },
                wgpu::BindGroupEntry {
                    binding: 131,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        }));
    }

    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        uniform_bind_group: &wgpu::BindGroup,
        half: &wgpu::TextureView,
        blur: &wgpu::TextureView,
    ) {
        let (Some(prefilter), Some(blur_group)) = (
            self.prefilter_bind_group.as_ref(),
            self.blur_bind_group.as_ref(),
        ) else {
            return;
        };
        for (label, pipeline, group, target) in [
            ("DofPrefilter", &self.prefilter_pipeline, prefilter, half),
            ("DofBlur", &self.blur_pipeline, blur_group, blur),
        ] {
            let mut pass = begin_color_pass(
                encoder,
                label,
                ColorAttachment::single(target),
                wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                None,
            );
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, uniform_bind_group, &[]);
            pass.set_bind_group(1, group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}
