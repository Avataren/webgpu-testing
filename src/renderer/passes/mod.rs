use crate::asset::{Assets, Handle, MaterialAsset, Mesh};
use crate::environment::Environment;
use crate::renderer::frame_graph::{
    FrameGraph, FrameGraphColorAttachment, FrameGraphDepthAttachment, PassPlan,
};
use crate::renderer::internal::culling::instance_visible;
use crate::renderer::internal::shadows::{MaskTextures, ShadowInvocation, ShadowPassOutcome};
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
    /// Draw with the camera that has no TAA jitter (things drawn after post-processing).
    pub(crate) stable_camera: bool,
    pub(crate) color_format: wgpu::TextureFormat,
    pub(crate) color_sample_count: u32,
    pub(crate) use_gbuffer: bool,
    /// The pick pass: entity ids only (see `pick_pass`).
    pub(crate) pick_only: bool,
    /// Batches the depth prepass drew keep its depth instead of writing their own.
    pub(crate) after_prepass: bool,
    /// Skip the instances the camera's frustum does not see (`OrderedBatch::camera_visible`):
    /// for passes drawn with the scene camera, not the stable one.
    pub(crate) cull: bool,
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

    pub(crate) fn draw_environment_background(&mut self, pass: &mut wgpu::RenderPass<'_>) {
        self.renderer.draw_environment_background(pass);
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
            let visible: &[bool] = if options.cull {
                if !batch.camera_sees_any() {
                    continue;
                }
                &batch.camera_visible
            } else {
                &[]
            };
            let Some(mesh) = mesh_for_batch(self.assets, &self.renderer.skinning, batch) else {
                continue;
            };

            let wants_wireframe = matches!(batch.pass, RenderPass::Gizmo);
            let wireframe = wants_wireframe && self.renderer.context.supports_wireframe;
            let depth_write =
                batch.depth_state.depth_write && !(options.after_prepass && batch.prepassed);
            let pipeline_key = PipelineKey::new(
                batch.depth_state.depth_test,
                depth_write,
                batch.alpha_blend,
                options.color_format,
                options.color_sample_count,
                batch.sampler_filtering,
                batch.cull_mode,
                options.use_gbuffer,
                options.pick_only,
                wireframe,
            );
            let camera = if options.stable_camera {
                &self.renderer.camera_buffer.stable_bind_group
            } else {
                &self.renderer.camera_buffer.bind_group
            };
            pass.set_bind_group(0, camera, &[]);
            pass.set_bind_group(1, &self.renderer.objects_buffer.bind_group, &[]);
            pass.set_bind_group(2, &self.renderer.lights_buffer.bind_group, &[]);
            pass.set_vertex_buffer(0, mesh.vertex_buffer().slice(..));
            // Last frame's positions, for motion vectors: a skinned instance's previous
            // pose, else the mesh itself (only the model matrix moved it).
            let previous = batch
                .skin
                .and_then(|key| self.renderer.skinning.previous_vertex_buffer(key))
                .unwrap_or(mesh.vertex_buffer())
                .clone();
            pass.set_vertex_buffer(1, previous.slice(..));
            pass.set_index_buffer(mesh.index_buffer().slice(..), mesh.index_format());

            if let Some(group) = bindless_group {
                pass.set_bind_group(3, group, &[]);
            }

            let mut local_offset = 0usize;
            while local_offset < batch.instances.len() {
                if !instance_visible(visible, local_offset) {
                    local_offset += 1;
                    continue;
                }
                let material_index = batch.instances[local_offset].material_index as usize;
                let material_key = self
                    .material_pipeline_keys
                    .get(material_index)
                    .copied()
                    .unwrap_or(MaterialPipelineKey::Pbr);
                let pipeline =
                    self.renderer
                        .pipeline_for_material(self.assets, pipeline_key, material_key);
                pass.set_pipeline(pipeline);

                if bindless_group.is_none() {
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
                }

                let run_length =
                    material_run_length(batch.instances.as_slice(), visible, local_offset);
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

fn batch_uses_shader(batch: &OrderedBatch, material_pipeline_keys: &[MaterialPipelineKey]) -> bool {
    batch_shader(batch, material_pipeline_keys).is_some()
}

/// The shader material a batch draws with (batches never mix materials kinds) and its
/// texture filtering.
fn batch_shader(
    batch: &OrderedBatch,
    material_pipeline_keys: &[MaterialPipelineKey],
) -> Option<(Handle<MaterialAsset>, crate::renderer::SamplerFilterMode)> {
    let key = batch.instances.first().and_then(|instance| {
        material_pipeline_keys
            .get(instance.material_index as usize)
            .copied()
    })?;
    match key {
        MaterialPipelineKey::Shader(handle) => Some((handle, batch.sampler_filtering)),
        _ => None,
    }
}

pub(crate) struct SurfacePassConfig<'a, 'b> {
    label: &'static str,
    color: &'a wgpu::TextureView,
    resolve: Option<&'a wgpu::TextureView>,
    depth: Option<&'a wgpu::TextureView>,
    render_region: Option<RenderRegion>,
    batches: &'a [OrderedBatch],
    recorder: &'b mut BatchRecorder<'a>,
    options: BatchPassOptions,
    bindless_group: Option<&'a wgpu::BindGroup>,
}

impl<'a, 'b> SurfacePassConfig<'a, 'b> {
    pub(crate) fn new(
        label: &'static str,
        color: &'a wgpu::TextureView,
        batches: &'a [OrderedBatch],
        recorder: &'b mut BatchRecorder<'a>,
        options: BatchPassOptions,
    ) -> Self {
        Self {
            label,
            color,
            resolve: None,
            depth: None,
            render_region: None,
            batches,
            recorder,
            options,
            bindless_group: None,
        }
    }

    pub(crate) fn resolve(mut self, resolve: Option<&'a wgpu::TextureView>) -> Self {
        self.resolve = resolve;
        self
    }

    pub(crate) fn depth(mut self, depth: Option<&'a wgpu::TextureView>) -> Self {
        self.depth = depth;
        self
    }

    pub(crate) fn render_region(mut self, render_region: Option<RenderRegion>) -> Self {
        self.render_region = render_region;
        self
    }

    pub(crate) fn bindless_group(mut self, bindless_group: Option<&'a wgpu::BindGroup>) -> Self {
        self.bindless_group = bindless_group;
        self
    }
}

pub(crate) fn run_surface_pass(
    encoder: &mut wgpu::CommandEncoder,
    config: SurfacePassConfig<'_, '_>,
) -> u32 {
    if config.batches.is_empty() {
        return 0;
    }

    let SurfacePassConfig {
        label,
        color,
        resolve,
        depth,
        render_region,
        batches,
        recorder,
        options,
        bindless_group,
    } = config;

    let mut plan = PassPlan::new(label).add_color(FrameGraphColorAttachment::load(color, resolve));

    if let Some(depth) = depth {
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
    skip: u32,
) -> ShadowPassOutcome {
    shadow_invocations.clear();
    let mask_textures = cutout_textures(renderer, assets, prepared_batches);
    let geometry = BatchGeometry {
        assets,
        skinning: &renderer.skinning,
    };
    let texture_generation = renderer.texture_generation;
    let outcome = renderer.shadows.render(
        &renderer.context,
        encoder,
        &geometry,
        prepared_batches,
        lights,
        &renderer.objects_buffer,
        custom_shadow_enabled,
        shadow_invocations,
        skip,
        mask_textures.as_ref(),
        texture_generation,
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
    outcome
}

/// Textures for the alpha test of this frame's opaque cut-out surfaces.
fn cutout_textures(
    renderer: &mut Renderer,
    assets: &Assets,
    prepared_batches: &PreparedBatches,
) -> Option<MaskTextures> {
    let masked = prepared_batches
        .opaque()
        .iter()
        .filter(|batch| batch.alpha_masked);
    if let Some(group) = renderer.texture_binder.global_bind_group() {
        return masked
            .clone()
            .next()
            .map(|_| MaskTextures::Bindless(group.clone()));
    }
    let mut groups = std::collections::HashMap::new();
    for instance in masked.flat_map(|batch| batch.instances.iter()) {
        if groups.contains_key(&instance.material_index) {
            continue;
        }
        let Some(material) = prepared_batches
            .materials()
            .get(instance.material_index as usize)
            .copied()
        else {
            continue;
        };
        if let Some(group) = renderer.material_bind_group(assets, material) {
            groups.insert(instance.material_index, group.clone());
        }
    }
    (!groups.is_empty()).then_some(MaskTextures::PerMaterial(groups))
}

/// Depth for the opaque batches the main pass draws with the PBR shader, so its lighting
/// runs once per pixel, and which the ray-traced shadow mask is traced from. Cut-out
/// surfaces draw with the alpha test. Shader materials are left out: their own vertex
/// stage (which may move vertices) writes their depth in the main pass.
pub(crate) fn depth_prepass(
    renderer: &mut Renderer,
    assets: &Assets,
    prepared_batches: &mut PreparedBatches,
    encoder: &mut wgpu::CommandEncoder,
    depth_view: &wgpu::TextureView,
    render_region: Option<RenderRegion>,
) -> u32 {
    let cutouts = cutout_textures(renderer, assets, prepared_batches);
    let mut frame_graph = FrameGraph::new(encoder);
    let plan =
        PassPlan::new("DepthPrepass").set_depth(FrameGraphDepthAttachment::clear(depth_view, 1.0));
    let pipeline = renderer.pipeline.depth_prepass();
    let masked_pipeline = renderer.pipeline.depth_prepass_masked();
    let camera_bind_group = &renderer.camera_buffer.bind_group;
    let object_bind_group = &renderer.objects_buffer.bind_group;
    let lights_bind_group = &renderer.lights_buffer.bind_group;
    let material_pipeline_keys = prepared_batches.material_pipeline_keys.clone();
    let batches = prepared_batches.opaque_mut();
    let mut draw_calls = 0u32;

    frame_graph.execute_pass(plan, render_region, |pass| {
        pass.set_bind_group(0, camera_bind_group, &[]);
        pass.set_bind_group(1, object_bind_group, &[]);
        pass.set_bind_group(2, lights_bind_group, &[]);

        for masked in [false, true] {
            pass.set_pipeline(if masked { masked_pipeline } else { pipeline });
            for batch in batches.iter_mut() {
                if batch.alpha_masked != masked
                    || batch.alpha_blend
                    || !batch.depth_state.depth_write
                    || !batch.depth_state.depth_test
                    || batch_uses_shader(batch, &material_pipeline_keys)
                {
                    continue;
                }
                // Instances outside the view are left out of both this pass and the main
                // pass, so the batch still counts as drawn whole.
                if !batch.camera_sees_any() {
                    batch.prepassed = true;
                    continue;
                }
                let Some(mesh) = mesh_for_batch(assets, &renderer.skinning, batch) else {
                    continue;
                };
                let visible = &batch.camera_visible;
                if !masked {
                    renderer.set_geometry_buffers(pass, &mesh);
                    let mut start = 0usize;
                    while start < batch.instances.len() {
                        if !instance_visible(visible, start) {
                            start += 1;
                            continue;
                        }
                        let run = visible_run_length(visible, batch.instances.len(), start);
                        let first = batch.first_instance + start as u32;
                        pass.draw_indexed(0..mesh.index_count(), 0, first..first + run as u32);
                        draw_calls += 1;
                        start += run;
                    }
                    batch.prepassed = true;
                    continue;
                }
                // Cut-outs: one draw per material, with its textures for the alpha test.
                let Some(textures) = cutouts.as_ref() else {
                    continue;
                };
                renderer.set_geometry_buffers(pass, &mesh);
                let mut drawn = true;
                let mut start = 0usize;
                while start < batch.instances.len() {
                    if !instance_visible(visible, start) {
                        start += 1;
                        continue;
                    }
                    let run = material_run_length(&batch.instances, visible, start);
                    match textures.group(batch.instances[start].material_index) {
                        Some(group) => {
                            pass.set_bind_group(3, group, &[]);
                            let first = batch.first_instance + start as u32;
                            pass.draw_indexed(0..mesh.index_count(), 0, first..first + run as u32);
                            draw_calls += 1;
                        }
                        None => drawn = false,
                    }
                    start += run;
                }
                // Only a batch drawn whole keeps the prepass depth in the main pass.
                batch.prepassed = drawn;
            }
        }
    });

    draw_calls
}

/// The opaque scene: lit colour and the G-buffer (normal and motion, position), over the
/// environment background.
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
) -> u32 {
    let (normal_view, normal_resolve, position_view, position_resolve) = {
        let gbuffer_views = renderer.postprocess.gbuffer_views();
        (
            gbuffer_views.normal.0.clone(),
            gbuffer_views.normal.1.cloned(),
            gbuffer_views.position.0.clone(),
            gbuffer_views.position.1.cloned(),
        )
    };

    let plan = PassPlan::new("MainPass")
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

    let hdr_background = environment.is_hdr_background_visible();
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
        if hdr_background {
            recorder.draw_environment_background(pass);
        }

        if !batches.is_empty() {
            let options = BatchPassOptions {
                stable_camera: false,
                color_format: scene_format,
                color_sample_count: sample_count,
                use_gbuffer: true,
                pick_only: false,
                after_prepass: true,
                cull: true,
            };

            draw_calls += recorder.record(pass, batches, options, bindless_group.as_ref());
        }
    });

    draw_calls
}

/// Shader materials' own colour over what the main pass left for them, where it is not
/// already theirs: shaders whose G-buffer the PBR shader stood in for (see
/// `RenderPipeline::shader_writes_gbuffer`), and every shader material when path tracing
/// replaced the opaque lighting (`path_traced`). Their depth is in place, so they draw
/// at equal depth without writing it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn shader_material_pass(
    renderer: &mut Renderer,
    assets: &Assets,
    prepared_batches: &PreparedBatches,
    encoder: &mut wgpu::CommandEncoder,
    scene_view: &wgpu::TextureView,
    scene_resolve_view: Option<&wgpu::TextureView>,
    depth_view: &wgpu::TextureView,
    render_region: Option<RenderRegion>,
    path_traced: bool,
) -> u32 {
    let material_pipeline_keys = prepared_batches.material_pipeline_keys();
    let device = renderer.context.device.clone();
    let shader_batches: Vec<OrderedBatch> = prepared_batches
        .opaque()
        .iter()
        .filter(|batch| match batch_shader(batch, material_pipeline_keys) {
            Some((handle, filtering)) => {
                path_traced
                    || !renderer
                        .pipeline
                        .shader_writes_gbuffer(&device, assets, handle, filtering)
            }
            None => false,
        })
        .cloned()
        .map(|mut batch| {
            // The main pass wrote their depth.
            batch.depth_state.depth_write = false;
            batch
        })
        .collect();
    if shader_batches.is_empty() {
        return 0;
    }

    let options = BatchPassOptions {
        stable_camera: false,
        color_format: renderer.context.scene_texture_format(),
        color_sample_count: renderer.context.sample_count,
        use_gbuffer: false,
        pick_only: false,
        after_prepass: false,
        cull: true,
    };
    let bindless_group = renderer.texture_binder.global_bind_group().cloned();
    let mut recorder = BatchRecorder::new(
        renderer,
        assets,
        prepared_batches.materials(),
        prepared_batches.material_handles(),
        material_pipeline_keys,
    );
    let config = SurfacePassConfig::new(
        "ShaderMaterialPass",
        scene_view,
        shader_batches.as_slice(),
        &mut recorder,
        options,
    )
    .resolve(scene_resolve_view)
    .depth(Some(depth_view))
    .render_region(render_region)
    .bindless_group(bindless_group.as_ref());

    run_surface_pass(encoder, config)
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
    scene_view: &wgpu::TextureView,
    scene_resolve_view: Option<&wgpu::TextureView>,
    depth_view: &wgpu::TextureView,
    render_region: Option<RenderRegion>,
    scene_format: wgpu::TextureFormat,
) -> u32 {
    let batches = prepared_batches.transparent();
    let materials = prepared_batches.materials();
    let material_handles = prepared_batches.material_handles();
    let material_pipeline_keys = prepared_batches.material_pipeline_keys();

    let sample_count = renderer.context.sample_count;
    let bindless_group = renderer.texture_binder.global_bind_group().cloned();
    let mut recorder = BatchRecorder::new(
        renderer,
        assets,
        materials,
        material_handles,
        material_pipeline_keys,
    );
    let options = BatchPassOptions {
        stable_camera: false,
        color_format: scene_format,
        color_sample_count: sample_count,
        use_gbuffer: false,
        pick_only: false,
        after_prepass: false,
        cull: true,
    };
    let config = SurfacePassConfig::new(
        "TransparentPass",
        scene_view,
        batches,
        &mut recorder,
        options,
    )
    .resolve(scene_resolve_view)
    .depth(Some(depth_view))
    .render_region(render_region)
    .bindless_group(bindless_group.as_ref());

    run_surface_pass(encoder, config)
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

    let bindless_group = renderer.texture_binder.global_bind_group().cloned();
    let mut recorder = BatchRecorder::new(
        renderer,
        assets,
        materials,
        material_handles,
        material_pipeline_keys,
    );
    let options = BatchPassOptions {
        stable_camera: true,
        color_format: surface_format,
        color_sample_count: 1,
        use_gbuffer: false,
        pick_only: false,
        after_prepass: false,
        cull: false,
    };
    let config =
        SurfacePassConfig::new("OverlayPass", surface_view, batches, &mut recorder, options)
            .depth(overlay_needs_depth.then_some(depth_view))
            .render_region(render_region)
            .bindless_group(bindless_group.as_ref());

    run_surface_pass(encoder, config)
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
    surface_format: wgpu::TextureFormat,
    label: &'static str,
) -> u32 {
    if batches.is_empty() {
        return 0;
    }

    let bindless_group = renderer.texture_binder.global_bind_group().cloned();
    let mut recorder = BatchRecorder::new(
        renderer,
        assets,
        materials,
        material_handles,
        material_pipeline_keys,
    );
    let options = BatchPassOptions {
        stable_camera: true,
        color_format: surface_format,
        color_sample_count: 1,
        use_gbuffer: false,
        pick_only: false,
        after_prepass: false,
        cull: false,
    };
    let config = SurfacePassConfig::new(label, surface_view, batches, &mut recorder, options)
        .render_region(render_region)
        .bindless_group(bindless_group.as_ref());

    run_surface_pass(encoder, config)
}

/// Entity ids for a pending pick: every pickable batch, in the order and with the depth
/// tests it is drawn with (opaque, transparent, overlays, then gizmos on top), into the
/// single-sample pick target at output resolution through the unjittered camera. Then
/// the requested texel is copied for reading back. Nothing happens without a request.
pub(crate) fn pick_pass(
    renderer: &mut Renderer,
    assets: &Assets,
    prepared_batches: &PreparedBatches,
    encoder: &mut wgpu::CommandEncoder,
    render_region: Option<RenderRegion>,
) -> u32 {
    if !renderer.pick_state.has_request() {
        return 0;
    }
    let device = renderer.context.device.clone();
    let (width, height) = (
        renderer.context.config.width,
        renderer.context.config.height,
    );
    let (view, depth_view) = {
        let target = renderer.pick_state.target(&device, width, height);
        (target.view.clone(), target.depth_view.clone())
    };

    let plan = PassPlan::new("PickPass")
        .add_color(FrameGraphColorAttachment::clear(
            &view,
            None,
            wgpu::Color::TRANSPARENT,
        ))
        .set_depth(FrameGraphDepthAttachment::clear(&depth_view, 1.0));
    let options = BatchPassOptions {
        stable_camera: true,
        color_format: crate::renderer::postprocess::GBUFFER_PICK_FORMAT,
        color_sample_count: 1,
        use_gbuffer: false,
        pick_only: true,
        after_prepass: false,
        cull: false,
    };
    let bindless_group = renderer.texture_binder.global_bind_group().cloned();
    let mut draw_calls = 0;
    {
        let mut recorder = BatchRecorder::new(
            renderer,
            assets,
            prepared_batches.materials(),
            prepared_batches.material_handles(),
            prepared_batches.material_pipeline_keys(),
        );
        let mut frame_graph = FrameGraph::new(encoder);
        frame_graph.execute_pass(plan, render_region, |pass| {
            for batches in [
                prepared_batches.opaque(),
                prepared_batches.transparent(),
                prepared_batches.overlay(),
                prepared_batches.gizmos(),
                prepared_batches.gizmo_solids(),
            ] {
                // Batches without a pickable instance (pick id 0, such as the selection
                // outline) must not hide what is under them.
                let pickable: Vec<OrderedBatch> = batches
                    .iter()
                    .filter(|batch| batch.instances.iter().any(|instance| instance.pick_id != 0))
                    .cloned()
                    .collect();
                draw_calls += recorder.record(pass, &pickable, options, bindless_group.as_ref());
            }
        });
    }
    renderer.pick_state.copy_request(&device, encoder);
    draw_calls
}

pub(crate) fn finish_frame(
    renderer: &mut Renderer,
    encoder: wgpu::CommandEncoder,
    frame: wgpu::SurfaceTexture,
) -> RenderFrame {
    let index = renderer.context.queue.submit(Some(encoder.finish()));
    renderer.frames_in_flight.push_back(index);
    renderer.gpu_timer.after_submit();
    renderer.pick_state.after_submit();

    RenderFrame {
        frame,
        output_resolved: std::cell::Cell::new(false),
    }
}

/// Where batches get their geometry: the mesh assets plus this frame's skinned copies.
pub(crate) struct BatchGeometry<'a> {
    pub(crate) assets: &'a Assets,
    pub(crate) skinning: &'a crate::renderer::skinning::SkinningStage,
}

impl BatchGeometry<'_> {
    pub(crate) fn mesh(&self, batch: &OrderedBatch) -> Option<Mesh> {
        mesh_for_batch(self.assets, self.skinning, batch)
    }
}

/// The geometry a batch draws: a skinned instance's deformed copy, or the mesh itself
/// (also the fallback when skinning was skipped, which draws the bind pose).
pub(crate) fn mesh_for_batch(
    assets: &Assets,
    skinning: &crate::renderer::skinning::SkinningStage,
    batch: &OrderedBatch,
) -> Option<Mesh> {
    if let Some(skinned) = batch.skin.and_then(|key| skinning.mesh(key)) {
        return Some(skinned.clone());
    }
    let mesh = assets.meshes.get(batch.mesh);
    if mesh.is_none() {
        log::warn!("Skipping batch with invalid mesh handle");
    }
    mesh.cloned()
}

/// Consecutive instances from `start` that share its material and are visible (`visible`
/// as `OrderedBatch::camera_visible`), so they can go in one draw.
pub(crate) fn material_run_length(
    instances: &[InstanceData],
    visible: &[bool],
    start: usize,
) -> usize {
    let material = instances[start].material_index;
    let mut length = 1usize;
    while start + length < instances.len()
        && instances[start + length].material_index == material
        && instance_visible(visible, start + length)
    {
        length += 1;
    }
    length
}

/// Consecutive visible instances from `start` (of `count`).
pub(crate) fn visible_run_length(visible: &[bool], count: usize, start: usize) -> usize {
    let mut length = 1usize;
    while start + length < count && instance_visible(visible, start + length) {
        length += 1;
    }
    length
}
