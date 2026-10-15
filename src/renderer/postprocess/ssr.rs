use crate::renderer::PipelineBuilder;

use super::pipelines::{sampler_entry, texture_entry};
use super::{begin_color_pass, fullscreen_vertex, ColorAttachment, HDR_FORMAT};
use crate::renderer::RenderRegion;

/// Screen-space reflections: a half-resolution trace, a separable blur, and a pass that
/// adds the result to the HDR scene before the temporal and later passes read it.
pub struct SsrStage {
    trace_layout: wgpu::BindGroupLayout,
    trace_pipeline: wgpu::RenderPipeline,
    blur_layout: wgpu::BindGroupLayout,
    blur_horizontal_pipeline: wgpu::RenderPipeline,
    blur_vertical_pipeline: wgpu::RenderPipeline,
    apply_layout: wgpu::BindGroupLayout,
    apply_pipeline: wgpu::RenderPipeline,
    trace_bind_group: Option<wgpu::BindGroup>,
    blur_horizontal_bind_group: Option<wgpu::BindGroup>,
    blur_vertical_bind_group: Option<wgpu::BindGroup>,
    apply_bind_group: Option<wgpu::BindGroup>,
}

pub struct SsrInputs<'a> {
    pub scene: &'a wgpu::TextureView,
    pub normal: &'a wgpu::TextureView,
    pub position: &'a wgpu::TextureView,
    pub sampler: &'a wgpu::Sampler,
    /// Half resolution: the trace, then the vertical blur's output.
    pub reflection: &'a wgpu::TextureView,
    /// Half resolution: the horizontal blur's output.
    pub ping: &'a wgpu::TextureView,
}

impl SsrStage {
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
        let trace_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("SsrTraceLayout"),
            entries: &[
                texture_entry(140),
                unfilterable(141),
                unfilterable(142),
                sampler_entry(143),
            ],
        });
        let blur_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("SsrBlurLayout"),
            entries: &[texture_entry(150), sampler_entry(151)],
        });
        let apply_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("SsrApplyLayout"),
            entries: &[
                texture_entry(160),
                sampler_entry(161),
                unfilterable(162),
                unfilterable(163),
            ],
        });
        let make_pipeline =
            |label: &str, entry: &str, layout: &wgpu::BindGroupLayout, blend: wgpu::BlendState| {
                let pipeline_layout =
                    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some(label),
                        bind_group_layouts: &[Some(uniform_layout), Some(layout)],
                        immediate_size: 0,
                    });
                PipelineBuilder::new(device, &pipeline_layout, shader)
                    .with_label(label)
                    .with_vertex_entry("vs_fullscreen")
                    .with_fragment_entry(entry)
                    .with_color_target(HDR_FORMAT, Some(blend))
                    .with_vertex_state(fullscreen_vertex(shader))
                    .with_no_culling()
                    .build()
            };
        let replace = wgpu::BlendState::REPLACE;
        // Adds the reflection to the scene's colour and leaves its alpha alone.
        let add = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        Self {
            trace_pipeline: make_pipeline(
                "SsrTracePipeline",
                "fs_ssr_trace",
                &trace_layout,
                replace,
            ),
            blur_horizontal_pipeline: make_pipeline(
                "SsrBlurHorizontalPipeline",
                "fs_ssr_blur_horizontal",
                &blur_layout,
                replace,
            ),
            blur_vertical_pipeline: make_pipeline(
                "SsrBlurVerticalPipeline",
                "fs_ssr_blur_vertical",
                &blur_layout,
                replace,
            ),
            apply_pipeline: make_pipeline("SsrApplyPipeline", "fs_ssr_apply", &apply_layout, add),
            trace_layout,
            blur_layout,
            apply_layout,
            trace_bind_group: None,
            blur_horizontal_bind_group: None,
            blur_vertical_bind_group: None,
            apply_bind_group: None,
        }
    }

    pub fn invalidate(&mut self) {
        self.trace_bind_group = None;
        self.blur_horizontal_bind_group = None;
        self.blur_vertical_bind_group = None;
        self.apply_bind_group = None;
    }

    pub fn ensure_bind_groups(&mut self, device: &wgpu::Device, inputs: SsrInputs<'_>) {
        let texture = |binding, view| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(view),
        };
        let sampler = |binding| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::Sampler(inputs.sampler),
        };
        self.trace_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("SsrTraceBindGroup"),
            layout: &self.trace_layout,
            entries: &[
                texture(140, inputs.scene),
                texture(141, inputs.normal),
                texture(142, inputs.position),
                sampler(143),
            ],
        }));
        self.blur_horizontal_bind_group =
            Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("SsrBlurHorizontalBindGroup"),
                layout: &self.blur_layout,
                entries: &[texture(150, inputs.reflection), sampler(151)],
            }));
        self.blur_vertical_bind_group =
            Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("SsrBlurVerticalBindGroup"),
                layout: &self.blur_layout,
                entries: &[texture(150, inputs.ping), sampler(151)],
            }));
        self.apply_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("SsrApplyBindGroup"),
            layout: &self.apply_layout,
            entries: &[
                texture(160, inputs.reflection),
                sampler(161),
                texture(162, inputs.normal),
                texture(163, inputs.position),
            ],
        }));
    }

    /// Trace and blur into `reflection`, then add it to `scene` inside `region`.
    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        uniform_bind_group: &wgpu::BindGroup,
        reflection: &wgpu::TextureView,
        ping: &wgpu::TextureView,
        scene: &wgpu::TextureView,
        region: Option<RenderRegion>,
    ) {
        let Some(trace) = self.trace_bind_group.as_ref() else {
            return;
        };
        Self::fullscreen(
            encoder,
            "SsrTrace",
            &self.trace_pipeline,
            uniform_bind_group,
            trace,
            reflection,
        );
        self.resolve(encoder, uniform_bind_group, reflection, ping, scene, region);
    }

    /// Blur a traced `reflection` (screen-space or ray-traced) and add it to `scene`
    /// inside `region`.
    pub fn resolve(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        uniform_bind_group: &wgpu::BindGroup,
        reflection: &wgpu::TextureView,
        ping: &wgpu::TextureView,
        scene: &wgpu::TextureView,
        region: Option<RenderRegion>,
    ) {
        let (Some(horizontal), Some(vertical), Some(apply)) = (
            self.blur_horizontal_bind_group.as_ref(),
            self.blur_vertical_bind_group.as_ref(),
            self.apply_bind_group.as_ref(),
        ) else {
            return;
        };
        Self::fullscreen(
            encoder,
            "SsrBlurHorizontal",
            &self.blur_horizontal_pipeline,
            uniform_bind_group,
            horizontal,
            ping,
        );
        Self::fullscreen(
            encoder,
            "SsrBlurVertical",
            &self.blur_vertical_pipeline,
            uniform_bind_group,
            vertical,
            reflection,
        );
        let mut pass = begin_color_pass(
            encoder,
            "SsrApply",
            ColorAttachment::single(scene),
            wgpu::LoadOp::Load,
            region,
        );
        pass.set_pipeline(&self.apply_pipeline);
        pass.set_bind_group(0, uniform_bind_group, &[]);
        pass.set_bind_group(1, apply, &[]);
        pass.draw(0..3, 0..1);
    }

    fn fullscreen(
        encoder: &mut wgpu::CommandEncoder,
        label: &'static str,
        pipeline: &wgpu::RenderPipeline,
        uniform_bind_group: &wgpu::BindGroup,
        group: &wgpu::BindGroup,
        target: &wgpu::TextureView,
    ) {
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
