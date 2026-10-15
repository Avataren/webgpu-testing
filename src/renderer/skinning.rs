//! Skinning: deforms each skinned mesh instance into a vertex buffer of its own, once
//! per frame, before any pass draws.
//!
//! Every pass (shadows, depth prepass, G-buffer, picking, transparency, user shader
//! materials) then draws the instance like a rigid mesh, so none of them needs a
//! skinning variant. The same buffers can later feed ray-tracing acceleration
//! structures.
//!
//! Two backends write identical buffers:
//! - **Compute** (default): `src/shader/skinning.wgsl`, one dispatch per instance.
//! - **CPU**: [`skin_vertices`] plus a buffer upload, for devices without compute or
//!   storage buffers (WebGL2/GLES). `WGPU_CUBE_FORCE_CPU_SKINNING=1` forces it, to test
//!   that path natively.

use crate::renderer::shader_source::wgsl;
use std::collections::HashMap;
use std::mem::size_of;
use std::num::NonZeroU64;

use glam::{Mat3, Mat4, Vec3, Vec4};

use crate::asset::{Assets, Handle, Mesh};
use crate::renderer::batch::{SkinJob, SkinKey};
use crate::renderer::{SkinVertex, Vertex};

const WORKGROUP_SIZE: u32 = 64;
/// Frames an instance may go undrawn before its buffer is freed.
const EVICT_AFTER_FRAMES: u64 = 120;
const INITIAL_PALETTE_CAPACITY: usize = 1024;

/// Per-dispatch parameters (std140 uniform, padded to 16 bytes).
#[repr(C, align(16))]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SkinParams {
    vertex_count: u32,
    palette_offset: u32,
    joint_count: u32,
    _padding: u32,
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
            && limits.max_storage_buffers_per_shader_stage >= 4;
        if compute && !forced_cpu {
            Self::Compute
        } else {
            Self::Cpu
        }
    }
}

struct SkinnedInstance {
    source: Handle<Mesh>,
    /// The source geometry drawn from the instance's buffer; shares the source mesh's
    /// data, so a replaced mesh is noticed and the instance rebuilt.
    mesh: Mesh,
    bind_group: Option<wgpu::BindGroup>,
    last_used: u64,
}

pub(crate) struct SkinningStage {
    backend: SkinningBackend,
    compute: Option<ComputeResources>,
    instances: HashMap<SkinKey, SkinnedInstance>,
    frame: u64,
    /// Instances skinned in the last frame.
    pub(crate) instance_count: u32,
    pub(crate) vertex_count: u32,
    cpu_scratch: Vec<Vertex>,
}

struct ComputeResources {
    pipeline: wgpu::ComputePipeline,
    instance_layout: wgpu::BindGroupLayout,
    shared_layout: wgpu::BindGroupLayout,
    palette: wgpu::Buffer,
    palette_capacity: usize,
    params: wgpu::Buffer,
    params_capacity: usize,
    params_stride: u64,
    shared_bind_group: wgpu::BindGroup,
    palette_scratch: Vec<[[f32; 4]; 4]>,
    params_scratch: Vec<u8>,
}

impl SkinningStage {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let backend = SkinningBackend::for_device(device);
        log::info!("Skinning backend: {backend:?}");
        Self {
            backend,
            compute: (backend == SkinningBackend::Compute).then(|| ComputeResources::new(device)),
            instances: HashMap::new(),
            frame: 0,
            instance_count: 0,
            vertex_count: 0,
            cpu_scratch: Vec::new(),
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
        self.instance_count = 0;
        self.vertex_count = 0;

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
            self.ensure_instance(device, key, job.mesh, source);
            ready.push((key, job));
        }

        match self.backend {
            SkinningBackend::Compute => self.run_compute(device, queue, encoder, assets, &ready),
            SkinningBackend::Cpu => self.run_cpu(queue, assets, &ready),
        }

        let frame = self.frame;
        self.instances
            .retain(|_, instance| frame - instance.last_used <= EVICT_AFTER_FRAMES);
    }

    /// The skinned geometry of an instance, if it was skinned.
    pub(crate) fn mesh(&self, key: SkinKey) -> Option<&Mesh> {
        self.instances.get(&key).map(|instance| &instance.mesh)
    }

    fn ensure_instance(
        &mut self,
        device: &wgpu::Device,
        key: SkinKey,
        handle: Handle<Mesh>,
        source: &Mesh,
    ) {
        let stale = self.instances.get(&key).is_none_or(|instance| {
            instance.source != handle || !instance.mesh.shares_data_with(source)
        });
        if stale {
            // COPY_SRC lets tools and tests read the deformed vertices back.
            let mut usage = wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_SRC;
            usage |= match self.backend {
                SkinningBackend::Compute => wgpu::BufferUsages::STORAGE,
                SkinningBackend::Cpu => wgpu::BufferUsages::COPY_DST,
            };
            let output = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("SkinnedVertexBuffer"),
                size: (source.data().vertices.len() * size_of::<Vertex>()).max(16) as u64,
                usage,
                mapped_at_creation: false,
            });
            let bind_group = self.compute.as_ref().and_then(|compute| {
                Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("SkinningInstance"),
                    layout: &compute.instance_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: source.vertex_buffer().as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: source.skin_buffer()?.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: output.as_entire_binding(),
                        },
                    ],
                }))
            });
            self.instances.insert(
                key,
                SkinnedInstance {
                    source: handle,
                    mesh: source.with_vertex_buffer(output),
                    bind_group,
                    last_used: self.frame,
                },
            );
        }
        if let Some(instance) = self.instances.get_mut(&key) {
            instance.last_used = self.frame;
        }
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
        compute.params_scratch.clear();
        let mut dispatches: Vec<(SkinKey, u32)> = Vec::with_capacity(ready.len());
        for (key, job) in ready {
            let Some(source) = assets.meshes.get(job.mesh) else {
                continue;
            };
            if self
                .instances
                .get(key)
                .is_none_or(|i| i.bind_group.is_none())
            {
                continue;
            }
            let params = SkinParams {
                vertex_count: source.vertex_count(),
                palette_offset: compute.palette_scratch.len() as u32,
                joint_count: job.palette.len() as u32,
                _padding: 0,
            };
            compute
                .palette_scratch
                .extend(job.palette.iter().map(Mat4::to_cols_array_2d));
            // Each block sits at a multiple of the uniform offset alignment.
            let start = dispatches.len() * compute.params_stride as usize;
            compute
                .params_scratch
                .resize(start + compute.params_stride as usize, 0);
            compute.params_scratch[start..start + size_of::<SkinParams>()]
                .copy_from_slice(bytemuck::bytes_of(&params));
            dispatches.push((*key, params.vertex_count));
            self.vertex_count += params.vertex_count;
        }
        if dispatches.is_empty() {
            return;
        }
        compute.reserve(device, compute.palette_scratch.len(), dispatches.len());
        queue.write_buffer(
            &compute.palette,
            0,
            bytemuck::cast_slice(&compute.palette_scratch),
        );
        queue.write_buffer(&compute.params, 0, &compute.params_scratch);

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Skinning"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&compute.pipeline);
        for (index, (key, vertex_count)) in dispatches.iter().enumerate() {
            let Some(bind_group) = self.instances.get(key).and_then(|i| i.bind_group.as_ref())
            else {
                continue;
            };
            pass.set_bind_group(0, bind_group, &[]);
            let offset = (index as u64 * compute.params_stride) as u32;
            pass.set_bind_group(1, &compute.shared_bind_group, &[offset]);
            pass.dispatch_workgroups(vertex_count.div_ceil(WORKGROUP_SIZE), 1, 1);
        }
        self.instance_count = dispatches.len() as u32;
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
                instance.mesh.vertex_buffer(),
                0,
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
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
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

        let alignment = device.limits().min_uniform_buffer_offset_alignment as u64;
        let params_stride = (size_of::<SkinParams>() as u64).div_ceil(alignment) * alignment;
        let palette = create_palette_buffer(device, INITIAL_PALETTE_CAPACITY);
        let params = create_params_buffer(device, 64, params_stride);
        let shared_bind_group = create_shared_bind_group(device, &shared_layout, &palette, &params);
        Self {
            pipeline,
            instance_layout,
            shared_layout,
            palette,
            palette_capacity: INITIAL_PALETTE_CAPACITY,
            params,
            params_capacity: 64,
            params_stride,
            shared_bind_group,
            palette_scratch: Vec::new(),
            params_scratch: Vec::new(),
        }
    }

    /// Grow the shared buffers to hold `matrices` palette entries and `dispatches`
    /// parameter blocks.
    fn reserve(&mut self, device: &wgpu::Device, matrices: usize, dispatches: usize) {
        let mut changed = false;
        if matrices > self.palette_capacity {
            self.palette_capacity = matrices.next_power_of_two();
            self.palette = create_palette_buffer(device, self.palette_capacity);
            changed = true;
        }
        if dispatches > self.params_capacity {
            self.params_capacity = dispatches.next_power_of_two();
            self.params = create_params_buffer(device, self.params_capacity, self.params_stride);
            changed = true;
        }
        if changed {
            self.shared_bind_group =
                create_shared_bind_group(device, &self.shared_layout, &self.palette, &self.params);
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
        assert_eq!(size_of::<SkinVertex>(), 16);
        assert_eq!(size_of::<Vertex>(), 16 * 4);
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

    /// The compute shader and the CPU fallback must produce the same vertices.
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
        let palette: Arc<[Mat4]> = (0..24)
            .map(|i| {
                let f = i as f32;
                Mat4::from_scale_rotation_translation(
                    Vec3::splat(1.0 + 0.01 * f),
                    Quat::from_euler(glam::EulerRot::XYZ, 0.1 * f, -0.07 * f, 0.03 * f),
                    Vec3::new(0.02 * f, -0.01 * f, 0.005 * f),
                )
            })
            .collect::<Vec<_>>()
            .into();

        let mut expected = Vec::new();
        skin_vertices(
            &data.vertices,
            data.skin.as_ref().unwrap(),
            &palette,
            &mut expected,
        );

        let mut assets = Assets::new();
        let mesh = assets.meshes.insert(Mesh::from_data(&device, data));
        let mut stage = SkinningStage::new(&device);
        if stage.backend() != SkinningBackend::Compute {
            eprintln!("Skipping: adapter cannot run compute skinning");
            return;
        }
        let jobs = HashMap::from([(7, SkinJob { mesh, palette })]);
        let mut encoder = device.create_command_encoder(&Default::default());
        stage.run(&device, &queue, &mut encoder, &assets, &jobs);
        let output = stage.mesh(7).unwrap().vertex_buffer().clone();
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: output.size(),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, output.size());
        queue.submit([encoder.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let bytes = readback.slice(..).get_mapped_range().unwrap().to_vec();
        let gpu: &[Vertex] = bytemuck::cast_slice(&bytes[..expected.len() * size_of::<Vertex>()]);

        for (index, (gpu, cpu)) in gpu.iter().zip(&expected).enumerate() {
            let close = |a: &[f32], b: &[f32]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-4);
            assert!(
                close(&gpu.pos, &cpu.pos)
                    && close(&gpu.normal, &cpu.normal)
                    && close(&gpu.tangent, &cpu.tangent)
                    && gpu.uv == cpu.uv,
                "vertex {index}: gpu {gpu:?} cpu {cpu:?}"
            );
        }
        assert_eq!(stage.instance_count, 1);
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
