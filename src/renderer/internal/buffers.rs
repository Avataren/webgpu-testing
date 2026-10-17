use std::collections::HashMap;
use std::mem;
use std::num::NonZeroU64;

use bytemuck::Zeroable;
use wgpu::util::DeviceExt;

use crate::renderer::internal::{
    environment::EnvironmentResources, OrderedBatch, RenderContext, ShadowResources,
};
use crate::renderer::light_culling::{light_lists_size, INITIAL_POOL};
use crate::renderer::lights::{LightsHeader, LocalLightRaw, ShadowsUniform, LOCAL_LIGHTS_OFFSET};
use crate::renderer::material::Material;
use crate::renderer::uniforms::CameraUniform;
use crate::renderer::{batch::InstanceSource, MaterialData, ObjectData};

/// Last frame's model matrix of every instance, for motion vectors. Indexed like the
/// objects buffer; an all-zero entry means "no history, use the current matrix".
type PrevModel = [[f32; 4]; 4];

/// Identifies an instance across frames: its pick id (the entity), mesh, and which
/// occurrence of that pair it is this frame.
type InstanceKey = (u64, usize, u32);

pub(crate) struct DynamicObjectsBuffer {
    pub(crate) objects: wgpu::Buffer,
    prev_models: wgpu::Buffer,
    prev_scratch: Vec<PrevModel>,
    last_transforms: HashMap<InstanceKey, PrevModel>,
    current_transforms: HashMap<InstanceKey, PrevModel>,
    pub(crate) materials: wgpu::Buffer,
    pub(crate) object_capacity: u32,
    pub(crate) material_capacity: u32,
    pub(crate) bind_group: wgpu::BindGroup,
    pub(crate) bind_layout: wgpu::BindGroupLayout,
    pub(crate) object_scratch: Vec<ObjectData>,
    pub(crate) material_scratch: Vec<MaterialData>,
    cpu_segments: Vec<CpuSegment>,
}

#[derive(Clone, Copy, Debug)]
struct CpuSegment {
    start_index: u32,
    length: usize,
    scratch_start: usize,
}

impl DynamicObjectsBuffer {
    pub(crate) fn new(device: &wgpu::Device, capacity: u32) -> Self {
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ObjectsBindLayout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(mem::size_of::<ObjectData>() as u64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(mem::size_of::<MaterialData>() as u64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(mem::size_of::<PrevModel>() as u64),
                    },
                    count: None,
                },
            ],
        });

        let object_buffer_size = (capacity as usize * mem::size_of::<ObjectData>()) as u64;
        let material_buffer_size = (capacity as usize * mem::size_of::<MaterialData>()) as u64;

        let objects = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ObjectsBuffer"),
            size: object_buffer_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let materials = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("MaterialsBuffer"),
            size: material_buffer_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let prev_models = create_prev_models(device, capacity);
        let bind_group =
            create_objects_bind_group(device, &bind_layout, &objects, &materials, &prev_models);

        Self {
            objects,
            prev_models,
            prev_scratch: Vec::with_capacity(capacity as usize),
            last_transforms: HashMap::new(),
            current_transforms: HashMap::new(),
            materials,
            object_capacity: capacity,
            material_capacity: capacity,
            bind_group,
            bind_layout,
            object_scratch: Vec::with_capacity(capacity as usize),
            material_scratch: Vec::with_capacity(capacity as usize),
            cpu_segments: Vec::new(),
        }
    }

    pub(crate) fn update(
        &mut self,
        context: &RenderContext,
        batches: &[OrderedBatch],
        materials: &[Material],
    ) -> Result<(), crate::renderer::SurfaceError> {
        self.object_scratch.clear();
        self.prev_scratch.clear();
        self.cpu_segments.clear();
        self.current_transforms.clear();
        let mut occurrences: HashMap<(u64, usize), u32> = HashMap::new();

        let mut current_segment: Option<CpuSegment> = None;
        let mut total_instances: u32 = 0;

        for batch in batches {
            for (local_index, inst) in batch.instances.iter().enumerate() {
                let global_index = if inst.source == InstanceSource::Gpu {
                    inst.gpu_index
                        .unwrap_or(batch.first_instance + local_index as u32)
                } else {
                    batch.first_instance + local_index as u32
                };
                total_instances = total_instances.max(global_index + 1);

                if inst.source == InstanceSource::Gpu {
                    if let Some(segment) = current_segment.take() {
                        self.cpu_segments.push(segment);
                    }
                    continue;
                }

                let model = inst.transform.matrix();
                let data = ObjectData::new(model, inst.material_index, inst.pick_id);
                let scratch_index = self.object_scratch.len();
                self.object_scratch.push(data);
                // Unidentified instances (pick id 0) get no history, so no motion.
                let prev = if inst.pick_id == 0 {
                    PrevModel::zeroed()
                } else {
                    let pair = (inst.pick_id, batch.mesh.index());
                    let occurrence = occurrences.entry(pair).or_insert(0);
                    let key = (pair.0, pair.1, *occurrence);
                    *occurrence += 1;
                    let current = model.to_cols_array_2d();
                    self.current_transforms.insert(key, current);
                    self.last_transforms.get(&key).copied().unwrap_or(current)
                };
                self.prev_scratch.push(prev);

                if let Some(segment) = current_segment.as_mut() {
                    if global_index == segment.start_index + segment.length as u32 {
                        segment.length += 1;
                    } else {
                        self.cpu_segments.push(*segment);
                        *segment = CpuSegment {
                            start_index: global_index,
                            length: 1,
                            scratch_start: scratch_index,
                        };
                    }
                } else {
                    current_segment = Some(CpuSegment {
                        start_index: global_index,
                        length: 1,
                        scratch_start: scratch_index,
                    });
                }
            }
        }

        if let Some(segment) = current_segment.take() {
            self.cpu_segments.push(segment);
        }

        if total_instances > self.object_capacity {
            self.grow_objects(context, total_instances);
        }

        for segment in &self.cpu_segments {
            let start = segment.start_index as usize;
            let offset = (start * mem::size_of::<ObjectData>()) as u64;
            let end = segment.scratch_start + segment.length;
            let slice = &self.object_scratch[segment.scratch_start..end];
            context
                .queue
                .write_buffer(&self.objects, offset, bytemuck::cast_slice(slice));
            let prev_offset = (start * mem::size_of::<PrevModel>()) as u64;
            context.queue.write_buffer(
                &self.prev_models,
                prev_offset,
                bytemuck::cast_slice(&self.prev_scratch[segment.scratch_start..end]),
            );
        }
        std::mem::swap(&mut self.last_transforms, &mut self.current_transforms);

        self.material_scratch.clear();
        self.material_scratch
            .extend(materials.iter().map(MaterialData::from_material));

        let required_materials = self.material_scratch.len() as u32;
        if required_materials > self.material_capacity {
            self.grow_materials(context, required_materials);
        }

        if !self.material_scratch.is_empty() {
            context.queue.write_buffer(
                &self.materials,
                0,
                bytemuck::cast_slice(&self.material_scratch),
            );
        }

        Ok(())
    }

    fn grow_objects(&mut self, context: &RenderContext, required: u32) {
        let new_capacity = required.max(self.object_capacity * 2);
        log::info!(
            "Growing objects buffer: {} -> {}",
            self.object_capacity,
            new_capacity
        );

        let buffer_size = (new_capacity as usize * mem::size_of::<ObjectData>()) as u64;
        self.objects = context.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ObjectsBuffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        self.prev_models = create_prev_models(&context.device, new_capacity);
        self.object_capacity = new_capacity;
        self.rebuild_bind_group(context);
    }

    fn grow_materials(&mut self, context: &RenderContext, required: u32) {
        let new_capacity = required.max(self.material_capacity * 2);
        log::info!(
            "Growing materials buffer: {} -> {}",
            self.material_capacity,
            new_capacity
        );

        let buffer_size = (new_capacity as usize * mem::size_of::<MaterialData>()) as u64;
        self.materials = context.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("MaterialsBuffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        self.material_capacity = new_capacity;
        self.rebuild_bind_group(context);
    }

    fn rebuild_bind_group(&mut self, context: &RenderContext) {
        self.bind_group = create_objects_bind_group(
            &context.device,
            &self.bind_layout,
            &self.objects,
            &self.materials,
            &self.prev_models,
        );
    }

    pub(crate) fn ensure_capacity(&mut self, context: &RenderContext, required: u32) {
        if required > self.object_capacity {
            self.grow_objects(context, required);
        }
    }

    pub(crate) fn buffer(&self) -> &wgpu::Buffer {
        &self.objects
    }
}

fn create_prev_models(device: &wgpu::Device, capacity: u32) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("PrevModelsBuffer"),
        size: (capacity.max(1) as usize * mem::size_of::<PrevModel>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn create_objects_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    objects: &wgpu::Buffer,
    materials: &wgpu::Buffer,
    prev_models: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ObjectsBindGroup"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: objects.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: materials.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: prev_models.as_entire_binding(),
            },
        ],
    })
}

pub(crate) struct CameraBuffer {
    pub(crate) buffer: wgpu::Buffer,
    pub(crate) bind_group: wgpu::BindGroup,
    pub(crate) bind_layout: wgpu::BindGroupLayout,
    /// The same camera without the TAA jitter, for overlays and gizmos drawn on the
    /// finished image (jittered, thin lines would shimmer).
    pub(crate) stable_buffer: wgpu::Buffer,
    pub(crate) stable_bind_group: wgpu::BindGroup,
}

impl CameraBuffer {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let camera = CameraUniform::new();
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("CameraBuffer"),
            contents: bytemuck::bytes_of(&camera),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("CameraBindLayout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: Some(
                        NonZeroU64::new(mem::size_of::<CameraUniform>() as u64).unwrap(),
                    ),
                },
                count: None,
            }],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("CameraBindGroup"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });

        let stable_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("StableCameraBuffer"),
            contents: bytemuck::bytes_of(&camera),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let stable_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("StableCameraBindGroup"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: stable_buffer.as_entire_binding(),
            }],
        });

        Self {
            buffer,
            bind_group,
            bind_layout,
            stable_buffer,
            stable_bind_group,
        }
    }
}

/// The lights group (group 2 of the lit passes): the lights and their cluster lists,
/// shadows, environment, the ray-traced shadow mask, the light probes and the fog. The
/// fog's froxel volume reads it from a compute pass, so it is visible to compute shaders
/// too.
pub(crate) struct LightsBuffer {
    /// `LightsHeader`, then the local lights (see `lights.rs`); grows with them.
    pub(crate) buffer: wgpu::Buffer,
    local_capacity: usize,
    pub(crate) shadow_buffer: wgpu::Buffer,
    /// Which lights reach each cluster and world grid cell (`light_culling.rs`).
    pub(crate) lists: wgpu::Buffer,
    /// Light indices the lists' pool holds.
    pub(crate) pool: u32,
    pub(crate) bind_group: wgpu::BindGroup,
    pub(crate) bind_layout: wgpu::BindGroupLayout,
    last_header: Option<LightsHeader>,
    last_locals: Vec<LocalLightRaw>,
    last_shadows: Option<Box<ShadowsUniform>>,
    /// The ray-traced shadow mask (binding 11), or a white 1x1 texture without one.
    rt_shadow_mask: Option<wgpu::TextureView>,
    _fallback_mask: wgpu::Texture,
    fallback_mask_view: wgpu::TextureView,
    /// Lights whose shadows the main pass reads from the mask (`counts.w`).
    rt_shadow_bits: u32,
    /// The light probe grid (bindings 12-14): its uniform and current atlases.
    probe_buffer: wgpu::Buffer,
    probe_irradiance: wgpu::TextureView,
    probe_distance: wgpu::TextureView,
    /// The fog (bindings 16-17): its uniform and integrated froxel volume.
    fog_uniform: wgpu::Buffer,
    fog_volume: wgpu::TextureView,
}

/// The shader stages the lights group is visible to: the lit passes' fragments, and the
/// fog volume's compute pass.
const LIGHTS_STAGES: wgpu::ShaderStages =
    wgpu::ShaderStages::FRAGMENT.union(wgpu::ShaderStages::COMPUTE);

/// Local lights the lights buffer has room for at first.
const INITIAL_LOCAL_LIGHTS: usize = 64;

fn create_lights_buffer(device: &wgpu::Device, local_capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("LightsBuffer"),
        size: LOCAL_LIGHTS_OFFSET
            + (local_capacity.max(1) * mem::size_of::<LocalLightRaw>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn create_light_lists(device: &wgpu::Device, pool: u32) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("LightListsBuffer"),
        size: light_lists_size(pool),
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

impl LightsBuffer {
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        shadows: &ShadowResources,
        environment: &EnvironmentResources,
        probes: &crate::renderer::light_probes::LightProbes,
        fog: &crate::renderer::fog::FogResources,
    ) -> Self {
        let probe_texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: LIGHTS_STAGES,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let depth_texture = |binding, view_dimension| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: LIGHTS_STAGES,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("LightsBindLayout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: LIGHTS_STAGES,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        // The header and room for one local light (the shader's runtime
                        // array counts as one element).
                        min_binding_size: NonZeroU64::new(
                            LOCAL_LIGHTS_OFFSET + mem::size_of::<LocalLightRaw>() as u64,
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: LIGHTS_STAGES,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: Some(
                            NonZeroU64::new(mem::size_of::<ShadowsUniform>() as u64).unwrap(),
                        ),
                    },
                    count: None,
                },
                depth_texture(2, wgpu::TextureViewDimension::D2Array),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: LIGHTS_STAGES,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                depth_texture(4, wgpu::TextureViewDimension::D2),
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: LIGHTS_STAGES,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: LIGHTS_STAGES,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: LIGHTS_STAGES,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
                    visibility: LIGHTS_STAGES,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 12,
                    visibility: LIGHTS_STAGES,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                probe_texture(13),
                probe_texture(14),
                wgpu::BindGroupLayoutEntry {
                    binding: 15,
                    visibility: LIGHTS_STAGES,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 16,
                    visibility: LIGHTS_STAGES,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 17,
                    visibility: LIGHTS_STAGES,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let local_capacity = INITIAL_LOCAL_LIGHTS;
        let buffer = create_lights_buffer(device, local_capacity);
        queue.write_buffer(&buffer, 0, bytemuck::bytes_of(&LightsHeader::zeroed()));

        let shadow_initial = ShadowsUniform::zeroed();
        let shadow_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ShadowUniformBuffer"),
            contents: bytemuck::bytes_of(&shadow_initial),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let pool = INITIAL_POOL;
        let lists = create_light_lists(device, pool);

        // Every light reaches every pixel: what the main pass reads when no frame traced
        // shadows.
        let fallback_mask = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("RtShadowMaskFallback"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: crate::renderer::raytracing::RT_SHADOW_MASK_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &[255; 4],
        );
        let fallback_mask_view = fallback_mask.create_view(&Default::default());

        let probe_buffer = probes.grid_buffer().clone();
        let probe_irradiance = probes.irradiance_view().clone();
        let probe_distance = probes.distance_view().clone();
        let fog_uniform = fog.uniform.clone();
        let fog_volume = fog.fallback_view.clone();
        let bind_group = Self::create_bind_group(
            device,
            &layout,
            [&buffer, &shadow_buffer, &lists, &fog_uniform],
            shadows,
            environment,
            &fallback_mask_view,
            [&probe_buffer, &probe_buffer],
            [&probe_irradiance, &probe_distance, &fog_volume],
        );

        Self {
            buffer,
            local_capacity,
            shadow_buffer,
            lists,
            pool,
            bind_group,
            bind_layout: layout,
            last_header: None,
            last_locals: Vec::new(),
            last_shadows: None,
            rt_shadow_mask: None,
            _fallback_mask: fallback_mask,
            fallback_mask_view,
            rt_shadow_bits: 0,
            probe_buffer,
            probe_irradiance,
            probe_distance,
            fog_uniform,
            fog_volume,
        }
    }

    /// `buffers`: the lights, the shadows uniform, the light lists and the fog uniform;
    /// `views`: the light probes' irradiance and distances, and the fog volume.
    #[allow(clippy::too_many_arguments)]
    fn create_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        buffers: [&wgpu::Buffer; 4],
        shadows: &ShadowResources,
        environment: &EnvironmentResources,
        rt_shadow_mask: &wgpu::TextureView,
        probe_buffers: [&wgpu::Buffer; 2],
        views: [&wgpu::TextureView; 3],
    ) -> wgpu::BindGroup {
        let [lights_buffer, shadow_buffer, lists, fog_uniform] = buffers;
        let [probe_irradiance, probe_distance, fog_volume] = views;
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("LightsBindGroup"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: lights_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: shadow_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(shadows.directional_array_view()),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(shadows.sampler()),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(shadows.atlas_view()),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: environment.uniform_buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(environment.texture_view()),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::Sampler(environment.sampler()),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: wgpu::BindingResource::TextureView(rt_shadow_mask),
                },
                wgpu::BindGroupEntry {
                    binding: 12,
                    resource: probe_buffers[0].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 13,
                    resource: wgpu::BindingResource::TextureView(probe_irradiance),
                },
                wgpu::BindGroupEntry {
                    binding: 14,
                    resource: wgpu::BindingResource::TextureView(probe_distance),
                },
                wgpu::BindGroupEntry {
                    binding: 15,
                    resource: lists.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 16,
                    resource: fog_uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 17,
                    resource: wgpu::BindingResource::TextureView(fog_volume),
                },
            ],
        })
    }

    /// Upload the frame's lights: the header, the local lights and the shadows uniform
    /// (each only when it changed). Grows the lights buffer when the lights outgrow it;
    /// returns whether it did (the bind groups that hold it were rebuilt here, the light
    /// culling pass rebuilds its own).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        header: &LightsHeader,
        locals: &[LocalLightRaw],
        shadow_data: &ShadowsUniform,
        shadows: &ShadowResources,
        environment: &EnvironmentResources,
    ) -> bool {
        let mut grown = false;
        if locals.len() > self.local_capacity {
            self.local_capacity = locals.len().next_power_of_two();
            self.buffer = create_lights_buffer(device, self.local_capacity);
            self.last_header = None;
            self.last_locals.clear();
            self.rebuild_bind_group(device, shadows, environment);
            grown = true;
        }
        if self.last_header.as_ref() != Some(header) {
            queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(header));
            self.last_header = Some(*header);
            // The upload wrote counts.w (the ray-traced shadow bits) as 0.
            self.rt_shadow_bits = 0;
        }
        if self.last_locals != locals && !locals.is_empty() {
            queue.write_buffer(
                &self.buffer,
                LOCAL_LIGHTS_OFFSET,
                bytemuck::cast_slice(locals),
            );
        }
        if self.last_locals != locals {
            self.last_locals.clear();
            self.last_locals.extend_from_slice(locals);
        }
        if self.last_shadows.as_deref() != Some(shadow_data) {
            queue.write_buffer(&self.shadow_buffer, 0, bytemuck::bytes_of(shadow_data));
            self.last_shadows = Some(Box::new(*shadow_data));
        }
        grown
    }

    /// Make the light lists' pool hold `pool` light indices.
    pub(crate) fn resize_lists(
        &mut self,
        device: &wgpu::Device,
        pool: u32,
        shadows: &ShadowResources,
        environment: &EnvironmentResources,
    ) {
        self.pool = pool;
        self.lists = create_light_lists(device, pool);
        self.rebuild_bind_group(device, shadows, environment);
    }

    /// Which lights (bits 0-3 directional, 4-31 point and spot lights given a bit) take
    /// their shadow from the ray-traced mask this frame (`lights.counts.w`).
    pub(crate) fn set_rt_shadow_bits(&mut self, queue: &wgpu::Queue, bits: u32) {
        if bits != self.rt_shadow_bits {
            queue.write_buffer(&self.buffer, 12, bytemuck::bytes_of(&bits));
            self.rt_shadow_bits = bits;
        }
    }

    /// Bind `mask` as the ray-traced shadow mask (None: the white fallback).
    pub(crate) fn set_rt_shadow_mask(
        &mut self,
        device: &wgpu::Device,
        shadows: &ShadowResources,
        environment: &EnvironmentResources,
        mask: Option<&wgpu::TextureView>,
    ) {
        self.rt_shadow_mask = mask.cloned();
        self.rebuild_bind_group(device, shadows, environment);
    }

    pub(crate) fn rebuild_bind_group(
        &mut self,
        device: &wgpu::Device,
        shadows: &ShadowResources,
        environment: &EnvironmentResources,
    ) {
        self.bind_group = Self::create_bind_group(
            device,
            &self.bind_layout,
            [
                &self.buffer,
                &self.shadow_buffer,
                &self.lists,
                &self.fog_uniform,
            ],
            shadows,
            environment,
            self.rt_shadow_mask
                .as_ref()
                .unwrap_or(&self.fallback_mask_view),
            [&self.probe_buffer, &self.probe_buffer],
            [
                &self.probe_irradiance,
                &self.probe_distance,
                &self.fog_volume,
            ],
        );
    }

    /// Bind `volume` as the fog's integrated froxel volume (after it was recreated, or
    /// the volume of no fog).
    pub(crate) fn set_fog_volume(
        &mut self,
        device: &wgpu::Device,
        shadows: &ShadowResources,
        environment: &EnvironmentResources,
        volume: &wgpu::TextureView,
    ) {
        self.fog_volume = volume.clone();
        self.rebuild_bind_group(device, shadows, environment);
    }

    /// Bind the light probe grid's current atlases (after they were recreated).
    pub(crate) fn set_light_probes(
        &mut self,
        device: &wgpu::Device,
        shadows: &ShadowResources,
        environment: &EnvironmentResources,
        probes: &crate::renderer::light_probes::LightProbes,
    ) {
        self.probe_buffer = probes.grid_buffer().clone();
        self.probe_irradiance = probes.irradiance_view().clone();
        self.probe_distance = probes.distance_view().clone();
        self.rebuild_bind_group(device, shadows, environment);
    }
}
