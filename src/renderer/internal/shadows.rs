use crate::renderer::shader_source::wgsl;
use std::mem;
use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};
use glam::Mat4;

use crate::renderer::batch::{CullMode, InstanceSource};
use crate::renderer::internal::culling::{instance_visible, Frustum, Visibility};
use crate::renderer::internal::{
    DynamicObjectsBuffer, OrderedBatch, PreparedBatches, RenderContext,
};
use crate::renderer::lights::{
    LightsData, MAX_DIRECTIONAL_LIGHTS, MAX_POINT_LIGHTS, MAX_SPOT_LIGHTS,
};
use crate::renderer::{PipelineBuilder, RenderPass, ShaderBuilder, ShadowPassStage, Vertex};

const POINT_SHADOW_FACE_COUNT: usize = 6;
const POINT_SHADOW_LAYERS: u32 = (MAX_POINT_LIGHTS * POINT_SHADOW_FACE_COUNT) as u32;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ShadowViewUniform {
    view_proj: [[f32; 4]; 4],
}

struct ShadowArray {
    // CRITICAL: views must drop before texture
    array_view: wgpu::TextureView,
    layer_views: Vec<wgpu::TextureView>,
    texture: wgpu::Texture,
}

/// Material textures for the alpha test of cut-out surfaces (shadow maps, depth
/// prepass): the one bindless group, or a group per material index (classic binding).
pub(crate) enum MaskTextures {
    Bindless(wgpu::BindGroup),
    PerMaterial(std::collections::HashMap<u32, wgpu::BindGroup>),
}

impl MaskTextures {
    pub(crate) fn group(&self, material_index: u32) -> Option<&wgpu::BindGroup> {
        match self {
            Self::Bindless(group) => Some(group),
            Self::PerMaterial(groups) => groups.get(&material_index),
        }
    }
}

#[derive(Clone)]
pub(crate) struct ShadowInvocation {
    pub stage: ShadowPassStage,
    pub view: wgpu::TextureView,
    pub view_proj: Mat4,
}

impl ShadowArray {
    fn new(device: &wgpu::Device, label: &str, layers: u32, size: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: layers.max(1),
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let array_view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some(&format!("{label}ArrayView")),
            format: Some(wgpu::TextureFormat::Depth32Float),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            aspect: wgpu::TextureAspect::All,
            base_mip_level: 0,
            mip_level_count: None,
            base_array_layer: 0,
            array_layer_count: Some(layers.max(1)),
            ..Default::default()
        });

        let mut layer_views = Vec::with_capacity(layers.max(1) as usize);
        for layer in 0..layers.max(1) {
            layer_views.push(texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some(&format!("{label}Layer{layer}")),
                format: Some(wgpu::TextureFormat::Depth32Float),
                dimension: Some(wgpu::TextureViewDimension::D2),
                aspect: wgpu::TextureAspect::All,
                base_mip_level: 0,
                mip_level_count: None,
                base_array_layer: layer,
                array_layer_count: Some(1),
                ..Default::default()
            }));
        }

        Self {
            texture,
            array_view,
            layer_views,
        }
    }

    fn layer_view(&self, index: usize) -> &wgpu::TextureView {
        let clamped = index.min(self.layer_views.len().saturating_sub(1));
        if clamped != index {
            log::warn!(
                "Shadow layer index {} clamped to {} (max: {})",
                index,
                clamped,
                self.layer_views.len() - 1
            );
        }
        &self.layer_views[clamped]
    }

    fn array_view(&self) -> &wgpu::TextureView {
        &self.array_view
    }
}

/// Shadow views a frame can render: one per directional and spot light, six per point
/// light.
const MAX_SHADOW_VIEWS: usize =
    MAX_DIRECTIONAL_LIGHTS + MAX_SPOT_LIGHTS + MAX_POINT_LIGHTS * POINT_SHADOW_FACE_COUNT;

/// What the shadow pass did this frame.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ShadowPassOutcome {
    pub draw_calls: u32,
    /// Views whose map was rendered this frame.
    pub views_rendered: u32,
    /// Views whose map was kept from an earlier frame (nothing it shows changed).
    pub views_cached: u32,
}

/// What each light's map holds: a hash of the inputs it was rendered from (the view
/// projection and every caster), or `None` when its content is unknown.
#[derive(Default)]
struct ShadowCache {
    directional: [Option<u64>; MAX_DIRECTIONAL_LIGHTS],
    spot: [Option<u64>; MAX_SPOT_LIGHTS],
    point: [Option<u64>; MAX_POINT_LIGHTS],
}

pub(crate) struct ShadowResources {
    directional: ShadowArray,
    spot: ShadowArray,
    point: ShadowArray,
    sampler: wgpu::Sampler,
    /// The view projection of every view rendered this frame, `view_stride` bytes apart,
    /// bound at a dynamic offset.
    view_buffer: wgpu::Buffer,
    view_stride: u32,
    view_scratch: Vec<u8>,
    uniform_bind_group: wgpu::BindGroup,
    uniform_layout: wgpu::BindGroupLayout,
    /// Culling back faces ([0]), or both sides for double-sided materials ([1]).
    pipeline: [wgpu::RenderPipeline; 2],
    /// Cut-out surfaces' pipelines (with the alpha test), by sidedness like `pipeline`;
    /// `None` until [`Self::enable_alpha_masking`], and they then cast solid shadows.
    masked_pipeline: Option<[wgpu::RenderPipeline; 2]>,
    cache: ShadowCache,
    /// Scratch for culling each view.
    visibility: Visibility,
}

impl ShadowResources {
    /// The shadow maps, for reading back (see [`crate::renderer::readback`]).
    pub(crate) fn fetchable_textures<'a>(
        &'a self,
        out: &mut Vec<crate::renderer::readback::FetchableTexture<'a>>,
    ) {
        use crate::renderer::readback::FetchableTexture;
        out.push(FetchableTexture::new(
            "shadows.directional",
            "Directional shadow maps (light-space depth, 0 near to 1 far): one array layer per \
             directional light; layers are unused while ray-traced shadows replace them",
            &self.directional.texture,
        ));
        out.push(FetchableTexture::new(
            "shadows.spot",
            "Spot light shadow maps (light-space depth): one array layer per spot light",
            &self.spot.texture,
        ));
        out.push(FetchableTexture::new(
            "shadows.point",
            "Point light shadow maps (light-space depth): six array layers (cube faces) per point light",
            &self.point.texture,
        ));
    }

    pub(crate) fn new(
        device: &wgpu::Device,
        objects: &DynamicObjectsBuffer,
        shadow_map_size: u32,
    ) -> Self {
        let directional = ShadowArray::new(
            device,
            "DirectionalShadowMap",
            MAX_DIRECTIONAL_LIGHTS as u32,
            shadow_map_size,
        );
        let spot = ShadowArray::new(
            device,
            "SpotShadowMap",
            MAX_SPOT_LIGHTS as u32,
            shadow_map_size,
        );
        let point = ShadowArray::new(
            device,
            "PointShadowMap",
            POINT_SHADOW_LAYERS,
            shadow_map_size,
        );

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ShadowSampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            lod_min_clamp: 0.0,
            lod_max_clamp: 1.0,
            ..Default::default()
        });

        let uniform_size = mem::size_of::<ShadowViewUniform>() as u64;
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ShadowUniformLayout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: Some(NonZeroU64::new(uniform_size).unwrap()),
                },
                count: None,
            }],
        });

        // One slot per view, each at the alignment dynamic offsets need.
        let view_stride = (device.limits().min_uniform_buffer_offset_alignment as u64)
            .max(uniform_size)
            .next_multiple_of(uniform_size) as u32;
        let view_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ShadowViewBuffer"),
            size: view_stride as u64 * MAX_SHADOW_VIEWS as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ShadowUniformBindGroup"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &view_buffer,
                    offset: 0,
                    size: Some(NonZeroU64::new(uniform_size).unwrap()),
                }),
            }],
        });

        let shader_source = shadow_shader_source(None);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ShadowShader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ShadowPipelineLayout"),
            bind_group_layouts: &[Some(&uniform_layout), Some(&objects.bind_layout)],
            immediate_size: 0,
        });

        let pipeline = [false, true].map(|double_sided| {
            PipelineBuilder::new(device, &pipeline_layout, &shader)
                .with_label(if double_sided {
                    "ShadowDoubleSidedPipeline"
                } else {
                    "ShadowPipeline"
                })
                .with_vertex_entry("vs_main")
                .depth_only() // No fragment shader for shadow pass
                .with_vertex_buffer(Vertex::layout())
                .with_depth_stencil_biased(
                    wgpu::TextureFormat::Depth32Float,
                    true,
                    wgpu::CompareFunction::LessEqual,
                    2,   // constant bias
                    2.0, // slope bias
                )
                .with_cull_mode((!double_sided).then_some(wgpu::Face::Back))
                .build()
        });

        Self {
            directional,
            spot,
            point,
            sampler,
            view_buffer,
            view_stride,
            view_scratch: Vec::with_capacity(view_stride as usize * MAX_SHADOW_VIEWS),
            uniform_bind_group,
            uniform_layout,
            pipeline,
            masked_pipeline: None,
            cache: ShadowCache::default(),
            visibility: Visibility::default(),
        }
    }

    /// Build the pipeline that alpha-tests cut-out surfaces, for material textures bound
    /// as group 3 with `textures_layout` (bindless or classic, as `bindless` says).
    pub(crate) fn enable_alpha_masking(
        &mut self,
        device: &wgpu::Device,
        objects: &DynamicObjectsBuffer,
        textures_layout: &wgpu::BindGroupLayout,
        bindless: bool,
    ) {
        let source = shadow_shader_source(Some(bindless));
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ShadowMaskedShader"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ShadowMaskedPipelineLayout"),
            bind_group_layouts: &[
                Some(&self.uniform_layout),
                Some(&objects.bind_layout),
                None,
                Some(textures_layout),
            ],
            immediate_size: 0,
        });
        self.masked_pipeline = Some([false, true].map(|double_sided| {
            PipelineBuilder::new(device, &layout, &shader)
                .with_label(if double_sided {
                    "ShadowMaskedDoubleSidedPipeline"
                } else {
                    "ShadowMaskedPipeline"
                })
                .with_vertex_entry("vs_masked")
                .with_fragment_entry("fs_masked")
                .with_vertex_buffer(Vertex::layout())
                .with_depth_stencil_biased(
                    wgpu::TextureFormat::Depth32Float,
                    true,
                    wgpu::CompareFunction::LessEqual,
                    2,
                    2.0,
                )
                .with_cull_mode((!double_sided).then_some(wgpu::Face::Back))
                .build()
        }));
        // The maps were drawn without the alpha test.
        self.cache = ShadowCache::default();
    }

    /// Width (and height) in texels of the directional shadow maps.
    pub(crate) fn directional_map_size(&self) -> u32 {
        self.directional.texture.width()
    }

    pub(crate) fn directional_array_view(&self) -> &wgpu::TextureView {
        self.directional.array_view()
    }

    pub(crate) fn spot_array_view(&self) -> &wgpu::TextureView {
        self.spot.array_view()
    }

    pub(crate) fn point_array_view(&self) -> &wgpu::TextureView {
        self.point.array_view()
    }

    pub(crate) fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }

    /// Render the shadow maps of `lights`' shadow-casting lights, except those in `skip`
    /// (bits as the ray-traced shadow mask: 0-3 directional, 4-7 point, 8-11 spot),
    /// whose maps nothing reads this frame. A map whose inputs (the light's view and
    /// every caster's geometry, transform and cut-out textures, by `texture_generation`)
    /// are the same as when it was last rendered is kept as it is, unless custom passes
    /// draw into the maps too (`record_custom_passes`), which also lists each rendered
    /// view in `custom_invocations`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
        &mut self,
        context: &RenderContext,
        encoder: &mut wgpu::CommandEncoder,
        geometry: &crate::renderer::renderer_core::passes::BatchGeometry<'_>,
        batches: &PreparedBatches,
        lights: &LightsData,
        objects: &DynamicObjectsBuffer,
        record_custom_passes: bool,
        custom_invocations: &mut Vec<ShadowInvocation>,
        skip: u32,
        mask_textures: Option<&MaskTextures>,
        texture_generation: u64,
    ) -> ShadowPassOutcome {
        let mut outcome = ShadowPassOutcome::default();
        // Lights whose map nothing reads this frame (bits as the ray-traced shadow mask:
        // 0-3 directional, 4-7 point, 8-11 spot).
        let skipped = |bit: usize| skip & (1 << bit) != 0;
        let casters = if record_custom_passes {
            None
        } else {
            casters_signature(
                batches,
                |batch| {
                    geometry
                        .mesh(batch)
                        .map(|mesh| std::sync::Arc::as_ptr(mesh.shared_data()) as usize as u64)
                },
                texture_generation,
            )
        };

        // The views to render, each with the matrices of its faces and where its cache
        // entry is.
        struct View {
            kind: ViewKind,
            index: usize,
            key: Option<u64>,
        }
        enum ViewKind {
            Directional,
            Spot,
            Point,
        }
        let mut views: Vec<View> = Vec::new();
        for (index, shadow) in lights
            .directional_shadows()
            .iter()
            .enumerate()
            .take(MAX_DIRECTIONAL_LIGHTS)
        {
            if shadow.params[0] == 0.0 {
                continue;
            }
            if skipped(index) {
                self.cache.directional[index] = None;
                continue;
            }
            let key = casters.map(|casters| view_key(casters, &[shadow.view_proj]));
            if key.is_some() && self.cache.directional[index] == key {
                outcome.views_cached += 1;
                continue;
            }
            views.push(View {
                kind: ViewKind::Directional,
                index,
                key,
            });
        }
        for (index, shadow) in lights
            .spot_shadows()
            .iter()
            .enumerate()
            .take(MAX_SPOT_LIGHTS)
        {
            if shadow.params[0] == 0.0 {
                continue;
            }
            if skipped(8 + index) {
                self.cache.spot[index] = None;
                continue;
            }
            let key = casters.map(|casters| view_key(casters, &[shadow.view_proj]));
            if key.is_some() && self.cache.spot[index] == key {
                outcome.views_cached += 1;
                continue;
            }
            views.push(View {
                kind: ViewKind::Spot,
                index,
                key,
            });
        }
        for (index, shadow) in lights
            .point_shadows()
            .iter()
            .enumerate()
            .take(MAX_POINT_LIGHTS)
        {
            if shadow.params[0] == 0.0 {
                continue;
            }
            if skipped(4 + index) {
                self.cache.point[index] = None;
                continue;
            }
            let key = casters.map(|casters| view_key(casters, &shadow.view_proj));
            if key.is_some() && self.cache.point[index] == key {
                outcome.views_cached += POINT_SHADOW_FACE_COUNT as u32;
                continue;
            }
            views.push(View {
                kind: ViewKind::Point,
                index,
                key,
            });
        }
        if views.is_empty() {
            return outcome;
        }

        // Every rendered face's view projection, one slot each, uploaded at once.
        let stride = self.view_stride as usize;
        self.view_scratch.clear();
        let mut faces: Vec<(usize, usize, Mat4, ShadowPassStage)> = Vec::new();
        for (view_index, view) in views.iter().enumerate() {
            let matrices: Vec<(Mat4, ShadowPassStage)> = match view.kind {
                ViewKind::Directional => {
                    let shadow = &lights.directional_shadows()[view.index];
                    vec![(
                        Mat4::from_cols_array_2d(&shadow.view_proj),
                        ShadowPassStage::Directional {
                            light_index: view.index as u32,
                        },
                    )]
                }
                ViewKind::Spot => {
                    let shadow = &lights.spot_shadows()[view.index];
                    vec![(
                        Mat4::from_cols_array_2d(&shadow.view_proj),
                        ShadowPassStage::Spot {
                            light_index: view.index as u32,
                        },
                    )]
                }
                ViewKind::Point => {
                    let shadow = &lights.point_shadows()[view.index];
                    (0..POINT_SHADOW_FACE_COUNT)
                        .map(|face| {
                            (
                                Mat4::from_cols_array_2d(&shadow.view_proj[face]),
                                ShadowPassStage::Point {
                                    light_index: view.index as u32,
                                    face_index: face as u32,
                                },
                            )
                        })
                        .collect()
                }
            };
            for (matrix, stage) in matrices {
                let slot = faces.len();
                let uniform = ShadowViewUniform {
                    view_proj: matrix.to_cols_array_2d(),
                };
                self.view_scratch.resize((slot + 1) * stride, 0);
                self.view_scratch
                    [slot * stride..slot * stride + mem::size_of::<ShadowViewUniform>()]
                    .copy_from_slice(bytemuck::bytes_of(&uniform));
                faces.push((view_index, slot, matrix, stage));
            }
        }
        context
            .queue
            .write_buffer(&self.view_buffer, 0, &self.view_scratch);

        let mut visibility = std::mem::take(&mut self.visibility);
        for (view_index, slot, matrix, stage) in faces {
            let view = &views[view_index];
            let layer = match stage {
                ShadowPassStage::Directional { light_index } => {
                    self.directional.layer_view(light_index as usize)
                }
                ShadowPassStage::Spot { light_index } => self.spot.layer_view(light_index as usize),
                ShadowPassStage::Point {
                    light_index,
                    face_index,
                } => self.point.layer_view(
                    light_index as usize * POINT_SHADOW_FACE_COUNT + face_index as usize,
                ),
            };
            visibility.compute(batches.bounds(), &Frustum::from_view_proj(matrix));
            outcome.draw_calls += self.render_pass(
                encoder,
                layer,
                geometry,
                batches,
                objects,
                mask_textures,
                (slot * stride) as u32,
                &visibility,
            );
            outcome.views_rendered += 1;
            if record_custom_passes {
                custom_invocations.push(ShadowInvocation {
                    stage,
                    view: layer.clone(),
                    view_proj: matrix,
                });
            }
            match view.kind {
                ViewKind::Directional => self.cache.directional[view.index] = view.key,
                ViewKind::Spot => self.cache.spot[view.index] = view.key,
                ViewKind::Point => self.cache.point[view.index] = view.key,
            }
        }
        self.visibility = visibility;
        outcome
    }

    /// Draw the casters `visibility` keeps into one shadow map layer. Returns the draws.
    #[allow(clippy::too_many_arguments)]
    fn render_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        geometry: &crate::renderer::renderer_core::passes::BatchGeometry<'_>,
        batches: &PreparedBatches,
        objects: &DynamicObjectsBuffer,
        mask_textures: Option<&MaskTextures>,
        uniform_offset: u32,
        visibility: &Visibility,
    ) -> u32 {
        let materials = batches.materials();
        let mut draws = 0u32;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ShadowPass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view,
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

        pass.set_bind_group(0, &self.uniform_bind_group, &[uniform_offset]);
        pass.set_bind_group(1, &objects.bind_group, &[]);
        let mut binder = crate::renderer::renderer_core::passes::GeometryBinder::default();

        for (batch_index, batch) in batches.all().iter().enumerate() {
            if !casts_shadows(batch.pass) || !visibility.batch(batch_index) {
                continue;
            }
            let Some(mesh) = geometry.mesh(batch) else {
                continue;
            };
            let visible = visibility.instances(batch_index);
            // Cut-out surfaces alpha-test with their own pipeline, one draw per material;
            // double-sided ones cast shadows from both sides.
            let sides = usize::from(batch.cull_mode == CullMode::None);
            let masked = match (&self.masked_pipeline, mask_textures) {
                (Some(pipelines), Some(textures)) if batch.alpha_masked => {
                    Some((&pipelines[sides], textures))
                }
                _ => None,
            };
            pass.set_pipeline(masked.map_or(&self.pipeline[sides], |(pipeline, _)| pipeline));

            let instance_count = batch.instances.len() as u32;
            binder.bind(&mut pass, mesh, None);
            let base_vertex = mesh.base_vertex() as i32;
            let mut current_range_start: Option<u32> = None;
            let mut current_material: Option<u32> = None;

            for (local_index, instance) in batch.instances.iter().enumerate() {
                let global_index = batch.first_instance + local_index as u32;
                let material_index = instance.material_index as usize;
                let Some(material) = materials.get(material_index) else {
                    log::warn!(
                        "Material index {} out of bounds during shadow rendering ({} materials)",
                        material_index,
                        materials.len()
                    );
                    if let Some(start) = current_range_start.take() {
                        pass.draw_indexed(0..mesh.index_count(), base_vertex, start..global_index);
                        draws += 1;
                    }
                    continue;
                };
                // A masked batch draws each material with its own textures.
                let new_material = masked.is_some()
                    && current_material.replace(instance.material_index)
                        != Some(instance.material_index);
                if new_material {
                    if let Some(start) = current_range_start.take() {
                        pass.draw_indexed(0..mesh.index_count(), base_vertex, start..global_index);
                        draws += 1;
                    }
                }
                if !material.casts_shadow() || !instance_visible(visible, local_index) {
                    if let Some(start) = current_range_start.take() {
                        pass.draw_indexed(0..mesh.index_count(), base_vertex, start..global_index);
                        draws += 1;
                    }
                } else if current_range_start.is_none() {
                    if let Some((_, textures)) = masked {
                        let Some(group) = textures.group(instance.material_index) else {
                            continue;
                        };
                        pass.set_bind_group(3, group, &[]);
                    }
                    current_range_start = Some(global_index);
                }
            }

            if let Some(start) = current_range_start.take() {
                pass.draw_indexed(
                    0..mesh.index_count(),
                    base_vertex,
                    start..(batch.first_instance + instance_count),
                );
                draws += 1;
            }
        }
        draws
    }
}

/// Whether a pass's batches cast shadows.
fn casts_shadows(pass: RenderPass) -> bool {
    !matches!(
        pass,
        RenderPass::Transparent | RenderPass::Overlay | RenderPass::Gizmo | RenderPass::GizmoSolid
    )
}

/// One step of a fast, order-sensitive 64-bit mix (as FxHash).
fn mix(hash: u64, value: u64) -> u64 {
    (hash.rotate_left(5) ^ value).wrapping_mul(0x517c_c1b7_2722_0a95)
}

fn mix_floats(mut hash: u64, values: &[f32]) -> u64 {
    for value in values {
        hash = mix(hash, u64::from(value.to_bits()));
    }
    hash
}

/// A hash of everything that decides what the shadow casters look like from any view:
/// each instance's geometry (`mesh_identity`, None for a batch with no mesh to draw),
/// transform, whether it casts, and for cut-outs its material and the texture set's
/// generation. Order-independent, so reordered batches do not count as a change. `None`
/// when the casters move every frame (skinned meshes, GPU-placed instances), in which
/// case the maps are rendered every frame.
fn casters_signature(
    batches: &PreparedBatches,
    mesh_identity: impl Fn(&OrderedBatch) -> Option<u64>,
    texture_generation: u64,
) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let materials = batches.materials();
    let mut sum = 0u64;
    let mut count = 0u64;
    for batch in batches.all() {
        if !casts_shadows(batch.pass) {
            continue;
        }
        if batch.skin.is_some()
            || batch
                .instances
                .iter()
                .any(|instance| instance.source == InstanceSource::Gpu)
        {
            return None;
        }
        let Some(mesh_id) = mesh_identity(batch) else {
            continue;
        };
        let batch_hash = mix(
            mix(0x9e37_79b9_7f4a_7c15, mesh_id),
            u64::from(batch.alpha_masked),
        );
        for instance in &batch.instances {
            let Some(material) = materials.get(instance.material_index as usize) else {
                continue;
            };
            if !material.casts_shadow() {
                continue;
            }
            let transform = &instance.transform;
            let mut hash = mix_floats(batch_hash, &transform.translation.to_array());
            hash = mix_floats(hash, &transform.rotation.to_array());
            hash = mix_floats(hash, &transform.scale.to_array());
            if batch.alpha_masked {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                material.hash(&mut hasher);
                hash = mix(hash, hasher.finish());
            }
            // Spread the bits, since the sum below is commutative.
            hash ^= hash >> 29;
            hash = hash.wrapping_mul(0xbf58_476d_1ce4_e5b9);
            hash ^= hash >> 32;
            sum = sum.wrapping_add(hash);
            count += 1;
        }
    }
    Some(mix(mix(sum, count), texture_generation))
}

/// The cache key of a view: the casters and the view projections of its faces.
fn view_key(casters: u64, view_projs: &[[[f32; 4]; 4]]) -> u64 {
    let mut hash = casters;
    for matrix in view_projs {
        for column in matrix {
            hash = mix_floats(hash, column);
        }
    }
    hash
}

/// The shadow map shader; with `textures` (bindless or not) the cut-out entry points too
/// (shadow_masked.wgsl), with the material texture bindings (group 3).
fn shadow_shader_source(textures: Option<bool>) -> String {
    let builder = ShaderBuilder::new().with_objects();
    match textures {
        Some(bindless) => builder
            .with_constants()
            .with_bindings_for_filter(bindless, crate::renderer::SamplerFilterMode::Linear)
            .with_module(wgsl!("shadow.wgsl"))
            .build(wgsl!("shadow_masked.wgsl")),
        None => builder.build(wgsl!("shadow.wgsl")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::{Assets, Handle, MaterialAsset};
    use crate::renderer::batch::{CullMode, RenderBatcher, RenderObject};
    use crate::renderer::material::Material;
    use crate::scene::components::DepthState;
    use crate::scene::transform::Transform;
    use glam::Vec3;
    use std::path::PathBuf;

    fn prepared(objects: &[(usize, Vec3, InstanceSource)]) -> PreparedBatches {
        let mut batcher = RenderBatcher::new();
        let mut assets = Assets::new();
        let material = Material::white();
        let handle = assets
            .materials
            .insert(MaterialAsset::from_material(material, PathBuf::new()));
        for (index, (mesh, position, source)) in objects.iter().enumerate() {
            batcher
                .add(
                    RenderObject {
                        mesh: Handle::new(*mesh),
                        material: handle,
                        resolved_material: Some(material),
                        transform: Transform::from_translation(*position),
                        depth_state: DepthState::default(),
                        force_overlay: false,
                        render_pass: None,
                        instance_source: *source,
                        gpu_index: (*source == InstanceSource::Gpu).then_some(index as u32),
                        cull_mode: CullMode::Back,
                        pick_id: index as u64 + 1,
                        skin: None,
                    },
                    &assets,
                )
                .unwrap();
        }
        PreparedBatches::from_batcher(&batcher, Vec3::ZERO, &|_| None)
    }

    fn signature(objects: &[(usize, Vec3, InstanceSource)], generation: u64) -> Option<u64> {
        casters_signature(
            &prepared(objects),
            |batch| Some(batch.mesh.index() as u64),
            generation,
        )
    }

    #[test]
    fn caster_signature_changes_only_with_the_casters() {
        let cpu = InstanceSource::Cpu;
        let a = (1, Vec3::new(1.0, 0.0, 0.0), cpu);
        let b = (2, Vec3::new(0.0, 2.0, 0.0), cpu);
        let moved = (1, Vec3::new(1.5, 0.0, 0.0), cpu);
        let same = signature(&[a, b], 0);
        assert!(same.is_some());
        assert_eq!(signature(&[b, a], 0), same, "order does not matter");
        assert_ne!(signature(&[moved, b], 0), same, "a moved caster counts");
        assert_ne!(signature(&[a], 0), same, "a removed caster counts");
        assert_ne!(signature(&[a, b], 1), same, "new textures count");
        assert_eq!(
            signature(&[a, b, (3, Vec3::ZERO, InstanceSource::Gpu)], 0),
            None,
            "GPU-placed instances disable the cache"
        );
    }

    #[test]
    fn view_keys_differ_per_view() {
        let identity = glam::Mat4::IDENTITY.to_cols_array_2d();
        let moved = glam::Mat4::from_translation(Vec3::X).to_cols_array_2d();
        assert_eq!(view_key(7, &[identity]), view_key(7, &[identity]));
        assert_ne!(view_key(7, &[identity]), view_key(7, &[moved]));
        assert_ne!(view_key(7, &[identity]), view_key(8, &[identity]));
    }

    #[test]
    fn shadow_shaders_validate() {
        for textures in [None, Some(true), Some(false)] {
            let source = super::shadow_shader_source(textures);
            if let Err(error) = crate::renderer::shader_source::validate(&source) {
                panic!("{textures:?}: {error}");
            }
        }
    }
}
