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

pub(crate) struct DynamicObjectsBuffer {
    pub(crate) objects: wgpu::Buffer,
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

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ObjectsBindGroup"),
            layout: &bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: objects.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: materials.as_entire_binding(),
                },
            ],
        });

        Self {
            objects,
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
    ) -> Result<(), wgpu::SurfaceError> {
        self.object_scratch.clear();
        self.cpu_segments.clear();

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

                let data = ObjectData::new(inst.transform.matrix(), inst.material_index);
                let scratch_index = self.object_scratch.len();
                self.object_scratch.push(data);

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
        }

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
        self.bind_group = context
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ObjectsBindGroup"),
                layout: &self.bind_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.objects.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.materials.as_entire_binding(),
                    },
                ],
            });
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

pub(crate) struct CameraBuffer {
    pub(crate) buffer: wgpu::Buffer,
    pub(crate) bind_group: wgpu::BindGroup,
    pub(crate) bind_layout: wgpu::BindGroupLayout,
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

        Self {
            buffer,
            bind_group,
            bind_layout,
        }
    }
}

pub(crate) struct LightsBuffer {
    pub(crate) buffer: wgpu::Buffer,
    pub(crate) shadow_buffer: wgpu::Buffer,
    pub(crate) bind_group: wgpu::BindGroup,
    pub(crate) bind_layout: wgpu::BindGroupLayout,
    last_lights: LightsUniform,
    last_shadows: ShadowsUniform,
}

impl LightsBuffer {
    pub(crate) fn new(
        device: &wgpu::Device,
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

        let bind_group = Self::create_bind_group(
            device,
            &layout,
            &buffer,
            &shadow_buffer,
            shadows,
            environment,
        );

        Self {
            buffer,
            shadow_buffer,
            bind_group,
            bind_layout: layout,
            last_lights: initial,
            last_shadows: shadow_initial,
        }
    }

    fn create_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        lights_buffer: &wgpu::Buffer,
        shadow_buffer: &wgpu::Buffer,
        shadows: &ShadowResources,
        environment: &EnvironmentResources,
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
            ],
        })
    }

    pub(crate) fn update(&mut self, queue: &wgpu::Queue, lights: &LightsData) {
        let data = LightsUniform::from_data(lights);
        if bytemuck::bytes_of(&self.last_lights) != bytemuck::bytes_of(&data) {
            queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&data));
            self.last_lights = data;
        }

        let shadow_data = ShadowsUniform::from_data(lights);
        if bytemuck::bytes_of(&self.last_shadows) != bytemuck::bytes_of(&shadow_data) {
            queue.write_buffer(&self.shadow_buffer, 0, bytemuck::bytes_of(&shadow_data));
            self.last_shadows = shadow_data;
        }
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
        );
    }
}
