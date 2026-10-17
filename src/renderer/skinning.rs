//! Skinning: deforms each skinned mesh instance into vertices of its own, once per
//! frame, before any pass draws.
//!
//! Every pass (shadows, depth prepass, G-buffer, picking, transparency, user shader
//! materials) then draws the instance like a rigid mesh, so none of them needs a
//! skinning variant. The same buffers can later feed ray-tracing acceleration
//! structures.
//!
//! The deformed vertices of all instances live in a few large pool buffers ("pages")
//! rather than one buffer per instance: an instance keeps a slot (a vertex range in a
//! page) for as long as it is drawn, and its [`Mesh`] names the page buffer and the
//! slot's base vertex. Passes bind a page once and draw instance after instance with
//! `draw_indexed`'s base vertex. Each page is two buffers that take turns: this frame's
//! pose is written to one while the other keeps last frame's, which motion vectors
//! compare it with.
//!
//! Two backends write identical vertices:
//! - **Compute** (default): `src/shader/skinning.wgsl`. The source meshes are pooled the
//!   same way (a page of vertices and one of skin weights each), so all instances in one
//!   output page are skinned by a single dispatch: each workgroup finds its instance's
//!   record by binary search. One dispatch per page matters: consecutive dispatches
//!   writing one buffer are serialized by a barrier between them.
//! - **CPU**: [`skin_vertices`] plus a buffer upload, for devices without compute or
//!   storage buffers (WebGL2/GLES). `WGPU_CUBE_FORCE_CPU_SKINNING=1` forces it, to test
//!   that path natively.

use crate::renderer::shader_source::wgsl;
use std::collections::HashMap;
use std::mem::size_of;
use std::num::NonZeroU64;
use std::sync::Arc;

use glam::{Mat3, Mat4, Vec3, Vec4};

use crate::asset::{Assets, Handle, Mesh, MeshData};
use crate::renderer::batch::{SkinJob, SkinKey};
use crate::renderer::{SkinVertex, Vertex};

const WORKGROUP_SIZE: u32 = 64;
/// Frames an instance may go undrawn before its slot is freed (and a bind group unused
/// before it is dropped).
const EVICT_AFTER_FRAMES: u64 = 120;
const INITIAL_PALETTE_CAPACITY: usize = 1024;
const INITIAL_RECORD_CAPACITY: usize = 256;
const INITIAL_PARAMS_CAPACITY: usize = 64;
/// Vertices per pool page: 64 MiB at 64 bytes per vertex, within WebGPU's default
/// storage binding limit (128 MiB), so a page binds whole on every device.
const PAGE_VERTICES: u32 = 1 << 20;
/// Most workgroups along one dispatch dimension (WebGPU's `maxComputeWorkgroupsPerDimension`).
const MAX_WORKGROUPS_PER_DIMENSION: u32 = 65_535;
const VERTEX_STRIDE: u64 = size_of::<Vertex>() as u64;

/// Per-dispatch parameters (std140 uniform, padded to 16 bytes).
#[repr(C, align(16))]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SkinParams {
    /// The dispatch's instances are `records[first_record..first_record + record_count]`,
    /// in workgroup order.
    first_record: u32,
    record_count: u32,
    /// Workgroups in the dispatch (its grid may be wider, see `dispatch_workgroups`).
    workgroup_count: u32,
    _padding: u32,
}

/// One skinned instance's entry in the record buffer.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SkinRecord {
    palette_offset: u32,
    joint_count: u32,
    /// Where the source mesh's vertices start in the source page, in vertices.
    source_offset: u32,
    /// Where its vertices start in the output page, in vertices.
    output_offset: u32,
    vertex_count: u32,
    /// The dispatch's first workgroup for this instance (one per `WORKGROUP_SIZE` vertices).
    first_workgroup: u32,
    _padding: [u32; 2],
}

/// Which skinning implementation the device gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkinningBackend {
    Compute,
    Cpu,
}

impl SkinningBackend {
    /// Compute where the device has compute and enough storage buffers, else CPU.
    pub fn for_device(device: &wgpu::Device) -> Self {
        let forced_cpu = std::env::var("WGPU_CUBE_FORCE_CPU_SKINNING")
            .is_ok_and(|value| value != "0" && !value.is_empty());
        let limits = device.limits();
        let compute = limits.max_compute_workgroup_size_x >= WORKGROUP_SIZE
            && limits.max_compute_invocations_per_workgroup >= WORKGROUP_SIZE
            && limits.max_storage_buffers_per_shader_stage >= 5;
        if compute && !forced_cpu {
            Self::Compute
        } else {
            Self::Cpu
        }
    }
}

/// A vertex range in one pool page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Slot {
    page: usize,
    /// In vertices.
    offset: u32,
    vertex_count: u32,
}

/// The slots of one page: a bump allocator plus freed slots by size. Instances of one
/// mesh are the same size, so a freed slot is reused exactly by the next instance of it
/// (or of any mesh with as many vertices).
#[derive(Debug, Default)]
struct PageSlots {
    capacity: u32,
    cursor: u32,
    free: HashMap<u32, Vec<u32>>,
}

/// Where instances' slots go, independent of the GPU buffers that back the pages.
#[derive(Debug)]
struct SlotAllocator {
    pages: Vec<PageSlots>,
    /// Vertices per ordinary page.
    page_vertices: u32,
    /// Most vertices one page may hold (the device's storage binding limit): a mesh
    /// larger than an ordinary page gets a page of its own up to this size.
    max_page_vertices: u32,
}

impl SlotAllocator {
    fn new(page_vertices: u32, max_page_vertices: u32) -> Self {
        Self {
            pages: Vec::new(),
            page_vertices: page_vertices.min(max_page_vertices).max(1),
            max_page_vertices,
        }
    }

    /// A slot of `vertex_count` vertices, and whether it opened a new page (the caller
    /// then creates that page's buffers). `None` when no page can hold the mesh.
    fn allocate(&mut self, vertex_count: u32) -> Option<(Slot, bool)> {
        if vertex_count == 0 || vertex_count > self.max_page_vertices {
            return None;
        }
        for (page, slots) in self.pages.iter_mut().enumerate() {
            if let Some(offset) = slots.free.get_mut(&vertex_count).and_then(Vec::pop) {
                return Some((
                    Slot {
                        page,
                        offset,
                        vertex_count,
                    },
                    false,
                ));
            }
        }
        for (page, slots) in self.pages.iter_mut().enumerate() {
            if slots.capacity - slots.cursor >= vertex_count {
                let offset = slots.cursor;
                slots.cursor += vertex_count;
                return Some((
                    Slot {
                        page,
                        offset,
                        vertex_count,
                    },
                    false,
                ));
            }
        }
        let capacity = self.page_vertices.max(vertex_count);
        self.pages.push(PageSlots {
            capacity,
            cursor: vertex_count,
            free: HashMap::new(),
        });
        Some((
            Slot {
                page: self.pages.len() - 1,
                offset: 0,
                vertex_count,
            },
            true,
        ))
    }

    fn free(&mut self, slot: Slot) {
        if let Some(page) = self.pages.get_mut(slot.page) {
            page.free
                .entry(slot.vertex_count)
                .or_default()
                .push(slot.offset);
        }
    }

    fn page_capacity(&self, page: usize) -> u32 {
        self.pages.get(page).map_or(0, |slots| slots.capacity)
    }
}

/// One skinned instance: its slot in the pool and the geometry drawn from it.
struct SkinnedInstance {
    source: Handle<Mesh>,
    /// The source geometry drawn from each of the slot's page buffers; shares the
    /// source mesh's data, so a replaced mesh is noticed and the instance rebuilt.
    meshes: [Mesh; 2],
    slot: Slot,
    /// The other page buffer holds the pose of the frame before (not right after
    /// creation or a frame without skinning).
    has_previous: bool,
    last_used: u64,
    /// The palette last skinned with, and the number of that pose (see
    /// [`SkinningStage::pose`]).
    palette: Option<Arc<[Mat4]>>,
    pose: u64,
}

/// The source geometry of every skinned mesh in use, copied into pool pages so that one
/// dispatch can read any of them: a page holds a buffer of vertices and one of skin
/// weights. Meshes keep their slot while instances of them are skinned.
struct SourcePool {
    slots: SlotAllocator,
    pages: Vec<SourcePage>,
    /// By the address of the mesh data, which the entry keeps alive.
    entries: HashMap<usize, SourceEntry>,
}

struct SourcePage {
    vertices: wgpu::Buffer,
    skin: wgpu::Buffer,
}

struct SourceEntry {
    slot: Slot,
    _data: Arc<MeshData>,
    last_used: u64,
}

impl SourcePool {
    /// The slot holding `source`'s geometry, uploaded on first use. `None` when the mesh
    /// has no skin weights or fits no page.
    fn ensure(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        source: &Mesh,
        frame: u64,
    ) -> Option<Slot> {
        let data = source.shared_data();
        let key = Arc::as_ptr(data) as usize;
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.last_used = frame;
            return Some(entry.slot);
        }
        let skin = data.skin.as_ref()?;
        let (slot, new_page) = self.slots.allocate(source.vertex_count())?;
        if new_page {
            let capacity = u64::from(self.slots.page_capacity(slot.page));
            let page = |label: &str, stride: u64| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: (capacity * stride).max(16),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            };
            self.pages.push(SourcePage {
                vertices: page("SkinSourceVertices", VERTEX_STRIDE),
                skin: page("SkinSourceWeights", size_of::<SkinVertex>() as u64),
            });
        }
        let page = &self.pages[slot.page];
        queue.write_buffer(
            &page.vertices,
            u64::from(slot.offset) * VERTEX_STRIDE,
            bytemuck::cast_slice(&data.vertices),
        );
        queue.write_buffer(
            &page.skin,
            u64::from(slot.offset) * size_of::<SkinVertex>() as u64,
            bytemuck::cast_slice(skin),
        );
        self.entries.insert(
            key,
            SourceEntry {
                slot,
                _data: Arc::clone(data),
                last_used: frame,
            },
        );
        Some(slot)
    }

    /// Free the slots of meshes no instance has used lately.
    fn evict(&mut self, frame: u64) {
        let slots = &mut self.slots;
        self.entries.retain(|_, entry| {
            let keep = frame - entry.last_used <= EVICT_AFTER_FRAMES;
            if !keep {
                slots.free(entry.slot);
            }
            keep
        });
    }
}

pub(crate) struct SkinningStage {
    backend: SkinningBackend,
    compute: Option<ComputeResources>,
    instances: HashMap<SkinKey, SkinnedInstance>,
    slots: SlotAllocator,
    /// The compute backend's copies of the source meshes.
    sources: SourcePool,
    /// Each page's two buffers; `pages[p][current]` is written this frame.
    pages: Vec<[wgpu::Buffer; 2]>,
    /// Which of each page's buffers holds this frame's poses; flips every frame.
    current: usize,
    frame: u64,
    /// The last pose number handed out; numbers are never reused.
    poses: u64,
    /// Instances skinned in the last frame.
    pub(crate) instance_count: u32,
    pub(crate) vertex_count: u32,
    /// Compute dispatches recorded in the last frame.
    pub(crate) dispatch_count: u32,
    cpu_scratch: Vec<Vertex>,
    /// Meshes too large for any page, reported once each.
    oversized_reported: Vec<Handle<Mesh>>,
}

struct ComputeResources {
    pipeline: wgpu::ComputePipeline,
    instance_layout: wgpu::BindGroupLayout,
    shared_layout: wgpu::BindGroupLayout,
    palette: wgpu::Buffer,
    palette_capacity: usize,
    records: wgpu::Buffer,
    record_capacity: usize,
    params: wgpu::Buffer,
    params_capacity: usize,
    params_stride: u64,
    shared_bind_group: wgpu::BindGroup,
    /// The geometry bind groups per (source page, output page), one per output buffer.
    /// Pages are never freed, so these stay valid.
    page_bind_groups: HashMap<(usize, usize), [wgpu::BindGroup; 2]>,
    max_workgroups_per_dimension: u32,
    palette_scratch: Vec<[[f32; 4]; 4]>,
    record_scratch: Vec<SkinRecord>,
    params_scratch: Vec<u8>,
}

/// One compute dispatch: the instances read from one source page and written to one
/// output page.
struct Dispatch {
    bind_key: (usize, usize),
    first_record: u32,
    count: u32,
    workgroups: u32,
}

impl SkinningStage {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let backend = SkinningBackend::for_device(device);
        log::info!("Skinning backend: {backend:?}");
        let limits = device.limits();
        let max_page_bytes = limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size);
        let max_page_vertices = (max_page_bytes / VERTEX_STRIDE).min(u64::from(u32::MAX)) as u32;
        Self {
            backend,
            compute: (backend == SkinningBackend::Compute).then(|| ComputeResources::new(device)),
            instances: HashMap::new(),
            slots: SlotAllocator::new(PAGE_VERTICES, max_page_vertices),
            sources: SourcePool {
                slots: SlotAllocator::new(PAGE_VERTICES, max_page_vertices),
                pages: Vec::new(),
                entries: HashMap::new(),
            },
            pages: Vec::new(),
            current: 0,
            frame: 0,
            poses: 0,
            instance_count: 0,
            vertex_count: 0,
            dispatch_count: 0,
            cpu_scratch: Vec::new(),
            oversized_reported: Vec::new(),
        }
    }

    pub(crate) fn backend(&self) -> SkinningBackend {
        self.backend
    }

    /// Skin this frame's instances. Compute work is recorded into `encoder`, so it runs
    /// before the passes recorded after it; CPU results are uploaded right away.
    pub(crate) fn run(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        assets: &Assets,
        jobs: &HashMap<SkinKey, SkinJob>,
    ) {
        self.frame += 1;
        self.current = 1 - self.current;
        self.instance_count = 0;
        self.vertex_count = 0;
        self.dispatch_count = 0;

        // Deterministic order (the map is unordered) keeps palette offsets stable.
        let mut keys: Vec<SkinKey> = jobs.keys().copied().collect();
        keys.sort_unstable();
        let mut ready: Vec<(SkinKey, &SkinJob)> = Vec::with_capacity(keys.len());
        for key in keys {
            let job = &jobs[&key];
            let Some(source) = assets.meshes.get(job.mesh) else {
                continue;
            };
            if !source.is_skinned() || job.palette.is_empty() {
                continue;
            }
            if self.ensure_instance(device, key, job.mesh, source) {
                self.note_pose(key, &job.palette);
                ready.push((key, job));
            }
        }

        match self.backend {
            SkinningBackend::Compute => self.run_compute(device, queue, encoder, assets, &ready),
            SkinningBackend::Cpu => self.run_cpu(queue, assets, &ready),
        }

        let frame = self.frame;
        let slots = &mut self.slots;
        self.instances.retain(|_, instance| {
            let keep = frame - instance.last_used <= EVICT_AFTER_FRAMES;
            if !keep {
                slots.free(instance.slot);
            }
            keep
        });
        self.sources.evict(frame);
    }

    /// The skinned geometry of an instance, if it was skinned: its page buffer and base
    /// vertex.
    pub(crate) fn mesh(&self, key: SkinKey) -> Option<&Mesh> {
        self.instances
            .get(&key)
            .map(|instance| &instance.meshes[self.current])
    }

    /// A number for the pose [`Self::mesh`] holds, which changes only when the instance
    /// is skinned with a different palette (or is created anew): the same number means
    /// the same deformed vertices, so copies of them made earlier are still current.
    pub(crate) fn pose(&self, key: SkinKey) -> Option<u64> {
        self.instances.get(&key).map(|instance| instance.pose)
    }

    /// Give `key` a new pose number when `palette` differs from the one it was last
    /// skinned with. Skinned vertices depend only on the mesh and the palette.
    fn note_pose(&mut self, key: SkinKey, palette: &Arc<[Mat4]>) {
        let Some(instance) = self.instances.get_mut(&key) else {
            return;
        };
        let same = instance
            .palette
            .as_ref()
            .is_some_and(|last| Arc::ptr_eq(last, palette) || last[..] == palette[..]);
        if !same {
            self.poses += 1;
            instance.pose = self.poses;
            instance.palette = Some(Arc::clone(palette));
        }
    }

    /// An instance's vertices as last frame deformed them (this frame's when there is no
    /// earlier pose), for motion vectors. The same base vertex as [`Self::mesh`].
    pub(crate) fn previous_vertex_buffer(&self, key: SkinKey) -> Option<&wgpu::Buffer> {
        let instance = self.instances.get(&key)?;
        let previous = if instance.has_previous {
            1 - self.current
        } else {
            self.current
        };
        Some(&self.pages[instance.slot.page][previous])
    }

    /// Create or refresh the instance for `key`; false when it cannot be skinned (the mesh
    /// does not fit any page).
    fn ensure_instance(
        &mut self,
        device: &wgpu::Device,
        key: SkinKey,
        handle: Handle<Mesh>,
        source: &Mesh,
    ) -> bool {
        let stale = self.instances.get(&key).is_none_or(|instance| {
            instance.source != handle || !instance.meshes[0].shares_data_with(source)
        });
        if stale {
            if let Some(old) = self.instances.remove(&key) {
                self.slots.free(old.slot);
            }
            let Some((slot, new_page)) = self.slots.allocate(source.vertex_count()) else {
                if !self.oversized_reported.contains(&handle) {
                    self.oversized_reported.push(handle);
                    log::warn!(
                        "Skinned mesh {handle:?} has {} vertices, more than a skinning pool page holds ({}); drawing it in the bind pose",
                        source.vertex_count(),
                        self.slots.max_page_vertices
                    );
                }
                return false;
            };
            if new_page {
                self.pages
                    .push(self.create_page(device, self.slots.page_capacity(slot.page)));
            }
            let buffers = &self.pages[slot.page];
            self.instances.insert(
                key,
                SkinnedInstance {
                    source: handle,
                    meshes: [0, 1].map(|index| {
                        source.with_vertex_buffer(buffers[index].clone(), slot.offset)
                    }),
                    slot,
                    has_previous: false,
                    last_used: self.frame,
                    palette: None,
                    pose: 0,
                },
            );
            return true;
        }
        let Some(instance) = self.instances.get_mut(&key) else {
            return false;
        };
        if instance.last_used != self.frame {
            // The buffer written last frame becomes the previous pose, if the instance
            // was skinned in the frame right before.
            instance.has_previous = instance.last_used + 1 == self.frame;
            instance.last_used = self.frame;
        }
        true
    }

    fn create_page(&self, device: &wgpu::Device, vertices: u32) -> [wgpu::Buffer; 2] {
        // COPY_SRC lets tools, tests and ray tracing read the deformed vertices back.
        let mut usage = wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_SRC;
        usage |= match self.backend {
            SkinningBackend::Compute => wgpu::BufferUsages::STORAGE,
            SkinningBackend::Cpu => wgpu::BufferUsages::COPY_DST,
        };
        let size = (u64::from(vertices) * VERTEX_STRIDE).max(16);
        [0, 1].map(|_| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("SkinnedVertexPool"),
                size,
                usage,
                mapped_at_creation: false,
            })
        })
    }

    fn run_compute(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        assets: &Assets,
        ready: &[(SkinKey, &SkinJob)],
    ) {
        let Some(compute) = self.compute.as_mut() else {
            return;
        };
        compute.palette_scratch.clear();
        compute.record_scratch.clear();
        compute.params_scratch.clear();

        // Group the instances by source page and output page: one dispatch each. The
        // key order (`ready` is sorted) keeps the records stable within a frame.
        let mut grouped: Vec<((usize, usize), usize, Slot)> = Vec::with_capacity(ready.len());
        for (index, (key, job)) in ready.iter().enumerate() {
            let Some(source) = assets.meshes.get(job.mesh) else {
                continue;
            };
            let Some(instance) = self.instances.get(key) else {
                continue;
            };
            let Some(source_slot) = self.sources.ensure(device, queue, source, self.frame) else {
                if !self.oversized_reported.contains(&job.mesh) {
                    self.oversized_reported.push(job.mesh);
                    log::warn!(
                        "Skinned mesh {:?} ({} vertices) cannot be copied into the skinning source pool; drawing it in the bind pose",
                        job.mesh,
                        source.vertex_count()
                    );
                }
                continue;
            };
            grouped.push(((source_slot.page, instance.slot.page), index, source_slot));
        }
        grouped.sort_unstable_by_key(|(pages, index, _)| (*pages, *index));

        // The meshes of one character share its palette (the same allocation): upload it
        // once and point them all at it.
        let mut uploaded: HashMap<*const Mat4, u32> = HashMap::with_capacity(ready.len());
        let mut dispatches: Vec<Dispatch> = Vec::new();
        for &(bind_key, index, source_slot) in &grouped {
            let (key, job) = &ready[index];
            let Some(instance) = self.instances.get(key) else {
                continue;
            };
            let palette_offset = *uploaded
                .entry(Arc::as_ptr(&job.palette) as *const Mat4)
                .or_insert_with(|| {
                    let offset = compute.palette_scratch.len() as u32;
                    compute
                        .palette_scratch
                        .extend(job.palette.iter().map(Mat4::to_cols_array_2d));
                    offset
                });
            let vertex_count = source_slot.vertex_count;
            let workgroups = vertex_count.div_ceil(WORKGROUP_SIZE);
            let record_index = compute.record_scratch.len() as u32;
            let first_workgroup = match dispatches.last_mut() {
                Some(last) if last.bind_key == bind_key => {
                    let first = last.workgroups;
                    last.workgroups += workgroups;
                    last.count += 1;
                    first
                }
                _ => {
                    dispatches.push(Dispatch {
                        bind_key,
                        first_record: record_index,
                        count: 1,
                        workgroups,
                    });
                    0
                }
            };
            compute.record_scratch.push(SkinRecord {
                palette_offset,
                joint_count: job.palette.len() as u32,
                source_offset: source_slot.offset,
                output_offset: instance.slot.offset,
                vertex_count,
                first_workgroup,
                _padding: [0; 2],
            });
            self.vertex_count += vertex_count;
            compute.ensure_page_bind_groups(
                device,
                bind_key,
                &self.sources.pages[bind_key.0],
                &self.pages[bind_key.1],
            );
        }
        self.instance_count = compute.record_scratch.len() as u32;
        if dispatches.is_empty() {
            return;
        }

        let stride = compute.params_stride as usize;
        compute.params_scratch.resize(dispatches.len() * stride, 0);
        for (index, dispatch) in dispatches.iter().enumerate() {
            // Each block sits at a multiple of the uniform offset alignment.
            let params = SkinParams {
                first_record: dispatch.first_record,
                record_count: dispatch.count,
                workgroup_count: dispatch.workgroups,
                _padding: 0,
            };
            let start = index * stride;
            compute.params_scratch[start..start + size_of::<SkinParams>()]
                .copy_from_slice(bytemuck::bytes_of(&params));
        }
        compute.reserve(
            device,
            compute.palette_scratch.len(),
            compute.record_scratch.len(),
            dispatches.len(),
        );
        queue.write_buffer(
            &compute.palette,
            0,
            bytemuck::cast_slice(&compute.palette_scratch),
        );
        queue.write_buffer(
            &compute.records,
            0,
            bytemuck::cast_slice(&compute.record_scratch),
        );
        queue.write_buffer(&compute.params, 0, &compute.params_scratch);

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Skinning"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&compute.pipeline);
        let max_dimension = compute.max_workgroups_per_dimension;
        for (index, dispatch) in dispatches.iter().enumerate() {
            let Some(groups) = compute.page_bind_groups.get(&dispatch.bind_key) else {
                continue;
            };
            pass.set_bind_group(0, &groups[self.current], &[]);
            let offset = (index as u64 * compute.params_stride) as u32;
            pass.set_bind_group(1, &compute.shared_bind_group, &[offset]);
            // A grid as wide as the limit allows; the shader numbers workgroups row by row.
            pass.dispatch_workgroups(
                dispatch.workgroups.min(max_dimension),
                dispatch.workgroups.div_ceil(max_dimension),
                1,
            );
            self.dispatch_count += 1;
        }
    }

    fn run_cpu(&mut self, queue: &wgpu::Queue, assets: &Assets, ready: &[(SkinKey, &SkinJob)]) {
        for (key, job) in ready {
            let Some(source) = assets.meshes.get(job.mesh) else {
                continue;
            };
            let Some(instance) = self.instances.get(key) else {
                continue;
            };
            let data = source.data();
            let Some(skin) = data.skin.as_deref() else {
                continue;
            };
            skin_vertices(&data.vertices, skin, &job.palette, &mut self.cpu_scratch);
            queue.write_buffer(
                &self.pages[instance.slot.page][self.current],
                u64::from(instance.slot.offset) * VERTEX_STRIDE,
                bytemuck::cast_slice(&self.cpu_scratch),
            );
            self.instance_count += 1;
            self.vertex_count += data.vertices.len() as u32;
        }
    }
}

impl ComputeResources {
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
        let instance_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("SkinningInstanceLayout"),
            entries: &[storage(0, true), storage(1, true), storage(2, false)],
        });
        let shared_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("SkinningSharedLayout"),
            entries: &[
                storage(0, true),
                storage(1, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: NonZeroU64::new(size_of::<SkinParams>() as u64),
                    },
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Skinning"),
            source: wgpu::ShaderSource::Wgsl(wgsl!("skinning.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("SkinningLayout"),
            bind_group_layouts: &[Some(&instance_layout), Some(&shared_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Skinning"),
            layout: Some(&layout),
            module: &shader,
            entry_point: Some("cs_main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let limits = device.limits();
        let alignment = limits.min_uniform_buffer_offset_alignment as u64;
        let params_stride = (size_of::<SkinParams>() as u64).div_ceil(alignment) * alignment;
        let palette = create_palette_buffer(device, INITIAL_PALETTE_CAPACITY);
        let records = create_record_buffer(device, INITIAL_RECORD_CAPACITY);
        let params = create_params_buffer(device, INITIAL_PARAMS_CAPACITY, params_stride);
        let shared_bind_group =
            create_shared_bind_group(device, &shared_layout, &palette, &records, &params);
        Self {
            pipeline,
            instance_layout,
            shared_layout,
            palette,
            palette_capacity: INITIAL_PALETTE_CAPACITY,
            records,
            record_capacity: INITIAL_RECORD_CAPACITY,
            params,
            params_capacity: INITIAL_PARAMS_CAPACITY,
            params_stride,
            shared_bind_group,
            page_bind_groups: HashMap::new(),
            max_workgroups_per_dimension: limits
                .max_compute_workgroups_per_dimension
                .clamp(1, MAX_WORKGROUPS_PER_DIMENSION),
            palette_scratch: Vec::new(),
            record_scratch: Vec::new(),
            params_scratch: Vec::new(),
        }
    }

    /// The bind groups reading `source` and writing each of `output`'s buffers.
    fn ensure_page_bind_groups(
        &mut self,
        device: &wgpu::Device,
        key: (usize, usize),
        source: &SourcePage,
        output: &[wgpu::Buffer; 2],
    ) {
        if self.page_bind_groups.contains_key(&key) {
            return;
        }
        let groups = [0, 1].map(|index| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("SkinningPages"),
                layout: &self.instance_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: source.vertices.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: source.skin.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: output[index].as_entire_binding(),
                    },
                ],
            })
        });
        self.page_bind_groups.insert(key, groups);
    }

    /// Grow the shared buffers to hold `matrices` palette entries, `records` instance
    /// records and `dispatches` parameter blocks.
    fn reserve(
        &mut self,
        device: &wgpu::Device,
        matrices: usize,
        records: usize,
        dispatches: usize,
    ) {
        let mut changed = false;
        if matrices > self.palette_capacity {
            self.palette_capacity = matrices.next_power_of_two();
            self.palette = create_palette_buffer(device, self.palette_capacity);
            changed = true;
        }
        if records > self.record_capacity {
            self.record_capacity = records.next_power_of_two();
            self.records = create_record_buffer(device, self.record_capacity);
            changed = true;
        }
        if dispatches > self.params_capacity {
            self.params_capacity = dispatches.next_power_of_two();
            self.params = create_params_buffer(device, self.params_capacity, self.params_stride);
            changed = true;
        }
        if changed {
            self.shared_bind_group = create_shared_bind_group(
                device,
                &self.shared_layout,
                &self.palette,
                &self.records,
                &self.params,
            );
        }
    }
}

fn create_palette_buffer(device: &wgpu::Device, matrices: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("SkinPalette"),
        size: (matrices.max(1) * size_of::<Mat4>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn create_record_buffer(device: &wgpu::Device, records: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("SkinRecords"),
        size: (records.max(1) * size_of::<SkinRecord>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn create_params_buffer(device: &wgpu::Device, dispatches: usize, stride: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("SkinParams"),
        size: dispatches.max(1) as u64 * stride,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn create_shared_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    palette: &wgpu::Buffer,
    records: &wgpu::Buffer,
    params: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("SkinningShared"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: palette.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: records.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: params,
                    offset: 0,
                    size: NonZeroU64::new(size_of::<SkinParams>() as u64),
                }),
            },
        ],
    })
}

/// Linear blend skinning on the CPU, the same math as `skinning.wgsl`: each vertex is
/// moved by the weighted sum of its joints' palette matrices; normals and tangents are
/// rotated and renormalized; UVs and colours are copied. `out` is overwritten.
///
/// ```
/// use glam::{Mat4, Vec3};
/// use wgpu_cube::renderer::{skinning::skin_vertices, vertex::v, SkinVertex};
/// let vertices = [v([0.0, 1.0, 0.0], [0.0, 1.0, 0.0], [0.0; 2], [1.0, 0.0, 0.0, 1.0])];
/// let skin = [SkinVertex::from_influences([(0, 1.0)])];
/// let palette = [Mat4::from_translation(Vec3::X)];
/// let mut out = Vec::new();
/// skin_vertices(&vertices, &skin, &palette, &mut out);
/// assert_eq!(out[0].pos, [1.0, 1.0, 0.0]);
/// ```
pub fn skin_vertices(
    vertices: &[Vertex],
    skin: &[SkinVertex],
    palette: &[Mat4],
    out: &mut Vec<Vertex>,
) {
    out.clear();
    if palette.is_empty() {
        out.extend_from_slice(vertices);
        return;
    }
    let last = palette.len() - 1;
    out.extend(vertices.iter().zip(skin).map(|(vertex, influence)| {
        let weights = influence.weights_f32();
        let mut m = Mat4::ZERO;
        for (joint, weight) in influence.joints.iter().zip(weights) {
            if weight > 0.0 {
                m += palette[(*joint as usize).min(last)] * weight;
            }
        }
        let m3 = Mat3::from_mat4(m);
        let normal = Vec3::from(vertex.normal);
        let tangent = Vec3::new(vertex.tangent[0], vertex.tangent[1], vertex.tangent[2]);
        let position = m * Vec4::from((Vec3::from(vertex.pos), 1.0));
        let n = (m3 * normal).try_normalize().unwrap_or(normal);
        let t = (m3 * tangent).try_normalize().unwrap_or(tangent);
        Vertex {
            pos: position.truncate().to_array(),
            normal: n.to_array(),
            uv: vertex.uv,
            tangent: [t.x, t.y, t.z, vertex.tangent[3]],
            color: vertex.color,
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer::vertex::v;
    use glam::Quat;
    use std::sync::Arc;

    #[test]
    fn shader_validates() {
        let source = wgsl!("skinning.wgsl");
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
    fn params_match_the_shader_struct() {
        assert_eq!(size_of::<SkinParams>(), 16);
        assert_eq!(size_of::<SkinRecord>(), 32);
        assert_eq!(size_of::<SkinVertex>(), 16);
        assert_eq!(size_of::<Vertex>(), 16 * 4);
    }

    #[test]
    fn slots_pack_into_pages_and_are_reused() {
        let mut slots = SlotAllocator::new(100, 150);
        let (a, new_page) = slots.allocate(40).unwrap();
        assert!(new_page);
        let (b, new_page) = slots.allocate(40).unwrap();
        assert!(!new_page);
        assert_eq!((a.page, a.offset), (0, 0));
        assert_eq!((b.page, b.offset), (0, 40));
        // No room left in page 0 for 40 more: a second page.
        let (c, new_page) = slots.allocate(40).unwrap();
        assert!(new_page);
        assert_eq!((c.page, c.offset), (1, 0));
        // A freed slot is handed out again before any page grows.
        slots.free(a);
        let (d, new_page) = slots.allocate(40).unwrap();
        assert!(!new_page);
        assert_eq!(d, a);
        // A mesh larger than a page gets a page of its own, up to the binding limit.
        let (big, new_page) = slots.allocate(120).unwrap();
        assert!(new_page);
        assert_eq!(slots.page_capacity(big.page), 120);
        assert!(slots.allocate(151).is_none());
        assert!(slots.allocate(0).is_none());
    }

    #[test]
    fn cpu_skinning_blends_two_joints() {
        let vertex = v(
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.25, 0.5],
            [1.0, 0.0, 0.0, -1.0],
        );
        let palette = [
            Mat4::IDENTITY,
            Mat4::from_rotation_translation(
                Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
                Vec3::new(0.0, 2.0, 0.0),
            ),
        ];
        let skin = [SkinVertex::from_influences([(0, 0.5), (1, 0.5)])];
        let mut out = Vec::new();
        skin_vertices(&[vertex], &skin, &palette, &mut out);
        // Joint 0 leaves it at (1,0,0); joint 1 turns it to (0,1,0) and lifts it to
        // (0,3,0). Linear blending averages the matrices.
        let pos = Vec3::from(out[0].pos);
        assert!(pos.abs_diff_eq(Vec3::new(0.5, 1.5, 0.0), 1e-4), "{pos}");
        assert_eq!(out[0].uv, [0.25, 0.5]);
        assert_eq!(out[0].tangent[3], -1.0);
        assert!((Vec3::from(out[0].normal).length() - 1.0).abs() < 1e-5);
    }

    fn headless_device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
            ..Default::default()
        }))
        .ok()?;
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("skinning-test"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            memory_hints: wgpu::MemoryHints::Performance,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            trace: wgpu::Trace::Off,
        }))
        .ok()
    }

    fn fox_palette(seed: f32) -> Arc<[Mat4]> {
        (0..24)
            .map(|i| {
                let f = i as f32 + seed;
                Mat4::from_scale_rotation_translation(
                    Vec3::splat(1.0 + 0.01 * f),
                    Quat::from_euler(glam::EulerRot::XYZ, 0.1 * f, -0.07 * f, 0.03 * f),
                    Vec3::new(0.02 * f, -0.01 * f, 0.005 * f),
                )
            })
            .collect::<Vec<_>>()
            .into()
    }

    fn read_vertices(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: wgpu::CommandEncoder,
        source: &wgpu::Buffer,
        offset: u64,
        count: usize,
    ) -> Vec<Vertex> {
        let mut encoder = encoder;
        let size = count as u64 * VERTEX_STRIDE;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(source, offset, &readback, 0, size);
        queue.submit([encoder.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let bytes = readback.slice(..).get_mapped_range().unwrap().to_vec();
        bytemuck::cast_slice(&bytes).to_vec()
    }

    fn assert_vertices_match(gpu: &[Vertex], cpu: &[Vertex]) {
        assert_eq!(gpu.len(), cpu.len());
        for (index, (gpu, cpu)) in gpu.iter().zip(cpu).enumerate() {
            let close = |a: &[f32], b: &[f32]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-4);
            assert!(
                close(&gpu.pos, &cpu.pos)
                    && close(&gpu.normal, &cpu.normal)
                    && close(&gpu.tangent, &cpu.tangent)
                    && gpu.uv == cpu.uv,
                "vertex {index}: gpu {gpu:?} cpu {cpu:?}"
            );
        }
    }

    /// The compute shader and the CPU fallback must produce the same vertices, for two
    /// instances of one mesh skinned by a single dispatch into one page.
    #[test]
    fn compute_skinning_matches_the_cpu_path() {
        let Some((device, queue)) = headless_device() else {
            eprintln!("Skipping: no wgpu adapter");
            return;
        };
        let (document, buffers, _) = crate::scene::SceneLoader::import_gltf_native(
            std::path::Path::new("web/assets/skinned/Fox/Fox.glb"),
        )
        .unwrap();
        let primitive = document
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap();
        let data = crate::scene::loader::primitive_mesh_data_for_tests(&primitive, &buffers, 0.01)
            .unwrap();
        let palettes = [fox_palette(0.0), fox_palette(3.0)];
        let expected: Vec<Vec<Vertex>> = palettes
            .iter()
            .map(|palette| {
                let mut out = Vec::new();
                skin_vertices(
                    &data.vertices,
                    data.skin.as_ref().unwrap(),
                    palette,
                    &mut out,
                );
                out
            })
            .collect();

        let mut assets = Assets::new();
        let mesh = assets.meshes.insert(Mesh::from_data(&device, data));
        let mut stage = SkinningStage::new(&device);
        if stage.backend() != SkinningBackend::Compute {
            eprintln!("Skipping: adapter cannot run compute skinning");
            return;
        }
        let jobs = HashMap::from([
            (
                7,
                SkinJob {
                    mesh,
                    palette: Arc::clone(&palettes[0]),
                },
            ),
            (
                9,
                SkinJob {
                    mesh,
                    palette: Arc::clone(&palettes[1]),
                },
            ),
        ]);
        let mut encoder = device.create_command_encoder(&Default::default());
        stage.run(&device, &queue, &mut encoder, &assets, &jobs);
        assert_eq!(stage.instance_count, 2);
        assert_eq!(stage.dispatch_count, 1, "one mesh, one page: one dispatch");
        let (a, b) = (
            stage.mesh(7).unwrap().clone(),
            stage.mesh(9).unwrap().clone(),
        );
        assert_eq!(a.vertex_buffer(), b.vertex_buffer(), "both in one page");
        assert_ne!(a.base_vertex(), b.base_vertex());
        let count = expected[0].len();
        let got = read_vertices(
            &device,
            &queue,
            encoder,
            a.vertex_buffer(),
            a.vertex_byte_offset(),
            count,
        );
        assert_vertices_match(&got, &expected[0]);
        let encoder = device.create_command_encoder(&Default::default());
        let got = read_vertices(
            &device,
            &queue,
            encoder,
            b.vertex_buffer(),
            b.vertex_byte_offset(),
            count,
        );
        assert_vertices_match(&got, &expected[1]);

        // The next frame writes the other page buffer; the first keeps the previous pose.
        let mut encoder = device.create_command_encoder(&Default::default());
        stage.run(&device, &queue, &mut encoder, &assets, &jobs);
        queue.submit([encoder.finish()]);
        let current = stage.mesh(7).unwrap().clone();
        assert_ne!(current.vertex_buffer(), a.vertex_buffer());
        assert_eq!(current.base_vertex(), a.base_vertex());
        assert_eq!(stage.previous_vertex_buffer(7), Some(a.vertex_buffer()));
    }

    /// An instance keeps its pose number while it is skinned with an equal palette (even
    /// a new `Arc` of one) and gets a new one, never another instance's, when it changes.
    #[test]
    fn pose_numbers_change_only_with_the_palette() {
        let Some((device, queue)) = headless_device() else {
            eprintln!("Skipping: no wgpu adapter");
            return;
        };
        let (document, buffers, _) = crate::scene::SceneLoader::import_gltf_native(
            std::path::Path::new("web/assets/skinned/Fox/Fox.glb"),
        )
        .unwrap();
        let primitive = document
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap();
        let data = crate::scene::loader::primitive_mesh_data_for_tests(&primitive, &buffers, 0.01)
            .unwrap();
        let mut assets = Assets::new();
        let mesh = assets.meshes.insert(Mesh::from_data(&device, data));
        let mut stage = SkinningStage::new(&device);
        // Instances 1 and 2, skinned with the given palettes.
        let mut run = |palettes: [Arc<[Mat4]>; 2]| {
            let jobs: HashMap<SkinKey, SkinJob> = (1..)
                .zip(palettes)
                .map(|(key, palette)| (key, SkinJob { mesh, palette }))
                .collect();
            let mut encoder = device.create_command_encoder(&Default::default());
            stage.run(&device, &queue, &mut encoder, &assets, &jobs);
            queue.submit([encoder.finish()]);
            (stage.pose(1).unwrap(), stage.pose(2).unwrap())
        };

        let first = run([fox_palette(0.0), fox_palette(0.0)]);
        assert_ne!(first.0, first.1, "instances never share a pose number");
        let same = run([fox_palette(0.0), fox_palette(0.0)]);
        assert_eq!(same, first, "an equal palette keeps the pose");
        let moved = run([fox_palette(0.0), fox_palette(1.0)]);
        assert_eq!(moved.0, first.0);
        assert!(moved.1 != first.1 && moved.1 != first.0);
    }

    #[test]
    fn out_of_range_joints_clamp_to_the_last_matrix() {
        let vertex = v([0.0; 3], [0.0, 1.0, 0.0], [0.0; 2], [1.0, 0.0, 0.0, 1.0]);
        let skin = [SkinVertex::from_influences([(40, 1.0)])];
        let palette = [Mat4::IDENTITY, Mat4::from_translation(Vec3::Z)];
        let mut out = Vec::new();
        skin_vertices(&[vertex], &skin, &palette, &mut out);
        assert_eq!(out[0].pos, [0.0, 0.0, 1.0]);
    }
}
