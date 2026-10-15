use crate::renderer::shader_source::wgsl;
use crate::renderer::{PipelineBuilder, ShaderBuilder};

use super::resources::TextureBundle;
use super::{fullscreen_vertex, RenderRegion};

/// Depth at output resolution while the scene renders smaller: what is drawn after
/// post-processing (overlays, gizmos, the editor grid and particles) depth-tests at that
/// resolution. Each output pixel takes its nearest scene pixel.
fn shader_source() -> String {
    ShaderBuilder::new()
        .with_module(wgsl!("postprocess/fullscreen.wgsl"))
        .with_module(wgsl!("postprocess/common.wgsl"))
        .build(wgsl!("postprocess/upscale_depth.wgsl"))
}

pub struct UpscaledTargets {
    layout: wgpu::BindGroupLayout,
    depth_pipeline: wgpu::RenderPipeline,
    depth: Option<TextureBundle>,
    bind_group: Option<wgpu::BindGroup>,
}

impl UpscaledTargets {
    pub fn new(device: &wgpu::Device, uniform_layout: &wgpu::BindGroupLayout) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("UpscaleDepthLayout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });

        let source = shader_source();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("UpscaleDepthShader"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("UpscaleDepthPipeline"),
            bind_group_layouts: &[Some(uniform_layout), Some(&layout)],
            immediate_size: 0,
        });
        let depth_pipeline = PipelineBuilder::new(device, &pipeline_layout, &shader)
            .with_label("UpscaleDepthPipeline")
            .with_vertex_entry("vs_fullscreen")
            .with_fragment_entry("fs_upscale_depth")
            .with_depth_stencil(
                wgpu::TextureFormat::Depth32Float,
                true,
                wgpu::CompareFunction::Always,
            )
            .with_vertex_state(fullscreen_vertex(&shader))
            .with_no_culling()
            .build();

        Self {
            layout,
            depth_pipeline,
            depth: None,
            bind_group: None,
        }
    }

    /// Drop the target (its size follows the surface).
    pub fn invalidate(&mut self) {
        self.depth = None;
        self.bind_group = None;
    }

    pub fn invalidate_bind_group(&mut self) {
        self.bind_group = None;
    }

    pub fn depth_view(&self) -> Option<&wgpu::TextureView> {
        self.depth.as_ref().map(|depth| &depth.view)
    }

    /// Upscale `depth` into the output region.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        uniform_bind_group: &wgpu::BindGroup,
        size: &wgpu::Extent3d,
        depth: &wgpu::TextureView,
        region: Option<RenderRegion>,
    ) {
        if self.depth.is_none() {
            self.depth = Some(TextureBundle::depth(device, size, "UpscaledDepth"));
        }
        if self.bind_group.is_none() {
            self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("UpscaleDepthBindGroup"),
                layout: &self.layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(depth),
                }],
            }));
        }
        let (Some(group), Some(target)) = (self.bind_group.as_ref(), self.depth.as_ref()) else {
            return;
        };

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("UpscaleDepthPass"),
            color_attachments: &[],
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
        pass.set_pipeline(&self.depth_pipeline);
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
