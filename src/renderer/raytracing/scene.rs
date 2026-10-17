//! The ray-traced view of the scene: one bottom-level acceleration structure (BLAS) per
//! mesh and per skinned instance, a top-level structure (TLAS) rebuilt each frame from the
//! opaque batches, and the geometry a hit shader reads to shade what a ray found.
//!
//! All geometry lives in one pool of storage buffers (vertices in the engine's own
//! [`Vertex`] layout, plus mesh-relative indices), so a shader can fetch any hit triangle
//! through one binding. Each BLAS is built from its range of the pool. A rigid mesh is
//! uploaded and built once; a skinned instance gets a range of its own, which each frame
//! receives a GPU copy of the vertices the skinning pass just deformed, and its BLAS is
//! rebuilt from it (wgpu 30 cannot refit). The pool only grows; when geometry that is gone
//! or changed wastes most of it, it is rebuilt from what is still in use.
//!
//! GPU particle systems handed in by the application are geometry too: each keeps its
//! particle mesh (the template) and room for a copy per particle slot in the pool. Every
//! frame a compute pass places a copy at each live particle, as the particle shader
//! does, and the system's BLAS is rebuilt from the slots in use. Opaque particles are
//! instanced like any surface; blended ones under their own mask, for reflection rays to
//! gather as they pass through them instead of stopping.

use crate::renderer::shader_source::wgsl;
use std::collections::HashMap;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

use super::{RayTracedParticleBlend, RayTracedParticles, RayTracingStats};
use crate::asset::{Assets, Handle, Mesh, MeshData};
use crate::renderer::batch::{InstanceSource, SkinKey};
use crate::renderer::internal::PreparedBatches;
use crate::renderer::material::MaterialFlags;
use crate::renderer::skinning::SkinningStage;
use crate::renderer::{MaterialData, Vertex};

/// What a hit shader needs about a TLAS instance (`RtInstance` in the shader), found by
/// the instance's custom data. `material_index` indexes the scene's materials, or the
/// particle materials for a particle system (`INSTANCE_PARTICLES`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct RtInstance {
    vertex_base: u32,
    index_base: u32,
    material_index: u32,
    flags: u32,
}

/// `RtInstance::flags` (RT_INSTANCE_* in rt_reflections.wgsl).
const INSTANCE_PARTICLES: u32 = 1;
const INSTANCE_ADDITIVE: u32 = 2;

/// The camera billboarded particles turn to, as the particle shader turns them.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RtCamera {
    pub forward: Vec3,
    pub up: Vec3,
}

/// `ExpandParams` in rt_particles.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct ExpandParams {
    camera_forward: [f32; 4],
    camera_up: [f32; 4],
    template_base: u32,
    template_count: u32,
    output_base: u32,
    particle_count: u32,
    billboarded: u32,
    _padding: [u32; 3],
}

const EXPAND_WORKGROUP_SIZE: u32 = 64;
/// Most vertices one particle system may expand to (one dispatch row of workgroups).
const MAX_PARTICLE_VERTICES: u64 = 65535 * EXPAND_WORKGROUP_SIZE as u64;
const INITIAL_PARTICLE_MATERIALS: u64 = 16;
/// Most surfaces caustic photons are aimed at (`CAUSTIC_MAX_CASTERS` in pt_caustics.wgsl).
pub(crate) const MAX_CAUSTIC_CASTERS: usize = 32;
/// Metals at least this metallic and this smooth reflect light into caustics.
const CAUSTIC_MIN_METALLIC: f32 = 0.5;
const CAUSTIC_MAX_ROUGHNESS: f32 = 0.2;

/// Pool vertices are the engine's [`Vertex`] (`RtVertex` in rt_reflections.wgsl), so
/// skinned vertex buffers copy straight in and BLAS builds read the position at offset 0.
const VERTEX_STRIDE: u64 = std::mem::size_of::<Vertex>() as u64;
const INITIAL_POOL_VERTICES: u64 = 64 * 1024;
const INITIAL_POOL_INDICES: u64 = 192 * 1024;
const INITIAL_INSTANCES: u32 = 256;
/// Frames a mesh may go unused before its BLAS is dropped.
const UNUSED_FRAMES_BEFORE_EVICT: u64 = 600;
/// TLAS instance custom data is 24 bits wide.
const MAX_INSTANCES: u32 = 1 << 24;
/// TLAS instance masks: rays that only ask whether light gets through (shadows, ambient
/// occlusion) cull with `MASK_OPAQUE` and pass transmissive surfaces such as glass;
/// reflection and refraction rays see both.
pub(crate) const MASK_OPAQUE: u8 = 0x01;
pub(crate) const MASK_TRANSMISSIVE: u8 = 0x02;
/// Particles stop reflection and refraction rays like surfaces when opaque; blended ones
/// are only gathered along a reflection ray (by a query of their own, with this mask).
pub(crate) const MASK_PARTICLES_OPAQUE: u8 = 0x04;
pub(crate) const MASK_PARTICLES_BLENDED: u8 = 0x08;
/// Opaque surfaces that cast no shadow (`cast_shadow: false`, left out of the shadow
/// maps): reflection, refraction and probe rays see them, but shadow and ambient
/// occlusion rays pass them as the shadow maps do, so a light inside a lantern that casts
/// no shadow still gets out.
pub(crate) const MASK_OPAQUE_NO_SHADOW: u8 = 0x10;

/// What a BLAS is built from: a rigid mesh, one skinned instance's deformed copy, or a
/// particle system (by [`RayTracedParticles::id`]). Meshes drawn with cut-out materials
/// get a BLAS of their own (`masked`), built non-opaque so ray queries alpha-test their
/// hits (rt_alpha.wgsl).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum GeometryKey {
    Mesh { mesh: Handle<Mesh>, masked: bool },
    Skin { skin: SkinKey, masked: bool },
    Particles(u64),
}

impl GeometryKey {
    fn masked(self) -> bool {
        matches!(
            self,
            Self::Mesh { masked: true, .. } | Self::Skin { masked: true, .. }
        )
    }
}

/// A particle system's geometry in the pool: its mesh once (the template), then room for
/// a copy per particle slot (the output), with indices for every slot written once.
struct ParticleEntry {
    /// The mesh the template came from: a different `Arc` means it changed.
    source: Arc<MeshData>,
    capacity: u32,
    blend: RayTracedParticleBlend,
    blas: wgpu::Blas,
    /// Flags the BLAS was created with; every build must repeat them.
    geometry_flags: wgpu::AccelerationStructureGeometryFlags,
    template_base: u32,
    template_vertices: u32,
    /// Indices per particle (the mesh's).
    template_indices: u32,
    vertex_base: u32,
    index_base: u32,
    /// Slots expanded and built this frame; 0 when the system did not draw.
    count: u32,
    billboarded: bool,
    material: MaterialData,
    /// The system's particles, which the expansion reads.
    particles: wgpu::Buffer,
    params: wgpu::Buffer,
    /// The expansion's bindings, and the pool buffer they were made with.
    bind_group: Option<(wgpu::BindGroup, wgpu::Buffer)>,
    last_used: u64,
}

impl ParticleEntry {
    fn pool_vertices(&self) -> u64 {
        self.template_vertices as u64 * (1 + self.capacity as u64)
    }

    /// The BLAS geometry for this frame's slots.
    fn build_size(&self) -> wgpu::BlasTriangleGeometrySizeDescriptor {
        wgpu::BlasTriangleGeometrySizeDescriptor {
            vertex_format: wgpu::VertexFormat::Float32x3,
            vertex_count: self.count * self.template_vertices,
            index_format: Some(wgpu::IndexFormat::Uint32),
            index_count: Some(self.count * self.template_indices),
            flags: self.geometry_flags,
        }
    }
}

/// The compute pipeline that places particles (rt_particles.wgsl).
struct ExpandPipeline {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

impl ExpandPipeline {
    fn new(device: &wgpu::Device) -> Self {
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
            label: Some("RtParticleExpandLayout"),
            entries: &[
                storage(0, true),
                storage(1, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RtParticleExpandShader"),
            source: wgpu::ShaderSource::Wgsl(wgsl!("rt_particles.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("RtParticleExpandPipelineLayout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("RtParticleExpandPipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("cs_expand_particles"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self { pipeline, layout }
    }
}

struct MeshEntry {
    /// The geometry the BLAS was built from: a different `Arc` means the mesh changed.
    source: Arc<MeshData>,
    blas: wgpu::Blas,
    size: wgpu::BlasTriangleGeometrySizeDescriptor,
    vertex_base: u32,
    index_base: u32,
    needs_build: bool,
    last_used: u64,
    /// Bounding sphere in object space (centre, radius), for caustic photons.
    bounds: [f32; 4],
}

/// Growable storage buffer that keeps its contents when it grows, up to what the device
/// can bind as one storage buffer.
struct PoolBuffer {
    buffer: wgpu::Buffer,
    /// Capacity in elements.
    capacity: u64,
    /// Elements in use.
    len: u64,
    /// Most elements the device can bind (and allocate) in one buffer.
    max_elements: u64,
    element_size: u64,
    label: &'static str,
}

impl PoolBuffer {
    fn new(device: &wgpu::Device, label: &'static str, element_size: u64, capacity: u64) -> Self {
        let limits = device.limits();
        let max_bytes = limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size);
        let max_elements = max_bytes / element_size;
        let capacity = capacity.min(max_elements);
        Self {
            buffer: Self::create(device, label, element_size * capacity),
            capacity,
            len: 0,
            max_elements,
            element_size,
            label,
        }
    }

    /// Whether `count` more elements fit without passing the device's limit.
    fn fits(&self, count: u64) -> bool {
        self.len + count <= self.max_elements
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
    /// Callers check [`Self::fits`] first: the buffer never grows past the device's limit.
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
            let capacity = (start + count)
                .next_power_of_two()
                .max(self.capacity * 2)
                .min(self.max_elements);
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
    meshes: HashMap<GeometryKey, MeshEntry>,
    /// Particle systems by [`RayTracedParticles::id`]; dropped the first frame one is not
    /// handed in.
    particles: HashMap<u64, ParticleEntry>,
    expand_pipeline: Option<ExpandPipeline>,
    /// The particle systems' materials (`rt_particle_materials` in the shader).
    particle_materials: wgpu::Buffer,
    particle_material_capacity: u64,
    particle_material_scratch: Vec<MaterialData>,
    /// This frame's skinned vertex buffers and where in the pool they are copied.
    /// `(source buffer, source offset, pool offset, size)` in bytes.
    deformed_copies: Vec<(wgpu::Buffer, u64, u64, u64)>,
    tlas: wgpu::Tlas,
    tlas_capacity: u32,
    /// Instances written into `tlas` last frame (slots past this are already empty).
    tlas_used: u32,
    /// The TLAS has been built at least once, so it can be used as it is.
    tlas_built: bool,
    /// What the TLAS was last built from (see [`Self::instance_signature`]): with the
    /// same instances and no geometry to build, a frame keeps it instead of rebuilding.
    last_instance_signature: Option<u64>,
    instances: wgpu::Buffer,
    instance_capacity: u32,
    instance_scratch: Vec<RtInstance>,
    /// Geometry, transform and mask of each entry in `instance_scratch`.
    instance_transforms: Vec<(GeometryKey, [f32; 12], u8)>,
    /// World bounding spheres of what focuses light (see [`Self::caustic_casters`]).
    casters: Vec<[f32; 4]>,
    layout: wgpu::BindGroupLayout,
    bind_group: Option<wgpu::BindGroup>,
    /// The materials buffer `bind_group` was made with (it is replaced when it grows).
    bound_materials: Option<wgpu::Buffer>,
    max_primitives: u32,
    /// Geometry was left out because the pool is full (warned once).
    warned_pool_full: bool,
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
                storage(5),
            ],
        });
        Self {
            vertices: PoolBuffer::new(device, "RtVertexPool", VERTEX_STRIDE, INITIAL_POOL_VERTICES),
            indices: PoolBuffer::new(device, "RtIndexPool", 4, INITIAL_POOL_INDICES),
            live_vertices: 0,
            meshes: HashMap::new(),
            particles: HashMap::new(),
            expand_pipeline: None,
            particle_materials: Self::create_particle_materials(device, INITIAL_PARTICLE_MATERIALS),
            particle_material_capacity: INITIAL_PARTICLE_MATERIALS,
            particle_material_scratch: Vec::new(),
            deformed_copies: Vec::new(),
            tlas: Self::create_tlas(device, INITIAL_INSTANCES),
            tlas_capacity: INITIAL_INSTANCES,
            tlas_used: 0,
            tlas_built: false,
            last_instance_signature: None,
            instances: Self::create_instances(device, INITIAL_INSTANCES),
            instance_capacity: INITIAL_INSTANCES,
            instance_scratch: Vec::new(),
            instance_transforms: Vec::new(),
            casters: Vec::new(),
            layout,
            bind_group: None,
            bound_materials: None,
            max_primitives: device.limits().max_blas_primitive_count,
            warned_pool_full: false,
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

    fn create_particle_materials(device: &wgpu::Device, capacity: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RtParticleMaterials"),
            size: capacity * std::mem::size_of::<MaterialData>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Layout of [`Self::bind_group`]: TLAS, vertices, indices, instances, materials,
    /// particle materials.
    pub(crate) fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    pub(crate) fn bind_group(&self) -> Option<&wgpu::BindGroup> {
        self.bind_group.as_ref()
    }

    /// World bounding spheres (centre, radius) of the surfaces that focus light into
    /// caustics: transmissive ones, then smooth metals, at most [`MAX_CAUSTIC_CASTERS`].
    pub(crate) fn caustic_casters(&self) -> &[[f32; 4]] {
        &self.casters
    }

    /// A hash of this frame's instance list: each instance's geometry, transform, mask
    /// and shader-side entry.
    fn instance_signature(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for (key, transform, mask) in &self.instance_transforms {
            key.hash(&mut hasher);
            for value in transform {
                value.to_bits().hash(&mut hasher);
            }
            mask.hash(&mut hasher);
        }
        for instance in &self.instance_scratch {
            (
                instance.vertex_base,
                instance.index_base,
                instance.material_index,
                instance.flags,
            )
                .hash(&mut hasher);
        }
        hasher.finish()
    }

    pub(crate) fn stats(&self) -> RayTracingStats {
        self.stats
    }

    /// Bring the acceleration structures up to date with this frame's opaque batches and
    /// particle systems and record their builds into `encoder`, after the skinning pass
    /// `skinning` recorded. `materials` is the buffer the batches' material indices refer
    /// to; `camera` is what billboarded particles face.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        assets: &Assets,
        batches: &PreparedBatches,
        skinning: &SkinningStage,
        materials: &wgpu::Buffer,
        particles: &[RayTracedParticles],
        camera: RtCamera,
    ) {
        self.frame += 1;
        let mut buffers_changed = self.compact_if_wasteful();

        // Geometry first: the instances below need their pool offsets and BLAS.
        self.deformed_copies.clear();
        let opaque = &batches.batches[batches.opaque_range.clone()];
        for batch in opaque {
            if batch.alpha_blend {
                continue;
            }
            buffers_changed |=
                self.ensure_geometry(device, queue, encoder, assets, skinning, batch);
        }
        // Only systems handed in this frame are expanded and instanced.
        for entry in self.particles.values_mut() {
            entry.count = 0;
        }
        for system in particles {
            buffers_changed |= self.ensure_particles(device, queue, encoder, assets, system);
        }
        // After any pool growth above, which copies the old contents in this encoder.
        for (buffer, source_offset, offset, size) in &self.deformed_copies {
            encoder.copy_buffer_to_buffer(
                buffer,
                *source_offset,
                &self.vertices.buffer,
                *offset,
                *size,
            );
        }
        self.expand_particles(device, queue, encoder, camera);

        self.instance_scratch.clear();
        self.instance_transforms.clear();
        self.casters.clear();
        let mut metal_casters = Vec::new();
        for batch in opaque {
            if batch.alpha_blend {
                continue;
            }
            let key = Self::geometry_key(batch);
            let Some(entry) = self.meshes.get(&key) else {
                continue;
            };
            for instance in &batch.instances {
                if instance.source != InstanceSource::Cpu
                    || self.instance_scratch.len() as u32 >= MAX_INSTANCES
                {
                    continue;
                }
                let Some(material) = batches.materials.get(instance.material_index as usize) else {
                    continue;
                };
                if material.is_billboarded() {
                    // Billboards turn to the camera in the vertex shader; a ray would see
                    // them edge-on or from behind.
                    continue;
                }
                let mask = if material.is_transmissive() {
                    MASK_TRANSMISSIVE
                } else if material.flags.contains(MaterialFlags::NO_SHADOW) {
                    MASK_OPAQUE_NO_SHADOW
                } else {
                    MASK_OPAQUE
                };
                let caster = if material.is_transmissive() {
                    Some(&mut self.casters)
                } else if material.metallic_f32() >= CAUSTIC_MIN_METALLIC
                    && material.roughness_f32() <= CAUSTIC_MAX_ROUGHNESS
                {
                    Some(&mut metal_casters)
                } else {
                    None
                };
                if let Some(list) = caster {
                    list.push(world_sphere(entry.bounds, instance.transform.matrix()));
                }
                self.instance_scratch.push(RtInstance {
                    vertex_base: entry.vertex_base,
                    index_base: entry.index_base,
                    material_index: instance.material_index,
                    flags: 0,
                });
                self.instance_transforms.push((
                    key,
                    affine_rows(instance.transform.matrix()),
                    mask,
                ));
            }
        }
        // Glass first: past the limit, mirrors are left out before it is.
        self.casters.extend(metal_casters);
        self.casters.truncate(MAX_CAUSTIC_CASTERS);

        // Particle systems: their geometry is in world space already. Each one's material
        // goes into the particle materials, in instance order.
        self.particle_material_scratch.clear();
        let mut particle_count = 0u32;
        let mut ids: Vec<u64> = self
            .particles
            .iter()
            .filter(|(_, entry)| entry.count > 0)
            .map(|(id, _)| *id)
            .collect();
        ids.sort_unstable();
        for id in ids {
            if self.instance_scratch.len() as u32 >= MAX_INSTANCES {
                break;
            }
            let entry = &self.particles[&id];
            let (mask, flags) = match entry.blend {
                RayTracedParticleBlend::Opaque => (MASK_PARTICLES_OPAQUE, INSTANCE_PARTICLES),
                RayTracedParticleBlend::AlphaBlend => (MASK_PARTICLES_BLENDED, INSTANCE_PARTICLES),
                RayTracedParticleBlend::Additive => (
                    MASK_PARTICLES_BLENDED,
                    INSTANCE_PARTICLES | INSTANCE_ADDITIVE,
                ),
            };
            self.instance_scratch.push(RtInstance {
                vertex_base: entry.vertex_base,
                index_base: entry.index_base,
                material_index: self.particle_material_scratch.len() as u32,
                flags,
            });
            self.particle_material_scratch.push(entry.material);
            self.instance_transforms.push((
                GeometryKey::Particles(id),
                affine_rows(Mat4::IDENTITY),
                mask,
            ));
            particle_count += entry.count;
        }
        let particle_systems = self.particle_material_scratch.len() as u32;
        if particle_systems as u64 > self.particle_material_capacity {
            let capacity = (particle_systems as u64).next_power_of_two();
            self.particle_materials = Self::create_particle_materials(device, capacity);
            self.particle_material_capacity = capacity;
            buffers_changed = true;
        }
        if particle_systems > 0 {
            queue.write_buffer(
                &self.particle_materials,
                0,
                bytemuck::cast_slice(&self.particle_material_scratch),
            );
        }
        let count = self.instance_scratch.len() as u32;

        if count > self.tlas_capacity {
            let capacity = count.next_power_of_two();
            self.tlas = Self::create_tlas(device, capacity);
            self.tlas_capacity = capacity;
            self.tlas_used = 0;
            buffers_changed = true;
        }
        if count > self.instance_capacity {
            let capacity = count.next_power_of_two();
            self.instances = Self::create_instances(device, capacity);
            self.instance_capacity = capacity;
            buffers_changed = true;
        }

        // Nothing to build and the same instances as last frame: the structures still
        // describe the scene, so the frame keeps them (a static scene then costs no
        // rebuild).
        let pending_builds = self.meshes.values().any(|entry| entry.needs_build)
            || self.particles.values().any(|entry| entry.count > 0);
        let signature = self.instance_signature();
        let unchanged = self.tlas_built
            && !buffers_changed
            && !pending_builds
            && count == self.tlas_used
            && self.last_instance_signature == Some(signature);
        let blas_builds = if unchanged {
            0
        } else {
            // The custom data finds the instance's entry in `instances`.
            for (slot, (key, transform, mask)) in self.instance_transforms.iter().enumerate() {
                let blas = match key {
                    GeometryKey::Particles(id) => &self.particles[id].blas,
                    _ => &self.meshes[key].blas,
                };
                self.tlas[slot] = Some(wgpu::TlasInstance::new(
                    blas,
                    *transform,
                    slot as u32,
                    *mask,
                ));
            }
            for slot in count..self.tlas_used {
                self.tlas[slot as usize] = None;
            }
            self.tlas_used = count;
            if count > 0 {
                queue.write_buffer(
                    &self.instances,
                    0,
                    bytemuck::cast_slice(&self.instance_scratch),
                );
            }

            let pending: Vec<&MeshEntry> = self
                .meshes
                .values()
                .filter(|entry| entry.needs_build)
                .collect();
            let particle_sizes: Vec<(&ParticleEntry, wgpu::BlasTriangleGeometrySizeDescriptor)> =
                self.particles
                    .values()
                    .filter(|entry| entry.count > 0)
                    .map(|entry| (entry, entry.build_size()))
                    .collect();
            let particle_builds = particle_sizes
                .iter()
                .map(|(entry, size)| wgpu::BlasBuildEntry {
                    blas: &entry.blas,
                    geometry: wgpu::BlasGeometries::TriangleGeometries(vec![
                        wgpu::BlasTriangleGeometry {
                            size,
                            vertex_buffer: &self.vertices.buffer,
                            first_vertex: entry.vertex_base,
                            vertex_stride: VERTEX_STRIDE,
                            index_buffer: Some(&self.indices.buffer),
                            first_index: Some(entry.index_base),
                            transform_buffer: None,
                            transform_buffer_offset: None,
                        },
                    ]),
                });
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
                .chain(particle_builds)
                .collect();
            encoder.build_acceleration_structures(entries.iter(), std::iter::once(&self.tlas));
            let built = entries.len() as u32;
            drop(entries);
            drop(particle_sizes);
            drop(pending);
            for entry in self.meshes.values_mut() {
                entry.needs_build = false;
            }
            self.tlas_built = true;
            self.last_instance_signature = Some(signature);
            built
        };

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
            particle_systems,
            particles: particle_count,
        };
    }

    /// Make sure `system` has its template, output range and BLAS in the pool (creating
    /// them when new, or when its mesh, capacity or blend changed), and note how many of
    /// its slots to expand this frame. Returns whether a pool buffer was replaced.
    fn ensure_particles(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        assets: &Assets,
        system: &RayTracedParticles,
    ) -> bool {
        if system.count == 0 || system.capacity == 0 {
            return false;
        }
        let mesh = if assets.meshes.is_released(system.mesh) {
            None
        } else {
            assets.meshes.get(system.mesh)
        };
        let Some(mesh) = mesh else {
            return false;
        };
        let source = mesh.shared_data();
        let frame = self.frame;
        let material = MaterialData::from_material(&system.material);
        let billboarded = system.material.is_billboarded();
        if let Some(entry) = self.particles.get_mut(&system.id) {
            if Arc::ptr_eq(&entry.source, source)
                && entry.capacity == system.capacity
                && entry.blend == system.blend
            {
                entry.count = system.count.min(entry.capacity);
                entry.material = material;
                entry.billboarded = billboarded;
                entry.last_used = frame;
                if entry.particles != system.particles {
                    entry.particles = system.particles.clone();
                    entry.bind_group = None;
                }
                return false;
            }
        }
        if let Some(stale) = self.particles.remove(&system.id) {
            self.live_vertices -= stale.pool_vertices();
        }

        let data: &MeshData = source;
        let triangles = data.indices.len() / 3;
        let template_vertices = data.vertices.len() as u64;
        let capacity = system.capacity as u64;
        if template_vertices == 0
            || triangles == 0
            || triangles as u64 * capacity > self.max_primitives as u64
            || template_vertices * capacity > MAX_PARTICLE_VERTICES
        {
            log::debug!(
                "Ray tracing skips particle system {}: {} slots of a {}-triangle mesh",
                system.id,
                capacity,
                triangles
            );
            return false;
        }
        let mesh_indices = &data.indices[..triangles * 3];
        if mesh_indices
            .iter()
            .any(|&index| index as u64 >= template_vertices)
        {
            log::warn!(
                "Ray tracing skips particle system {}: index out of range",
                system.id
            );
            return false;
        }

        let index_count = mesh_indices.len() as u64 * capacity;
        if !self.vertices.fits(template_vertices * (1 + capacity))
            || !self.indices.fits(index_count)
        {
            self.warn_pool_full();
            return false;
        }

        let (template_base, template_grew) =
            self.vertices
                .append(device, queue, encoder, bytemuck::cast_slice(&data.vertices));
        // The output starts out zeroed; the expansion fills the slots in use each frame.
        let output = vec![0u8; (template_vertices * capacity * VERTEX_STRIDE) as usize];
        let (vertex_base, output_grew) = self.vertices.append(device, queue, encoder, &output);
        // Slot p's copy of the mesh is p * template_vertices further into the output.
        let indices: Vec<u32> = (0..system.capacity)
            .flat_map(|slot| {
                let offset = slot * template_vertices as u32;
                mesh_indices.iter().map(move |&index| offset + index)
            })
            .collect();
        let (index_base, indices_grew) =
            self.indices
                .append(device, queue, encoder, bytemuck::cast_slice(&indices));

        let geometry_flags = match system.blend {
            RayTracedParticleBlend::Opaque => wgpu::AccelerationStructureGeometryFlags::OPAQUE,
            // Blended particles come up as candidates, which the shader gathers.
            _ => wgpu::AccelerationStructureGeometryFlags::NO_DUPLICATE_ANY_HIT_INVOCATION,
        };
        let full_size = wgpu::BlasTriangleGeometrySizeDescriptor {
            vertex_format: wgpu::VertexFormat::Float32x3,
            vertex_count: (template_vertices * capacity) as u32,
            index_format: Some(wgpu::IndexFormat::Uint32),
            index_count: Some(indices.len() as u32),
            flags: geometry_flags,
        };
        let blas = device.create_blas(
            &wgpu::CreateBlasDescriptor {
                label: Some("ParticleBlas"),
                flags: wgpu::AccelerationStructureFlags::PREFER_FAST_BUILD,
                update_mode: wgpu::AccelerationStructureUpdateMode::Build,
            },
            wgpu::BlasGeometrySizeDescriptors::Triangles {
                descriptors: vec![full_size],
            },
        );
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RtParticleExpandParams"),
            size: std::mem::size_of::<ExpandParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let entry = ParticleEntry {
            source: Arc::clone(source),
            capacity: system.capacity,
            blend: system.blend,
            blas,
            geometry_flags,
            template_base: template_base as u32,
            template_vertices: template_vertices as u32,
            template_indices: mesh_indices.len() as u32,
            vertex_base: vertex_base as u32,
            index_base: index_base as u32,
            count: system.count.min(system.capacity),
            billboarded,
            material,
            particles: system.particles.clone(),
            params,
            bind_group: None,
            last_used: frame,
        };
        self.live_vertices += entry.pool_vertices();
        self.particles.insert(system.id, entry);
        template_grew || output_grew || indices_grew
    }

    /// Place this frame's particles into their systems' output ranges (one compute pass).
    fn expand_particles(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        camera: RtCamera,
    ) {
        let frame = self.frame;
        if !self
            .particles
            .values()
            .any(|entry| entry.last_used == frame && entry.count > 0)
        {
            return;
        }
        let expand = self
            .expand_pipeline
            .get_or_insert_with(|| ExpandPipeline::new(device));
        let pool = &self.vertices.buffer;
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("RtParticleExpandPass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&expand.pipeline);
        for entry in self.particles.values_mut() {
            if entry.last_used != frame || entry.count == 0 {
                continue;
            }
            let params = ExpandParams {
                camera_forward: camera.forward.extend(0.0).to_array(),
                camera_up: camera.up.extend(0.0).to_array(),
                template_base: entry.template_base,
                template_count: entry.template_vertices,
                output_base: entry.vertex_base,
                particle_count: entry.count,
                billboarded: entry.billboarded as u32,
                _padding: [0; 3],
            };
            queue.write_buffer(&entry.params, 0, bytemuck::bytes_of(&params));
            if entry
                .bind_group
                .as_ref()
                .is_none_or(|(_, bound_pool)| bound_pool != pool)
            {
                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("RtParticleExpandBindGroup"),
                    layout: &expand.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: entry.particles.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: pool.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: entry.params.as_entire_binding(),
                        },
                    ],
                });
                entry.bind_group = Some((bind_group, pool.clone()));
            }
            let Some((bind_group, _)) = entry.bind_group.as_ref() else {
                continue;
            };
            pass.set_bind_group(0, bind_group, &[]);
            let threads = entry.count * entry.template_vertices;
            pass.dispatch_workgroups(threads.div_ceil(EXPAND_WORKGROUP_SIZE), 1, 1);
        }
    }

    fn geometry_key(batch: &crate::renderer::internal::OrderedBatch) -> GeometryKey {
        let masked = batch.alpha_masked;
        match batch.skin {
            Some(skin) => GeometryKey::Skin { skin, masked },
            None => GeometryKey::Mesh {
                mesh: batch.mesh,
                masked,
            },
        }
    }

    /// Make sure `batch`'s geometry has an up-to-date BLAS, uploading it to the pool when
    /// it is new or changed. A skinned instance (only once the skinning pass deformed it)
    /// is queued in `deformed_copies` and rebuilt. Returns whether a pool buffer was
    /// replaced.
    fn ensure_geometry(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        assets: &Assets,
        skinning: &SkinningStage,
        batch: &crate::renderer::internal::OrderedBatch,
    ) -> bool {
        let key = Self::geometry_key(batch);
        let mesh = match batch.skin {
            Some(skin) => skinning.mesh(skin),
            None if assets.meshes.is_released(batch.mesh) => None,
            None => assets.meshes.get(batch.mesh),
        };
        let Some(mesh) = mesh else {
            return false;
        };
        let deformed = batch.skin.is_some();
        let source = mesh.shared_data();
        if let Some(entry) = self.meshes.get_mut(&key) {
            if Arc::ptr_eq(&entry.source, source) {
                if entry.last_used != self.frame && deformed {
                    entry.needs_build = true;
                    self.deformed_copies.push((
                        mesh.vertex_buffer().clone(),
                        mesh.vertex_byte_offset(),
                        entry.vertex_base as u64 * VERTEX_STRIDE,
                        entry.size.vertex_count as u64 * VERTEX_STRIDE,
                    ));
                }
                entry.last_used = self.frame;
                return false;
            }
        }
        if let Some(stale) = self.meshes.remove(&key) {
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
            log::warn!("Ray tracing skips {key:?}: index out of range");
            return false;
        }

        if !self.vertices.fits(vertex_count as u64) || !self.indices.fits(indices.len() as u64) {
            self.warn_pool_full();
            return false;
        }

        // A skinned instance starts from its rest pose; the deformed copy replaces it.
        let (vertex_base, vertices_grew) =
            self.vertices
                .append(device, queue, encoder, bytemuck::cast_slice(&data.vertices));
        let (index_base, indices_grew) =
            self.indices
                .append(device, queue, encoder, bytemuck::cast_slice(indices));

        let size = wgpu::BlasTriangleGeometrySizeDescriptor {
            vertex_format: wgpu::VertexFormat::Float32x3,
            vertex_count,
            index_format: Some(wgpu::IndexFormat::Uint32),
            index_count: Some(indices.len() as u32),
            // Cut-out surfaces come up as candidates, which the shaders alpha-test.
            flags: if key.masked() {
                wgpu::AccelerationStructureGeometryFlags::NO_DUPLICATE_ANY_HIT_INVOCATION
            } else {
                wgpu::AccelerationStructureGeometryFlags::OPAQUE
            },
        };
        let blas = device.create_blas(
            &wgpu::CreateBlasDescriptor {
                label: Some(if deformed { "SkinnedBlas" } else { "MeshBlas" }),
                // Skinned structures are rebuilt every frame, so build speed matters more.
                flags: if deformed {
                    wgpu::AccelerationStructureFlags::PREFER_FAST_BUILD
                } else {
                    wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE
                },
                update_mode: wgpu::AccelerationStructureUpdateMode::Build,
            },
            wgpu::BlasGeometrySizeDescriptors::Triangles {
                descriptors: vec![size.clone()],
            },
        );
        self.live_vertices += vertex_count as u64;
        if deformed {
            self.deformed_copies.push((
                mesh.vertex_buffer().clone(),
                mesh.vertex_byte_offset(),
                vertex_base * VERTEX_STRIDE,
                vertex_count as u64 * VERTEX_STRIDE,
            ));
        }
        self.meshes.insert(
            key,
            MeshEntry {
                source: Arc::clone(source),
                blas,
                size,
                vertex_base: vertex_base as u32,
                index_base: index_base as u32,
                needs_build: true,
                last_used: self.frame,
                bounds: bounding_sphere(&data.vertices),
            },
        );
        vertices_grew || indices_grew
    }

    /// Geometry past the pool's limit is left out of ray-traced effects (it still draws);
    /// say so once.
    fn warn_pool_full(&mut self) {
        if !self.warned_pool_full {
            self.warned_pool_full = true;
            log::warn!(
                "Ray tracing leaves out geometry: its pool reached the GPU's limit of {} MiB per buffer ({} vertices); the rest still draws, without ray-traced shadows or reflections",
                self.vertices.max_elements * self.vertices.element_size / (1024 * 1024),
                self.vertices.len,
            );
        }
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
        // A particle system that did not draw this frame may be gone for good, and its
        // output range is large: let it go now (it comes back as new if it returns).
        self.particles.retain(|_, entry| {
            let keep = entry.last_used == frame;
            if !keep {
                freed += entry.pool_vertices();
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
        self.particles.clear();
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
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: self.particle_materials.as_entire_binding(),
                },
            ],
        })
    }
}

/// A model matrix as the 3x4 row-major affine transform a TLAS instance takes.
/// A sphere around `vertices` (the box's centre and farthest vertex).
fn bounding_sphere(vertices: &[Vertex]) -> [f32; 4] {
    let Some(first) = vertices.first() else {
        return [0.0; 4];
    };
    let (mut min, mut max) = (Vec3::from(first.pos), Vec3::from(first.pos));
    for vertex in vertices {
        min = min.min(Vec3::from(vertex.pos));
        max = max.max(Vec3::from(vertex.pos));
    }
    let center = (min + max) * 0.5;
    let radius = vertices
        .iter()
        .map(|vertex| center.distance_squared(Vec3::from(vertex.pos)))
        .fold(0.0f32, f32::max)
        .sqrt();
    [center.x, center.y, center.z, radius]
}

/// `sphere` (object space) moved into the world by `model`.
fn world_sphere(sphere: [f32; 4], model: Mat4) -> [f32; 4] {
    let center = model.transform_point3(Vec3::new(sphere[0], sphere[1], sphere[2]));
    let scale = model
        .x_axis
        .truncate()
        .length()
        .max(model.y_axis.truncate().length())
        .max(model.z_axis.truncate().length());
    [center.x, center.y, center.z, sphere[3] * scale]
}

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
    fn particle_expansion_shader_validates() {
        let source = wgsl!("rt_particles.wgsl");
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|err| panic!("{}", err.emit_to_string(source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap_or_else(|err| panic!("{}", err.emit_to_string(source)));
    }

    #[test]
    fn gpu_structs_match_the_shader_layout() {
        // RtVertex in rt_reflections.wgsl: scalar position, normal and uv, then two vec4s.
        assert_eq!(std::mem::size_of::<Vertex>(), 64);
        assert_eq!(std::mem::offset_of!(Vertex, tangent), 32);
        assert_eq!(std::mem::offset_of!(Vertex, color), 48);
        assert_eq!(std::mem::size_of::<RtInstance>(), 16);
        // ExpandParams in rt_particles.wgsl: two vec4s, then eight u32s.
        assert_eq!(std::mem::size_of::<ExpandParams>(), 64);
    }
}
