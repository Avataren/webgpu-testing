use crate::asset::Assets;
use crate::renderer::frame_graph::{
    FrameGraph, FrameGraphColorAttachment, FrameGraphDepthAttachment, PassPlan,
};
use crate::renderer::internal::shadows::ShadowInvocation;
use crate::renderer::internal::{OrderedBatch, PipelineKey};
use crate::renderer::{
    CustomRenderContext, CustomRenderRequest, CustomRenderStage, Material, RenderPass,
    RenderRegion, Renderer,
};
use crate::scene::Scene;

pub(crate) struct BatchPassOptions {
    pub color_format: wgpu::TextureFormat,
    pub color_sample_count: u32,
    pub use_gbuffer: bool,
    pub write_pick: bool,
}

pub(crate) struct ShadowPassContext<'a> {
    pub renderer: &'a mut Renderer,
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub assets: &'a Assets,
    pub batches: &'a [OrderedBatch],
    pub lights: &'a crate::renderer::LightsData,
    pub materials: &'a [Material],
    pub record_custom_passes: bool,
}

pub(crate) struct ShadowPassOutput {
    pub invocations: Vec<ShadowInvocation>,
}

pub(crate) fn run_shadow_pass(ctx: ShadowPassContext<'_>) -> ShadowPassOutput {
    let mut invocations = Vec::new();
    ctx.renderer.render_shadow_pass(
        ctx.encoder,
        ctx.assets,
        ctx.batches,
        ctx.lights,
        ctx.materials,
        ctx.record_custom_passes,
        &mut invocations,
    );

    ShadowPassOutput { invocations }
}

pub(crate) struct CustomShadowContext<'a> {
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub renderer: &'a Renderer,
    pub scene: &'a Scene,
}

pub(crate) fn run_custom_shadow_hooks(
    ctx: CustomShadowContext<'_>,
    request: &mut CustomRenderRequest<'_>,
    invocations: &[ShadowInvocation],
) {
    for invocation in invocations {
        let mut custom = CustomRenderContext::new(
            ctx.encoder,
            ctx.renderer,
            ctx.scene,
            &invocation.view,
            &invocation.view,
            CustomRenderStage::Shadow(invocation.stage),
            Some(invocation.view_proj),
            None,
        );
        (request.callback)(&mut custom);
    }
}

pub(crate) struct DepthPrepassContext<'a> {
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub render_region: Option<RenderRegion>,
    pub depth_view: &'a wgpu::TextureView,
    pub renderer: &'a Renderer,
    pub assets: &'a Assets,
    pub batches: &'a mut [OrderedBatch],
}

pub(crate) fn run_depth_prepass(ctx: DepthPrepassContext<'_>) -> u32 {
    let mut frame_graph = FrameGraph::new(ctx.encoder);
    let plan = PassPlan::new("DepthPrepass")
        .set_depth(FrameGraphDepthAttachment::clear(ctx.depth_view, 1.0));
    let pipeline = ctx.renderer.depth_prepass_pipeline();
    let camera_bind_group = ctx.renderer.camera_bind_group();
    let object_bind_group = ctx.renderer.objects_bind_group();
    let assets = ctx.assets;
    let renderer = ctx.renderer;
    let mut draw_calls = 0u32;

    frame_graph.execute_pass(plan, ctx.render_region, |pass| {
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, camera_bind_group, &[]);
        pass.set_bind_group(1, object_bind_group, &[]);

        for batch in ctx.batches.iter_mut() {
            if batch.alpha_blend || !batch.depth_state.depth_write || !batch.depth_state.depth_test
            {
                continue;
            }

            let Some(mesh) = crate::renderer::renderer_core::mesh_for_batch(assets, batch) else {
                continue;
            };

            renderer.draw_full_batch(pass, mesh, batch);
            draw_calls += 1;
            batch.depth_state.depth_write = false;
        }
    });

    draw_calls
}

pub(crate) struct MainColorPassContext<'a> {
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub render_region: Option<RenderRegion>,
    pub scene_view: &'a wgpu::TextureView,
    pub scene_resolve_view: Option<&'a wgpu::TextureView>,
    pub normal_view: &'a wgpu::TextureView,
    pub normal_resolve: Option<&'a wgpu::TextureView>,
    pub position_view: &'a wgpu::TextureView,
    pub position_resolve: Option<&'a wgpu::TextureView>,
    pub pick_targets: Option<(&'a wgpu::TextureView, Option<&'a wgpu::TextureView>)>,
    pub renderer: &'a mut Renderer,
    pub assets: &'a Assets,
    pub batches: &'a [OrderedBatch],
    pub materials: &'a [Material],
    pub scene_format: wgpu::TextureFormat,
    pub sample_count: u32,
    pub write_pick: bool,
    pub hdr_enabled: bool,
    pub clear_color: wgpu::Color,
    pub depth_view: &'a wgpu::TextureView,
}

pub(crate) fn run_main_color_pass(ctx: MainColorPassContext<'_>) -> u32 {
    if ctx.batches.is_empty() {
        return 0;
    }

    let mut frame_graph = FrameGraph::new(ctx.encoder);
    let mut plan = PassPlan::new("MainPass")
        .add_color(FrameGraphColorAttachment::clear(
            ctx.scene_view,
            ctx.scene_resolve_view,
            ctx.clear_color,
        ))
        .add_color(FrameGraphColorAttachment::clear(
            ctx.normal_view,
            ctx.normal_resolve,
            wgpu::Color::TRANSPARENT,
        ))
        .add_color(FrameGraphColorAttachment::clear(
            ctx.position_view,
            ctx.position_resolve,
            wgpu::Color::TRANSPARENT,
        ))
        .set_depth(FrameGraphDepthAttachment::load(ctx.depth_view));

    if let Some((pick_view, pick_resolve)) = ctx.pick_targets {
        plan = plan.add_color(FrameGraphColorAttachment::clear(
            pick_view,
            pick_resolve,
            wgpu::Color::TRANSPARENT,
        ));
    }

    let mut recorder = BatchRecorder::new(ctx.renderer, ctx.assets, ctx.materials);
    let options = BatchPassOptions {
        color_format: ctx.scene_format,
        color_sample_count: ctx.sample_count,
        use_gbuffer: true,
        write_pick: ctx.write_pick,
    };
    let mut draw_calls = 0u32;

    frame_graph.execute_pass(plan, ctx.render_region, |pass| {
        if ctx.hdr_enabled {
            recorder.draw_environment_background(pass, ctx.write_pick);
        }

        draw_calls += recorder.record(pass, ctx.batches, options);
    });

    draw_calls
}

pub(crate) struct CustomBeforePassContext<'a> {
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub renderer: &'a Renderer,
    pub scene: &'a Scene,
    pub color_view: &'a wgpu::TextureView,
    pub depth_view: &'a wgpu::TextureView,
    pub render_region: Option<RenderRegion>,
}

pub(crate) fn run_custom_before_postprocess(
    ctx: CustomBeforePassContext<'_>,
    request: &mut CustomRenderRequest<'_>,
) {
    if request.stage != CustomRenderStage::BeforePostprocess {
        return;
    }

    let mut custom = CustomRenderContext::new(
        ctx.encoder,
        ctx.renderer,
        ctx.scene,
        ctx.color_view,
        ctx.depth_view,
        CustomRenderStage::BeforePostprocess,
        None,
        ctx.render_region,
    );
    (request.callback)(&mut custom);
}

pub(crate) struct CustomAfterPassContext<'a> {
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub renderer: &'a Renderer,
    pub scene: &'a Scene,
    pub color_view: &'a wgpu::TextureView,
    pub default_depth_view: &'a wgpu::TextureView,
    pub resolved_depth_view: Option<&'a wgpu::TextureView>,
    pub render_region: Option<RenderRegion>,
}

pub(crate) fn run_custom_after_postprocess(
    ctx: CustomAfterPassContext<'_>,
    request: &mut CustomRenderRequest<'_>,
) {
    if request.stage != CustomRenderStage::AfterPostprocess {
        return;
    }

    let depth_view = ctx.resolved_depth_view.unwrap_or(ctx.default_depth_view);
    let mut custom = CustomRenderContext::new(
        ctx.encoder,
        ctx.renderer,
        ctx.scene,
        ctx.color_view,
        depth_view,
        CustomRenderStage::AfterPostprocess,
        None,
        ctx.render_region,
    );
    (request.callback)(&mut custom);
}

pub(crate) struct SurfaceAttachments<'a> {
    pub color: &'a wgpu::TextureView,
    pub depth: Option<&'a wgpu::TextureView>,
    pub pick: Option<&'a wgpu::TextureView>,
}

pub(crate) struct SurfaceBatchPassContext<'a> {
    pub label: &'a str,
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub render_region: Option<RenderRegion>,
    pub attachments: SurfaceAttachments<'a>,
    pub renderer: &'a mut Renderer,
    pub assets: &'a Assets,
    pub batches: &'a [OrderedBatch],
    pub materials: &'a [Material],
    pub options: BatchPassOptions,
}

pub(crate) fn run_transparent_pass(ctx: SurfaceBatchPassContext<'_>) -> u32 {
    run_surface_batch_pass(ctx)
}

pub(crate) fn run_overlay_pass(ctx: SurfaceBatchPassContext<'_>) -> u32 {
    run_surface_batch_pass(ctx)
}

pub(crate) fn run_gizmo_pass(ctx: SurfaceBatchPassContext<'_>) -> u32 {
    run_surface_batch_pass(ctx)
}

fn run_surface_batch_pass(ctx: SurfaceBatchPassContext<'_>) -> u32 {
    if ctx.batches.is_empty() {
        return 0;
    }

    let mut frame_graph = FrameGraph::new(ctx.encoder);
    let mut plan = PassPlan::new(ctx.label)
        .add_color(FrameGraphColorAttachment::load(ctx.attachments.color, None));

    if let Some(pick) = ctx.attachments.pick {
        plan = plan.add_color(FrameGraphColorAttachment::load(pick, None));
    }

    if let Some(depth) = ctx.attachments.depth {
        plan = plan.set_depth(FrameGraphDepthAttachment::load(depth));
    }

    let mut recorder = BatchRecorder::new(ctx.renderer, ctx.assets, ctx.materials);
    let mut draw_calls = 0u32;

    frame_graph.execute_pass(plan, ctx.render_region, |pass| {
        draw_calls += recorder.record(pass, ctx.batches, ctx.options);
    });

    draw_calls
}

struct BatchRecorder<'a> {
    renderer: &'a mut Renderer,
    assets: &'a Assets,
    materials: &'a [Material],
}

impl<'a> BatchRecorder<'a> {
    fn new(renderer: &'a mut Renderer, assets: &'a Assets, materials: &'a [Material]) -> Self {
        Self {
            renderer,
            assets,
            materials,
        }
    }

    fn draw_environment_background(&mut self, pass: &mut wgpu::RenderPass<'_>, write_pick: bool) {
        self.renderer.draw_environment_background(pass, write_pick);
    }

    fn record(
        &mut self,
        pass: &mut wgpu::RenderPass<'_>,
        batches: &[OrderedBatch],
        options: BatchPassOptions,
    ) -> u32 {
        if batches.is_empty() {
            return 0;
        }

        let mut draw_calls = 0u32;
        let bindless_group = self.renderer.global_texture_bind_group().cloned();
        let supports_wireframe = self.renderer.render_context().supports_wireframe;

        for batch in batches {
            let Some(mesh) = crate::renderer::renderer_core::mesh_for_batch(self.assets, batch)
            else {
                continue;
            };

            let wants_wireframe = matches!(batch.pass, RenderPass::Gizmo);
            let wireframe = wants_wireframe && supports_wireframe;
            let pipeline_key = PipelineKey::new(
                batch.depth_state.depth_test,
                batch.depth_state.depth_write,
                batch.alpha_blend,
                options.color_format,
                options.color_sample_count,
                batch.sampler_filtering,
                batch.cull_mode,
                options.use_gbuffer,
                options.write_pick,
                wireframe,
            );
            let pipeline = self.renderer.render_pipeline().pipeline(pipeline_key);
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, self.renderer.camera_bind_group(), &[]);
            pass.set_bind_group(1, self.renderer.objects_bind_group(), &[]);
            pass.set_bind_group(2, self.renderer.lights_bind_group(), &[]);

            if let Some(group) = bindless_group.as_ref() {
                pass.set_bind_group(3, group, &[]);
                self.renderer.draw_full_batch(pass, mesh, batch);
                draw_calls += 1;
            } else {
                draw_calls +=
                    self.renderer
                        .draw_classic_batch(pass, self.assets, mesh, batch, self.materials)
                        as u32;
            }
        }

        draw_calls
    }
}
