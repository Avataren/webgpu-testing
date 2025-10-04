pub mod handle;
pub mod mesh;
pub mod cache;

pub use handle::Handle;
pub use mesh::Mesh;
pub use cache::AssetCache;

use crate::renderer::Texture;

pub struct Assets {
    pub meshes: AssetCache<Mesh>,
    pub textures: AssetCache<Texture>,
}

impl Assets {
    pub fn new() -> Self {
        Self {
            meshes: AssetCache::new(),
            textures: AssetCache::new(),
        }
    }
}

impl Default for Assets {
    fn default() -> Self {
        Self::new()
    }
}