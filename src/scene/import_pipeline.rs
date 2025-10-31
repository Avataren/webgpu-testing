use super::import_state::SceneImports;
use super::loader::SceneImportDevice;
use super::scene::Scene;
use crate::renderer::Renderer;
use hecs::Entity;

/// Coordinates asynchronous and queued scene imports.
pub(crate) struct SceneImportsManager {
    imports: SceneImports,
}

impl SceneImportsManager {
    pub(crate) fn new() -> Self {
        Self {
            imports: SceneImports::new(),
        }
    }

    pub(crate) fn merge_as_child(
        &mut self,
        scene: &mut Scene,
        parent_entity: Entity,
        other: Scene,
        renderer: &mut dyn SceneImportDevice,
    ) {
        self.imports
            .merge_as_child(scene, parent_entity, other, renderer);
    }

    pub(crate) fn process_pending_gltf_imports(
        &mut self,
        scene: &mut Scene,
        renderer: &mut Renderer,
    ) {
        self.imports.process_pending_gltf_imports(scene, renderer);
    }
}

impl Default for SceneImportsManager {
    fn default() -> Self {
        Self::new()
    }
}
