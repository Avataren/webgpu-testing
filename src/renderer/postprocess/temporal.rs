use crate::renderer::PipelineBuilder;

use super::pipelines::sampler_entry;
use super::resources::TextureBundle;
use super::{begin_color_pass, fullscreen_vertex, ColorAttachment, RenderRegion, HDR_FORMAT};

/// Temporal anti-aliasing (or upscaling) and motion blur: the first post passes, on the
/// HDR scene, driven by the motion vectors in the normal G-buffer.
pub struct TemporalStage {
    taa_layout: wgpu::BindGroupLayout,
    taa_pipeline: wgpu::RenderPipeline,
    /// The TAA pass when the scene renders smaller than the output.
    taau_pipeline: wgpu::RenderPipeline,
    blur_layout: wgpu::BindGroupLayout,
    blur_pipeline: wgpu::RenderPipeline,
    taa_bind_group: Option<wgpu::BindGroup>,
    blur_bind_group: Option<wgpu::BindGroup>,
}

pub struct TemporalInputs<'a> {
    pub scene: &'a wgpu::TextureView,
    pub history: &'a wgpu::TextureView,
    pub motion: &'a wgpu::TextureView,
    pub position: &'a wgpu::TextureView,
    pub sampler: &'a wgpu::Sampler,
    /// What motion blur reads: the TAA output when TAA runs, else the scene.
    pub blur_input: &'a wgpu::TextureView,
}

fn unfilterable(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn filterable(binding: u32) -> wgpu::BindGroupLayoutEntry {
    super::pipelines::texture_entry(binding)
}

impl TemporalStage {
    pub fn new(
        device: &wgpu::Device,
        uniform_layout: &wgpu::BindGroupLayout,
        shader: &wgpu::ShaderModule,
    ) -> Self {
        let taa_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("TaaLayout"),
            entries: &[
                unfilterable(140),
                filterable(141),
                unfilterable(142),
                unfilterable(143),
                sampler_entry(144),
            ],
        });
        let blur_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("MotionBlurLayout"),
            entries: &[
                filterable(150),
                unfilterable(151),
                unfilterable(152),
                sampler_entry(153),
            ],
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
                .with_color_target(HDR_FORMAT, Some(wgpu::BlendState::REPLACE))
                .with_vertex_state(fullscreen_vertex(shader))
                .with_no_culling()
                .build()
        };
        let taa_pipeline = make_pipeline("TaaPipeline", "fs_taa", &taa_layout);
        let taau_pipeline = make_pipeline("TaauPipeline", "fs_taau", &taa_layout);
        let blur_pipeline = make_pipeline("MotionBlurPipeline", "fs_motion_blur", &blur_layout);
        Self {
            taa_layout,
            taa_pipeline,
            taau_pipeline,
            blur_layout,
            blur_pipeline,
            taa_bind_group: None,
            blur_bind_group: None,
        }
    }

    pub fn invalidate(&mut self) {
        self.taa_bind_group = None;
        self.blur_bind_group = None;
    }

    pub fn ensure_bind_groups(&mut self, device: &wgpu::Device, inputs: TemporalInputs<'_>) {
        let view = |binding, view| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(view),
        };
        self.taa_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("TaaBindGroup"),
            layout: &self.taa_layout,
            entries: &[
                view(140, inputs.scene),
                view(141, inputs.history),
                view(142, inputs.motion),
                view(143, inputs.position),
                wgpu::BindGroupEntry {
                    binding: 144,
                    resource: wgpu::BindingResource::Sampler(inputs.sampler),
                },
            ],
        }));
        self.blur_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("MotionBlurBindGroup"),
            layout: &self.blur_layout,
            entries: &[
                view(150, inputs.blur_input),
                view(151, inputs.motion),
                view(152, inputs.position),
                wgpu::BindGroupEntry {
                    binding: 153,
                    resource: wgpu::BindingResource::Sampler(inputs.sampler),
                },
            ],
        }));
    }

    /// Resolve TAA into `output`, then keep it as next frame's history. With `upscale`
    /// the scene is smaller than the output and the pass reconstructs the output
    /// `region` from it.
    pub fn render_taa(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        uniform_bind_group: &wgpu::BindGroup,
        output: &TextureBundle,
        history: &TextureBundle,
        upscale: bool,
        region: Option<RenderRegion>,
    ) {
        let Some(group) = self.taa_bind_group.as_ref() else {
            return;
        };
        {
            let mut pass = begin_color_pass(
                encoder,
                "Taa",
                ColorAttachment::single(&output.view),
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                region.filter(|_| upscale),
            );
            pass.set_pipeline(if upscale {
                &self.taau_pipeline
            } else {
                &self.taa_pipeline
            });
            pass.set_bind_group(0, uniform_bind_group, &[]);
            pass.set_bind_group(1, group, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_texture(
            output.texture().as_image_copy(),
            history.texture().as_image_copy(),
            output.texture().size(),
        );
    }

    pub fn render_motion_blur(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        uniform_bind_group: &wgpu::BindGroup,
        output: &wgpu::TextureView,
    ) {
        let Some(group) = self.blur_bind_group.as_ref() else {
            return;
        };
        let mut pass = begin_color_pass(
            encoder,
            "MotionBlur",
            ColorAttachment::single(output),
            wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            None,
        );
        pass.set_pipeline(&self.blur_pipeline);
        pass.set_bind_group(0, uniform_bind_group, &[]);
        pass.set_bind_group(1, group, &[]);
        pass.draw(0..3, 0..1);
    }
}
