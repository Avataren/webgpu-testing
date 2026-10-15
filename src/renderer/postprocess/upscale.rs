use crate::renderer::{PipelineBuilder, ShaderBuilder};

use super::resources::TextureBundle;
use super::{fullscreen_vertex, RenderRegion, GBUFFER_PICK_FORMAT};

/// Depth and pick ids at output resolution while the scene renders smaller: what is drawn
/// after post-processing (overlays, gizmos, the editor grid and particles) depth-tests and
/// writes pick ids at that resolution. Each output pixel takes its nearest scene pixel.
fn shader_source() -> String {
    ShaderBuilder::new()
        .with_module(include_str!("../../shader/postprocess/fullscreen.wgsl"))
        .with_module(include_str!("../../shader/postprocess/common.wgsl"))
        .build(include_str!("../../shader/postprocess/upscale_depth.wgsl"))
}

pub struct UpscaledTargets {
    layout: wgpu::BindGroupLayout,
    pick_layout: wgpu::BindGroupLayout,
    depth_pipeline: wgpu::RenderPipeline,
    pick_pipeline: wgpu::RenderPipeline,
    depth: Option<TextureBundle>,
    pub(super) pick: Option<TextureBundle>,
    /// The bind group, and whether it reads pick ids.
    bind_group: Option<(wgpu::BindGroup, bool)>,
}

impl UpscaledTargets {
    pub fn new(device: &wgpu::Device, uniform_layout: &wgpu::BindGroupLayout) -> Self {
        let depth_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let pick_entry = wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Uint,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("UpscaleDepthLayout"),
            entries: &[depth_entry],
        });
        let pick_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("UpscaleDepthPickLayout"),
            entries: &[depth_entry, pick_entry],
        });

        let source = shader_source();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("UpscaleDepthShader"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });

        let make_pipeline = |label: &str, entry: &str, layout: &wgpu::BindGroupLayout| {
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(uniform_layout), Some(layout)],
                immediate_size: 0,
            });
            let mut builder = PipelineBuilder::new(device, &pipeline_layout, &shader)
                .with_label(label)
                .with_vertex_entry("vs_fullscreen")
                .with_fragment_entry(entry)
                .with_depth_stencil(
                    wgpu::TextureFormat::Depth32Float,
                    true,
                    wgpu::CompareFunction::Always,
                )
                .with_vertex_state(fullscreen_vertex(&shader))
                .with_no_culling();
            if entry == "fs_upscale_depth_pick" {
                builder = builder.with_color_target(GBUFFER_PICK_FORMAT, None);
            }
            builder.build()
        };
        let depth_pipeline = make_pipeline("UpscaleDepthPipeline", "fs_upscale_depth", &layout);
        let pick_pipeline = make_pipeline(
            "UpscaleDepthPickPipeline",
            "fs_upscale_depth_pick",
            &pick_layout,
        );

        Self {
            layout,
            pick_layout,
            depth_pipeline,
            pick_pipeline,
            depth: None,
            pick: None,
            bind_group: None,
        }
    }

    /// Drop the targets (their size follows the surface).
    pub fn invalidate(&mut self) {
        self.depth = None;
        self.pick = None;
        self.bind_group = None;
    }

    pub fn invalidate_bind_group(&mut self) {
        self.bind_group = None;
    }

    pub fn depth_view(&self) -> Option<&wgpu::TextureView> {
        self.depth.as_ref().map(|depth| &depth.view)
    }

    /// Upscale `depth` (and the scene's pick ids, when there are any) into the output
    /// region.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        uniform_bind_group: &wgpu::BindGroup,
        size: &wgpu::Extent3d,
        depth: &wgpu::TextureView,
        pick: Option<&wgpu::TextureView>,
        region: Option<RenderRegion>,
    ) {
        if self.depth.is_none() {
            self.depth = Some(TextureBundle::depth(device, size, "UpscaledDepth"));
        }
        if pick.is_some() && self.pick.is_none() {
            self.pick = Some(TextureBundle::pick(device, size));
        }
        let with_pick = pick.is_some();
        if self
            .bind_group
            .as_ref()
            .is_none_or(|(_, had_pick)| *had_pick != with_pick)
        {
            let mut entries = vec![wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(depth),
            }];
            if let Some(pick) = pick {
                entries.push(wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(pick),
                });
            }
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("UpscaleDepthBindGroup"),
                layout: if with_pick {
                    &self.pick_layout
                } else {
                    &self.layout
                },
                entries: &entries,
            });
            self.bind_group = Some((group, with_pick));
        }
        let (Some((group, _)), Some(target)) = (self.bind_group.as_ref(), self.depth.as_ref())
        else {
            return;
        };

        let pick_attachment =
            self.pick
                .as_ref()
                .filter(|_| with_pick)
                .map(|pick| wgpu::RenderPassColorAttachment {
                    view: &pick.view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("UpscaleDepthPass"),
            color_attachments: match pick_attachment.as_ref() {
                Some(_) => std::slice::from_ref(&pick_attachment),
                None => &[],
            },
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &target.view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if let Some(region) = region {
            region.apply_to_pass(&mut pass);
        }
        pass.set_pipeline(if with_pick {
            &self.pick_pipeline
        } else {
            &self.depth_pipeline
        });
        pass.set_bind_group(0, uniform_bind_group, &[]);
        pass.set_bind_group(1, group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shader_validates() {
        let source = super::shader_source();
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|err| panic!("{}", err.emit_to_string(&source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap_or_else(|err| panic!("{}", err.emit_to_string(&source)));
    }
}
