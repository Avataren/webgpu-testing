pub mod cache;
pub mod handle;
pub mod mesh;

pub use cache::AssetCache;
pub use handle::Handle;
pub use mesh::Mesh;

pub struct Assets {
    pub meshes: AssetCache<Mesh>,
}
impl Default for Assets {
    fn default() -> Self {
        Self {
            meshes: AssetCache::default(),
        }
    }
}
