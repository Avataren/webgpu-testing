use std::collections::HashMap;
use std::mem;
use std::num::NonZeroU64;

use bytemuck::Zeroable;
use wgpu::util::DeviceExt;

use crate::renderer::internal::{OrderedBatch, RenderContext, ShadowResources};
use crate::renderer::lights::{LightsData, LightsUniform, ShadowsUniform};
use crate::renderer::uniforms::CameraUniform;
use crate::renderer::{MaterialData, ObjectData};

pub(crate) struct DynamicObjectsBuffer {
    object_buffer: wgpu::Buffer,
    material_buffer: wgpu::Buffer,
    object_capacity: u32,
    material_capacity: u32,
    pub(crate) bind_group: wgpu::BindGroup,
    pub(crate) bind_layout: wgpu::BindGroupLayout,
    object_scratch: Vec<ObjectData>,
    material_scratch: Vec<MaterialData>,
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
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let object_buffer = Self::create_object_buffer(device, capacity);
        let material_buffer = Self::create_material_buffer(device, capacity);

        let bind_group =
            Self::create_bind_group(device, &bind_layout, &object_buffer, &material_buffer);

        Self {
            object_buffer,
            material_buffer,
            object_capacity: capacity,
            material_capacity: capacity,
            bind_group,
            bind_layout,
            object_scratch: Vec::with_capacity(capacity as usize),
            material_scratch: Vec::with_capacity(capacity as usize),
        }
    }

    pub(crate) fn update(
        &mut self,
        context: &RenderContext,
        batches: &[OrderedBatch],
    ) -> Result<(), wgpu::SurfaceError> {
        self.object_scratch.clear();
        self.material_scratch.clear();

        let mut material_lookup: HashMap<crate::renderer::Material, u32> = HashMap::new();

        for batch in batches {
            for instance in &batch.instances {
                let index = *material_lookup.entry(instance.material).or_insert_with(|| {
                    let next_index = self.material_scratch.len() as u32;
                    self.material_scratch
                        .push(MaterialData::from_material(&instance.material));
                    next_index
                });

                self.object_scratch.push(ObjectData::from_material_index(
                    instance.transform.matrix(),
                    index,
                ));
            }
        }

        let object_required = self.object_scratch.len() as u32;
        if object_required > self.object_capacity {
            self.grow_objects(context, object_required);
        }

        let material_required = self.material_scratch.len() as u32;
        if material_required > self.material_capacity {
            self.grow_materials(context, material_required);
        }

        if !self.object_scratch.is_empty() {
            context.queue.write_buffer(
                &self.object_buffer,
                0,
                bytemuck::cast_slice(&self.object_scratch),
            );
        }

        if !self.material_scratch.is_empty() {
            context.queue.write_buffer(
                &self.material_buffer,
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

        self.object_buffer = Self::create_object_buffer(&context.device, new_capacity);
        self.object_capacity = new_capacity;
        self.recreate_bind_group(&context.device);
    }

    fn grow_materials(&mut self, context: &RenderContext, required: u32) {
        let new_capacity = required.max(self.material_capacity * 2);
        log::info!(
            "Growing materials buffer: {} -> {}",
            self.material_capacity,
            new_capacity
        );

        self.material_buffer = Self::create_material_buffer(&context.device, new_capacity);
        self.material_capacity = new_capacity;
        self.recreate_bind_group(&context.device);
    }

    fn recreate_bind_group(&mut self, device: &wgpu::Device) {
        self.bind_group = Self::create_bind_group(
            device,
            &self.bind_layout,
            &self.object_buffer,
            &self.material_buffer,
        );
    }

    fn create_object_buffer(device: &wgpu::Device, capacity: u32) -> wgpu::Buffer {
        let buffer_size = (capacity as usize * mem::size_of::<ObjectData>()) as u64;
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ObjectsBuffer"),
            size: buffer_size.max(1),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn create_material_buffer(device: &wgpu::Device, capacity: u32) -> wgpu::Buffer {
        let buffer_size = (capacity as usize * mem::size_of::<MaterialData>()) as u64;
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("MaterialsBuffer"),
            size: buffer_size.max(1),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn create_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        object_buffer: &wgpu::Buffer,
        material_buffer: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ObjectsBindGroup"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: object_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: material_buffer.as_entire_binding(),
                },
            ],
        })
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
}

impl LightsBuffer {
    pub(crate) fn new(device: &wgpu::Device, shadows: &ShadowResources) -> Self {
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

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("LightsBindGroup"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
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
            ],
        });

        Self {
            buffer,
            shadow_buffer,
            bind_group,
            bind_layout: layout,
        }
    }

    pub(crate) fn update(&self, queue: &wgpu::Queue, lights: &LightsData) {
        let data = LightsUniform::from_data(lights);
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&data));
        let shadow_data = ShadowsUniform::from_data(lights);

        queue.write_buffer(&self.shadow_buffer, 0, bytemuck::bytes_of(&shadow_data));
    }
}
