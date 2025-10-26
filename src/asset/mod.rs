pub mod cache;
pub mod handle;
pub mod mesh;

pub use cache::AssetCache;
pub use handle::Handle;
pub use mesh::{Mesh, MeshData};

use crate::renderer::primitives::PrimitiveMeshDescriptor;
use crate::renderer::Texture;
use crate::scene::loader::SceneImportDevice;
use std::collections::HashMap;

#[derive(Clone)]
pub struct Assets {
    pub meshes: AssetCache<Mesh>,
    pub textures: AssetCache<Texture>,
    primitive_meshes: HashMap<PrimitiveMeshDescriptor, Handle<Mesh>>,
}

impl Assets {
    pub fn new() -> Self {
        Self {
            meshes: AssetCache::new(),
            textures: AssetCache::new(),
            primitive_meshes: HashMap::new(),
        }
    }

    pub fn ensure_primitive_mesh(
        &mut self,
        renderer: &mut dyn SceneImportDevice,
        descriptor: PrimitiveMeshDescriptor,
    ) -> Handle<Mesh> {
        if let Some(&handle) = self.primitive_meshes.get(&descriptor) {
            return handle;
        }

        let data = descriptor.mesh_data();
        let mesh = renderer.create_mesh(&data.vertices, &data.indices);
        let handle = self.meshes.insert(mesh);
        self.primitive_meshes.insert(descriptor, handle);
        handle
    }

    pub fn primitive_mesh_handle(
        &self,
        descriptor: PrimitiveMeshDescriptor,
    ) -> Option<Handle<Mesh>> {
        self.primitive_meshes.get(&descriptor).copied()
    }
}

impl Default for Assets {
    fn default() -> Self {
        Self::new()
    }
}
