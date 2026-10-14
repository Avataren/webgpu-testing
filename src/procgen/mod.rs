//! Procedural geometry, material maps and sound effects generated from data or code.
//!
//! - [`voxel`]: voxel models (from text art, a flat array or a fill function) turned into
//!   greedy meshes, one mesh per colour.
//! - [`noise`]: gradient noise for procedural shapes and terrain.

pub mod geometry;
pub mod noise;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod sound;
pub(crate) mod texture;
pub mod voxel;

use std::sync::Arc;

use glam::Vec3;

use crate::asset::MeshData;

/// A mesh built at runtime, ready to upload. `key` identifies the content: meshes with the
/// same key are the same geometry, so they share one GPU mesh (and draw instanced).
#[derive(Debug)]
pub struct GeneratedMesh {
    pub key: u64,
    pub data: MeshData,
    /// Local-space bounds of the vertices.
    pub min: Vec3,
    pub max: Vec3,
}

impl GeneratedMesh {
    pub fn new(key: u64, data: MeshData) -> Arc<Self> {
        let (min, max) = data.vertices.iter().fold(
            (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
            |(min, max), vertex| {
                let pos = Vec3::from(vertex.pos);
                (min.min(pos), max.max(pos))
            },
        );
        let (min, max) = if data.vertices.is_empty() {
            (Vec3::ZERO, Vec3::ZERO)
        } else {
            (min, max)
        };
        Arc::new(Self {
            key,
            data,
            min,
            max,
        })
    }

    pub fn triangles(&self) -> usize {
        self.data.indices.len() / 3
    }
}
