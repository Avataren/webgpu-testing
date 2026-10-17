//! Fog in the renderer (see [`crate::fog`] for the settings): the uniform every fogging
//! pass reads, the froxel volume that gathers volumetric fog near the camera, and the pass
//! that fogs the opaque scene.
//!
//! A frame with fog runs, when the graphics setting [`VolumetricFog`] is above off:
//!
//! 1. after the shadow maps and the light probes, `cs_fog_scatter` (fog_volume.wgsl): one
//!    invocation per froxel of a volume that follows the unjittered view (a froxel per
//!    8x8 pixels and 64 slices of view depth at medium quality, out to the fog's
//!    `volumetric_distance`, slices thin near the camera) lights a point inside it,
//!    jittered from frame to frame, with the shadowed directional lights, the point and
//!    spot lights of its cluster with their shadow maps and the ambient light (light
//!    probes, else the environment), and blends it with what the froxel held last frame
//!    there (reprojected);
//! 2. `cs_fog_integrate` (fog_integrate.wgsl) walks each column outward and stores the fog
//!    between the camera and each slice's far end;
//! 3. after the opaque passes, `fs_fog_apply` (fog_apply.wgsl) blends it over the scene,
//!    looking each pixel's surface up from the depth buffer.
//!
//! Past the volume, and everywhere while it does not run, the fog is the height fog's
//! closed form, lit by the ambient and directional lights without shadows. Transparent
//! surfaces and particles fog themselves with the same functions (fog.wgsl, bindings 16
//! and 17 of the lights group). The volume needs the shadow maps, so they are rendered
//! even where ray-traced shadows replace them for the main pass.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

use crate::fog::Fog;
use crate::renderer::shader_source::wgsl;
use crate::renderer::{PipelineBuilder, RenderRegion, ShaderBuilder};

/// The froxel volumes' format: scattered light in rgb, extinction (the scattering
/// volume) or transmittance (the integrated one) in alpha.
const VOLUME_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// The most froxels across and down the volume (it keeps the view's aspect).
const MAX_FROXELS_X: u32 = 320;
const MAX_FROXELS_Y: u32 = 240;
/// The share of last frame's scattering a froxel keeps.
const HISTORY_WEIGHT: f32 = 0.85;
/// Camera moves that start the history over: farther in one frame (m), or a wider turn
/// (cosine of 15 degrees).
const CUT_DISTANCE: f32 = 2.0;
const CUT_COS: f32 = 0.966;

/// Volumetric fog quality, a graphics setting: how finely the froxel volume near the
/// camera is cut. Off keeps the height fog, lit without shadows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VolumetricFog {
    Off,
    /// A froxel per 16x16 pixels, 32 slices.
    Low,
    /// A froxel per 8x8 pixels, 64 slices.
    #[default]
    Medium,
    /// A froxel per 6x6 pixels, 96 slices.
    High,
}

impl VolumetricFog {
    pub const ALL: [Self; 4] = [Self::Off, Self::Low, Self::Medium, Self::High];

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
        }
    }

    /// Pixels per froxel across, and slices of view depth; `None` when off.
    fn layout(self) -> Option<(u32, u32)> {
        match self {
            Self::Off => None,
            Self::Low => Some((16, 32)),
            Self::Medium => Some((8, 64)),
            Self::High => Some((6, 96)),
        }
    }

    /// The volume's froxels along x, y and depth for a view `width` x `height` pixels.
    pub(crate) fn froxels(self, width: u32, height: u32) -> Option<[u32; 3]> {
        let (tile, slices) = self.layout()?;
        let mut x = width.div_ceil(tile).max(1);
        let mut y = height.div_ceil(tile).max(1);
        let shrink = (x as f32 / MAX_FROXELS_X as f32).max(y as f32 / MAX_FROXELS_Y as f32);
        if shrink > 1.0 {
            x = ((x as f32 / shrink).ceil() as u32).max(1);
            y = ((y as f32 / shrink).ceil() as u32).max(1);
        }
        Some([x, y, slices])
    }
}

/// Mirrors `FogParams` in fog_common.wgsl (see there for each field).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub(crate) struct FogUniform {
    flags: [f32; 4],
    density: [f32; 4],
    albedo: [f32; 4],
    emission: [f32; 4],
    volume: [f32; 4],
    dims: [u32; 4],
    view_proj: [[f32; 4]; 4],
    inverse_view_proj: [[f32; 4]; 4],
    prev_view_proj: [[f32; 4]; 4],
    camera_position: [f32; 4],
    camera_forward: [f32; 4],
    prev_camera_position: [f32; 4],
    prev_camera_forward: [f32; 4],
    viewport: [f32; 4],
}

/// The camera the volume follows: the scene camera without TAA jitter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FogCamera {
    pub view: Mat4,
    pub projection: Mat4,
}

impl FogCamera {
    fn view_proj(&self) -> Mat4 {
        self.projection * self.view
    }

    fn position(&self) -> Vec3 {
        self.view.inverse().w_axis.truncate()
    }

    fn forward(&self) -> Vec3 {
        -Vec3::new(self.view.x_axis.z, self.view.y_axis.z, self.view.z_axis.z)
    }
}

/// What the lights group binds for fog (bindings 16 and 17): the uniform, and a volume of
/// no fog for frames without the froxel volume.
pub(crate) struct FogResources {
    pub(crate) uniform: wgpu::Buffer,
    _fallback: wgpu::Texture,
    pub(crate) fallback_view: wgpu::TextureView,
}

impl FogResources {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        use wgpu::util::DeviceExt;
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("FogUniform"),
            contents: bytemuck::bytes_of(&FogUniform::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        // No light scattered, all of it let through: half floats 0, 0, 0, 1.
        let fallback = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("FogVolumeNone"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D3,
                format: VOLUME_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &[0, 0, 0, 0, 0, 0, 0x00, 0x3c],
        );
        let fallback_view = fallback.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            uniform,
            _fallback: fallback,
            fallback_view,
        }
    }
}

/// The froxel volume's textures, sized for the view.
struct FogVolume {
    dims: [u32; 3],
    _scatter: [wgpu::Texture; 2],
    _integrated: wgpu::Texture,
    integrated_view: wgpu::TextureView,
    /// By which scatter texture is this frame's: it is written, the other read as history.
    scatter_groups: [wgpu::BindGroup; 2],
    integrate_groups: [wgpu::BindGroup; 2],
}

/// The fog passes' pipelines, rebuilt by a shader reload.
pub(crate) struct FogPipelines {
    scatter: wgpu::ComputePipeline,
    integrate: wgpu::ComputePipeline,
    apply: wgpu::RenderPipeline,
}

/// Where the camera was the frame before, for the volume's history.
#[derive(Clone, Copy, Debug)]
struct FogHistory {
    view_proj: Mat4,
    position: Vec3,
    forward: Vec3,
    far: f32,
    dims: [u32; 3],
}

pub(crate) struct FogStage {
    resources: FogResources,
    scatter_layout: wgpu::BindGroupLayout,
    integrate_layout: wgpu::BindGroupLayout,
    apply_layout: wgpu::BindGroupLayout,
    scatter_pipeline_layout: wgpu::PipelineLayout,
    integrate_pipeline_layout: wgpu::PipelineLayout,
    apply_pipeline_layout: wgpu::PipelineLayout,
    pipelines: FogPipelines,
    sampler: wgpu::Sampler,
    sample_count: u32,
    scene_format: wgpu::TextureFormat,
    quality: VolumetricFog,
    volume: Option<FogVolume>,
    /// The fog pass's depth binding, and the depth view it was made for.
    apply_group: Option<(wgpu::BindGroup, wgpu::TextureView)>,
    last_uniform: Option<FogUniform>,
    /// This frame has fog, and runs the volume.
    enabled: bool,
    volumetric: bool,
    /// Steps the froxels' jitter (held while noise is frozen).
    noise_frame: u32,
    /// Which scatter texture is this frame's.
    parity: usize,
    history: Option<FogHistory>,
}

impl FogStage {
    pub(crate) fn new(
        device: &wgpu::Device,
        resources: FogResources,
        camera_layout: &wgpu::BindGroupLayout,
        lights_layout: &wgpu::BindGroupLayout,
        sample_count: u32,
        scene_format: wgpu::TextureFormat,
    ) -> Self {
        let volume_texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D3,
                multisampled: false,
            },
            count: None,
        };
        let volume_storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: VOLUME_FORMAT,
                view_dimension: wgpu::TextureViewDimension::D3,
            },
            count: None,
        };
        let scatter_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("FogScatterLayout"),
            entries: &[
                volume_texture(0),
                volume_storage(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let integrate_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("FogIntegrateLayout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                volume_texture(1),
                volume_storage(2),
            ],
        });
        let apply_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("FogApplyLayout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: sample_count > 1,
                },
                count: None,
            }],
        });
        // The volume's lighting reads the lights group (group 2) like the scene passes.
        let scatter_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("FogScatterPipelineLayout"),
                bind_group_layouts: &[Some(&scatter_layout), None, Some(lights_layout)],
                immediate_size: 0,
            });
        let integrate_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("FogIntegratePipelineLayout"),
                bind_group_layouts: &[Some(&integrate_layout)],
                immediate_size: 0,
            });
        let apply_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("FogApplyPipelineLayout"),
                bind_group_layouts: &[
                    Some(camera_layout),
                    Some(&apply_layout),
                    Some(lights_layout),
                ],
                immediate_size: 0,
            });
        let pipelines = build_pipelines(
            device,
            &scatter_pipeline_layout,
            &integrate_pipeline_layout,
            &apply_pipeline_layout,
            sample_count,
            scene_format,
        )
        .expect("built-in fog shaders should compile through naga");
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("FogVolumeSampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            resources,
            scatter_layout,
            integrate_layout,
            apply_layout,
            scatter_pipeline_layout,
            integrate_pipeline_layout,
            apply_pipeline_layout,
            pipelines,
            sampler,
            sample_count,
            scene_format,
            quality: VolumetricFog::default(),
            volume: None,
            apply_group: None,
            last_uniform: None,
            enabled: false,
            volumetric: false,
            noise_frame: 0,
            parity: 0,
            history: None,
        }
    }

    /// Build the pipelines again from the current shader sources (a shader reload);
    /// nothing changes until [`Self::install_pipelines`].
    pub(crate) fn rebuild_pipelines(&self, device: &wgpu::Device) -> Result<FogPipelines, String> {
        build_pipelines(
            device,
            &self.scatter_pipeline_layout,
            &self.integrate_pipeline_layout,
            &self.apply_pipeline_layout,
            self.sample_count,
            self.scene_format,
        )
    }

    pub(crate) fn install_pipelines(&mut self, pipelines: FogPipelines) {
        self.pipelines = pipelines;
        self.history = None;
    }

    pub(crate) fn uniform_buffer(&self) -> &wgpu::Buffer {
        &self.resources.uniform
    }

    /// The integrated volume the lights group binds: this frame's, or the volume of no fog.
    pub(crate) fn volume_view(&self) -> &wgpu::TextureView {
        self.volume
            .as_ref()
            .map_or(&self.resources.fallback_view, |volume| {
                &volume.integrated_view
            })
    }

    pub(crate) fn quality(&self) -> VolumetricFog {
        self.quality
    }

    pub(crate) fn set_quality(&mut self, quality: VolumetricFog) {
        self.quality = quality;
    }

    /// Whether this frame has fog.
    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    /// Whether this frame runs the froxel volume (it reads the shadow maps).
    pub(crate) fn volumetric(&self) -> bool {
        self.enabled && self.volumetric
    }

    /// The froxel volume's size this frame, if it runs.
    pub(crate) fn froxels(&self) -> Option<[u32; 3]> {
        self.volume
            .as_ref()
            .filter(|_| self.volumetric())
            .map(|volume| volume.dims)
    }

    /// Settle this frame's fog: whether there is any, the volume's size for the scene's
    /// viewport `region`, and the uniform. Returns true when the volume texture the lights
    /// group binds changed.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        fog: Option<&Fog>,
        camera: &FogCamera,
        region: Option<RenderRegion>,
        frozen_noise: bool,
    ) -> bool {
        // Nothing to fog without a viewport.
        let (fog, region) = match fog.zip(region) {
            Some((fog, region)) => (Some(fog), region),
            None => (None, RenderRegion::new(0, 0, 1, 1).expect("a 1x1 region")),
        };
        let froxels = fog.and_then(|_| self.quality.froxels(region.width(), region.height()));
        self.enabled = fog.is_some();
        self.volumetric = froxels.is_some();
        let mut changed = false;
        match froxels {
            Some(dims) if self.volume.as_ref().map(|volume| volume.dims) != Some(dims) => {
                self.volume = Some(self.create_volume(device, dims));
                self.history = None;
                changed = true;
            }
            None if self.volume.is_some() => {
                self.volume = None;
                self.history = None;
                changed = true;
            }
            _ => {}
        }

        let uniform = match fog {
            Some(fog) => {
                let view_proj = camera.view_proj();
                let current = FogHistory {
                    view_proj,
                    position: camera.position(),
                    forward: camera.forward(),
                    far: fog.volumetric_distance,
                    dims: froxels.unwrap_or([1, 1, 1]),
                };
                let previous = if self.volumetric {
                    self.history.replace(current)
                } else {
                    self.history = None;
                    None
                };
                let keep = previous
                    .filter(|previous| {
                        previous.far == current.far
                            && previous.dims == current.dims
                            && previous.position.distance(current.position) <= CUT_DISTANCE
                            && previous.forward.dot(current.forward) >= CUT_COS
                    })
                    .map_or(0.0, |_| HISTORY_WEIGHT);
                let previous = previous.unwrap_or(current);
                if self.volumetric {
                    self.parity ^= 1;
                    if !frozen_noise {
                        self.noise_frame = (self.noise_frame + 1) % 4096;
                    }
                }
                fog_uniform(
                    fog,
                    &current,
                    &previous,
                    camera.view_proj().inverse(),
                    self.volumetric,
                    keep,
                    self.noise_frame,
                    region,
                )
            }
            None => FogUniform::zeroed(),
        };
        if self.last_uniform != Some(uniform) {
            queue.write_buffer(&self.resources.uniform, 0, bytemuck::bytes_of(&uniform));
            self.last_uniform = Some(uniform);
        }
        changed
    }

    fn create_volume(&self, device: &wgpu::Device, dims: [u32; 3]) -> FogVolume {
        let texture = |label| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: dims[0],
                    height: dims[1],
                    depth_or_array_layers: dims[2],
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D3,
                format: VOLUME_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
                view_formats: &[],
            })
        };
        let scatter = [texture("FogScatterA"), texture("FogScatterB")];
        let integrated = texture("FogIntegrated");
        let view = |texture: &wgpu::Texture| texture.create_view(&Default::default());
        let scatter_views = [view(&scatter[0]), view(&scatter[1])];
        let integrated_view = view(&integrated);
        let scatter_groups = [0, 1].map(|current| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("FogScatterBindGroup"),
                layout: &self.scatter_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&scatter_views[1 - current]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&scatter_views[current]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            })
        });
        let integrate_groups = [0, 1].map(|current| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("FogIntegrateBindGroup"),
                layout: &self.integrate_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.resources.uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&scatter_views[current]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&integrated_view),
                    },
                ],
            })
        });
        FogVolume {
            dims,
            _scatter: scatter,
            _integrated: integrated,
            integrated_view,
            scatter_groups,
            integrate_groups,
        }
    }

    /// Light and integrate this frame's froxel volume (after the shadow maps, light
    /// culling and light probes; `lights` is the lights group).
    pub(crate) fn dispatch_volume(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        lights: &wgpu::BindGroup,
    ) {
        let Some(volume) = self.volume.as_ref().filter(|_| self.volumetric()) else {
            return;
        };
        let [x, y, z] = volume.dims;
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("FogScatter"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipelines.scatter);
            pass.set_bind_group(0, &volume.scatter_groups[self.parity], &[]);
            pass.set_bind_group(2, lights, &[]);
            pass.dispatch_workgroups(x.div_ceil(4), y.div_ceil(4), z.div_ceil(4));
        }
        // Its own pass: it writes the volume the lights group binds.
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("FogIntegrate"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipelines.integrate);
        pass.set_bind_group(0, &volume.integrate_groups[self.parity], &[]);
        pass.dispatch_workgroups(x.div_ceil(8), y.div_ceil(8), 1);
    }

    /// Blend the fog over the opaque scene in `color` (resolving into `resolve` with
    /// MSAA), within the scene's viewport `region`; `depth` is the scene's depth.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        resolve: Option<&wgpu::TextureView>,
        depth: &wgpu::TextureView,
        camera: &wgpu::BindGroup,
        lights: &wgpu::BindGroup,
        region: Option<RenderRegion>,
    ) {
        if !self.enabled {
            return;
        }
        if self
            .apply_group
            .as_ref()
            .is_none_or(|(_, bound)| bound != depth)
        {
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("FogApplyBindGroup"),
                layout: &self.apply_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(depth),
                }],
            });
            self.apply_group = Some((group, depth.clone()));
        }
        let Some((group, _)) = self.apply_group.as_ref() else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("FogApply"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                resolve_target: resolve,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if let Some(region) = region {
            region.apply_to_pass(&mut pass);
        }
        pass.set_pipeline(&self.pipelines.apply);
        pass.set_bind_group(0, camera, &[]);
        pass.set_bind_group(1, group, &[]);
        pass.set_bind_group(2, lights, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[allow(clippy::too_many_arguments)]
fn fog_uniform(
    fog: &Fog,
    current: &FogHistory,
    previous: &FogHistory,
    inverse_view_proj: Mat4,
    volumetric: bool,
    keep: f32,
    noise_frame: u32,
    region: RenderRegion,
) -> FogUniform {
    let vec4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
    let [r, g, b] = fog.albedo;
    let [er, eg, eb] = fog.emission;
    FogUniform {
        flags: [
            1.0,
            if volumetric { 1.0 } else { 0.0 },
            noise_frame as f32,
            keep,
        ],
        density: [
            fog.density,
            fog.height,
            if fog.height_falloff > 0.0 {
                1.0 / fog.height_falloff
            } else {
                0.0
            },
            fog.max_distance,
        ],
        albedo: [r, g, b, fog.anisotropy],
        emission: [er, eg, eb, fog.ambient],
        volume: [current.far, current.dims[2] as f32, fog.light, 0.0],
        dims: [current.dims[0], current.dims[1], current.dims[2], 0],
        view_proj: current.view_proj.to_cols_array_2d(),
        inverse_view_proj: inverse_view_proj.to_cols_array_2d(),
        prev_view_proj: previous.view_proj.to_cols_array_2d(),
        camera_position: vec4(current.position, 0.0),
        camera_forward: vec4(current.forward, 0.0),
        prev_camera_position: vec4(previous.position, 0.0),
        prev_camera_forward: vec4(previous.forward, 0.0),
        viewport: [
            region.x() as f32,
            region.y() as f32,
            region.width() as f32,
            region.height() as f32,
        ],
    }
}

fn scatter_source() -> String {
    ShaderBuilder::new()
        .with_constants()
        .with_lighting()
        .with_shadows()
        .with_environment()
        .with_fog()
        .build(wgsl!("fog_volume.wgsl"))
}

fn integrate_source() -> String {
    ShaderBuilder::new()
        .with_module(wgsl!("fog_common.wgsl"))
        .build(wgsl!("fog_integrate.wgsl"))
}

fn apply_source(multisampled: bool) -> String {
    let source = ShaderBuilder::new()
        .with_camera()
        .with_constants()
        .with_lighting()
        .with_shadows()
        .with_environment()
        .with_fog()
        .build(wgsl!("fog_apply.wgsl"));
    if multisampled {
        source.replace(
            "var fog_scene_depth: texture_depth_2d;",
            "var fog_scene_depth: texture_depth_multisampled_2d;",
        )
    } else {
        source
    }
}

fn build_pipelines(
    device: &wgpu::Device,
    scatter_layout: &wgpu::PipelineLayout,
    integrate_layout: &wgpu::PipelineLayout,
    apply_layout: &wgpu::PipelineLayout,
    sample_count: u32,
    scene_format: wgpu::TextureFormat,
) -> Result<FogPipelines, String> {
    let module = |label: &str, source: String| -> Result<wgpu::ShaderModule, String> {
        crate::renderer::shader_source::validate(&source)
            .map_err(|error| format!("{label}: {error}"))?;
        Ok(device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        }))
    };
    let scatter_module = module("FogScatterShader", scatter_source())?;
    let integrate_module = module("FogIntegrateShader", integrate_source())?;
    let apply_module = module("FogApplyShader", apply_source(sample_count > 1))?;
    let compute = |label, layout, module: &wgpu::ShaderModule, entry| {
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(label),
            layout: Some(layout),
            module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            cache: None,
        })
    };
    // color * transmittance (alpha) + scattered light; the alpha (glass marks) stays.
    let blend = wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::SrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Zero,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
    };
    Ok(FogPipelines {
        scatter: compute(
            "FogScatterPipeline",
            scatter_layout,
            &scatter_module,
            "cs_fog_scatter",
        ),
        integrate: compute(
            "FogIntegratePipeline",
            integrate_layout,
            &integrate_module,
            "cs_fog_integrate",
        ),
        apply: PipelineBuilder::new(device, apply_layout, &apply_module)
            .with_label("FogApplyPipeline")
            .with_vertex_entry("vs_fog_apply")
            .with_fragment_entry("fs_fog_apply")
            .with_color_target(scene_format, Some(blend))
            .with_no_culling()
            .with_multisample(sample_count)
            .build(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// fog_optical_depth in fog_common.wgsl, for checking its closed form.
    fn optical_depth(fog: &Fog, origin: Vec3, dir: Vec3, t0: f32, t1: f32) -> f32 {
        let k = if fog.height_falloff > 0.0 {
            1.0 / fog.height_falloff
        } else {
            0.0
        };
        if t1 <= t0 || fog.density <= 0.0 {
            return 0.0;
        }
        if k <= 0.0 {
            return fog.density * (t1 - t0);
        }
        let y0 = origin.y + dir.y * t0 - fog.height;
        let y1 = origin.y + dir.y * t1 - fog.height;
        if y0 <= 0.0 && y1 <= 0.0 {
            return fog.density * (t1 - t0);
        }
        let (mut below, mut a, mut b) = (0.0, t0, t1);
        if y0 < 0.0 || y1 < 0.0 {
            let cross = t0 + (t1 - t0) * (y0 / (y0 - y1));
            if y0 < 0.0 {
                below = cross - t0;
                a = cross;
            } else {
                below = t1 - cross;
                b = cross;
            }
        }
        let ya = (origin.y + dir.y * a - fog.height).max(0.0);
        let yb = (origin.y + dir.y * b - fog.height).max(0.0);
        let span = b - a;
        let above = if (k * (yb - ya)).abs() < 1e-4 {
            (-k * 0.5 * (ya + yb)).exp() * span
        } else {
            ((-k * ya).exp() - (-k * yb).exp()) * span / (k * (yb - ya))
        };
        fog.density * (below + above)
    }

    fn marched(fog: &Fog, origin: Vec3, dir: Vec3, t0: f32, t1: f32) -> f32 {
        let steps = 200_000;
        let dt = f64::from(t1 - t0) / steps as f64;
        let sum: f64 = (0..steps)
            .map(|i| {
                let t = f64::from(t0) + (i as f64 + 0.5) * dt;
                let y = f64::from(origin.y) + f64::from(dir.y) * t - f64::from(fog.height);
                let falloff = if fog.height_falloff > 0.0 && y > 0.0 {
                    (-y / f64::from(fog.height_falloff)).exp()
                } else {
                    1.0
                };
                f64::from(fog.density) * falloff * dt
            })
            .sum();
        sum as f32
    }

    #[test]
    fn the_closed_form_optical_depth_matches_marching() {
        let fog = Fog {
            density: 0.05,
            height: 2.0,
            height_falloff: 5.0,
            ..Fog::default()
        };
        let cases = [
            // Rising out of the fog, sinking into it, level above and below, and
            // starting above the base height.
            (
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(1.0, 0.5, 0.0),
                0.0,
                60.0,
            ),
            (
                Vec3::new(0.0, 12.0, 0.0),
                Vec3::new(1.0, -0.3, 0.0),
                0.0,
                80.0,
            ),
            (Vec3::new(0.0, 6.0, 0.0), Vec3::X, 0.0, 50.0),
            (Vec3::new(0.0, 0.0, 0.0), Vec3::X, 5.0, 50.0),
            (
                Vec3::new(0.0, 4.0, 0.0),
                Vec3::new(0.6, 0.8, 0.0),
                10.0,
                30.0,
            ),
        ];
        for (origin, dir, t0, t1) in cases {
            let dir = dir.normalize();
            let closed = optical_depth(&fog, origin, dir, t0, t1);
            let reference = marched(&fog, origin, dir, t0, t1);
            assert!(
                (closed - reference).abs() <= 1e-3 * reference.max(1.0),
                "{origin} {dir}: {closed} vs {reference}"
            );
        }
        let uniform = Fog {
            height_falloff: 0.0,
            ..fog
        };
        assert!((optical_depth(&uniform, Vec3::ZERO, Vec3::Y, 0.0, 10.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn froxels_follow_the_view_within_the_caps() {
        assert_eq!(VolumetricFog::Off.froxels(1920, 1080), None);
        assert_eq!(
            VolumetricFog::Medium.froxels(1600, 900),
            Some([200, 113, 64])
        );
        assert_eq!(VolumetricFog::Low.froxels(1600, 900), Some([100, 57, 32]));
        let [x, y, _] = VolumetricFog::High.froxels(3840, 2160).unwrap();
        assert!(x <= MAX_FROXELS_X && y <= MAX_FROXELS_Y);
        assert!((x as f32 / y as f32 - 16.0 / 9.0).abs() < 0.05);
        assert_eq!(VolumetricFog::Medium.froxels(1, 1), Some([1, 1, 64]));
    }

    #[test]
    fn the_camera_axes_come_from_the_view() {
        let view = Mat4::look_at_rh(Vec3::new(3.0, 2.0, 5.0), Vec3::new(3.0, 2.0, -5.0), Vec3::Y);
        let camera = FogCamera {
            view,
            projection: Mat4::perspective_rh(1.0, 1.5, 0.1, 500.0),
        };
        assert!(camera
            .position()
            .abs_diff_eq(Vec3::new(3.0, 2.0, 5.0), 1e-4));
        assert!(camera.forward().abs_diff_eq(Vec3::NEG_Z, 1e-5));
    }

    #[test]
    fn the_uniform_matches_the_shader_layout() {
        assert_eq!(std::mem::size_of::<FogUniform>(), 368);
    }

    #[test]
    fn the_fog_shaders_validate() {
        for (label, source) in [
            ("scatter", scatter_source()),
            ("integrate", integrate_source()),
            ("apply", apply_source(false)),
            ("apply msaa", apply_source(true)),
        ] {
            crate::renderer::shader_source::validate(&source)
                .unwrap_or_else(|error| panic!("{label}: {error}"));
        }
    }
}
