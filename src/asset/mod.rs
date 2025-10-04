pub mod handle;
pub mod mesh;
pub mod cache;

pub use handle::Handle;
pub use mesh::Mesh;
pub use cache::AssetCache;

pub struct Assets {
    pub meshes: AssetCache<Mesh>,
    // Future: textures, materials, etc.
}

impl Assets {
    pub fn new() -> Self {
        Self {
            meshes: AssetCache::new(),
        }
    }
}

impl Default for Assets {
    fn default() -> Self {
        Self::new()
    }
}