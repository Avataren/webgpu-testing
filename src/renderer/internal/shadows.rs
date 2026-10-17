//! Shadow maps: a layer per directional light, and one atlas for every point and spot
//! light (tiles placed by `shadow_atlas.rs`).
//!
//! The directional maps follow the camera at `shadow_map_size`. A local light's faces
//! (six cube faces for a point light, one for a spot light) are square tiles of the atlas,
//! sized by how large the light looks on screen and by the shadow budget; each face is
//! rendered with a small border so the shaders' filter taps stay inside its tile.
//!
//! A face is kept from frame to frame in two parts: its static casters' depth in a
//! second atlas of the same layout, rendered again only when a static caster near it
//! changes (`shadow_casters.rs`), and the shadow map itself, rebuilt from it when dynamic
//! casters (characters, moving props) are in its view: a copy of the static depth, then
//! the dynamic casters drawn over it. Faces with nothing new are left as they are, and so
//! are faces the camera cannot see. At most `update_budget` faces are rebuilt a frame,
//! the highest-scoring lights' first and every face within a few frames of its last
//! update; the rest keep last frame's map. The static renders share one pass over the
//! static atlas and the rebuilds one over the atlas, each face in its own viewport.

use crate::renderer::shader_source::wgsl;
use std::collections::HashMap;
use std::mem;
use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

use crate::renderer::batch::{CullMode, InstanceSource};
use crate::renderer::internal::culling::{
    frustum_corners, instance_visible, Frustum, Sphere, Visibility,
};
use crate::renderer::internal::shadow_atlas::{
    ShadowAtlasPlanner, ShadowCamera, ShadowState, Tile, TILE_GUARD,
};
use crate::renderer::internal::shadow_casters::{casts_shadows, CasterTracker};
use crate::renderer::internal::{
    DynamicObjectsBuffer, OrderedBatch, PreparedBatches, RenderContext,
};
use crate::renderer::lights::{
    LightsData, LocalShadowRaw, MAX_DIRECTIONAL_LIGHTS, MAX_SHADOWED_LOCAL_LIGHTS,
    POINT_SHADOW_FACES,
};
use crate::renderer::{PipelineBuilder, ShaderBuilder, ShadowPassStage, Vertex};

const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

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
    PerMaterial(HashMap<u32, wgpu::BindGroup>),
}

impl MaskTextures {
    pub(crate) fn group(&self, material_index: u32) -> Option<&wgpu::BindGroup> {
        match self {
            Self::Bindless(group) => Some(group),
            Self::PerMaterial(groups) => groups.get(&material_index),
        }
    }
}

/// A shadow view rendered this frame, for custom shadow passes: draw into `view` (in
/// `viewport` of it, an atlas tile, when given) with `view_proj`.
#[derive(Clone)]
pub(crate) struct ShadowInvocation {
    pub stage: ShadowPassStage,
    pub view: wgpu::TextureView,
    pub view_proj: Mat4,
    pub viewport: Option<[u32; 4]>,
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
            format: SHADOW_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let array_view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some(&format!("{label}ArrayView")),
            format: Some(SHADOW_FORMAT),
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
                format: Some(SHADOW_FORMAT),
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
        &self.layer_views[clamped]
    }

    fn array_view(&self) -> &wgpu::TextureView {
        &self.array_view
    }
}

/// The local lights' shadow atlas texture.
struct AtlasTexture {
    view: wgpu::TextureView,
    texture: wgpu::Texture,
}

impl AtlasTexture {
    fn new(device: &wgpu::Device, label: &str, size: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size.max(1),
                height: size.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHADOW_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self { view, texture }
    }
}

/// Shadow views a frame can render: one per directional light and per local light face.
const MAX_SHADOW_VIEWS: usize =
    MAX_DIRECTIONAL_LIGHTS + MAX_SHADOWED_LOCAL_LIGHTS * POINT_SHADOW_FACES;

/// What the shadow pass did this frame.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ShadowPassOutcome {
    pub draw_calls: u32,
    /// Views whose map was rendered (or rebuilt from its static casters) this frame.
    pub views_rendered: u32,
    /// Views whose map was kept from an earlier frame (nothing it shows changed, or the
    /// camera cannot see it).
    pub views_cached: u32,
    /// Views that wanted an update but waited for a later frame (the update budget).
    pub views_deferred: u32,
}

/// Faces rebuilt per frame at most, unless more have never been rendered.
pub(crate) const DEFAULT_SHADOW_UPDATES_PER_FRAME: u32 = 48;
/// A face that waited this many frames goes ahead of the others.
const MAX_FACE_WAIT: u32 = 4;

/// What a local light's face holds, from frame to frame.
struct FaceState {
    /// What it was rendered for: its view projection and tile.
    view_proj: Mat4,
    tile: Tile,
    /// Its static casters' depth (in the static atlas) is up to date.
    static_ok: bool,
    /// Its shadow map has been rendered since it got this tile.
    rendered: bool,
    /// Its shadow map holds dynamic casters, which may have moved since.
    has_dynamic: bool,
    /// Frames it has waited for an update.
    waited: u32,
}

/// The local shadows a frame has: each shadowed light's slot in the shadows uniform and
/// the faces to render.
#[derive(Default)]
pub(crate) struct LocalShadowFrame {
    /// Per local light (as `LightsData::local_lights`), its slot.
    pub slots: Vec<Option<usize>>,
    /// The slots' entries of the shadows uniform.
    pub uniforms: Vec<LocalShadowRaw>,
    faces: Vec<LocalFace>,
}

/// One face of a shadowed local light.
struct LocalFace {
    key: u64,
    light: usize,
    face: usize,
    tile: Tile,
    view_proj: Mat4,
    /// The camera sees part of what the face covers (else it is left as it is).
    visible: bool,
    /// Its light's score in the shadow budget.
    score: f32,
}

/// The shadow atlas's figures for the stats.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ShadowAtlasStats {
    /// Texels across the atlas (0: none yet), and the share of it in use.
    pub size: u32,
    pub used: f32,
    /// Shadow-casting local lights with a shadow map, and those left without one.
    pub shadowed: u32,
    pub over_budget: u32,
}

pub(crate) struct ShadowResources {
    directional: ShadowArray,
    /// Every local light's shadow map tiles; a 1x1 texture until a light needs one.
    atlas: AtlasTexture,
    /// The same tiles holding only the static casters, which each frame's dynamic casters
    /// are drawn over.
    static_atlas: AtlasTexture,
    planner: ShadowAtlasPlanner,
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
    /// Clears an atlas tile to the far plane, and copies a tile of the static atlas into
    /// the atlas (with `copy_group` binding the static atlas).
    clear_pipeline: wgpu::RenderPipeline,
    copy_pipeline: wgpu::RenderPipeline,
    copy_layout: wgpu::BindGroupLayout,
    copy_group: wgpu::BindGroup,
    /// What each directional map holds: a hash of the inputs it was rendered from (the
    /// view projection and every caster), or nothing when its content is unknown.
    directional_cache: [Option<u64>; MAX_DIRECTIONAL_LIGHTS],
    /// Each local light face's state, by light key and face.
    faces: HashMap<(u64, usize), FaceState>,
    /// Which casters stay put.
    casters: CasterTracker,
    /// Faces rebuilt per frame at most.
    update_budget: u32,
    /// This frame's local shadows ([`Self::plan_local`]).
    local: LocalShadowFrame,
    /// Each shadow-casting local light's state in the last plan, by light key.
    states: HashMap<u64, ShadowState>,
    stats: ShadowAtlasStats,
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
            "shadows.atlas",
            "Point and spot light shadow maps (light-space depth): square tiles, six cube faces \
             per point light and one per spot light, sized by the shadow budget",
            &self.atlas.texture,
        ));
    }

    /// `shadow_map_size`: the directional maps' size and the largest local tile;
    /// `atlas_size`: the largest the local shadow atlas may grow to.
    pub(crate) fn new(
        device: &wgpu::Device,
        objects: &DynamicObjectsBuffer,
        shadow_map_size: u32,
        atlas_size: u32,
    ) -> Self {
        let directional = ShadowArray::new(
            device,
            "DirectionalShadowMap",
            MAX_DIRECTIONAL_LIGHTS as u32,
            shadow_map_size,
        );
        let atlas_size = atlas_size.min(device.limits().max_texture_dimension_2d);
        let planner = ShadowAtlasPlanner::new(atlas_size, shadow_map_size.min(atlas_size / 8));
        let atlas = AtlasTexture::new(device, "LocalShadowAtlas", 1);
        let static_atlas = AtlasTexture::new(device, "LocalShadowStaticAtlas", 1);

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
                    SHADOW_FORMAT,
                    true,
                    wgpu::CompareFunction::LessEqual,
                    2,   // constant bias
                    2.0, // slope bias
                )
                .with_cull_mode((!double_sided).then_some(wgpu::Face::Back))
                .build()
        });

        let tile_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ShadowTileShader"),
            source: wgpu::ShaderSource::Wgsl(wgsl!("shadow_tile.wgsl").into()),
        });
        let empty_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ShadowTileClearLayout"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let clear_pipeline = PipelineBuilder::new(device, &empty_layout, &tile_shader)
            .with_label("ShadowTileClearPipeline")
            .with_vertex_entry("vs_tile")
            .depth_only()
            .with_depth_stencil(SHADOW_FORMAT, true, wgpu::CompareFunction::Always)
            .with_no_culling()
            .build();
        let copy_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ShadowTileCopyLayout"),
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
        let copy_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ShadowTileCopyPipelineLayout"),
            bind_group_layouts: &[Some(&copy_layout)],
            immediate_size: 0,
        });
        let copy_pipeline = PipelineBuilder::new(device, &copy_pipeline_layout, &tile_shader)
            .with_label("ShadowTileCopyPipeline")
            .with_vertex_entry("vs_tile")
            .with_fragment_entry("fs_copy")
            .with_depth_stencil(SHADOW_FORMAT, true, wgpu::CompareFunction::Always)
            .with_no_culling()
            .build();
        let copy_group = copy_bind_group(device, &copy_layout, &static_atlas.view);

        Self {
            directional,
            atlas,
            static_atlas,
            planner,
            sampler,
            view_buffer,
            view_stride,
            view_scratch: Vec::with_capacity(view_stride as usize * MAX_SHADOW_VIEWS),
            uniform_bind_group,
            uniform_layout,
            pipeline,
            masked_pipeline: None,
            clear_pipeline,
            copy_pipeline,
            copy_layout,
            copy_group,
            directional_cache: [None; MAX_DIRECTIONAL_LIGHTS],
            faces: HashMap::new(),
            casters: CasterTracker::default(),
            update_budget: DEFAULT_SHADOW_UPDATES_PER_FRAME,
            local: LocalShadowFrame::default(),
            states: HashMap::new(),
            stats: ShadowAtlasStats::default(),
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
                    SHADOW_FORMAT,
                    true,
                    wgpu::CompareFunction::LessEqual,
                    2,
                    2.0,
                )
                .with_cull_mode((!double_sided).then_some(wgpu::Face::Back))
                .build()
        }));
        // The maps were drawn without the alpha test.
        self.directional_cache = [None; MAX_DIRECTIONAL_LIGHTS];
        self.faces.clear();
    }

    /// Width (and height) in texels of the directional shadow maps.
    pub(crate) fn directional_map_size(&self) -> u32 {
        self.directional.texture.width()
    }

    pub(crate) fn directional_array_view(&self) -> &wgpu::TextureView {
        self.directional.array_view()
    }

    pub(crate) fn atlas_view(&self) -> &wgpu::TextureView {
        &self.atlas.view
    }

    pub(crate) fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }

    /// The local shadows planned for this frame.
    pub(crate) fn local_frame(&self) -> &LocalShadowFrame {
        &self.local
    }

    /// The shadow state of the local light with key `key` in the last frame, if it casts
    /// shadows.
    pub(crate) fn light_state(&self, key: u64) -> Option<ShadowState> {
        self.states.get(&key).copied()
    }

    pub(crate) fn atlas_stats(&self) -> ShadowAtlasStats {
        self.stats
    }

    /// The largest the shadow atlas grows to, in texels across.
    pub(crate) fn max_atlas_size(&self) -> u32 {
        self.planner.max_size()
    }

    /// Let the shadow atlas grow to `size` texels across (a power of two, within the
    /// device's limit) from the next frame; a smaller size than the atlas has starts it
    /// over.
    pub(crate) fn set_max_atlas_size(&mut self, device: &wgpu::Device, size: u32) {
        let size = size
            .clamp(
                super::shadow_atlas::MIN_ATLAS,
                device.limits().max_texture_dimension_2d,
            )
            .next_power_of_two()
            .min(device.limits().max_texture_dimension_2d);
        if size != self.planner.max_size() {
            let max_tile = self.directional_map_size().min(size / 8);
            self.planner = ShadowAtlasPlanner::new(size, max_tile);
            self.faces.clear();
        }
    }

    /// Rebuild at most `faces` local light faces a frame.
    pub(crate) fn set_update_budget(&mut self, faces: u32) {
        self.update_budget = faces.max(1);
    }

    pub(crate) fn update_budget(&self) -> u32 {
        self.update_budget
    }

    /// Decide which local lights have shadow maps this frame and where (see
    /// `shadow_atlas.rs`); lights flagged in `skip` take theirs from the ray-traced
    /// mask. Returns whether the atlas texture was recreated (whoever binds it must
    /// rebind).
    pub(crate) fn plan_local(
        &mut self,
        device: &wgpu::Device,
        lights: &LightsData,
        camera: &ShadowCamera,
        camera_corners: &[Vec3; 8],
        skip: &[bool],
    ) -> bool {
        let local_lights = lights.local_lights();
        let plan = self.planner.plan(local_lights, camera, skip);
        let mut recreated = false;
        if plan.atlas_changed || self.atlas.texture.width() != self.planner.size.max(1) {
            self.atlas = AtlasTexture::new(device, "LocalShadowAtlas", self.planner.size);
            self.static_atlas =
                AtlasTexture::new(device, "LocalShadowStaticAtlas", self.planner.size);
            self.copy_group = copy_bind_group(device, &self.copy_layout, &self.static_atlas.view);
            self.faces.clear();
            recreated = true;
        }

        // Faces of lights that gave their tiles up.
        let planner = &self.planner;
        self.faces
            .retain(|(key, _), _| planner.lights.contains_key(key));

        self.states.clear();
        for (light, state) in local_lights.iter().zip(&plan.states) {
            if let Some(state) = state {
                self.states.insert(light.key, *state);
            }
        }
        let atlas = self.planner.size.max(1);
        self.stats = ShadowAtlasStats {
            size: self.planner.size,
            used: plan.texels_used as f32 / (atlas as f32 * atlas as f32),
            shadowed: plan.shadowed.len() as u32,
            over_budget: plan.over_budget,
        };

        let frame = &mut self.local;
        frame.slots.clear();
        frame.slots.resize(local_lights.len(), None);
        frame.uniforms.clear();
        frame.faces.clear();
        for (&index, &score) in plan.shadowed.iter().zip(&plan.scores) {
            let light = &local_lights[index];
            let Some(tiles) = self.planner.lights.get(&light.key) else {
                continue;
            };
            let (matrices, tan_half) = light.shadow_view_projs(tiles.size, TILE_GUARD);
            let mut raw = LocalShadowRaw::zeroed();
            for (face, (matrix, tile)) in matrices.iter().zip(&tiles.tiles).enumerate() {
                raw.view_proj[face] = matrix.to_cols_array_2d();
                raw.rects[face] = tile.rect(atlas);
                let corners = frustum_corners(*matrix);
                let visible = !camera.frustum.excludes(&corners)
                    && !Frustum::from_view_proj(*matrix).excludes(camera_corners);
                frame.faces.push(LocalFace {
                    key: light.key,
                    light: index,
                    face,
                    tile: *tile,
                    view_proj: *matrix,
                    visible,
                    score,
                });
            }
            raw.params = [matrices.len() as f32, tiles.size as f32, tan_half, 0.0];
            frame.slots[index] = Some(frame.uniforms.len());
            frame.uniforms.push(raw);
        }
        recreated
    }

    /// Render the directional maps (except those in `skip`, bits as the ray-traced shadow
    /// mask's 0-3, whose maps nothing reads this frame) and update this frame's local
    /// faces ([`Self::plan_local`]; see the module comment). A directional map whose
    /// inputs (the light's view and every caster's geometry, transform and cut-out
    /// textures, by `texture_generation`) are the same as when it was last rendered is
    /// kept as it is. Custom passes that draw into the maps (`record_custom_passes`) count
    /// as dynamic casters in every view; each view updated is listed in
    /// `custom_invocations` for them.
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
        let mesh_identity = |batch: &OrderedBatch| {
            geometry
                .mesh(batch)
                .map(|mesh| std::sync::Arc::as_ptr(mesh.shared_data()) as usize as u64)
        };
        let casters = if record_custom_passes {
            None
        } else {
            casters_signature(batches, mesh_identity, texture_generation)
        };

        // Directional layers to render.
        let mut directional: Vec<(usize, Mat4, Option<u64>)> = Vec::new();
        for (index, shadow) in lights.directional_shadows().iter().enumerate() {
            if shadow.params[0] == 0.0 {
                continue;
            }
            if skip & (1 << index) != 0 {
                self.directional_cache[index] = None;
                continue;
            }
            let key = casters.map(|casters| view_key(casters, &[shadow.view_proj]));
            if key.is_some() && self.directional_cache[index] == key {
                outcome.views_cached += 1;
                continue;
            }
            directional.push((index, Mat4::from_cols_array_2d(&shadow.view_proj), key));
        }

        // Local faces to update: where static casters changed, faces lose their static
        // depth; faces with dynamic casters in view (or that held some) are rebuilt.
        let changes = self
            .casters
            .update(batches, mesh_identity, texture_generation);
        if changes.all_dirty {
            self.faces
                .values_mut()
                .for_each(|face| face.static_ok = false);
        } else if !changes.dirty.is_empty() {
            for face in self.faces.values_mut().filter(|face| face.static_ok) {
                let frustum = Frustum::from_view_proj(face.view_proj);
                if changes
                    .dirty
                    .iter()
                    .any(|sphere| frustum.intersects(*sphere))
                {
                    face.static_ok = false;
                }
            }
        }
        struct Job {
            face: usize,
            render_static: bool,
            dynamic: bool,
            priority: f32,
        }
        let mut jobs: Vec<Job> = Vec::new();
        let local_lights = lights.local_lights();
        for (index, face) in self.local.faces.iter().enumerate() {
            let state = self
                .faces
                .entry((face.key, face.face))
                .or_insert_with(|| FaceState {
                    view_proj: face.view_proj,
                    tile: face.tile,
                    static_ok: false,
                    rendered: false,
                    has_dynamic: false,
                    waited: 0,
                });
            if state.view_proj != face.view_proj || state.tile != face.tile {
                *state = FaceState {
                    view_proj: face.view_proj,
                    tile: face.tile,
                    static_ok: false,
                    rendered: false,
                    has_dynamic: false,
                    waited: 0,
                };
            }
            if !face.visible {
                outcome.views_cached += 1;
                continue;
            }
            let light = &local_lights[face.light];
            let reach = Sphere {
                center: light.position,
                radius: light.reach(),
            };
            let frustum = Frustum::from_view_proj(face.view_proj);
            let dynamic = record_custom_passes
                || changes
                    .dynamic_spheres
                    .iter()
                    .any(|sphere| sphere_meets(reach, *sphere) && frustum.intersects(*sphere));
            if state.static_ok && state.rendered && !dynamic && !state.has_dynamic {
                outcome.views_cached += 1;
                state.waited = 0;
                continue;
            }
            // Faces never rendered first (their tile holds nothing of theirs), then those
            // that waited long, then by their light's score and how long they waited.
            let priority = if !state.rendered {
                f32::INFINITY
            } else if state.waited >= MAX_FACE_WAIT {
                f32::MAX / 2.0 + state.waited as f32
            } else {
                face.score * (1.0 + 0.5 * state.waited as f32)
            };
            jobs.push(Job {
                face: index,
                render_static: !state.static_ok,
                dynamic,
                priority,
            });
        }
        jobs.sort_by(|a, b| b.priority.total_cmp(&a.priority));
        let forced = jobs.iter().filter(|job| job.priority.is_infinite()).count();
        let take = (self.update_budget as usize).max(forced).min(jobs.len());
        for job in &jobs[take..] {
            let face = &self.local.faces[job.face];
            if let Some(state) = self.faces.get_mut(&(face.key, face.face)) {
                state.waited += 1;
            }
            outcome.views_deferred += 1;
        }
        jobs.truncate(take);
        if directional.is_empty() && jobs.is_empty() {
            return outcome;
        }

        // Every view's projection, one slot each, uploaded at once: directional layers,
        // then faces.
        let stride = self.view_stride as usize;
        self.view_scratch.clear();
        let matrices = directional
            .iter()
            .map(|(_, matrix, _)| *matrix)
            .chain(jobs.iter().map(|job| self.local.faces[job.face].view_proj));
        for (slot, matrix) in matrices.enumerate() {
            let uniform = ShadowViewUniform {
                view_proj: matrix.to_cols_array_2d(),
            };
            self.view_scratch.resize((slot + 1) * stride, 0);
            self.view_scratch[slot * stride..slot * stride + mem::size_of::<ShadowViewUniform>()]
                .copy_from_slice(bytemuck::bytes_of(&uniform));
        }
        context
            .queue
            .write_buffer(&self.view_buffer, 0, &self.view_scratch);

        let mut visibility = std::mem::take(&mut self.visibility);
        // Directional layers, a pass each.
        for (slot, &(index, matrix, key)) in directional.iter().enumerate() {
            let layer = self.directional.layer_view(index);
            visibility.compute(batches.bounds(), &Frustum::from_view_proj(matrix));
            let mut pass = begin_shadow_pass(encoder, layer, true);
            outcome.draw_calls += self.draw_casters(
                &mut pass,
                geometry,
                batches,
                objects,
                mask_textures,
                (slot * stride) as u32,
                &visibility,
            );
            drop(pass);
            outcome.views_rendered += 1;
            if record_custom_passes {
                custom_invocations.push(ShadowInvocation {
                    stage: ShadowPassStage::Directional {
                        light_index: index as u32,
                    },
                    view: layer.clone(),
                    view_proj: matrix,
                    viewport: None,
                });
            }
            self.directional_cache[index] = key;
        }
        let first_face_slot = directional.len();
        let set_tile = |pass: &mut wgpu::RenderPass<'_>, tile: Tile| {
            pass.set_viewport(
                tile.x as f32,
                tile.y as f32,
                tile.size as f32,
                tile.size as f32,
                0.0,
                1.0,
            );
            pass.set_scissor_rect(tile.x, tile.y, tile.size, tile.size);
        };

        // Static casters of the faces that lost them, into the static atlas.
        if jobs.iter().any(|job| job.render_static) {
            let static_view = self.static_atlas.view.clone();
            let mut pass = begin_shadow_pass(encoder, &static_view, false);
            for (offset, job) in jobs.iter().enumerate() {
                if !job.render_static {
                    continue;
                }
                let face = &self.local.faces[job.face];
                set_tile(&mut pass, face.tile);
                pass.set_pipeline(&self.clear_pipeline);
                pass.draw(0..3, 0..1);
                visibility.compute_filtered(
                    batches,
                    &Frustum::from_view_proj(face.view_proj),
                    |batch, instance| !changes.is_dynamic(batch, instance),
                );
                outcome.draw_calls += self.draw_casters(
                    &mut pass,
                    geometry,
                    batches,
                    objects,
                    mask_textures,
                    ((first_face_slot + offset) * stride) as u32,
                    &visibility,
                );
            }
        }

        // The shadow maps: each face's static depth, then its dynamic casters.
        let atlas_view = self.atlas.view.clone();
        {
            let mut pass = begin_shadow_pass(encoder, &atlas_view, false);
            for (offset, job) in jobs.iter().enumerate() {
                let face = &self.local.faces[job.face];
                set_tile(&mut pass, face.tile);
                pass.set_pipeline(&self.copy_pipeline);
                pass.set_bind_group(0, &self.copy_group, &[]);
                pass.draw(0..3, 0..1);
                if job.dynamic {
                    visibility.compute_filtered(
                        batches,
                        &Frustum::from_view_proj(face.view_proj),
                        |batch, instance| changes.is_dynamic(batch, instance),
                    );
                    outcome.draw_calls += self.draw_casters(
                        &mut pass,
                        geometry,
                        batches,
                        objects,
                        mask_textures,
                        ((first_face_slot + offset) * stride) as u32,
                        &visibility,
                    );
                }
            }
        }
        for job in &jobs {
            let face = &self.local.faces[job.face];
            outcome.views_rendered += 1;
            if let Some(state) = self.faces.get_mut(&(face.key, face.face)) {
                state.static_ok = true;
                state.rendered = true;
                state.has_dynamic = job.dynamic;
                state.waited = 0;
            }
            if record_custom_passes {
                let spot = local_lights
                    .get(face.light)
                    .is_some_and(|light| light.cone.is_some());
                let tile = face.tile;
                custom_invocations.push(ShadowInvocation {
                    stage: if spot {
                        ShadowPassStage::Spot {
                            light_index: face.light as u32,
                        }
                    } else {
                        ShadowPassStage::Point {
                            light_index: face.light as u32,
                            face_index: face.face as u32,
                        }
                    },
                    view: atlas_view.clone(),
                    view_proj: face.view_proj,
                    viewport: Some([tile.x, tile.y, tile.size, tile.size]),
                });
            }
        }
        self.visibility = visibility;
        outcome
    }

    /// Draw the casters `visibility` keeps into the current shadow view. Returns the draws.
    #[allow(clippy::too_many_arguments)]
    fn draw_casters(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        geometry: &crate::renderer::renderer_core::passes::BatchGeometry<'_>,
        batches: &PreparedBatches,
        objects: &DynamicObjectsBuffer,
        mask_textures: Option<&MaskTextures>,
        uniform_offset: u32,
        visibility: &Visibility,
    ) -> u32 {
        let materials = batches.materials();
        let mut draws = 0u32;
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
            binder.bind(pass, mesh, None);
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

/// The bind group the tile copy reads `static_atlas` through.
fn copy_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    static_atlas: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ShadowTileCopyBindGroup"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(static_atlas),
        }],
    })
}

/// Whether two spheres overlap (an unbounded one meets everything).
fn sphere_meets(a: Sphere, b: Sphere) -> bool {
    if !a.radius.is_finite() || !b.radius.is_finite() {
        return true;
    }
    a.center.distance_squared(b.center) <= (a.radius + b.radius).powi(2)
}

/// A depth pass over `view`: cleared to the far plane, or keeping what is there (the
/// atlases, whose tiles each view clears or copies itself).
fn begin_shadow_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    clear: bool,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("ShadowPass"),
        color_attachments: &[],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view,
            depth_ops: Some(wgpu::Operations {
                load: if clear {
                    wgpu::LoadOp::Clear(1.0)
                } else {
                    wgpu::LoadOp::Load
                },
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
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
        PreparedBatches::from_batcher(&batcher, Vec3::ZERO, &|_| None, &|_, _| None)
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
