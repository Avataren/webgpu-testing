use wgpu::{self, StoreOp};

use super::render_region::RenderRegion;

/// Describes a color attachment for a render pass, including the target view,
/// optional resolve target, and the load/store operations to apply.
#[derive(Clone, Copy, Debug)]
pub struct FrameGraphColorAttachment<'a> {
    view: &'a wgpu::TextureView,
    resolve_target: Option<&'a wgpu::TextureView>,
    load: wgpu::LoadOp<wgpu::Color>,
    store: StoreOp,
}

impl<'a> FrameGraphColorAttachment<'a> {
    /// Clears the target view with the provided color at the start of the pass.
    pub fn clear(
        view: &'a wgpu::TextureView,
        resolve_target: Option<&'a wgpu::TextureView>,
        color: wgpu::Color,
    ) -> Self {
        Self {
            view,
            resolve_target,
            load: wgpu::LoadOp::Clear(color),
            store: StoreOp::Store,
        }
    }

    /// Loads the previous contents of the attachment, keeping the result.
    pub fn load(
        view: &'a wgpu::TextureView,
        resolve_target: Option<&'a wgpu::TextureView>,
    ) -> Self {
        Self {
            view,
            resolve_target,
            load: wgpu::LoadOp::Load,
            store: StoreOp::Store,
        }
    }

    /// Overrides whether the render pass should store the attachment contents.
    pub fn with_store_op(mut self, store: StoreOp) -> Self {
        self.store = store;
        self
    }

    fn into_wgpu(self) -> wgpu::RenderPassColorAttachment<'a> {
        wgpu::RenderPassColorAttachment {
            view: self.view,
            resolve_target: self.resolve_target,
            depth_slice: None,
            ops: wgpu::Operations {
                load: self.load,
                store: self.store,
            },
        }
    }
}

/// Describes the depth/stencil attachment configuration for a render pass.
#[derive(Clone, Copy, Debug)]
pub struct FrameGraphDepthAttachment<'a> {
    view: &'a wgpu::TextureView,
    depth_ops: Option<wgpu::Operations<f32>>,
    stencil_ops: Option<wgpu::Operations<u32>>,
}

impl<'a> FrameGraphDepthAttachment<'a> {
    /// Clears the depth attachment and stores the result.
    pub fn clear(view: &'a wgpu::TextureView, depth: f32) -> Self {
        Self {
            view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(depth),
                store: StoreOp::Store,
            }),
            stencil_ops: None,
        }
    }

    /// Loads the existing depth contents and stores the result.
    pub fn load(view: &'a wgpu::TextureView) -> Self {
        Self {
            view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: StoreOp::Store,
            }),
            stencil_ops: None,
        }
    }

    /// Overrides the operations applied to the depth component.
    pub fn with_depth_ops(mut self, ops: Option<wgpu::Operations<f32>>) -> Self {
        self.depth_ops = ops;
        self
    }

    /// Overrides the operations applied to the stencil component.
    pub fn with_stencil_ops(mut self, ops: Option<wgpu::Operations<u32>>) -> Self {
        self.stencil_ops = ops;
        self
    }

    fn into_wgpu(self) -> wgpu::RenderPassDepthStencilAttachment<'a> {
        wgpu::RenderPassDepthStencilAttachment {
            view: self.view,
            depth_ops: self.depth_ops,
            stencil_ops: self.stencil_ops,
        }
    }
}

/// Captures the attachments for a render pass and the label used for debugging.
pub struct PassPlan<'a> {
    label: &'a str,
    color_attachments: Vec<Option<wgpu::RenderPassColorAttachment<'a>>>,
    depth_attachment: Option<wgpu::RenderPassDepthStencilAttachment<'a>>,
}

impl<'a> PassPlan<'a> {
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            color_attachments: Vec::new(),
            depth_attachment: None,
        }
    }

    pub fn add_color(mut self, attachment: FrameGraphColorAttachment<'a>) -> Self {
        self.color_attachments.push(Some(attachment.into_wgpu()));
        self
    }

    pub fn add_optional_color(mut self, attachment: Option<FrameGraphColorAttachment<'a>>) -> Self {
        if let Some(attachment) = attachment {
            self.color_attachments.push(Some(attachment.into_wgpu()));
        }
        self
    }

    pub fn set_depth(mut self, attachment: FrameGraphDepthAttachment<'a>) -> Self {
        self.depth_attachment = Some(attachment.into_wgpu());
        self
    }

    pub fn depth_attachment(mut self, attachment: Option<FrameGraphDepthAttachment<'a>>) -> Self {
        self.depth_attachment = attachment.map(FrameGraphDepthAttachment::into_wgpu);
        self
    }
}

/// Small helper that schedules render passes sequentially using a shared encoder.
pub struct FrameGraph<'a> {
    encoder: &'a mut wgpu::CommandEncoder,
}

impl<'a> FrameGraph<'a> {
    pub fn new(encoder: &'a mut wgpu::CommandEncoder) -> Self {
        Self { encoder }
    }

    pub fn execute_pass<'pass, F>(
        &'a mut self,
        plan: PassPlan<'pass>,
        region: Option<RenderRegion>,
        f: F,
    ) where
        F: FnOnce(&mut wgpu::RenderPass<'pass>),
        'a: 'pass,
    {
        let mut pass = self.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(plan.label),
            color_attachments: &plan.color_attachments,
            depth_stencil_attachment: plan.depth_attachment,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        if let Some(region) = region {
            region.apply_to_pass(&mut pass);
        }

        f(&mut pass);
    }
}
