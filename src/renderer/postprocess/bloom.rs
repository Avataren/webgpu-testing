use crate::renderer::PipelineBuilder;

use super::pipelines::{sampler_entry, texture_entry};
use super::resources::BloomMip;
use super::{begin_color_pass, fullscreen_vertex, ColorAttachment};

/// Energy-conserving bloom (Jimenez, "Next Generation Post Processing in Call of Duty"):
/// a thresholded 13-tap downsample chain, then a tent-filtered upsample back up the same
/// chain where each level blends in the wider levels by the scatter amount. The finished
/// glow ends up in the first mip.
pub struct BloomStage {
    prefilter_layout: wgpu::BindGroupLayout,
    prefilter_pipeline: wgpu::RenderPipeline,
    downsample_layout: wgpu::BindGroupLayout,
    downsample_pipeline: wgpu::RenderPipeline,
    upsample_layout: wgpu::BindGroupLayout,
    upsample_pipeline: wgpu::RenderPipeline,
    prefilter_bind_group: Option<wgpu::BindGroup>,
    /// Bind group reading mip `i - 1`, for writing mip `i`.
    downsample_bind_groups: Vec<wgpu::BindGroup>,
    /// Bind group reading mip `i + 1`, for blending into mip `i`.
    upsample_bind_groups: Vec<wgpu::BindGroup>,
}

pub struct BloomBindGroupInputs<'a> {
    pub scene_view: &'a wgpu::TextureView,
    pub sampler_linear: &'a wgpu::Sampler,
    pub chain: &'a [BloomMip],
}

impl BloomStage {
    pub fn new(
        device: &wgpu::Device,
        uniform_layout: &wgpu::BindGroupLayout,
        shader: &wgpu::ShaderModule,
    ) -> Self {
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
                    .with_color_target(super::BLOOM_FORMAT, Some(blend))
                    .with_vertex_state(fullscreen_vertex(shader))
                    .with_no_culling()
                    .build()
            };

        let prefilter_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("BloomPrefilterLayout"),
            entries: &[texture_entry(20), sampler_entry(21)],
        });
        let downsample_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("BloomDownsampleLayout"),
            entries: &[texture_entry(30), sampler_entry(31)],
        });
        let upsample_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("BloomUpsampleLayout"),
            entries: &[texture_entry(40), sampler_entry(42)],
        });

        // dst = lerp(dst, upsampled, scatter), with scatter as the blend constant.
        let scatter_blend = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Constant,
            dst_factor: wgpu::BlendFactor::OneMinusConstant,
            operation: wgpu::BlendOperation::Add,
        };

        let prefilter_pipeline = make_pipeline(
            "BloomPrefilterPipeline",
            "fs_bloom_prefilter",
            &prefilter_layout,
            wgpu::BlendState::REPLACE,
        );
        let downsample_pipeline = make_pipeline(
            "BloomDownsamplePipeline",
            "fs_bloom_downsample",
            &downsample_layout,
            wgpu::BlendState::REPLACE,
        );
        let upsample_pipeline = make_pipeline(
            "BloomUpsamplePipeline",
            "fs_bloom_upsample",
            &upsample_layout,
            wgpu::BlendState {
                color: scatter_blend,
                alpha: scatter_blend,
            },
        );

        Self {
            prefilter_layout,
            prefilter_pipeline,
            downsample_layout,
            downsample_pipeline,
            upsample_layout,
            upsample_pipeline,
            prefilter_bind_group: None,
            downsample_bind_groups: Vec::new(),
            upsample_bind_groups: Vec::new(),
        }
    }

    pub fn invalidate(&mut self) {
        self.prefilter_bind_group = None;
        self.downsample_bind_groups.clear();
        self.upsample_bind_groups.clear();
    }

    pub fn ensure_bind_groups(&mut self, device: &wgpu::Device, inputs: BloomBindGroupInputs<'_>) {
        let read = |layout: &wgpu::BindGroupLayout,
                    label: &str,
                    view: &wgpu::TextureView,
                    texture_binding: u32,
                    sampler_binding: u32| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: texture_binding,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                    wgpu::BindGroupEntry {
                        binding: sampler_binding,
                        resource: wgpu::BindingResource::Sampler(inputs.sampler_linear),
                    },
                ],
            })
        };

        self.prefilter_bind_group = Some(read(
            &self.prefilter_layout,
            "BloomPrefilterBindGroup",
            inputs.scene_view,
            20,
            21,
        ));
        self.downsample_bind_groups = inputs
            .chain
            .windows(2)
            .enumerate()
            .map(|(level, pair)| {
                read(
                    &self.downsample_layout,
                    &format!("BloomDownsampleBindGroup{}", level + 1),
                    &pair[0].view,
                    30,
                    31,
                )
            })
            .collect();
        self.upsample_bind_groups = inputs
            .chain
            .windows(2)
            .enumerate()
            .map(|(level, pair)| {
                read(
                    &self.upsample_layout,
                    &format!("BloomUpsampleBindGroup{level}"),
                    &pair[1].view,
                    40,
                    42,
                )
            })
            .collect();
    }

    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        uniform_bind_group: &wgpu::BindGroup,
        chain: &[BloomMip],
        scatter: f32,
    ) {
        let prefilter_bind_group = self
            .prefilter_bind_group
            .as_ref()
            .expect("Bloom prefilter bind group not initialized");

        {
            let mut pass = begin_color_pass(
                encoder,
                "BloomPrefilter",
                ColorAttachment::single(&chain[0].view),
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                None,
            );
            pass.set_pipeline(&self.prefilter_pipeline);
            pass.set_bind_group(0, uniform_bind_group, &[]);
            pass.set_bind_group(1, prefilter_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        for (index, bind_group) in self.downsample_bind_groups.iter().enumerate() {
            let mut pass = begin_color_pass(
                encoder,
                "BloomDownsample",
                ColorAttachment::single(&chain[index + 1].view),
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                None,
            );
            pass.set_pipeline(&self.downsample_pipeline);
            pass.set_bind_group(0, uniform_bind_group, &[]);
            pass.set_bind_group(1, bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        let scatter = f64::from(scatter.clamp(0.0, 1.0));
        for (index, bind_group) in self.upsample_bind_groups.iter().enumerate().rev() {
            let mut pass = begin_color_pass(
                encoder,
                "BloomUpsample",
                ColorAttachment::single(&chain[index].view),
                wgpu::LoadOp::Load,
                None,
            );
            pass.set_pipeline(&self.upsample_pipeline);
            pass.set_blend_constant(wgpu::Color {
                r: scatter,
                g: scatter,
                b: scatter,
                a: scatter,
            });
            pass.set_bind_group(0, uniform_bind_group, &[]);
            pass.set_bind_group(1, bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}
