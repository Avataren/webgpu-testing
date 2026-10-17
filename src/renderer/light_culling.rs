//! Clustered lighting (Forward+): which point and spot lights can reach each part of the
//! view.
//!
//! The camera's view is cut into clusters: [`CLUSTER_COLUMNS`] columns, as many rows as
//! keep the tiles about square, and [`CLUSTER_SLICES`] slices of view depth, logarithmic
//! between [`SLICE_NEAR`] and [`SLICE_FAR`] (linear for an orthographic camera). Each
//! frame one compute dispatch (`shader/light_culling.wgsl`) lists the lights whose reach
//! (and cone) touches each cluster's box, in a pool of light indices; a lit surface finds
//! its cluster from its world position and loops over that list only. The same dispatch
//! fills a world-space grid over the lights for the passes that shade ray hits, which can
//! be anywhere in the scene (reflections, refraction, light probes, path tracing).
//!
//! The lists live in one storage buffer (`light_lists`, bound with the lights group): a
//! header, an (offset, count) entry per cluster and per grid cell, then the pool. The pool
//! grows when a frame needed more room than it had (read back without stalling), so a
//! frame that overflows leaves some lights out of some clusters for a frame or two.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

use crate::renderer::lights::{
    LightsData, LocalLight, LOCAL_LIGHTS_OFFSET, MAX_LIGHTS_PER_CLUSTER,
};
use crate::renderer::shader_source::wgsl;

/// Cluster columns across the view; rows follow from the aspect ratio.
pub(crate) const CLUSTER_COLUMNS: u32 = 16;
pub(crate) const MAX_CLUSTER_ROWS: u32 = 16;
pub(crate) const CLUSTER_SLICES: u32 = 24;
pub(crate) const MAX_CLUSTERS: u32 = CLUSTER_COLUMNS * MAX_CLUSTER_ROWS * CLUSTER_SLICES;
/// The view depths the logarithmic slices span (within the camera's clip planes): nearer
/// surfaces share the first slice, farther ones the last.
pub(crate) const SLICE_NEAR: f32 = 0.5;
pub(crate) const SLICE_FAR: f32 = 300.0;
/// Cells of the world grid at most, and how far from the camera it reaches.
pub(crate) const MAX_GRID_CELLS: u32 = 16384;
pub(crate) const GRID_REACH: f32 = 256.0;

/// Where things are in the light lists buffer, in u32s.
pub(crate) const LISTS_HEADER: u32 = 4;
pub(crate) const FIRST_CLUSTER_ENTRY: u32 = LISTS_HEADER;
pub(crate) const FIRST_GRID_ENTRY: u32 = FIRST_CLUSTER_ENTRY + 2 * MAX_CLUSTERS;
pub(crate) const POOL_START: u32 = FIRST_GRID_ENTRY + 2 * MAX_GRID_CELLS;
/// Light indices the pool holds at first and at most.
pub(crate) const INITIAL_POOL: u32 = 1 << 18;
pub(crate) const MAX_POOL: u32 = 1 << 23;

const WORKGROUP: u32 = 64;

/// The light lists buffer's size in bytes for a pool of `pool` indices.
pub(crate) fn light_lists_size(pool: u32) -> u64 {
    u64::from(POOL_START + pool) * 4
}

/// The camera clusters are cut for: its view, projection (without TAA jitter) and planes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ClusterCamera {
    pub view: Mat4,
    pub projection: Mat4,
    pub near: f32,
    pub far: f32,
    pub orthographic: bool,
    pub aspect: f32,
}

impl Default for ClusterCamera {
    fn default() -> Self {
        Self {
            view: Mat4::IDENTITY,
            projection: Mat4::perspective_rh(1.0, 1.0, 0.1, 100.0),
            near: 0.1,
            far: 100.0,
            orthographic: false,
            aspect: 1.0,
        }
    }
}

impl ClusterCamera {
    /// The camera's position in the world.
    pub(crate) fn position(&self) -> Vec3 {
        self.view.inverse().w_axis.truncate()
    }
}

/// How the view is cut into clusters this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ClusterGrid {
    pub dims: [u32; 3],
    /// slice = floor(log(depth) * scale + bias), or linear in depth when `linear`.
    pub scale: f32,
    pub bias: f32,
    pub linear: bool,
}

impl ClusterGrid {
    pub(crate) fn new(camera: &ClusterCamera) -> Self {
        let rows = ((CLUSTER_COLUMNS as f32 / camera.aspect.max(1e-3)).round() as u32)
            .clamp(4, MAX_CLUSTER_ROWS);
        let dims = [CLUSTER_COLUMNS, rows, CLUSTER_SLICES];
        let slices = CLUSTER_SLICES as f32;
        let far = camera.far.max(camera.near + 1e-3);
        if camera.orthographic {
            let scale = slices / (far - camera.near);
            return Self {
                dims,
                scale,
                bias: -camera.near * scale,
                linear: true,
            };
        }
        let end = SLICE_FAR.min(far);
        let start = SLICE_NEAR.max(camera.near).min(end * 0.5);
        let scale = slices / (end / start).ln();
        Self {
            dims,
            scale,
            bias: -start.ln() * scale,
            linear: false,
        }
    }

    pub(crate) fn count(&self) -> u32 {
        self.dims[0] * self.dims[1] * self.dims[2]
    }

    /// The cluster holding world point `point` (as `light_cluster` in
    /// lighting_common.wgsl finds it), for tests.
    #[cfg(test)]
    fn cluster_of(&self, camera: &ClusterCamera, point: Vec3) -> u32 {
        let clip = camera.projection * camera.view * point.extend(1.0);
        let ndc = clip.truncate().truncate() / clip.w.max(1e-6);
        let uv = glam::Vec2::new(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
        let x = ((uv.x * self.dims[0] as f32).floor() as i64).clamp(0, self.dims[0] as i64 - 1);
        let y = ((uv.y * self.dims[1] as f32).floor() as i64).clamp(0, self.dims[1] as i64 - 1);
        let depth = -(camera.view * point.extend(1.0)).z;
        let slice = if self.linear {
            depth * self.scale + self.bias
        } else {
            depth.max(1e-4).ln() * self.scale + self.bias
        };
        let slice = (slice.floor() as i64).clamp(0, self.dims[2] as i64 - 1);
        ((slice as u32 * self.dims[1] + y as u32) * self.dims[0]) + x as u32
    }
}

/// A uniform grid over the lights' reach, for passes that shade ray hits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WorldGrid {
    pub origin: Vec3,
    pub cell: f32,
    pub dims: [u32; 3],
}

impl WorldGrid {
    /// The grid around every light's sphere of reach (within [`GRID_REACH`] of
    /// `camera`), with about cubic cells, at most [`MAX_GRID_CELLS`] of them; `None`
    /// without lights there.
    pub(crate) fn fit(lights: &[LocalLight], camera: Vec3) -> Option<Self> {
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for light in lights {
            let reach = Vec3::splat(light.reach());
            lo = lo.min(light.position - reach);
            hi = hi.max(light.position + reach);
        }
        lo = lo.max(camera - Vec3::splat(GRID_REACH));
        hi = hi.min(camera + Vec3::splat(GRID_REACH));
        if !(lo.cmplt(hi).all()) {
            return None;
        }
        let extent = (hi - lo).max(Vec3::splat(1e-3));
        let mut cell = (extent.x * extent.y * extent.z / MAX_GRID_CELLS as f32)
            .cbrt()
            .max(0.25);
        loop {
            let dims = (extent / cell).ceil().max(Vec3::ONE);
            if dims.x * dims.y * dims.z <= MAX_GRID_CELLS as f32 {
                return Some(Self {
                    origin: lo,
                    cell,
                    dims: [dims.x as u32, dims.y as u32, dims.z as u32],
                });
            }
            cell *= 1.1;
        }
    }

    pub(crate) fn count(&self) -> u32 {
        self.dims[0] * self.dims[1] * self.dims[2]
    }
}

/// The clusters, grid and lights of one frame, for the header and the dispatch.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FrameClusters {
    pub camera: ClusterCamera,
    pub grid: ClusterGrid,
    pub world: Option<WorldGrid>,
    pub light_count: u32,
}

impl FrameClusters {
    pub(crate) fn new(camera: ClusterCamera, lights: &LightsData, world_grid: bool) -> Self {
        let local = lights.local_lights();
        Self {
            camera,
            grid: ClusterGrid::new(&camera),
            world: world_grid
                .then(|| WorldGrid::fit(local, camera.position()))
                .flatten(),
            light_count: local.len() as u32,
        }
    }

    /// The header fields lit passes read to find a surface's cluster and grid cell
    /// (see `LightsHeader`).
    pub(crate) fn header_fields(&self) -> ClusterHeader {
        let view = self.camera.view;
        let row = Vec3::new(view.x_axis.z, view.y_axis.z, view.z_axis.z);
        let dims = self.grid.dims;
        let (grid_origin, grid_dims) = match self.world {
            Some(world) => (
                [
                    world.origin.x,
                    world.origin.y,
                    world.origin.z,
                    1.0 / world.cell,
                ],
                [
                    world.dims[0],
                    world.dims[1],
                    world.dims[2],
                    FIRST_GRID_ENTRY,
                ],
            ),
            None => ([0.0; 4], [0, 0, 0, FIRST_GRID_ENTRY]),
        };
        ClusterHeader {
            view_proj: (self.camera.projection * view).to_cols_array_2d(),
            depth_axis: [-row.x, -row.y, -row.z, -view.w_axis.z],
            dims: [dims[0], dims[1], dims[2], self.grid.count()],
            slices: [
                self.grid.scale,
                self.grid.bias,
                0.0,
                if self.grid.linear { 1.0 } else { 0.0 },
            ],
            grid_origin,
            grid_dims,
        }
    }
}

/// See [`FrameClusters::header_fields`].
pub(crate) struct ClusterHeader {
    pub view_proj: [[f32; 4]; 4],
    pub depth_axis: [f32; 4],
    pub dims: [u32; 4],
    pub slices: [f32; 4],
    pub grid_origin: [f32; 4],
    pub grid_dims: [u32; 4],
}

/// Mirrors `CullParams` in light_culling.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
struct CullUniform {
    view: [[f32; 4]; 4],
    inverse_projection: [[f32; 4]; 4],
    cluster_dims: [u32; 4],
    slices: [f32; 4],
    grid_origin: [f32; 4],
    grid_dims: [u32; 4],
    offsets: [u32; 4],
    /// x: the pool's capacity, y: workgroups for clusters, z: the camera's near plane
    /// (floats; both counts stay far below 2^24).
    pool: [f32; 4],
}

/// The light culling dispatch and the readback that sizes its pool.
pub(crate) struct LightCulling {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    last_uniform: Option<CullUniform>,
    /// The bind group and the buffers it was made for.
    bind_group: Option<(wgpu::BindGroup, wgpu::Buffer, wgpu::Buffer)>,
    readback: crate::gpu_particles::system::readback::AsyncReadback,
    /// Light indices the last read-back frame needed, and how many it left out.
    pub(crate) last_needed: u32,
    pub(crate) last_dropped: u32,
}

impl LightCulling {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("LightCullingLayout"),
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
                storage(1, true),
                storage(2, false),
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("LightCullingShader"),
            source: wgpu::ShaderSource::Wgsl(light_culling_shader_source().into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("LightCullingPipelineLayout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("LightCullingPipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("cs_cull"),
            compilation_options: Default::default(),
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("LightCullingUniform"),
            size: std::mem::size_of::<CullUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            layout,
            uniform,
            last_uniform: None,
            bind_group: None,
            readback: crate::gpu_particles::system::readback::AsyncReadback::new(
                device,
                8,
                "LightListsReadback",
            ),
            last_needed: 0,
            last_dropped: 0,
        }
    }

    /// Record the culling dispatch for `frame` into `lights` (the lights buffer, local
    /// lights at [`LOCAL_LIGHTS_OFFSET`]) and `lists` (holding `pool` indices).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn dispatch(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        lights: &wgpu::Buffer,
        lists: &wgpu::Buffer,
        pool: u32,
        frame: &FrameClusters,
    ) {
        if frame.light_count == 0 {
            return;
        }
        let stale = self
            .bind_group
            .as_ref()
            .is_none_or(|(_, bound_lights, bound_lists)| {
                bound_lights != lights || bound_lists != lists
            });
        if stale {
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("LightCullingBindGroup"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: lights,
                            offset: LOCAL_LIGHTS_OFFSET,
                            size: None,
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: lists.as_entire_binding(),
                    },
                ],
            });
            self.bind_group = Some((group, lights.clone(), lists.clone()));
        }

        let cluster_count = frame.grid.count();
        let cells = frame.world.map_or(0, |world| world.count());
        let cluster_groups = cluster_count.div_ceil(WORKGROUP);
        let cell_groups = cells.div_ceil(WORKGROUP);
        let world = frame.world;
        let uniform = CullUniform {
            view: frame.camera.view.to_cols_array_2d(),
            inverse_projection: frame.camera.projection.inverse().to_cols_array_2d(),
            cluster_dims: [
                frame.grid.dims[0],
                frame.grid.dims[1],
                frame.grid.dims[2],
                cluster_count,
            ],
            slices: [
                frame.grid.scale,
                frame.grid.bias,
                if frame.grid.linear { 1.0 } else { 0.0 },
                frame.camera.far,
            ],
            grid_origin: world.map_or([0.0; 4], |world| {
                [world.origin.x, world.origin.y, world.origin.z, world.cell]
            }),
            grid_dims: world.map_or([0; 4], |world| {
                [world.dims[0], world.dims[1], world.dims[2], cells]
            }),
            offsets: [
                frame.light_count,
                FIRST_CLUSTER_ENTRY,
                FIRST_GRID_ENTRY,
                POOL_START,
            ],
            pool: [pool as f32, cluster_groups as f32, frame.camera.near, 0.0],
        };
        if self.last_uniform != Some(uniform) {
            queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
            self.last_uniform = Some(uniform);
        }

        encoder.clear_buffer(lists, 0, Some(u64::from(LISTS_HEADER) * 4));
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("LightCulling"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            if let Some((group, _, _)) = &self.bind_group {
                pass.set_bind_group(0, group, &[]);
            }
            pass.dispatch_workgroups(cluster_groups + cell_groups, 1, 1);
        }
        self.readback.schedule(encoder, lists, 8);
    }

    /// Advance the readback of the pool's fill. Returns the pool size the frames need,
    /// when a read-back frame found the pool too small.
    pub(crate) fn poll(&mut self, device: &wgpu::Device, pool: u32) -> Option<u32> {
        let mut grow = None;
        let mut read = None;
        self.readback.poll(device, |bytes| {
            let words: &[u32] = bytemuck::cast_slice(&bytes[..8]);
            read = Some((words[0], words[1]));
        });
        if let Some((needed, dropped)) = read {
            self.last_needed = needed;
            self.last_dropped = dropped;
            if needed > pool && pool < MAX_POOL {
                // Room for growth, so a slowly changing view does not grow it every frame.
                grow = Some(
                    (needed + needed / 4)
                        .next_power_of_two()
                        .clamp(pool, MAX_POOL),
                );
            }
        }
        grow
    }

    /// The buffers changed size: start the readback afresh.
    pub(crate) fn reset_readback(&mut self) {
        self.readback.reset();
    }
}

/// The culling shader, with the per-cluster cap patched in.
pub(crate) fn light_culling_shader_source() -> String {
    wgsl!("light_culling.wgsl").replace(
        "MAX_LIGHTS_PER_CLUSTER_VALUE",
        &format!("{MAX_LIGHTS_PER_CLUSTER}u"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> ClusterCamera {
        let view = Mat4::look_at_rh(Vec3::new(0.0, 2.0, 10.0), Vec3::new(0.0, 1.0, 0.0), Vec3::Y);
        ClusterCamera {
            view,
            projection: Mat4::perspective_rh(1.0, 16.0 / 9.0, 0.1, 400.0),
            near: 0.1,
            far: 400.0,
            orthographic: false,
            aspect: 16.0 / 9.0,
        }
    }

    #[test]
    fn the_grid_keeps_tiles_square_and_slices_logarithmic() {
        let grid = ClusterGrid::new(&camera());
        assert_eq!(grid.dims, [16, 9, 24]);
        assert!(grid.count() <= MAX_CLUSTERS);
        // Slice edges grow by a constant factor from SLICE_NEAR to SLICE_FAR.
        let edge = |slice: f32| ((slice - grid.bias) / grid.scale).exp();
        assert!((edge(0.0) - SLICE_NEAR).abs() < 1e-3);
        assert!((edge(24.0) - SLICE_FAR).abs() < 0.05);
        assert!((edge(2.0) / edge(1.0) - edge(13.0) / edge(12.0)).abs() < 1e-3);
    }

    #[test]
    fn points_find_the_cluster_of_their_tile_and_depth() {
        let camera = camera();
        let grid = ClusterGrid::new(&camera);
        let ahead = grid.cluster_of(&camera, Vec3::new(0.0, 1.0, 0.0));
        let farther = grid.cluster_of(&camera, Vec3::new(0.0, 1.0, -40.0));
        let tiles = grid.dims[0] * grid.dims[1];
        assert!(farther / tiles > ahead / tiles, "deeper slice");
        let left = grid.cluster_of(&camera, Vec3::new(-3.0, 1.0, 0.0));
        assert_eq!(left / tiles, ahead / tiles, "same depth");
        assert!(
            left % grid.dims[0] < ahead % grid.dims[0],
            "a column further left"
        );
        // Behind the near slice: the first slice.
        assert_eq!(
            grid.cluster_of(&camera, Vec3::new(0.0, 2.0, 9.8)) / tiles,
            0
        );
    }

    #[test]
    fn the_world_grid_covers_the_lights_within_reach_of_the_camera() {
        let lights = [
            LocalLight {
                position: Vec3::new(-20.0, 2.0, 0.0),
                range: 5.0,
                ..LocalLight::default()
            },
            LocalLight {
                position: Vec3::new(20.0, 2.0, 10.0),
                range: 5.0,
                ..LocalLight::default()
            },
        ];
        let grid = WorldGrid::fit(&lights, Vec3::ZERO).unwrap();
        assert!(grid.count() <= MAX_GRID_CELLS);
        assert!(grid.origin.abs_diff_eq(Vec3::new(-25.0, -3.0, -5.0), 1e-4));
        let covered = grid.origin
            + Vec3::new(
                grid.dims[0] as f32,
                grid.dims[1] as f32,
                grid.dims[2] as f32,
            ) * grid.cell;
        assert!(covered.cmpge(Vec3::new(25.0, 7.0, 15.0)).all());
        // A light far away from the camera is left out.
        let far = [LocalLight {
            position: Vec3::new(5000.0, 0.0, 0.0),
            range: 5.0,
            ..LocalLight::default()
        }];
        assert!(WorldGrid::fit(&far, Vec3::ZERO).is_none());
    }

    #[test]
    fn the_culling_shader_validates() {
        crate::renderer::shader_source::validate(&light_culling_shader_source())
            .unwrap_or_else(|error| panic!("{error}"));
    }

    #[test]
    fn the_uniform_matches_the_shader_layout() {
        assert_eq!(std::mem::size_of::<CullUniform>(), 224);
    }
}
