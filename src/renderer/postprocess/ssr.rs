use crate::renderer::PipelineBuilder;

use super::pipelines::{sampler_entry, texture_entry};
use super::resources::TextureBundle;
use super::{begin_color_pass, fullscreen_vertex, ColorAttachment, HDR_FORMAT};
use crate::renderer::RenderRegion;

/// Screen-space reflections: a half-resolution trace, temporal accumulation, a separable
/// blur, and a pass that adds the result to the HDR scene before the temporal and later
/// passes read it.
pub struct SsrStage {
    trace_layout: wgpu::BindGroupLayout,
    trace_pipeline: wgpu::RenderPipeline,
    temporal_layout: wgpu::BindGroupLayout,
    temporal_pipeline: wgpu::RenderPipeline,
    blur_layout: wgpu::BindGroupLayout,
    blur_horizontal_pipeline: wgpu::RenderPipeline,
    blur_vertical_pipeline: wgpu::RenderPipeline,
    apply_layout: wgpu::BindGroupLayout,
    apply_pipeline: wgpu::RenderPipeline,
    trace_bind_group: Option<wgpu::BindGroup>,
    temporal_bind_group: Option<wgpu::BindGroup>,
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
    /// Half resolution: the trace accumulated over frames.
    pub accumulated: &'a wgpu::TextureView,
    /// Half resolution: last frame's `accumulated`.
    pub history: &'a wgpu::TextureView,
    /// The fog's uniform and integrated froxel volume (`renderer/fog.rs`): the apply pass
    /// dims reflections by the fog in front of them.
    pub fog_uniform: &'a wgpu::Buffer,
    pub fog_volume: &'a wgpu::TextureView,
}

/// The half-resolution textures one frame of reflections goes through.
#[derive(Clone, Copy)]
pub struct SsrTargets<'a> {
    /// The trace, then the blurred result.
    pub reflection: &'a TextureBundle,
    pub ping: &'a wgpu::TextureView,
    pub accumulated: &'a TextureBundle,
    pub history: &'a TextureBundle,
    /// Whether `history` holds last frame's accumulated reflection of this view, traced
    /// the same way; when not, accumulation restarts from this frame's trace.
    pub history_valid: bool,
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
        let temporal_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("SsrTemporalLayout"),
            entries: &[
                unfilterable(155),
                texture_entry(156),
                unfilterable(157),
                sampler_entry(158),
            ],
        });
        let blur_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("SsrBlurLayout"),
            entries: &[texture_entry(150), sampler_entry(151), unfilterable(152)],
        });
        let apply_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("SsrApplyLayout"),
            entries: &[
                texture_entry(160),
                sampler_entry(161),
                unfilterable(162),
                unfilterable(163),
                wgpu::BindGroupLayoutEntry {
                    binding: 164,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 165,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
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
            temporal_pipeline: make_pipeline(
                "SsrTemporalPipeline",
                "fs_ssr_temporal",
                &temporal_layout,
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
            temporal_layout,
            blur_layout,
            apply_layout,
            trace_bind_group: None,
            temporal_bind_group: None,
            blur_horizontal_bind_group: None,
            blur_vertical_bind_group: None,
            apply_bind_group: None,
        }
    }

    pub fn invalidate(&mut self) {
        self.trace_bind_group = None;
        self.temporal_bind_group = None;
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
        self.temporal_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("SsrTemporalBindGroup"),
            layout: &self.temporal_layout,
            entries: &[
                texture(155, inputs.reflection),
                texture(156, inputs.history),
                texture(157, inputs.normal),
                sampler(158),
            ],
        }));
        self.blur_horizontal_bind_group =
            Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("SsrBlurHorizontalBindGroup"),
                layout: &self.blur_layout,
                entries: &[
                    texture(150, inputs.accumulated),
                    sampler(151),
                    texture(152, inputs.position),
                ],
            }));
        self.blur_vertical_bind_group =
            Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("SsrBlurVerticalBindGroup"),
                layout: &self.blur_layout,
                entries: &[
                    texture(150, inputs.ping),
                    sampler(151),
                    texture(152, inputs.position),
                ],
            }));
        self.apply_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("SsrApplyBindGroup"),
            layout: &self.apply_layout,
            entries: &[
                texture(160, inputs.reflection),
                sampler(161),
                texture(162, inputs.normal),
                texture(163, inputs.position),
                wgpu::BindGroupEntry {
                    binding: 164,
                    resource: inputs.fog_uniform.as_entire_binding(),
                },
                texture(165, inputs.fog_volume),
            ],
        }));
    }

    /// Trace into `targets.reflection`, accumulate and blur it, then add it to `scene`
    /// inside `region`.
    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        uniform_bind_group: &wgpu::BindGroup,
        targets: &SsrTargets<'_>,
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
            &targets.reflection.view,
        );
        self.resolve(encoder, uniform_bind_group, targets, scene, region);
    }

    /// Accumulate and blur a traced `targets.reflection` (screen-space or ray-traced) and
    /// add it to `scene` inside `region`.
    pub fn resolve(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        uniform_bind_group: &wgpu::BindGroup,
        targets: &SsrTargets<'_>,
        scene: &wgpu::TextureView,
        region: Option<RenderRegion>,
    ) {
        let (Some(temporal), Some(horizontal), Some(vertical), Some(apply)) = (
            self.temporal_bind_group.as_ref(),
            self.blur_horizontal_bind_group.as_ref(),
            self.blur_vertical_bind_group.as_ref(),
            self.apply_bind_group.as_ref(),
        ) else {
            return;
        };
        let SsrTargets {
            reflection,
            ping,
            accumulated,
            history,
            history_valid,
        } = *targets;
        if !history_valid {
            // Start the accumulation from this frame's trace.
            reflection.copy_to(encoder, history);
        }
        Self::fullscreen(
            encoder,
            "SsrTemporal",
            &self.temporal_pipeline,
            uniform_bind_group,
            temporal,
            &accumulated.view,
        );
        accumulated.copy_to(encoder, history);
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
            &reflection.view,
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
