pub mod cache;
pub mod handle;
pub mod mesh;

pub use cache::AssetCache;
pub use handle::Handle;
pub use mesh::Mesh;

use crate::renderer::Texture;

#[derive(Clone)]
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
