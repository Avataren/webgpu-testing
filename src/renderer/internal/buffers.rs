use std::mem;
use std::num::NonZeroU64;

use bytemuck::Zeroable;
use wgpu::util::DeviceExt;

use crate::renderer::internal::{OrderedBatch, RenderContext, ShadowResources};
use crate::renderer::lights::{LightsData, LightsUniform, ShadowsUniform};
use crate::renderer::uniforms::CameraUniform;

pub(crate) struct DynamicObjectsBuffer {
    pub(crate) objects_buffer: wgpu::Buffer,
    pub(crate) materials_buffer: wgpu::Buffer,
    pub(crate) object_capacity: u32,
    pub(crate) material_capacity: u32,
    pub(crate) bind_group: wgpu::BindGroup,
    pub(crate) bind_layout: wgpu::BindGroupLayout,
    pub(crate) object_scratch: Vec<crate::renderer::ObjectData>,
    pub(crate) material_scratch: Vec<crate::renderer::MaterialData>,
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

        let object_buffer_size =
            (capacity as usize * mem::size_of::<crate::renderer::ObjectData>()) as u64;
        let objects_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ObjectsBuffer"),
            size: object_buffer_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let material_buffer_size =
            (capacity as usize * mem::size_of::<crate::renderer::MaterialData>()) as u64;
        let materials_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("MaterialsBuffer"),
            size: material_buffer_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group =
            Self::create_bind_group(device, &bind_layout, &objects_buffer, &materials_buffer);

        Self {
            objects_buffer,
            materials_buffer,
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
        materials: &[crate::renderer::Material],
    ) -> Result<(), wgpu::SurfaceError> {
        self.object_scratch.clear();
        for batch in batches {
            self.object_scratch
                .extend(batch.instances.iter().map(|inst| {
                    crate::renderer::ObjectData::from_instance(
                        inst.transform.matrix(),
                        inst.material_index,
                    )
                }));
        }

        let required = self.object_scratch.len() as u32;
        if required > self.object_capacity {
            self.grow_objects(context, required);
        }

        if !self.object_scratch.is_empty() {
            context.queue.write_buffer(
                &self.objects_buffer,
                0,
                bytemuck::cast_slice(&self.object_scratch),
            );
        }

        self.material_scratch.clear();
        self.material_scratch.extend(
            materials
                .iter()
                .map(crate::renderer::MaterialData::from_material),
        );

        let material_required = self.material_scratch.len() as u32;
        if material_required > self.material_capacity {
            self.grow_materials(context, material_required);
        }

        if !self.material_scratch.is_empty() {
            context.queue.write_buffer(
                &self.materials_buffer,
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

        let buffer_size =
            (new_capacity as usize * mem::size_of::<crate::renderer::ObjectData>()) as u64;
        self.objects_buffer = context.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ObjectsBuffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

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

        let buffer_size =
            (new_capacity as usize * mem::size_of::<crate::renderer::MaterialData>()) as u64;
        self.materials_buffer = context.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("MaterialsBuffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        self.material_capacity = new_capacity;
        self.recreate_bind_group(&context.device);
    }

    fn recreate_bind_group(&mut self, device: &wgpu::Device) {
        self.bind_group = Self::create_bind_group(
            device,
            &self.bind_layout,
            &self.objects_buffer,
            &self.materials_buffer,
        );
    }

    fn create_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        objects: &wgpu::Buffer,
        materials: &wgpu::Buffer,
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
