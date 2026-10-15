use std::collections::HashMap;
use std::mem;
use std::num::NonZeroU64;

use bytemuck::Zeroable;
use wgpu::util::DeviceExt;

use crate::renderer::internal::{
    environment::EnvironmentResources, OrderedBatch, RenderContext, ShadowResources,
};
use crate::renderer::lights::{LightsData, LightsUniform, ShadowsUniform};
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

pub(crate) struct LightsBuffer {
    pub(crate) buffer: wgpu::Buffer,
    pub(crate) shadow_buffer: wgpu::Buffer,
    pub(crate) bind_group: wgpu::BindGroup,
    pub(crate) bind_layout: wgpu::BindGroupLayout,
    last_lights: Option<LightsUniform>,
    last_shadows: Option<ShadowsUniform>,
    /// The ray-traced shadow mask (binding 11), or a white 1x1 texture without one.
    rt_shadow_mask: Option<wgpu::TextureView>,
    _fallback_mask: wgpu::Texture,
    fallback_mask_view: wgpu::TextureView,
    /// Directional lights whose shadows the main pass reads from the mask (`counts.w`).
    rt_shadow_bits: u32,
}

impl LightsBuffer {
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        shadows: &ShadowResources,
        environment: &EnvironmentResources,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("LightsBindLayout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: Some(
                            NonZeroU64::new(mem::size_of::<LightsUniform>() as u64).unwrap(),
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: Some(
                            NonZeroU64::new(mem::size_of::<ShadowsUniform>() as u64).unwrap(),
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let initial = LightsUniform::zeroed();
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("LightsBuffer"),
            contents: bytemuck::bytes_of(&initial),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        let shadow_initial = ShadowsUniform::zeroed();
        let shadow_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ShadowUniformBuffer"),
            contents: bytemuck::bytes_of(&shadow_initial),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

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

        let bind_group = Self::create_bind_group(
            device,
            &layout,
            &buffer,
            &shadow_buffer,
            shadows,
            environment,
            &fallback_mask_view,
        );

        Self {
            buffer,
            shadow_buffer,
            bind_group,
            bind_layout: layout,
            last_lights: None,
            last_shadows: None,
            rt_shadow_mask: None,
            _fallback_mask: fallback_mask,
            fallback_mask_view,
            rt_shadow_bits: 0,
        }
    }

    fn create_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        lights_buffer: &wgpu::Buffer,
        shadow_buffer: &wgpu::Buffer,
        shadows: &ShadowResources,
        environment: &EnvironmentResources,
        rt_shadow_mask: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
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
                    resource: wgpu::BindingResource::TextureView(shadows.spot_array_view()),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(shadows.sampler()),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(shadows.point_array_view()),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::Sampler(shadows.sampler()),
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
            ],
        })
    }

    pub(crate) fn update(&mut self, queue: &wgpu::Queue, lights: &LightsData) {
        let data = LightsUniform::from_data(lights);
        let should_update_lights = self
            .last_lights
            .as_ref()
            .map(|last| last != &data)
            .unwrap_or(true);

        if should_update_lights {
            queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&data));
            self.last_lights = Some(data);
            // The upload wrote counts.w (the ray-traced shadow bits) as 0.
            self.rt_shadow_bits = 0;
        }

        let shadow_data = ShadowsUniform::from_data(lights);
        let should_update_shadows = self
            .last_shadows
            .as_ref()
            .map(|last| last != &shadow_data)
            .unwrap_or(true);

        if should_update_shadows {
            queue.write_buffer(&self.shadow_buffer, 0, bytemuck::bytes_of(&shadow_data));
            self.last_shadows = Some(shadow_data);
        }
    }

    /// Which lights (bits 0-3 directional, 4-7 point, 8-11 spot) take their shadow from the
    /// ray-traced mask this frame (`lights.counts.w`).
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
            &self.buffer,
            &self.shadow_buffer,
            shadows,
            environment,
            self.rt_shadow_mask
                .as_ref()
                .unwrap_or(&self.fallback_mask_view),
        );
    }
}
