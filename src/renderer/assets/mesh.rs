use crate::renderer::Vertex;
use wgpu::util::DeviceExt;

pub struct Mesh {
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    index_count: u32,
}

impl Mesh {
    pub fn from_vertices(device: &wgpu::Device, vertices: &[Vertex], indices: &[u16]) -> Self {
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Mesh.VertexBuffer"),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Mesh.IndexBuffer"),
            contents: bytemuck::cast_slice(indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        Self {
            vbuf,
            ibuf,
            index_count: indices.len() as u32,
        }
    }

    /// Get the vertex buffer for rendering
    pub fn vertex_buffer(&self) -> &wgpu::Buffer {
        &self.vbuf
    }

    /// Get the index buffer for rendering
    pub fn index_buffer(&self) -> &wgpu::Buffer {
        &self.ibuf
    }

    /// Get the number of indices to draw
    pub fn index_count(&self) -> u32 {
        self.index_count
    }
}
