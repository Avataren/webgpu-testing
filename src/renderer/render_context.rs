// src/renderer/render_context.rs

/// Determines when a custom render callback runs relative to the renderer's
/// post-processing step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CustomRenderStage {
    /// Execute the custom hook before post-processing so any output is
    /// affected by the configured effects (default behaviour).
    BeforePostprocess,
    /// Execute the custom hook after post-processing, matching the legacy
    /// behaviour where the callback rendered directly to the surface.
    AfterPostprocess,
}

/// Convenience alias for the callback type accepted by the renderer.
pub type CustomRenderCallback = dyn for<'a> FnMut(&mut CustomRenderContext<'a>);

/// Request passed to the renderer describing when and how to invoke a custom
/// render hook.
pub struct CustomRenderRequest<'a> {
    pub callback: &'a mut CustomRenderCallback,
    pub stage: CustomRenderStage,
}

/// Context provided to custom render callbacks
///
/// This bundles commonly needed rendering resources to simplify
/// the custom_render callback signature and provide helper methods. The views
/// exposed here point either to the scene color target (before post-process)
/// or the surface (after post-process) depending on the configured
/// [`CustomRenderStage`]. The [`stage`](CustomRenderContext::stage) also
/// drives the [`color_format`](CustomRenderContext::color_format) and
/// [`sample_count`](CustomRenderContext::sample_count) helpers so custom
/// pipelines can be configured correctly.
pub struct CustomRenderContext<'a> {
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub renderer: &'a crate::renderer::Renderer,
    pub scene: &'a crate::scene::Scene,
    pub color_view: &'a wgpu::TextureView,
    pub depth_view: &'a wgpu::TextureView,
    pub stage: CustomRenderStage,
}

impl<'a> CustomRenderContext<'a> {
    /// Create a new custom render context
    pub fn new(
        encoder: &'a mut wgpu::CommandEncoder,
        renderer: &'a crate::renderer::Renderer,
        scene: &'a crate::scene::Scene,
        color_view: &'a wgpu::TextureView,
        depth_view: &'a wgpu::TextureView,
        stage: CustomRenderStage,
    ) -> Self {
        Self {
            encoder,
            renderer,
            scene,
            color_view,
            depth_view,
            stage,
        }
    }

    /// Returns the texture format of the color target for the current custom
    /// render stage.
    pub fn color_format(&self) -> wgpu::TextureFormat {
        self.renderer.color_format_for_stage(self.stage)
    }

    /// Returns the sample count that must be used when rendering during the
    /// current custom render stage.
    pub fn sample_count(&self) -> u32 {
        self.renderer.sample_count_for_stage(self.stage)
    }

    /// Begin a render pass with sensible defaults for custom rendering
    ///
    /// The pass loads existing color and depth, allowing you to draw on top
    /// of the main scene rendering.
    pub fn begin_render_pass(&mut self, label: &str) -> wgpu::RenderPass<'_> {
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
