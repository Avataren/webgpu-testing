//! Shadow maps: cascades for each directional light, and one atlas for every point and
//! spot light (tiles placed by `shadow_atlas.rs`).
//!
//! A directional light has up to four cascades, layers of one array texture, fitted to
//! the camera's view each frame (`cascades.rs`). A cascade's map is kept while its view
//! and casters stay as they were. When only dynamic casters changed, it is rebuilt like
//! a local face (below) from its static casters' depth in a second array, restored only
//! where dynamic casters were or are; when the camera moved it, it is drawn anew. The two farthest cascades take turns being
//! redrawn while the camera moves, and keep their last view in between. A local light's faces
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

use crate::renderer::batch::{CullMode, RenderPass};
use crate::renderer::internal::cascades::{
    cascade_view, caster_reach, fit_receivers, light_rotation, settle_radius, slice_sphere,
    split_depths, CascadeCamera, CascadeView, ReceiverDepths,
};
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
    DirectionalShadowRaw, LightsData, LocalShadowRaw, ShadowCascadeRaw, MAX_DIRECTIONAL_LIGHTS,
    MAX_SHADOWED_LOCAL_LIGHTS, MAX_SHADOW_CASCADES, POINT_SHADOW_FACES,
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

    fn layers(&self) -> u32 {
        self.texture.depth_or_array_layers()
    }

    fn size(&self) -> u32 {
        self.texture.width()
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

/// Shadow views a frame can render: each directional light's cascades and every local
/// light face.
const MAX_SHADOW_VIEWS: usize =
    MAX_DIRECTIONAL_LIGHTS * MAX_SHADOW_CASCADES + MAX_SHADOWED_LOCAL_LIGHTS * POINT_SHADOW_FACES;

/// Cascades and their maps' size unless the settings say otherwise.
pub(crate) const DEFAULT_SHADOW_CASCADES: usize = 4;
/// Casters up to this far above a receiver (along the light) widen its soft shadow edge;
/// with the light's apparent size this sets how far the filter looks for them.
const MAX_SOFT_SHADOW_CASTER_DISTANCE: f32 = 32.0;
/// Casters smaller than this share of a cascade's texel are left out of its map: they
/// would only flicker in and out of it.
const MIN_CASTER_TEXELS: f32 = 0.5;
/// Texels around a dynamic caster's footprint that are restored and redrawn with it (the
/// depth bias and rounding reach a little past its sphere).
const DYNAMIC_RECT_PAD: i64 = 2;

/// Texels of a cascade's map: x and y from (inclusive) and to (exclusive).
type TexelRect = [u32; 4];

fn union_rect(a: Option<TexelRect>, b: Option<TexelRect>) -> Option<TexelRect> {
    match (a, b) {
        (Some(a), Some(b)) => Some([
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[2].max(b[2]),
            a[3].max(b[3]),
        ]),
        (a, b) => a.or(b),
    }
}

/// Where in a cascade's map (`view`, `size` texels across, light space `light_view`) the
/// casters `spheres` that `frustum` meets land, padded; the whole map for one without
/// bounds. None when no sphere is in view.
fn caster_rect(
    spheres: &[Sphere],
    frustum: &Frustum,
    light_view: Mat4,
    view: &CascadeView,
    size: u32,
) -> Option<TexelRect> {
    let mut rect = None;
    let [sx, sy, ox, oy] = view.uv_transform;
    let texels = size as f32;
    for sphere in spheres.iter().filter(|sphere| frustum.intersects(**sphere)) {
        if !sphere.radius.is_finite() || !sphere.center.is_finite() {
            return Some([0, 0, size, size]);
        }
        let local = light_view.transform_point3(sphere.center);
        let (u, v) = (local.x * sx + ox, local.y * sy + oy);
        let (ru, rv) = (sphere.radius * sx.abs(), sphere.radius * sy.abs());
        let clamp = |value: f32, pad: i64| ((value as i64) + pad).clamp(0, size as i64) as u32;
        let texel_rect = [
            clamp(((u - ru) * texels).floor(), -DYNAMIC_RECT_PAD),
            clamp(((v - rv) * texels).floor(), -DYNAMIC_RECT_PAD),
            clamp(((u + ru) * texels).ceil(), DYNAMIC_RECT_PAD),
            clamp(((v + rv) * texels).ceil(), DYNAMIC_RECT_PAD),
        ];
        if texel_rect[0] < texel_rect[2] && texel_rect[1] < texel_rect[3] {
            rect = union_rect(rect, Some(texel_rect));
        }
    }
    rect
}

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

/// A directional light's cascade, from frame to frame.
#[derive(Clone, Copy, Default)]
struct CascadeState {
    /// The view its map was drawn with (None: its content is not usable).
    view: Option<CascadeView>,
    /// Its static casters' depth (in the static layers) is up to date for that view.
    static_ok: bool,
    /// Where its map holds dynamic casters, which may have moved since.
    dynamic_rect: Option<TexelRect>,
    /// The sphere radius it was fitted with (see `settle_radius`).
    radius: f32,
}

/// A directional light's cascades, from frame to frame.
#[derive(Clone, Copy, Default)]
struct DirectionalState {
    light_view: Option<Mat4>,
    cascades: [CascadeState; MAX_SHADOW_CASCADES],
}

/// A cascade of this frame: drawn, kept, or (moved) waiting for its turn.
struct CascadeJob {
    light: usize,
    cascade: usize,
    layer: u32,
    view: CascadeView,
    /// It keeps last frame's view and map until its turn comes.
    deferred: bool,
}

/// The directional shadows of a frame ([`ShadowResources::plan_directional`]).
#[derive(Default)]
pub(crate) struct DirectionalFrame {
    /// Per directional light (as `LightsData::directional_lights`), its cascades.
    pub uniforms: Vec<DirectionalShadowRaw>,
    jobs: Vec<CascadeJob>,
    /// The view depths where the first shadowed light's cascades meet (for the stats).
    pub splits: Vec<f32>,
}

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
    /// Every directional light's cascades, layer `light * cascades + cascade`; one 1x1
    /// layer until a light casts shadows.
    directional: ShadowArray,
    /// The same layers holding only the static casters, made when a cascade first needs
    /// them, with a bind group per layer for restoring a map from it.
    directional_static: Option<(ShadowArray, Vec<wgpu::BindGroup>)>,
    /// Cascades per directional light, and texels across each map.
    cascade_count: usize,
    cascade_size: u32,
    directional_states: [DirectionalState; MAX_DIRECTIONAL_LIGHTS],
    /// This frame's directional shadows ([`Self::plan_directional`]).
    directional_frame: DirectionalFrame,
    /// Frames planned, for the far cascades' turns.
    frame: u64,
    /// The largest local light tile.
    max_local_tile: u32,
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
            "Directional shadow cascades (light-space depth, 0 near to 1 far): array layer \
             light * cascades + cascade, nearest cascade first; layers are unused while \
             ray-traced shadows replace them",
            &self.directional.texture,
        ));
        out.push(FetchableTexture::new(
            "shadows.atlas",
            "Point and spot light shadow maps (light-space depth): square tiles, six cube faces \
             per point light and one per spot light, sized by the shadow budget",
            &self.atlas.texture,
        ));
    }

    /// `shadow_map_size`: the directional cascades' size (until
    /// [`Self::set_cascades`]) and the largest local tile; `atlas_size`: the largest the
    /// local shadow atlas may grow to.
    pub(crate) fn new(
        device: &wgpu::Device,
        objects: &DynamicObjectsBuffer,
        shadow_map_size: u32,
        atlas_size: u32,
    ) -> Self {
        let directional = ShadowArray::new(device, "DirectionalShadowCascades", 1, 1);
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
            directional_static: None,
            cascade_count: DEFAULT_SHADOW_CASCADES,
            cascade_size: shadow_map_size.clamp(256, device.limits().max_texture_dimension_2d),
            directional_states: [DirectionalState::default(); MAX_DIRECTIONAL_LIGHTS],
            directional_frame: DirectionalFrame::default(),
            frame: 0,
            max_local_tile: shadow_map_size,
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
        self.directional_states = [DirectionalState::default(); MAX_DIRECTIONAL_LIGHTS];
        self.faces.clear();
    }

    /// Cascades per directional light and texels across each of their maps.
    pub(crate) fn cascades(&self) -> (usize, u32) {
        (self.cascade_count, self.cascade_size)
    }

    /// Give each directional light `count` cascades (1 to 4) of `size` texels across
    /// (within the device's limit), from the next frame.
    pub(crate) fn set_cascades(&mut self, device: &wgpu::Device, count: usize, size: u32) {
        let count = count.clamp(1, MAX_SHADOW_CASCADES);
        let size = size.clamp(256, device.limits().max_texture_dimension_2d);
        if (count, size) != (self.cascade_count, self.cascade_size) {
            self.cascade_count = count;
            self.cascade_size = size;
            self.directional_states = [DirectionalState::default(); MAX_DIRECTIONAL_LIGHTS];
        }
    }

    /// The directional shadows planned for this frame.
    pub(crate) fn directional_frame(&self) -> &DirectionalFrame {
        &self.directional_frame
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
            let max_tile = self.max_local_tile.min(size / 8);
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

    /// Fit each shadow-casting directional light's cascades to `camera` and what it sees
    /// in `batches` (see `cascades.rs`), and decide which cascades are drawn this frame.
    /// `sun_size_degrees` is the lights' apparent size, for their soft shadow edges.
    /// Returns whether the cascade maps were recreated (whoever binds them must rebind).
    pub(crate) fn plan_directional(
        &mut self,
        device: &wgpu::Device,
        lights: &LightsData,
        camera: &CascadeCamera,
        batches: &PreparedBatches,
        sun_size_degrees: f32,
    ) -> bool {
        self.frame += 1;
        let shadows = lights.directional_shadows();
        let directional = lights.directional_lights();
        let count = self.cascade_count;
        let size = self.cascade_size;
        let frame = &mut self.directional_frame;
        frame.uniforms.clear();
        frame.jobs.clear();
        frame.splits.clear();

        // Layers by light index, so a light keeps its own when another comes or goes.
        let layers = shadows
            .iter()
            .rposition(Option::is_some)
            .map_or(0, |last| (last + 1) * count) as u32;
        let mut recreated = false;
        if layers > 0 && (self.directional.layers() < layers || self.directional.size() != size) {
            self.directional = ShadowArray::new(device, "DirectionalShadowCascades", layers, size);
            self.directional_static = None;
            self.directional_states = [DirectionalState::default(); MAX_DIRECTIONAL_LIGHTS];
            recreated = true;
        }
        if layers == 0 {
            frame
                .uniforms
                .resize(directional.len(), DirectionalShadowRaw::zeroed());
            return recreated;
        }

        // The depths of what the camera sees that can receive a shadow (a batch without
        // bounds: anything, so the whole view counts).
        let receiver_batches = || {
            batches
                .all()
                .iter()
                .filter(|batch| matches!(batch.pass, RenderPass::Opaque | RenderPass::Transparent))
        };
        let receivers = if receiver_batches()
            .any(|batch| batch.bounds.is_empty() && !batch.instances.is_empty())
        {
            ReceiverDepths::Unknown
        } else {
            ReceiverDepths::of(
                camera,
                receiver_batches().flat_map(|batch| {
                    batch
                        .bounds
                        .iter()
                        .enumerate()
                        .filter_map(|(index, sphere)| {
                            batch
                                .camera_visible
                                .get(index)
                                .copied()
                                .unwrap_or(true)
                                .then_some(sphere)
                        })
                }),
            )
        };
        let casters = || {
            batches
                .all()
                .iter()
                .filter(|batch| casts_shadows(batch.pass))
                .flat_map(|batch| batch.bounds.iter())
        };

        let camera_world = camera.view.inverse();
        let (eye, forward) = camera.eye_and_forward();
        let tan_half = (0.5 * sun_size_degrees.max(0.0)).to_radians().tan();
        for (index, light) in directional.iter().enumerate() {
            let state = &mut self.directional_states[index];
            let settings = shadows.get(index).copied().flatten();
            let range =
                settings.and_then(|shadow| fit_receivers(camera, receivers, shadow.distance));
            let (Some(_), Some(range)) = (settings, range) else {
                // No shadows, or nothing in view to receive them: the maps keep what they
                // hold for when they are needed again.
                frame.uniforms.push(DirectionalShadowRaw::zeroed());
                continue;
            };
            let [x, y, z, _] = light.direction;
            let light_view = light_rotation(Vec3::new(x, y, z));
            let rotated = state.light_view != Some(light_view);
            state.light_view = Some(light_view);
            let reach = caster_reach(light_view, casters());
            let splits = split_depths(range, count);
            if frame.splits.is_empty() {
                frame.splits.extend_from_slice(&splits[..=count]);
            }

            let mut raw = DirectionalShadowRaw::zeroed();
            raw.light_view = light_view.to_cols_array_2d();
            for cascade in 0..count {
                let cascade_state = &mut state.cascades[cascade];
                let (center, radius) = slice_sphere(camera, splits[cascade], splits[cascade + 1]);
                cascade_state.radius = settle_radius(radius, cascade_state.radius);
                let sphere = Sphere {
                    center: camera_world.transform_point3(center),
                    radius: cascade_state.radius,
                };
                let view = cascade_view(light_view, sphere, size, reach);
                // The far cascades take turns while the camera moves them: each keeps its
                // last view and map every other frame (the shaders pick, per point, the
                // nearest cascade whose map holds it, so a cascade a frame behind still
                // serves what it covers).
                let deferred = cascade >= 2
                    && !rotated
                    && cascade_state
                        .view
                        .is_some_and(|old| old != view && old.texel == view.texel)
                    && (self.frame + cascade as u64) % 2 == 1;
                let used = match (deferred, cascade_state.view) {
                    (true, Some(old)) => old,
                    _ => view,
                };
                raw.cascades[cascade] = ShadowCascadeRaw {
                    uv_transform: used.uv_transform,
                    depth_transform: [
                        used.depth_transform[0],
                        used.depth_transform[1],
                        used.texel,
                        0.0,
                    ],
                };
                frame.jobs.push(CascadeJob {
                    light: index,
                    cascade,
                    layer: (index * count + cascade) as u32,
                    view: used,
                    deferred,
                });
            }
            raw.params = [count as f32, (index * count) as f32, size as f32, tan_half];
            raw.view_depth = [forward.x, forward.y, forward.z, -forward.dot(eye)];
            // Shadows cut off by the shadow distance fade out over its last tenth.
            let fade_length = (range.far * 0.1).max(1e-3);
            raw.fade = [
                if range.fade {
                    range.far - fade_length
                } else {
                    f32::MAX
                },
                if range.fade { 1.0 / fade_length } else { 0.0 },
                tan_half * MAX_SOFT_SHADOW_CASTER_DISTANCE,
                0.0,
            ];
            frame.uniforms.push(raw);
        }
        recreated
    }

    /// Draw this frame's directional cascades ([`Self::plan_directional`]) except those of
    /// the lights in `skip` (bits as the ray-traced shadow mask's 0-3, whose maps nothing
    /// reads this frame), and update this frame's local faces ([`Self::plan_local`]; see
    /// the module comment). Custom passes that draw into the maps (`record_custom_passes`)
    /// count as dynamic casters in every view; each view updated is listed in
    /// `custom_invocations` for them. `after_cascades` runs between the cascades and the
    /// local faces (a GPU timer mark).
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
        after_cascades: &mut dyn FnMut(&mut wgpu::CommandEncoder),
    ) -> ShadowPassOutcome {
        let mut outcome = ShadowPassOutcome::default();
        let mesh_identity = |batch: &OrderedBatch| {
            geometry
                .mesh(batch)
                .map(|mesh| std::sync::Arc::as_ptr(mesh.shared_data()) as usize as u64)
        };
        // Which casters stay put, and where static ones changed (`shadow_casters.rs`).
        let changes = self
            .casters
            .update(batches, mesh_identity, texture_generation);

        // Cascades to draw: anew when the camera or light moved them (or static casters in
        // view changed and nothing moves there), rebuilt from their static casters' depth
        // when dynamic casters are in view (or were), else kept as they are. A rebuild
        // restores the static depth only where dynamic casters were or are (`restore`; the
        // whole map after new static depth) and draws those in `dynamic`.
        enum CascadeWork {
            Full,
            Layered {
                render_static: bool,
                restore: TexelRect,
                dynamic: Option<TexelRect>,
            },
        }
        let size = self.directional.size();
        let mut cascades: Vec<(usize, CascadeWork)> = Vec::new();
        for (index, job) in self.directional_frame.jobs.iter().enumerate() {
            let light_view = self.directional_states[job.light]
                .light_view
                .unwrap_or(Mat4::IDENTITY);
            let state = &mut self.directional_states[job.light].cascades[job.cascade];
            if skip & (1 << job.light) != 0 {
                // Nothing reads the map this frame; it is drawn again when something does.
                state.view = None;
                state.static_ok = false;
                continue;
            }
            if job.deferred {
                outcome.views_deferred += 1;
                continue;
            }
            let frustum = Frustum::from_view_proj(job.view.view_proj);
            let static_dirty = changes.all_dirty
                || changes
                    .dirty
                    .iter()
                    .any(|sphere| frustum.intersects(*sphere));
            let dynamic = if record_custom_passes {
                Some([0, 0, size, size])
            } else {
                caster_rect(
                    &changes.dynamic_spheres,
                    &frustum,
                    light_view,
                    &job.view,
                    size,
                )
            };
            let moved = state.view != Some(job.view);
            if moved || static_dirty {
                state.static_ok = false;
            }
            let work = if moved || (static_dirty && dynamic.is_none()) {
                CascadeWork::Full
            } else if static_dirty || dynamic.is_some() || state.dynamic_rect.is_some() {
                let render_static = !state.static_ok;
                CascadeWork::Layered {
                    render_static,
                    restore: if render_static {
                        [0, 0, size, size]
                    } else {
                        union_rect(state.dynamic_rect, dynamic).unwrap_or([0, 0, size, size])
                    },
                    dynamic,
                }
            } else {
                outcome.views_cached += 1;
                continue;
            };
            if matches!(work, CascadeWork::Layered { .. }) {
                state.static_ok = true;
            }
            state.view = Some(job.view);
            state.dynamic_rect = dynamic;
            cascades.push((index, work));
        }

        // Local faces to update: where static casters changed, faces lose their static
        // depth; faces with dynamic casters in view (or that held some) are rebuilt.
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
        if cascades.is_empty() && jobs.is_empty() {
            after_cascades(encoder);
            return outcome;
        }

        // Every view's projection, one slot each, uploaded at once: cascades, then faces.
        let stride = self.view_stride as usize;
        self.view_scratch.clear();
        let cascade_jobs = &self.directional_frame.jobs;
        let matrices = cascades
            .iter()
            .map(|(job, _)| cascade_jobs[*job].view.view_proj)
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
        // The static layers, the first time a cascade needs them.
        let needs_static = cascades.iter().any(|(_, work)| {
            matches!(
                work,
                CascadeWork::Layered {
                    render_static: true,
                    ..
                }
            )
        });
        if needs_static
            && self.directional_static.as_ref().is_none_or(|(maps, _)| {
                maps.layers() != self.directional.layers() || maps.size() != self.directional.size()
            })
        {
            let maps = ShadowArray::new(
                &context.device,
                "DirectionalShadowStaticCascades",
                self.directional.layers(),
                self.directional.size(),
            );
            let groups = maps
                .layer_views
                .iter()
                .map(|view| copy_bind_group(&context.device, &self.copy_layout, view))
                .collect();
            self.directional_static = Some((maps, groups));
        }
        // Cascades, each in its own passes.
        for (slot, (job, work)) in cascades.iter().enumerate() {
            let job = &self.directional_frame.jobs[*job];
            let offset = (slot * stride) as u32;
            let frustum = Frustum::from_view_proj(job.view.view_proj);
            let min_radius = job.view.texel * MIN_CASTER_TEXELS;
            let big_enough = |batch: usize, instance: usize| {
                batches.all()[batch]
                    .bounds
                    .get(instance)
                    .is_none_or(|sphere| sphere.radius >= min_radius)
            };
            let layer = self.directional.layer_view(job.layer as usize).clone();
            match *work {
                CascadeWork::Full => {
                    visibility.compute_filtered(batches, &frustum, big_enough);
                    let mut pass = begin_shadow_pass(encoder, &layer, true, "ShadowCascade");
                    outcome.draw_calls += self.draw_casters(
                        &mut pass,
                        geometry,
                        batches,
                        objects,
                        mask_textures,
                        offset,
                        &visibility,
                    );
                }
                CascadeWork::Layered {
                    render_static,
                    restore,
                    dynamic,
                } => {
                    let Some((static_maps, copy_groups)) = self.directional_static.as_ref() else {
                        continue;
                    };
                    if render_static {
                        visibility.compute_filtered(batches, &frustum, |batch, instance| {
                            !changes.is_dynamic(batch, instance) && big_enough(batch, instance)
                        });
                        let static_layer = static_maps.layer_view(job.layer as usize);
                        let mut pass =
                            begin_shadow_pass(encoder, static_layer, true, "ShadowCascadeStatic");
                        outcome.draw_calls += self.draw_casters(
                            &mut pass,
                            geometry,
                            batches,
                            objects,
                            mask_textures,
                            offset,
                            &visibility,
                        );
                    }
                    // The static depth back where dynamic casters were or are, then those.
                    let mut pass =
                        begin_shadow_pass(encoder, &layer, false, "ShadowCascadeDynamic");
                    let [x0, y0, x1, y1] = restore;
                    pass.set_scissor_rect(x0, y0, x1 - x0, y1 - y0);
                    pass.set_pipeline(&self.copy_pipeline);
                    pass.set_bind_group(0, &copy_groups[job.layer as usize], &[]);
                    pass.draw(0..3, 0..1);
                    if dynamic.is_some() {
                        visibility.compute_filtered(batches, &frustum, |batch, instance| {
                            changes.is_dynamic(batch, instance) && big_enough(batch, instance)
                        });
                        outcome.draw_calls += self.draw_casters(
                            &mut pass,
                            geometry,
                            batches,
                            objects,
                            mask_textures,
                            offset,
                            &visibility,
                        );
                    }
                }
            }
            outcome.views_rendered += 1;
            if record_custom_passes {
                custom_invocations.push(ShadowInvocation {
                    stage: ShadowPassStage::Directional {
                        light_index: job.light as u32,
                        cascade: job.cascade as u32,
                    },
                    view: layer,
                    view_proj: job.view.view_proj,
                    viewport: None,
                });
            }
        }
        after_cascades(encoder);
        let first_face_slot = cascades.len();
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
            let mut pass = begin_shadow_pass(encoder, &static_view, false, "ShadowAtlasStatic");
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
            let mut pass = begin_shadow_pass(encoder, &atlas_view, false, "ShadowAtlas");
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

/// A depth pass `label` over `view`: cleared to the far plane, or keeping what is there
/// (the atlases, whose tiles each view clears or copies itself, and cascades rebuilt over
/// a copy of their static casters).
fn begin_shadow_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    clear: bool,
    label: &str,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
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
