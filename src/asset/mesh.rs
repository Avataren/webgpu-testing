use std::hash::{Hash, Hasher};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use wgpu::util::DeviceExt;

use crate::renderer::Vertex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

pub struct Mesh {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    index_format: wgpu::IndexFormat,
    data: Arc<MeshData>,
}

impl Mesh {
    pub fn from_vertices(device: &wgpu::Device, vertices: &[Vertex], indices: &[u32]) -> Self {
        let data = MeshData {
            vertices: vertices.to_vec(),
            indices: indices.to_vec(),
        };
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("VertexBuffer"),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let uses_u32_indices = indices.iter().any(|&idx| idx > u16::MAX as u32);
        let (index_buffer, index_format) = if uses_u32_indices {
            (
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("IndexBuffer"),
                    contents: bytemuck::cast_slice(indices),
                    usage: wgpu::BufferUsages::INDEX,
                }),
                wgpu::IndexFormat::Uint32,
            )
        } else {
            let index_data_u16: Vec<u16> = indices.iter().map(|&idx| idx as u16).collect();

            (
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("IndexBuffer"),
                    contents: bytemuck::cast_slice(&index_data_u16),
                    usage: wgpu::BufferUsages::INDEX,
                }),
                wgpu::IndexFormat::Uint16,
            )
        };

        Self {
            vertex_buffer,
            index_buffer,
            index_count: indices.len() as u32,
            index_format,
            data: Arc::new(data),
        }
    }

    pub fn from_data(device: &wgpu::Device, data: MeshData) -> Self {
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("VertexBuffer"),
            contents: bytemuck::cast_slice(&data.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let uses_u32_indices = data.indices.iter().any(|&idx| idx > u16::MAX as u32);
        let (index_buffer, index_format) = if uses_u32_indices {
            (
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("IndexBuffer"),
                    contents: bytemuck::cast_slice(&data.indices),
                    usage: wgpu::BufferUsages::INDEX,
                }),
                wgpu::IndexFormat::Uint32,
            )
        } else {
            let index_data_u16: Vec<u16> = data.indices.iter().map(|&idx| idx as u16).collect();
            (
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("IndexBuffer"),
                    contents: bytemuck::cast_slice(&index_data_u16),
                    usage: wgpu::BufferUsages::INDEX,
                }),
                wgpu::IndexFormat::Uint16,
            )
        };

        Self {
            vertex_buffer,
            index_buffer,
            index_count: data.indices.len() as u32,
            index_format,
            data: Arc::new(data),
        }
    }

    pub fn vertex_buffer(&self) -> &wgpu::Buffer {
        &self.vertex_buffer
    }

    pub fn index_buffer(&self) -> &wgpu::Buffer {
        &self.index_buffer
    }

    pub fn index_count(&self) -> u32 {
        self.index_count
    }

    pub fn index_format(&self) -> wgpu::IndexFormat {
        self.index_format
    }

    pub fn data(&self) -> &MeshData {
        &self.data
    }
}

impl Clone for Mesh {
    fn clone(&self) -> Self {
        Self {
            vertex_buffer: self.vertex_buffer.clone(),
            index_buffer: self.index_buffer.clone(),
            index_count: self.index_count,
            index_format: self.index_format,
            data: Arc::clone(&self.data),
        }
    }
}

impl std::fmt::Debug for Mesh {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mesh")
            .field("index_count", &self.index_count)
            .field("index_format", &self.index_format)
            .finish()
    }
}

impl PartialEq for Mesh {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.data, &other.data)
            && self.index_count == other.index_count
            && self.index_format == other.index_format
    }
}

impl Eq for Mesh {}

impl Hash for Mesh {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.data).hash(state);
        self.index_count.hash(state);
        self.index_format.hash(state);
    }
}
