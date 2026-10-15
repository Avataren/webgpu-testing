//! The ray-traced view of the scene: one bottom-level acceleration structure (BLAS) per
//! mesh, a top-level structure (TLAS) rebuilt each frame from the opaque batches, and the
//! geometry a hit shader reads to shade what a ray found.
//!
//! All mesh geometry lives in one pool of storage buffers (positions, normals, uvs and
//! colours, plus mesh-relative indices), so a shader can fetch any hit triangle through
//! one binding. Each BLAS is built from its mesh's range of the pool. The pool only
//! grows; when meshes that are gone or changed waste most of it, it is rebuilt from the
//! meshes still in use.

use std::collections::HashMap;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use glam::Mat4;

use super::RayTracingStats;
use crate::asset::{Assets, Handle, Mesh, MeshData};
use crate::renderer::batch::InstanceSource;
use crate::renderer::internal::PreparedBatches;
use crate::renderer::Material;

/// One vertex as the hit shader reads it (`RtVertex` in rt_reflections.wgsl). The
/// position comes first so the BLAS builds read it straight from the pool.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct RtVertex {
    position: [f32; 3],
    u: f32,
    normal: [f32; 3],
    v: f32,
    color: [f32; 4],
}

/// What a hit shader needs about a TLAS instance (`RtInstance` in the shader), found by
/// the instance's custom data.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct RtInstance {
    vertex_base: u32,
    index_base: u32,
    material_index: u32,
    _padding: u32,
}

const VERTEX_STRIDE: u64 = std::mem::size_of::<RtVertex>() as u64;
const INITIAL_POOL_VERTICES: u64 = 64 * 1024;
const INITIAL_POOL_INDICES: u64 = 192 * 1024;
const INITIAL_INSTANCES: u32 = 256;
/// Frames a mesh may go unused before its BLAS is dropped.
const UNUSED_FRAMES_BEFORE_EVICT: u64 = 600;
/// TLAS instance custom data is 24 bits wide.
const MAX_INSTANCES: u32 = 1 << 24;

struct MeshEntry {
    /// The geometry the BLAS was built from: a different `Arc` means the mesh changed.
    source: Arc<MeshData>,
    blas: wgpu::Blas,
    size: wgpu::BlasTriangleGeometrySizeDescriptor,
    vertex_base: u32,
    index_base: u32,
    needs_build: bool,
    last_used: u64,
}

/// Growable storage buffer that keeps its contents when it grows.
struct PoolBuffer {
    buffer: wgpu::Buffer,
    /// Capacity in elements.
    capacity: u64,
    /// Elements in use.
    len: u64,
    element_size: u64,
    label: &'static str,
}

impl PoolBuffer {
    fn new(device: &wgpu::Device, label: &'static str, element_size: u64, capacity: u64) -> Self {
        Self {
            buffer: Self::create(device, label, element_size * capacity),
            capacity,
            len: 0,
            element_size,
            label,
        }
    }

    fn create(device: &wgpu::Device, label: &'static str, size: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::BLAS_INPUT
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    }

    /// Append `data`, growing (and copying the old contents over in `encoder`) when it
    /// does not fit. Returns the first element's index and whether the buffer changed.
    fn append(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        data: &[u8],
    ) -> (u64, bool) {
        let count = data.len() as u64 / self.element_size;
        let start = self.len;
        let mut grew = false;
        if start + count > self.capacity {
            let capacity = (start + count).next_power_of_two().max(self.capacity * 2);
            let buffer = Self::create(device, self.label, capacity * self.element_size);
            if start > 0 {
                encoder.copy_buffer_to_buffer(
                    &self.buffer,
                    0,
                    &buffer,
                    0,
                    start * self.element_size,
                );
            }
            log::debug!("{} grew to {} elements", self.label, capacity);
            self.buffer = buffer;
            self.capacity = capacity;
            grew = true;
        }
        // Queue writes land before the encoder's copy runs; they touch different ranges.
        queue.write_buffer(&self.buffer, start * self.element_size, data);
        self.len = start + count;
        (start, grew)
    }
}

pub(crate) struct RayTracingScene {
    vertices: PoolBuffer,
    indices: PoolBuffer,
    /// Pool elements still referenced by live entries, to know when to compact.
    live_vertices: u64,
    meshes: HashMap<Handle<Mesh>, MeshEntry>,
    tlas: wgpu::Tlas,
    tlas_capacity: u32,
    /// Instances written into `tlas` last frame (slots past this are already empty).
    tlas_used: u32,
    instances: wgpu::Buffer,
    instance_capacity: u32,
    instance_scratch: Vec<RtInstance>,
    /// Mesh and transform of each entry in `instance_scratch`.
    instance_transforms: Vec<(Handle<Mesh>, [f32; 12])>,
    layout: wgpu::BindGroupLayout,
    bind_group: Option<wgpu::BindGroup>,
    /// The materials buffer `bind_group` was made with (it is replaced when it grows).
    bound_materials: Option<wgpu::Buffer>,
    max_primitives: u32,
    frame: u64,
    stats: RayTracingStats,
}

impl RayTracingScene {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RayTracingSceneLayout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::AccelerationStructure {
                        vertex_return: false,
                    },
                    count: None,
                },
                storage(1),
                storage(2),
                storage(3),
                storage(4),
            ],
        });
        Self {
            vertices: PoolBuffer::new(device, "RtVertexPool", VERTEX_STRIDE, INITIAL_POOL_VERTICES),
            indices: PoolBuffer::new(device, "RtIndexPool", 4, INITIAL_POOL_INDICES),
            live_vertices: 0,
            meshes: HashMap::new(),
            tlas: Self::create_tlas(device, INITIAL_INSTANCES),
            tlas_capacity: INITIAL_INSTANCES,
            tlas_used: 0,
            instances: Self::create_instances(device, INITIAL_INSTANCES),
            instance_capacity: INITIAL_INSTANCES,
            instance_scratch: Vec::new(),
            instance_transforms: Vec::new(),
            layout,
            bind_group: None,
            bound_materials: None,
            max_primitives: device.limits().max_blas_primitive_count,
            frame: 0,
            stats: RayTracingStats::default(),
        }
    }

    fn create_tlas(device: &wgpu::Device, max_instances: u32) -> wgpu::Tlas {
        device.create_tlas(&wgpu::CreateTlasDescriptor {
            label: Some("SceneTlas"),
            max_instances,
            flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
            update_mode: wgpu::AccelerationStructureUpdateMode::Build,
        })
    }

    fn create_instances(device: &wgpu::Device, capacity: u32) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RtInstances"),
            size: capacity as u64 * std::mem::size_of::<RtInstance>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Layout of [`Self::bind_group`]: TLAS, vertices, indices, instances, materials.
    pub(crate) fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    pub(crate) fn bind_group(&self) -> Option<&wgpu::BindGroup> {
        self.bind_group.as_ref()
    }

    pub(crate) fn stats(&self) -> RayTracingStats {
        self.stats
    }

    /// Bring the acceleration structures up to date with this frame's opaque batches and
    /// record their builds into `encoder`. `materials` is the buffer the batches'
    /// material indices refer to.
    pub(crate) fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        assets: &Assets,
        batches: &PreparedBatches,
        materials: &wgpu::Buffer,
    ) {
        self.frame += 1;
        let mut buffers_changed = self.compact_if_wasteful();

        // Meshes first: the instances below need their pool offsets and BLAS.
        let opaque = &batches.batches[batches.opaque_range.clone()];
        for batch in opaque {
            if !Self::traceable(batch) {
                continue;
            }
            buffers_changed |= self.ensure_mesh(device, queue, encoder, assets, batch.mesh);
        }

        self.instance_scratch.clear();
        self.instance_transforms.clear();
        for batch in opaque {
            if !Self::traceable(batch) {
                continue;
            }
            let Some(entry) = self.meshes.get(&batch.mesh) else {
                continue;
            };
            for instance in &batch.instances {
                if instance.source != InstanceSource::Cpu
                    || self.instance_scratch.len() as u32 >= MAX_INSTANCES
                {
                    continue;
                }
                let material = batches.materials.get(instance.material_index as usize);
                if material.is_none_or(Material::is_billboarded) {
                    // Billboards turn to the camera in the vertex shader; a ray would see
                    // them edge-on or from behind.
                    continue;
                }
                self.instance_scratch.push(RtInstance {
                    vertex_base: entry.vertex_base,
                    index_base: entry.index_base,
                    material_index: instance.material_index,
                    _padding: 0,
                });
                self.instance_transforms
                    .push((batch.mesh, affine_rows(instance.transform.matrix())));
            }
        }
        let count = self.instance_scratch.len() as u32;

        if count > self.tlas_capacity {
            let capacity = count.next_power_of_two();
            self.tlas = Self::create_tlas(device, capacity);
            self.tlas_capacity = capacity;
            self.tlas_used = 0;
            buffers_changed = true;
        }
        // The custom data finds the instance's entry in `instances`.
        for (slot, (mesh, transform)) in self.instance_transforms.iter().enumerate() {
            let blas = &self.meshes[mesh].blas;
            self.tlas[slot] = Some(wgpu::TlasInstance::new(blas, *transform, slot as u32, 0xff));
        }
        for slot in count..self.tlas_used {
            self.tlas[slot as usize] = None;
        }
        self.tlas_used = count;

        if count > self.instance_capacity {
            let capacity = count.next_power_of_two();
            self.instances = Self::create_instances(device, capacity);
            self.instance_capacity = capacity;
            buffers_changed = true;
        }
        if count > 0 {
            queue.write_buffer(
                &self.instances,
                0,
                bytemuck::cast_slice(&self.instance_scratch),
            );
        }

        let blas_builds = {
            let pending: Vec<&MeshEntry> = self
                .meshes
                .values()
                .filter(|entry| entry.needs_build)
                .collect();
            let entries: Vec<wgpu::BlasBuildEntry<'_>> = pending
                .iter()
                .map(|entry| wgpu::BlasBuildEntry {
                    blas: &entry.blas,
                    geometry: wgpu::BlasGeometries::TriangleGeometries(vec![
                        wgpu::BlasTriangleGeometry {
                            size: &entry.size,
                            vertex_buffer: &self.vertices.buffer,
                            first_vertex: entry.vertex_base,
                            vertex_stride: VERTEX_STRIDE,
                            index_buffer: Some(&self.indices.buffer),
                            first_index: Some(entry.index_base),
                            transform_buffer: None,
                            transform_buffer_offset: None,
                        },
                    ]),
                })
                .collect();
            encoder.build_acceleration_structures(entries.iter(), std::iter::once(&self.tlas));
            entries.len() as u32
        };
        for entry in self.meshes.values_mut() {
            entry.needs_build = false;
        }

        self.evict_unused();

        if buffers_changed
            || self.bind_group.is_none()
            || self.bound_materials.as_ref() != Some(materials)
        {
            self.bind_group = Some(self.create_bind_group(device, materials));
            self.bound_materials = Some(materials.clone());
        }

        self.stats = RayTracingStats {
            instances: count,
            meshes: self.meshes.len() as u32,
            blas_builds,
            pool_vertices: self.vertices.len as u32,
        };
    }

    /// Opaque, rigid batches. Skinned meshes are deformed per instance on the GPU and are
    /// not traced yet.
    fn traceable(batch: &crate::renderer::internal::OrderedBatch) -> bool {
        !batch.alpha_blend && batch.skin.is_none()
    }

    /// Make sure `handle` has an up-to-date BLAS, uploading its geometry to the pool when
    /// it is new or changed. Returns whether a pool buffer was replaced.
    fn ensure_mesh(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        assets: &Assets,
        handle: Handle<Mesh>,
    ) -> bool {
        if assets.meshes.is_released(handle) {
            return false;
        }
        let Some(mesh) = assets.meshes.get(handle) else {
            return false;
        };
        let source = mesh.shared_data();
        if let Some(entry) = self.meshes.get_mut(&handle) {
            if Arc::ptr_eq(&entry.source, source) {
                entry.last_used = self.frame;
                return false;
            }
        }
        if let Some(stale) = self.meshes.remove(&handle) {
            self.live_vertices -= stale.size.vertex_count as u64;
        }

        let data: &MeshData = source;
        let triangles = data.indices.len() / 3;
        if data.vertices.is_empty()
            || triangles == 0
            || triangles as u64 > self.max_primitives as u64
        {
            return false;
        }
        let vertex_count = data.vertices.len() as u32;
        let indices = &data.indices[..triangles * 3];
        if indices.iter().any(|&index| index >= vertex_count) {
            log::warn!("Ray tracing skips mesh {handle:?}: index out of range");
            return false;
        }

        let vertices: Vec<RtVertex> = data
            .vertices
            .iter()
            .map(|v| RtVertex {
                position: v.pos,
                u: v.uv[0],
                normal: v.normal,
                v: v.uv[1],
                color: v.color,
            })
            .collect();
        let (vertex_base, vertices_grew) =
            self.vertices
                .append(device, queue, encoder, bytemuck::cast_slice(&vertices));
        let (index_base, indices_grew) =
            self.indices
                .append(device, queue, encoder, bytemuck::cast_slice(indices));

        let size = wgpu::BlasTriangleGeometrySizeDescriptor {
            vertex_format: wgpu::VertexFormat::Float32x3,
            vertex_count,
            index_format: Some(wgpu::IndexFormat::Uint32),
            index_count: Some(indices.len() as u32),
            flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
        };
        let blas = device.create_blas(
            &wgpu::CreateBlasDescriptor {
                label: Some("MeshBlas"),
                flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                update_mode: wgpu::AccelerationStructureUpdateMode::Build,
            },
            wgpu::BlasGeometrySizeDescriptors::Triangles {
                descriptors: vec![size.clone()],
            },
        );
        self.live_vertices += vertex_count as u64;
        self.meshes.insert(
            handle,
            MeshEntry {
                source: Arc::clone(source),
                blas,
                size,
                vertex_base: vertex_base as u32,
                index_base: index_base as u32,
                needs_build: true,
                last_used: self.frame,
            },
        );
        vertices_grew || indices_grew
    }

    fn evict_unused(&mut self) {
        let frame = self.frame;
        let mut freed = 0u64;
        self.meshes.retain(|_, entry| {
            let keep = frame - entry.last_used < UNUSED_FRAMES_BEFORE_EVICT;
            if !keep {
                freed += entry.size.vertex_count as u64;
            }
            keep
        });
        self.live_vertices -= freed;
    }

    /// When most of the pool is dead space, forget every mesh so the ones still in use
    /// are uploaded and built again from an empty pool. Returns whether that happened.
    fn compact_if_wasteful(&mut self) -> bool {
        let used = self.vertices.len;
        if used < INITIAL_POOL_VERTICES || self.live_vertices * 2 > used {
            return false;
        }
        log::debug!(
            "Compacting ray tracing geometry pool ({} live of {} vertices)",
            self.live_vertices,
            used
        );
        self.meshes.clear();
        self.live_vertices = 0;
        self.vertices.len = 0;
        self.indices.len = 0;
        true
    }

    fn create_bind_group(
        &self,
        device: &wgpu::Device,
        materials: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("RayTracingSceneBindGroup"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.tlas.as_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.vertices.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.indices.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.instances.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: materials.as_entire_binding(),
                },
            ],
        })
    }
}

/// A model matrix as the 3x4 row-major affine transform a TLAS instance takes.
fn affine_rows(model: Mat4) -> [f32; 12] {
    let m = model.to_cols_array_2d();
    [
        m[0][0], m[1][0], m[2][0], m[3][0], //
        m[0][1], m[1][1], m[2][1], m[3][1], //
        m[0][2], m[1][2], m[2][2], m[3][2],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Quat, Vec3, Vec4};

    #[test]
    fn affine_rows_transform_points_like_the_matrix() {
        let model = Mat4::from_scale_rotation_translation(
            Vec3::new(1.0, 2.0, 3.0),
            Quat::from_rotation_y(0.7),
            Vec3::new(4.0, -5.0, 6.0),
        );
        let rows = affine_rows(model);
        let point = Vec4::new(0.3, -1.2, 2.5, 1.0);
        let expected = model * point;
        for row in 0..3 {
            let r = Vec4::from_slice(&rows[row * 4..row * 4 + 4]);
            assert!((r.dot(point) - expected[row]).abs() < 1e-5);
        }
    }

    #[test]
    fn gpu_structs_match_the_shader_layout() {
        assert_eq!(std::mem::size_of::<RtVertex>(), 48);
        assert_eq!(std::mem::size_of::<RtInstance>(), 16);
    }
}
