// src/renderer/render_context.rs

/// Context provided to custom render callbacks
/// 
/// This bundles commonly needed rendering resources to simplify
/// the custom_render callback signature and provide helper methods.
pub struct CustomRenderContext<'a> {
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub renderer: &'a crate::renderer::Renderer,
    pub scene: &'a crate::scene::Scene,
    pub color_view: &'a wgpu::TextureView,
    pub depth_view: &'a wgpu::TextureView,
}

impl<'a> CustomRenderContext<'a> {
    /// Create a new custom render context
    pub fn new(
        encoder: &'a mut wgpu::CommandEncoder,
        renderer: &'a crate::renderer::Renderer,
        scene: &'a crate::scene::Scene,
        color_view: &'a wgpu::TextureView,
        depth_view: &'a wgpu::TextureView,
    ) -> Self {
        Self {
            encoder,
            renderer,
            scene,
            color_view,
            depth_view,
        }
    }

    /// Begin a render pass with sensible defaults for custom rendering
    /// 
    /// The pass loads existing color and depth, allowing you to draw on top
    /// of the main scene rendering.
    pub fn begin_render_pass(&mut self, label: &str) -> wgpu::RenderPass<'_>  {
        self.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: self.color_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: self.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        })
    }

    /// Begin a render pass that clears the depth buffer
    /// 
    /// Useful when you want your custom rendering to ignore the main scene depth.
    pub fn begin_render_pass_clear_depth(&mut self, label: &str) -> wgpu::RenderPass<'_> {
        self.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: self.color_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: self.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        })
    }
}