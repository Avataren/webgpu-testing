// src/renderer/compute_resources.rs

use bytemuck::Pod;
use wgpu::util::DeviceExt;

/// A storage buffer that can be read/written by compute shaders
pub struct StorageBuffer {
    buffer: wgpu::Buffer,
    size: wgpu::BufferAddress,
}

impl StorageBuffer {
    /// Create a new storage buffer with initial data
    pub fn new<T: Pod>(device: &wgpu::Device, label: &str, data: &[T]) -> Self {
        let size = std::mem::size_of_val(data) as wgpu::BufferAddress;
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::cast_slice(data),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
        });

        Self { buffer, size }
    }

    /// Create an empty storage buffer with a given capacity
    pub fn with_capacity<T: Pod>(device: &wgpu::Device, label: &str, capacity: usize) -> Self {
        let size = (std::mem::size_of::<T>() * capacity) as wgpu::BufferAddress;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        Self { buffer, size }
    }

    /// Update the buffer with new data
    pub fn write<T: Pod>(&self, queue: &wgpu::Queue, data: &[T]) {
        queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(data));
    }

    /// Get the underlying buffer
    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    /// Get the buffer size
    pub fn size(&self) -> wgpu::BufferAddress {
        self.size
    }
}

/// A uniform buffer for shader constants
pub struct UniformBuffer {
    buffer: wgpu::Buffer,
}

impl UniformBuffer {
    /// Create a new uniform buffer with initial data
    pub fn new<T: Pod>(device: &wgpu::Device, label: &str, data: &T) -> Self {
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::bytes_of(data),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        Self { buffer }
    }

    /// Create an empty uniform buffer for a given type
    pub fn empty<T: Pod>(device: &wgpu::Device, label: &str) -> Self {
        let size = std::mem::size_of::<T>() as wgpu::BufferAddress;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self { buffer }
    }

    /// Update the uniform buffer
    pub fn write<T: Pod>(&self, queue: &wgpu::Queue, data: &T) {
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(data));
    }

    /// Get the underlying buffer
    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }
}

/// Helper for creating bind group layouts for common compute patterns
pub struct BindGroupLayoutBuilder<'a> {
    device: &'a wgpu::Device,
    label: Option<&'a str>,
    entries: Vec<wgpu::BindGroupLayoutEntry>,
}

impl<'a> BindGroupLayoutBuilder<'a> {
    pub fn new(device: &'a wgpu::Device) -> Self {
        Self {
            device,
            label: None,
            entries: Vec::new(),
        }
    }

    pub fn with_label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    /// Add a storage buffer (read/write)
    pub fn add_storage_buffer(mut self, binding: u32, read_only: bool) -> Self {
        self.entries.push(wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
        self
    }

    /// Add a uniform buffer
    pub fn add_uniform_buffer(mut self, binding: u32) -> Self {
        self.entries.push(wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
        self
    }

    /// Add a storage texture (read-only)
    pub fn add_storage_texture_read(mut self, binding: u32, format: wgpu::TextureFormat) -> Self {
        self.entries.push(wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::ReadOnly,
                format,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        });
        self
    }

    /// Add a storage texture (write-only)
    pub fn add_storage_texture_write(mut self, binding: u32, format: wgpu::TextureFormat) -> Self {
        self.entries.push(wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        });
        self
    }

    pub fn build(self) -> wgpu::BindGroupLayout {
        self.device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: self.label,
                entries: &self.entries,
            })
    }
}

/// Helper for creating bind groups
pub struct BindGroupBuilder<'a> {
    device: &'a wgpu::Device,
    layout: &'a wgpu::BindGroupLayout,
    label: Option<&'a str>,
    entries: Vec<wgpu::BindGroupEntry<'a>>,
}

impl<'a> BindGroupBuilder<'a> {
    pub fn new(device: &'a wgpu::Device, layout: &'a wgpu::BindGroupLayout) -> Self {
        Self {
            device,
            layout,
            label: None,
            entries: Vec::new(),
        }
    }

    pub fn with_label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    pub fn add_buffer(mut self, binding: u32, buffer: &'a wgpu::Buffer) -> Self {
        self.entries.push(wgpu::BindGroupEntry {
            binding,
            resource: buffer.as_entire_binding(),
        });
        self
    }

    pub fn add_texture_view(mut self, binding: u32, view: &'a wgpu::TextureView) -> Self {
        self.entries.push(wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(view),
        });
        self
    }

    pub fn build(self) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: self.label,
            layout: self.layout,
            entries: &self.entries,
        })
    }
}
