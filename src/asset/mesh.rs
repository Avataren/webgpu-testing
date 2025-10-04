use wgpu::util::DeviceExt;

#[derive(Clone, Hash, Eq, PartialEq, std::fmt::Debug)]
pub struct Mesh {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    index_format: wgpu::IndexFormat,
}

impl Mesh {
    pub fn from_vertices(
        device: &wgpu::Device,
        vertices: &[crate::renderer::Vertex],
        indices: &[u32],
    ) -> Self {
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
}
