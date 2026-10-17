use std::hash::{Hash, Hasher};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use wgpu::util::DeviceExt;

use crate::renderer::{SkinVertex, Vertex};
use glam::Vec3;

/// The axis-aligned box around a mesh's vertices in its own space, for culling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocalBounds {
    pub min: Vec3,
    pub max: Vec3,
}

impl LocalBounds {
    /// The box around `vertices`; a point at the origin for an empty mesh.
    pub fn from_vertices(vertices: &[Vertex]) -> Self {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for vertex in vertices {
            let pos = Vec3::from_array(vertex.pos);
            min = min.min(pos);
            max = max.max(pos);
        }
        if vertices.is_empty() || !min.is_finite() || !max.is_finite() {
            return Self {
                min: Vec3::ZERO,
                max: Vec3::ZERO,
            };
        }
        Self { min, max }
    }

    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    pub fn half_extents(&self) -> Vec3 {
        (self.max - self.min) * 0.5
    }
}

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
    index_count: u32,
    index_format: wgpu::IndexFormat,
    /// Where the vertices start in `vertex_buffer`, in vertices: a skinned copy lives in
    /// a pool shared with other instances. Drawn with `draw_indexed`'s base vertex, so
    /// instances in one pool need no rebinding between draws.
    base_vertex: u32,
    data: Arc<MeshData>,
    bounds: LocalBounds,
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
        // A skinned mesh's own buffer draws its bind pose (when skinning is skipped); the
        // skinning stage reads the vertices and skin weights from `data` into its pool.
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
            base_vertex: 0,
            bounds: LocalBounds::from_vertices(&data.vertices),
            data: Arc::new(data),
        }
    }

    /// The same geometry drawn from another vertex buffer (with the same layout), its
    /// vertices starting `base_vertex` vertices in, e.g. one instance's skinned vertices
    /// in a pool. Shares the index buffer.
    pub(crate) fn with_vertex_buffer(&self, vertex_buffer: wgpu::Buffer, base_vertex: u32) -> Self {
        Self {
            vertex_buffer,
            index_buffer: self.index_buffer.clone(),
            index_count: self.index_count,
            index_format: self.index_format,
            base_vertex,
            data: Arc::clone(&self.data),
            bounds: self.bounds,
        }
    }

    pub fn vertex_buffer(&self) -> &wgpu::Buffer {
        &self.vertex_buffer
    }

    /// Where the vertices start in [`vertex_buffer`](Self::vertex_buffer), in vertices
    /// (0 for a mesh asset's own buffer). Pass it to `draw_indexed` as the base vertex.
    pub fn base_vertex(&self) -> u32 {
        self.base_vertex
    }

    /// [`base_vertex`](Self::base_vertex) in bytes, for copies out of the buffer.
    pub fn vertex_byte_offset(&self) -> u64 {
        u64::from(self.base_vertex) * std::mem::size_of::<Vertex>() as u64
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

    /// The box around the mesh's vertices in its own space (the bind pose of a skinned
    /// mesh).
    pub fn local_bounds(&self) -> LocalBounds {
        self.bounds
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
            index_count: self.index_count,
            index_format: self.index_format,
            base_vertex: self.base_vertex,
            data: Arc::clone(&self.data),
            bounds: self.bounds,
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
