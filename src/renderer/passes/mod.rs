use crate::asset::{Assets, Handle, MaterialAsset, Mesh};
use crate::environment::Environment;
use crate::renderer::frame_graph::{
    FrameGraph, FrameGraphColorAttachment, FrameGraphDepthAttachment, PassPlan,
};
use crate::renderer::internal::shadows::ShadowInvocation;
use crate::renderer::internal::{MaterialPipelineKey, OrderedBatch, PipelineKey, PreparedBatches};
use crate::renderer::{
    CustomRenderContext, CustomRenderRequest, CustomRenderStage, LightsData, Material, RenderPass,
    RenderRegion,
};
use crate::scene::Scene;

use super::super::batch::InstanceData;
use super::{RenderFrame, Renderer};

#[derive(Clone, Copy)]
pub(crate) struct BatchPassOptions {
    pub(crate) color_format: wgpu::TextureFormat,
    pub(crate) color_sample_count: u32,
    pub(crate) use_gbuffer: bool,
    pub(crate) write_pick: bool,
}

pub(crate) struct BatchRecorder<'a> {
    renderer: &'a mut Renderer,
    assets: &'a Assets,
    materials: &'a [Material],
    material_handles: &'a [Handle<MaterialAsset>],
    material_pipeline_keys: &'a [MaterialPipelineKey],
}

impl<'a> BatchRecorder<'a> {
    pub(crate) fn new(
        renderer: &'a mut Renderer,
        assets: &'a Assets,
        materials: &'a [Material],
        material_handles: &'a [Handle<MaterialAsset>],
        material_pipeline_keys: &'a [MaterialPipelineKey],
    ) -> Self {
        Self {
            renderer,
            assets,
            materials,
            material_handles,
            material_pipeline_keys,
        }
    }

    pub(crate) fn draw_environment_background(
        &mut self,
        pass: &mut wgpu::RenderPass<'_>,
        write_pick: bool,
    ) {
        self.renderer.draw_environment_background(pass, write_pick);
    }

    pub(crate) fn record(
        &mut self,
        pass: &mut wgpu::RenderPass<'_>,
        batches: &[OrderedBatch],
        options: BatchPassOptions,
        bindless_group: Option<&wgpu::BindGroup>,
    ) -> u32 {
        let mut draw_calls = 0u32;

        for batch in batches {
            let Some(mesh) = mesh_for_batch(self.assets, batch) else {
                continue;
            };

            let wants_wireframe = matches!(batch.pass, RenderPass::Gizmo);
            let wireframe = wants_wireframe && self.renderer.context.supports_wireframe;
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
            pass.set_bind_group(0, &self.renderer.camera_buffer.bind_group, &[]);
            pass.set_bind_group(1, &self.renderer.objects_buffer.bind_group, &[]);
            pass.set_bind_group(2, &self.renderer.lights_buffer.bind_group, &[]);
            pass.set_vertex_buffer(0, mesh.vertex_buffer().slice(..));
            pass.set_index_buffer(mesh.index_buffer().slice(..), mesh.index_format());

            let device = self.renderer.get_device().clone();

            if let Some(group) = bindless_group {
                let pipeline = self.renderer.pipeline.pipeline_for_material(
                    &device,
                    self.assets,
                    pipeline_key,
                    batch.material_pipeline_key,
                );
                pass.set_pipeline(pipeline);
                pass.set_bind_group(3, group, &[]);

                let start_instance = batch.first_instance;
                let end_instance = start_instance + batch.instances.len() as u32;
                pass.draw_indexed(0..mesh.index_count(), 0, start_instance..end_instance);
                draw_calls += 1;
                continue;
            }

            let mut local_offset = 0usize;
            while local_offset < batch.instances.len() {
                let material_index = batch.instances[local_offset].material_index as usize;
                let material_key = self
                    .material_pipeline_keys
                    .get(material_index)
                    .copied()
                    .unwrap_or(MaterialPipelineKey::Pbr);
                let pipeline = self.renderer.pipeline.pipeline_for_material(
                    &device,
                    self.assets,
                    pipeline_key,
                    material_key,
                );
                pass.set_pipeline(pipeline);

                let Some(material) = self.materials.get(material_index).copied() else {
                    let handle = self.material_handles.get(material_index);
                    log::warn!(
                        "Material index {} out of bounds ({} materials, handle {:?})",
                        material_index,
                        self.materials.len(),
                        handle
                    );
                    local_offset += 1;
                    continue;
                };

                let Some(bind_group) = self.renderer.material_bind_group(self.assets, material)
                else {
                    local_offset += 1;
                    continue;
                };
                pass.set_bind_group(3, bind_group, &[]);

                let run_length = material_run_length(batch.instances.as_slice(), local_offset);
                let start_instance = batch.first_instance + local_offset as u32;
                let end_instance = start_instance + run_length as u32;

                pass.draw_indexed(0..mesh.index_count(), 0, start_instance..end_instance);
                draw_calls += 1;
                local_offset += run_length;
            }
        }

        draw_calls
    }
}

pub(crate) struct SurfacePassAttachments<'a> {
    pub(crate) color: &'a wgpu::TextureView,
    pub(crate) resolve: Option<&'a wgpu::TextureView>,
    pub(crate) depth: Option<&'a wgpu::TextureView>,
    pub(crate) pick: Option<&'a wgpu::TextureView>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_surface_pass(
    encoder: &mut wgpu::CommandEncoder,
    label: &'static str,
    attachments: SurfacePassAttachments<'_>,
    render_region: Option<RenderRegion>,
    batches: &[OrderedBatch],
    recorder: &mut BatchRecorder<'_>,
    options: BatchPassOptions,
    bindless_group: Option<&wgpu::BindGroup>,
) -> u32 {
    if batches.is_empty() {
        return 0;
    }

    let mut plan = PassPlan::new(label).add_color(FrameGraphColorAttachment::load(
        attachments.color,
        attachments.resolve,
    ));

    if let Some(pick) = attachments.pick {
        plan = plan.add_color(FrameGraphColorAttachment::load(pick, None));
    }

    if let Some(depth) = attachments.depth {
        plan = plan.set_depth(FrameGraphDepthAttachment::load(depth));
    }

    let mut frame_graph = FrameGraph::new(encoder);
    let mut draw_calls = 0u32;

    frame_graph.execute_pass(plan, render_region, |pass| {
        draw_calls += recorder.record(pass, batches, options, bindless_group);
    });

    draw_calls
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn shadow_pass(
    renderer: &mut Renderer,
    scene: &Scene,
    assets: &Assets,
    prepared_batches: &PreparedBatches,
    lights: &LightsData,
    encoder: &mut wgpu::CommandEncoder,
    custom_render: Option<&mut CustomRenderRequest<'_>>,
    custom_shadow_enabled: bool,
    shadow_invocations: &mut Vec<ShadowInvocation>,
) {
    shadow_invocations.clear();
    renderer.shadows.render(
        &renderer.context,
        encoder,
        assets,
        prepared_batches.all(),
        lights,
        &renderer.objects_buffer,
        prepared_batches.materials(),
        custom_shadow_enabled,
        shadow_invocations,
    );

    if custom_shadow_enabled {
        if let Some(request) = custom_render {
            for invocation in shadow_invocations.iter() {
                let mut ctx = CustomRenderContext::new(
                    encoder,
                    &*renderer,
                    scene,
                    &invocation.view,
                    &invocation.view,
                    CustomRenderStage::Shadow(invocation.stage),
                    Some(invocation.view_proj),
                    None,
                );
                (request.callback)(&mut ctx);
            }
        }
    }
}

pub(crate) fn depth_prepass(
    renderer: &mut Renderer,
    assets: &Assets,
    prepared_batches: &mut PreparedBatches,
    encoder: &mut wgpu::CommandEncoder,
    depth_view: &wgpu::TextureView,
    render_region: Option<RenderRegion>,
) -> u32 {
    let mut frame_graph = FrameGraph::new(encoder);
    let plan =
        PassPlan::new("DepthPrepass").set_depth(FrameGraphDepthAttachment::clear(depth_view, 1.0));
    let pipeline = renderer.pipeline.depth_prepass();
    let camera_bind_group = &renderer.camera_buffer.bind_group;
    let object_bind_group = &renderer.objects_buffer.bind_group;
    let batches = prepared_batches.opaque_mut();
    let mut draw_calls = 0u32;

    frame_graph.execute_pass(plan, render_region, |pass| {
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, camera_bind_group, &[]);
        pass.set_bind_group(1, object_bind_group, &[]);

        for batch in batches.iter_mut() {
            if batch.alpha_blend || !batch.depth_state.depth_write || !batch.depth_state.depth_test
            {
                continue;
            }

            let Some(mesh) = mesh_for_batch(assets, batch) else {
                continue;
            };

            renderer.draw_full_batch(pass, mesh, batch);
            draw_calls += 1;
            batch.depth_state.depth_write = false;
        }
    });

    draw_calls
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn main_color_pass(
    renderer: &mut Renderer,
    environment: &Environment,
    assets: &Assets,
    prepared_batches: &PreparedBatches,
    encoder: &mut wgpu::CommandEncoder,
    scene_view: &wgpu::TextureView,
    scene_resolve_view: Option<&wgpu::TextureView>,
    depth_view: &wgpu::TextureView,
    render_region: Option<RenderRegion>,
    scene_format: wgpu::TextureFormat,
    pick_active: bool,
) -> u32 {
    let (normal_view, normal_resolve, position_view, position_resolve, pick_targets) = {
        let gbuffer_views = renderer.postprocess.gbuffer_views();
        (
            gbuffer_views.normal.0.clone(),
            gbuffer_views.normal.1.cloned(),
            gbuffer_views.position.0.clone(),
            gbuffer_views.position.1.cloned(),
            if pick_active {
                gbuffer_views
                    .id
                    .map(|views| (views.multisample.clone(), views.resolve.cloned()))
            } else {
                None
            },
        )
    };

    let mut plan = PassPlan::new("MainPass")
        .add_color(FrameGraphColorAttachment::clear(
            scene_view,
            scene_resolve_view,
            environment.clear_color(),
        ))
        .add_color(FrameGraphColorAttachment::clear(
            &normal_view,
            normal_resolve.as_ref(),
            wgpu::Color::TRANSPARENT,
        ))
        .add_color(FrameGraphColorAttachment::clear(
            &position_view,
            position_resolve.as_ref(),
            wgpu::Color::TRANSPARENT,
        ))
        .set_depth(FrameGraphDepthAttachment::load(depth_view));

    if let Some((pick_view, pick_resolve)) = pick_targets.as_ref() {
        plan = plan.add_color(FrameGraphColorAttachment::clear(
            pick_view,
            pick_resolve.as_ref(),
            wgpu::Color::TRANSPARENT,
        ));
    }

    let hdr_enabled = environment.is_hdr_enabled();
    let sample_count = renderer.context.sample_count;
    let batches = prepared_batches.opaque();
    let materials = prepared_batches.materials();
    let material_handles = prepared_batches.material_handles();
    let material_pipeline_keys = prepared_batches.material_pipeline_keys();
    let bindless_group = renderer.texture_binder.global_bind_group().cloned();
    let mut frame_graph = FrameGraph::new(encoder);
    let mut recorder = BatchRecorder::new(
        renderer,
        assets,
        materials,
        material_handles,
        material_pipeline_keys,
    );
    let mut draw_calls = 0u32;

    frame_graph.execute_pass(plan, render_region, |pass| {
        if hdr_enabled {
            recorder.draw_environment_background(pass, pick_active);
        }

        if !batches.is_empty() {
            let options = BatchPassOptions {
                color_format: scene_format,
                color_sample_count: sample_count,
                use_gbuffer: true,
                write_pick: pick_active,
            };

            draw_calls += recorder.record(pass, batches, options, bindless_group.as_ref());
        }
    });

    draw_calls
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn custom_pass(
    renderer: &Renderer,
    scene: &Scene,
    encoder: &mut wgpu::CommandEncoder,
    color_view: &wgpu::TextureView,
    depth_view: &wgpu::TextureView,
    stage: CustomRenderStage,
    render_region: Option<RenderRegion>,
    request: Option<&mut CustomRenderRequest<'_>>,
) {
    if let Some(request) = request {
        if request.stage == stage {
            let mut ctx = CustomRenderContext::new(
                encoder,
                renderer,
                scene,
                color_view,
                depth_view,
                stage,
                None,
                render_region,
            );
            (request.callback)(&mut ctx);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn transparent_pass(
    renderer: &mut Renderer,
    assets: &Assets,
    prepared_batches: &PreparedBatches,
    encoder: &mut wgpu::CommandEncoder,
    surface_view: &wgpu::TextureView,
    depth_view: &wgpu::TextureView,
    render_region: Option<RenderRegion>,
    write_pick: bool,
    surface_format: wgpu::TextureFormat,
) -> u32 {
    let batches = prepared_batches.transparent();
    let materials = prepared_batches.materials();
    let material_handles = prepared_batches.material_handles();
    let material_pipeline_keys = prepared_batches.material_pipeline_keys();
    let pick_view = if write_pick {
        renderer
            .postprocess
            .pick_attachment_views()
            .map(|views| views.single_sample().clone())
    } else {
        None
    };

    let attachments = SurfacePassAttachments {
        color: surface_view,
        resolve: None,
        depth: Some(depth_view),
        pick: pick_view.as_ref(),
    };

    let bindless_group = renderer.texture_binder.global_bind_group().cloned();
    let mut recorder = BatchRecorder::new(
        renderer,
        assets,
        materials,
        material_handles,
        material_pipeline_keys,
    );
    let options = BatchPassOptions {
        color_format: surface_format,
        color_sample_count: 1,
        use_gbuffer: false,
        write_pick,
    };

    execute_surface_pass(
        encoder,
        "TransparentPass",
        attachments,
        render_region,
        batches,
        &mut recorder,
        options,
        bindless_group.as_ref(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn overlay_pass(
    renderer: &mut Renderer,
    assets: &Assets,
    prepared_batches: &PreparedBatches,
    encoder: &mut wgpu::CommandEncoder,
    surface_view: &wgpu::TextureView,
    depth_view: &wgpu::TextureView,
    render_region: Option<RenderRegion>,
    write_pick: bool,
    surface_format: wgpu::TextureFormat,
) -> u32 {
    let batches = prepared_batches.overlay();
    if batches.is_empty() {
        return 0;
    }

    let materials = prepared_batches.materials();
    let material_handles = prepared_batches.material_handles();
    let material_pipeline_keys = prepared_batches.material_pipeline_keys();
    let overlay_needs_depth = batches
        .iter()
        .any(|batch| batch.depth_state.depth_test || batch.depth_state.depth_write);
    let pick_view = if write_pick {
        renderer
            .postprocess
            .pick_attachment_views()
            .map(|views| views.single_sample().clone())
    } else {
        None
    };

    let attachments = SurfacePassAttachments {
        color: surface_view,
        resolve: None,
        depth: overlay_needs_depth.then_some(depth_view),
        pick: pick_view.as_ref(),
    };

    let bindless_group = renderer.texture_binder.global_bind_group().cloned();
    let mut recorder = BatchRecorder::new(
        renderer,
        assets,
        materials,
        material_handles,
        material_pipeline_keys,
    );
    let options = BatchPassOptions {
        color_format: surface_format,
        color_sample_count: 1,
        use_gbuffer: false,
        write_pick,
    };

    execute_surface_pass(
        encoder,
        "OverlayPass",
        attachments,
        render_region,
        batches,
        &mut recorder,
        options,
        bindless_group.as_ref(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn gizmo_pass(
    renderer: &mut Renderer,
    assets: &Assets,
    batches: &[OrderedBatch],
    materials: &[Material],
    material_handles: &[Handle<MaterialAsset>],
    material_pipeline_keys: &[MaterialPipelineKey],
    encoder: &mut wgpu::CommandEncoder,
    surface_view: &wgpu::TextureView,
    render_region: Option<RenderRegion>,
    write_pick: bool,
    surface_format: wgpu::TextureFormat,
    label: &'static str,
) -> u32 {
    if batches.is_empty() {
        return 0;
    }

    let pick_view = if write_pick {
        renderer
            .postprocess
            .pick_attachment_views()
            .map(|views| views.single_sample().clone())
    } else {
        None
    };

    let attachments = SurfacePassAttachments {
        color: surface_view,
        resolve: None,
        depth: None,
        pick: pick_view.as_ref(),
    };

    let bindless_group = renderer.texture_binder.global_bind_group().cloned();
    let mut recorder = BatchRecorder::new(
        renderer,
        assets,
        materials,
        material_handles,
        material_pipeline_keys,
    );
    let options = BatchPassOptions {
        color_format: surface_format,
        color_sample_count: 1,
        use_gbuffer: false,
        write_pick,
    };

    execute_surface_pass(
        encoder,
        label,
        attachments,
        render_region,
        batches,
        &mut recorder,
        options,
        bindless_group.as_ref(),
    )
}

pub(crate) fn process_pick(renderer: &mut Renderer, encoder: &mut wgpu::CommandEncoder) {
    renderer.process_pick_request(encoder);
}

pub(crate) fn finish_frame(
    renderer: &mut Renderer,
    encoder: wgpu::CommandEncoder,
    frame: wgpu::SurfaceTexture,
) -> RenderFrame {
    renderer.context.queue.submit(Some(encoder.finish()));

    if let Some(readback) = renderer.pick_state.pending_readback.as_mut() {
        readback.ensure_mapping_requested();
    }

    RenderFrame { frame }
}

pub(crate) fn mesh_for_batch<'a>(assets: &'a Assets, batch: &OrderedBatch) -> Option<&'a Mesh> {
    let mesh = assets.meshes.get(batch.mesh);
    if mesh.is_none() {
        log::warn!("Skipping batch with invalid mesh handle");
    }
    mesh
}

pub(crate) fn material_run_length(instances: &[InstanceData], start: usize) -> usize {
    let material = instances[start].material_index;
    let mut length = 1usize;
    while start + length < instances.len() && instances[start + length].material_index == material {
        length += 1;
    }
    length
}
