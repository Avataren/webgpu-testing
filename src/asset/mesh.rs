use std::hash::{Hash, Hasher};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use wgpu::util::DeviceExt;

use crate::renderer::{SkinVertex, Vertex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    /// Joint indices and weights per vertex (parallel to `vertices`) for meshes deformed
    /// by a skeleton; `None` for rigid meshes. Joint indices refer to the joint list of
    /// the entity's [`SkinnedMesh`](crate::scene::components::SkinnedMesh).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skin: Option<Vec<SkinVertex>>,
}

impl MeshData {
    /// A rigid mesh.
    pub fn new(vertices: Vec<Vertex>, indices: Vec<u32>) -> Self {
        Self {
            vertices,
            indices,
            skin: None,
        }
    }

    /// Whether the mesh carries a usable skin stream (one entry per vertex).
    pub fn is_skinned(&self) -> bool {
        self.skin
            .as_ref()
            .is_some_and(|skin| skin.len() == self.vertices.len() && !skin.is_empty())
    }

    /// The highest joint index any vertex uses with a non-zero weight, if skinned.
    pub fn max_joint_index(&self) -> Option<u16> {
        self.skin.as_ref()?.iter().fold(None, |max, vertex| {
            vertex
                .joints
                .iter()
                .zip(vertex.weights)
                .filter(|(_, weight)| *weight > 0)
                .map(|(joint, _)| *joint)
                .chain(max)
                .max()
        })
    }
}

pub struct Mesh {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    /// The skin stream, readable by the skinning compute shader. Present for skinned
    /// meshes on devices that support storage buffers.
    skin_buffer: Option<wgpu::Buffer>,
    index_count: u32,
    index_format: wgpu::IndexFormat,
    data: Arc<MeshData>,
}

impl Mesh {
    pub fn from_vertices(device: &wgpu::Device, vertices: &[Vertex], indices: &[u32]) -> Self {
        Self::from_data(device, MeshData::new(vertices.to_vec(), indices.to_vec()))
    }

    pub fn from_data(device: &wgpu::Device, data: MeshData) -> Self {
        let skinned = data.is_skinned();
        if data.skin.is_some() && !skinned {
            log::warn!(
                "Mesh skin stream has {} entries for {} vertices; drawing it rigid",
                data.skin.as_ref().map_or(0, Vec::len),
                data.vertices.len()
            );
        }
        // Skinned meshes are read by the skinning compute pass, which needs storage
        // access; downlevel devices (WebGL2) skin on the CPU instead.
        let storage = skinned && device.limits().max_storage_buffers_per_shader_stage >= 4;
        let mut vertex_usage = wgpu::BufferUsages::VERTEX;
        if storage {
            vertex_usage |= wgpu::BufferUsages::STORAGE;
        }
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("VertexBuffer"),
            contents: bytemuck::cast_slice(&data.vertices),
            usage: vertex_usage,
        });
        let skin_buffer = data.skin.as_ref().filter(|_| storage).map(|skin| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("SkinBuffer"),
                contents: bytemuck::cast_slice(skin),
                usage: wgpu::BufferUsages::STORAGE,
            })
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
            skin_buffer,
            index_count: data.indices.len() as u32,
            index_format,
            data: Arc::new(data),
        }
    }

    /// The same geometry drawn from another vertex buffer (with the same layout and
    /// vertex count), e.g. one instance's skinned vertices. Shares the index buffer.
    pub(crate) fn with_vertex_buffer(&self, vertex_buffer: wgpu::Buffer) -> Self {
        Self {
            vertex_buffer,
            index_buffer: self.index_buffer.clone(),
            skin_buffer: None,
            index_count: self.index_count,
            index_format: self.index_format,
            data: Arc::clone(&self.data),
        }
    }

    pub fn vertex_buffer(&self) -> &wgpu::Buffer {
        &self.vertex_buffer
    }

    pub fn index_buffer(&self) -> &wgpu::Buffer {
        &self.index_buffer
    }

    pub(crate) fn skin_buffer(&self) -> Option<&wgpu::Buffer> {
        self.skin_buffer.as_ref()
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

    /// The shared geometry: a new `Arc` means the mesh changed (ray tracing rebuilds).
    pub(crate) fn shared_data(&self) -> &Arc<MeshData> {
        &self.data
    }

    /// Whether this mesh is deformed by a skeleton when its entity has a
    /// [`SkinnedMesh`](crate::scene::components::SkinnedMesh).
    pub fn is_skinned(&self) -> bool {
        self.data.is_skinned()
    }

    /// Whether both meshes draw the same geometry data (one may be a skinned copy).
    pub(crate) fn shares_data_with(&self, other: &Mesh) -> bool {
        Arc::ptr_eq(&self.data, &other.data)
    }

    pub fn vertex_count(&self) -> u32 {
        self.data.vertices.len() as u32
    }
}

impl Clone for Mesh {
    fn clone(&self) -> Self {
        Self {
            vertex_buffer: self.vertex_buffer.clone(),
            index_buffer: self.index_buffer.clone(),
            skin_buffer: self.skin_buffer.clone(),
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
            .field("skinned", &self.is_skinned())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer::vertex::v;

    #[test]
    fn rigid_mesh_json_has_no_skin_field_and_old_json_loads() {
        let data = MeshData::new(
            vec![v([0.0; 3], [0.0, 1.0, 0.0], [0.0; 2], [1.0, 0.0, 0.0, 1.0])],
            vec![0],
        );
        let json = serde_json::to_string(&data).unwrap();
        assert!(!json.contains("skin"), "{json}");
        let back: MeshData = serde_json::from_str(&json).unwrap();
        assert!(back.skin.is_none());
    }

    #[test]
    fn skinned_mesh_round_trips_and_reports_joints() {
        let vertex = v([0.0; 3], [0.0, 1.0, 0.0], [0.0; 2], [1.0, 0.0, 0.0, 1.0]);
        let mut data = MeshData::new(vec![vertex; 2], vec![0, 1, 0]);
        data.skin = Some(vec![
            SkinVertex::from_influences([(2, 1.0)]),
            SkinVertex::from_influences([(5, 0.5), (1, 0.5)]),
        ]);
        assert!(data.is_skinned());
        assert_eq!(data.max_joint_index(), Some(5));
        let back: MeshData = serde_json::from_str(&serde_json::to_string(&data).unwrap()).unwrap();
        assert_eq!(back.skin, data.skin);

        data.skin = Some(vec![SkinVertex::default()]);
        assert!(
            !data.is_skinned(),
            "a skin stream must match the vertex count"
        );
    }
}
