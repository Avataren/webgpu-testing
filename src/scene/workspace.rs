use std::collections::HashMap;

use crate::asset::Assets;
use crate::project::{ProjectError, ProjectManifest, SceneDocument};
use crate::scene::loader::SceneImportDevice;
use crate::scene::{Scene, SceneAssetBundle, SceneAssetResources, SceneLibrary};
use slotmap::{new_key_type, SlotMap};

new_key_type! {
    /// Handle identifying an open scene instance within a [`SceneWorkspace`].
    pub struct SceneHandle;
}

/// Identifier used to refer to scene documents persisted on disk.
pub type SceneDocumentId = String;

/// Error conditions that can occur while interacting with a [`SceneWorkspace`].
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SceneWorkspaceError {
    /// Attempted to activate a scene document that is not currently open.
    #[error("scene document {0} is not open in the workspace")]
    MissingDocument(SceneDocumentId),
}

/// Callback invoked whenever a scene marks itself as dirty.
pub type SceneDirtyListener = Box<dyn FnMut(bool) + Send + 'static>;

struct SceneEntry {
    document_id: SceneDocumentId,
    scene: Scene,
    dirty_listener: Option<SceneDirtyListener>,
}

impl SceneEntry {
    fn new(
        document_id: SceneDocumentId,
        scene: Scene,
        dirty_listener: Option<SceneDirtyListener>,
    ) -> Self {
        Self {
            document_id,
            scene,
            dirty_listener,
        }
    }
}

/// Manages a collection of open scenes and shared rendering assets.
///
/// The workspace tracks which scene is currently active and ensures the shared
/// [`Assets`] mirror the active scene's asset state. Helper methods provide
/// ergonomic access to the active scene while keeping the workspace's shared
/// asset cache up to date.
#[derive(Default)]
pub struct SceneWorkspace {
    assets: Assets,
    scenes: SlotMap<SceneHandle, SceneEntry>,
    document_index: HashMap<SceneDocumentId, SceneHandle>,
    active_scene: Option<SceneHandle>,
}

impl SceneWorkspace {
    /// Creates an empty scene workspace with no open scenes.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the shared asset cache for the workspace.
    pub fn assets(&self) -> &Assets {
        &self.assets
    }

    /// Grants mutable access to the shared asset cache.
    ///
    /// When the guard is dropped the active scene's asset cache is synchronised
    /// with any modifications performed through the guard.
    pub fn assets_mut(&mut self) -> SceneWorkspaceAssetsMut<'_> {
        let active_scene = self
            .active_scene
            .and_then(|handle| self.scenes.get_mut(handle))
            .map(|entry| &mut entry.scene as *mut Scene);

        SceneWorkspaceAssetsMut {
            assets: &mut self.assets,
            active_scene,
        }
    }

    /// Returns the handle for the currently active scene, if any.
    pub fn active_scene_handle(&self) -> Option<SceneHandle> {
        self.active_scene
    }

    /// Returns the identifier for the currently active scene document, if any.
    pub fn active_document_id(&self) -> Option<&SceneDocumentId> {
        let handle = self.active_scene?;
        self.scenes.get(handle).map(|entry| &entry.document_id)
    }

    /// Returns an immutable reference to the currently active scene, if any.
    pub fn active_scene(&self) -> Option<&Scene> {
        let handle = self.active_scene?;
        self.scenes.get(handle).map(|entry| &entry.scene)
    }

    /// Returns a mutable view of the currently active scene.
    ///
    /// Changes made to the scene's asset cache automatically update the
    /// workspace's shared asset cache when the guard is dropped. The caller can
    /// explicitly mark the scene as dirty using [`SceneWorkspaceSceneMut::mark_dirty`].
    pub fn active_scene_mut(&mut self) -> Option<SceneWorkspaceSceneMut<'_>> {
        let handle = self.active_scene?;
        let entry = self.scenes.get_mut(handle)?;
        Some(SceneWorkspaceSceneMut {
            entry,
            workspace_assets: &mut self.assets,
            dirty: false,
        })
    }

    /// Returns the active scene handle together with a mutable guard to it.
    pub fn active_scene_handle_and_mut(
        &mut self,
    ) -> Option<(SceneHandle, SceneWorkspaceSceneMut<'_>)> {
        let handle = self.active_scene?;
        let entry = self.scenes.get_mut(handle)?;
        Some((
            handle,
            SceneWorkspaceSceneMut {
                entry,
                workspace_assets: &mut self.assets,
                dirty: false,
            },
        ))
    }

    /// Returns a mutable guard for the scene identified by the supplied handle.
    pub fn scene_mut_by_handle(
        &mut self,
        handle: SceneHandle,
    ) -> Option<SceneWorkspaceSceneMut<'_>> {
        let entry = self.scenes.get_mut(handle)?;
        Some(SceneWorkspaceSceneMut {
            entry,
            workspace_assets: &mut self.assets,
            dirty: false,
        })
    }

    /// Executes the provided closure with a mutable reference to the active scene.
    pub fn with_active_scene_mut<R>(&mut self, f: impl FnOnce(&mut Scene) -> R) -> Option<R> {
        let mut scene = self.active_scene_mut()?;
        Some(f(&mut scene))
    }

    /// Opens a new scene within the workspace, returning its handle.
    pub fn open_scene(&mut self, document_id: SceneDocumentId, scene: Scene) -> SceneHandle {
        self.open_scene_with_listener(document_id, scene, None)
    }

    /// Opens a new scene and attaches a dirty-state listener.
    pub fn open_scene_with_listener(
        &mut self,
        document_id: SceneDocumentId,
        scene: Scene,
        dirty_listener: Option<SceneDirtyListener>,
    ) -> SceneHandle {
        let handle =
            self.scenes
                .insert(SceneEntry::new(document_id.clone(), scene, dirty_listener));
        self.document_index.insert(document_id, handle);

        if self.active_scene.is_none() {
            self.active_scene = Some(handle);
            if let Some(entry) = self.scenes.get(handle) {
                self.assets = entry.scene.assets.clone();
            }
        }

        handle
    }

    /// Retrieves the handle for the supplied document identifier, if open.
    pub fn scene_handle(&self, document_id: &SceneDocumentId) -> Option<SceneHandle> {
        self.document_index.get(document_id).copied()
    }

    /// Sets the active scene by document identifier.
    pub fn set_active_scene(
        &mut self,
        document_id: &SceneDocumentId,
    ) -> Result<SceneHandle, SceneWorkspaceError> {
        let handle = self
            .document_index
            .get(document_id)
            .copied()
            .ok_or_else(|| SceneWorkspaceError::MissingDocument(document_id.clone()))?;
        self.set_active_scene_by_handle(handle);
        Ok(handle)
    }

    /// Sets the active scene by handle.
    pub fn set_active_scene_by_handle(&mut self, handle: SceneHandle) {
        if let Some(entry) = self.scenes.get(handle) {
            self.assets = entry.scene.assets.clone();
            self.active_scene = Some(handle);
        }
    }

    /// Marks the supplied scene as dirty and notifies any registered listener.
    pub fn mark_scene_dirty(&mut self, handle: SceneHandle) {
        if let Some(entry) = self.scenes.get_mut(handle) {
            if let Some(listener) = entry.dirty_listener.as_mut() {
                listener(true);
            }
        }
    }

    /// Closes the scene identified by the supplied document id.
    pub fn close_scene(&mut self, document_id: &SceneDocumentId) -> Option<Scene> {
        let handle = self.scene_handle(document_id)?;
        self.close_scene_by_handle(handle)
    }

    /// Closes the scene identified by the supplied handle.
    pub fn close_scene_by_handle(&mut self, handle: SceneHandle) -> Option<Scene> {
        let entry = self.scenes.remove(handle)?;
        self.document_index.remove(&entry.document_id);

        if self.active_scene == Some(handle) {
            self.active_scene = None;
            if let Some((new_handle, new_entry)) = self.scenes.iter().next() {
                self.active_scene = Some(new_handle);
                self.assets = new_entry.scene.assets.clone();
            } else {
                self.assets = Assets::new();
            }
        }

        Some(entry.scene)
    }

    /// Consumes the workspace, returning the active scene if one was present.
    pub fn into_active_scene(mut self) -> Option<Scene> {
        let handle = self.active_scene?;
        self.scenes.remove(handle).map(|entry| entry.scene)
    }

    /// Returns an iterator over the open scene handles.
    pub fn scene_handles(&self) -> impl Iterator<Item = SceneHandle> + '_ {
        self.scenes.keys()
    }
}

/// Guard type returned by [`SceneWorkspace::assets_mut`].
pub struct SceneWorkspaceAssetsMut<'a> {
    assets: &'a mut Assets,
    active_scene: Option<*mut Scene>,
}

impl<'a> std::ops::Deref for SceneWorkspaceAssetsMut<'a> {
    type Target = Assets;

    fn deref(&self) -> &Self::Target {
        self.assets
    }
}

impl<'a> std::ops::DerefMut for SceneWorkspaceAssetsMut<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.assets
    }
}

impl<'a> Drop for SceneWorkspaceAssetsMut<'a> {
    fn drop(&mut self) {
        if let Some(scene_ptr) = self.active_scene {
            // SAFETY: `scene_ptr` originates from a unique mutable borrow of the active
            // scene entry. The guard ensures that `SceneWorkspaceAssetsMut` holds the
            // only mutable reference to the active scene for the guard's lifetime.
            unsafe {
                (*scene_ptr).replace_assets(self.assets.clone());
            }
        }
    }
}

/// Mutable view of the active scene stored within a workspace.
pub struct SceneWorkspaceSceneMut<'a> {
    entry: &'a mut SceneEntry,
    workspace_assets: &'a mut Assets,
    dirty: bool,
}

impl<'a> SceneWorkspaceSceneMut<'a> {
    /// Marks the scene as having unsaved changes, notifying listeners when the
    /// guard is dropped.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Returns the document identifier associated with the active scene.
    pub fn document_id(&self) -> &SceneDocumentId {
        &self.entry.document_id
    }
}

impl<'a> std::ops::Deref for SceneWorkspaceSceneMut<'a> {
    type Target = Scene;

    fn deref(&self) -> &Self::Target {
        &self.entry.scene
    }
}

impl<'a> std::ops::DerefMut for SceneWorkspaceSceneMut<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.entry.scene
    }
}

impl<'a> Drop for SceneWorkspaceSceneMut<'a> {
    fn drop(&mut self) {
        self.workspace_assets.clone_from(&self.entry.scene.assets);
        if self.dirty {
            if let Some(listener) = self.entry.dirty_listener.as_mut() {
                listener(true);
            }
        }
    }
}

/// Builder responsible for hydrating a [`SceneWorkspace`] from a project
/// manifest.
type DirtyListenerFactory<'a> = Box<dyn FnMut(&SceneDocument) -> Option<SceneDirtyListener> + 'a>;

pub struct SceneWorkspaceBuilder<'a> {
    manifest: &'a ProjectManifest,
    project_root: &'a std::path::Path,
    library: &'a mut SceneLibrary,
    dirty_listener_factory: Option<DirtyListenerFactory<'a>>,
}

impl<'a> SceneWorkspaceBuilder<'a> {
    /// Creates a new builder for the supplied manifest and scene library.
    pub fn new(
        manifest: &'a ProjectManifest,
        project_root: &'a std::path::Path,
        library: &'a mut SceneLibrary,
    ) -> Self {
        Self {
            manifest,
            project_root,
            library,
            dirty_listener_factory: None,
        }
    }

    /// Attaches a factory used to create dirty-state listeners for each scene.
    pub fn with_dirty_listener_factory<F>(mut self, factory: F) -> Self
    where
        F: FnMut(&SceneDocument) -> Option<SceneDirtyListener> + 'a,
    {
        self.dirty_listener_factory = Some(Box::new(factory));
        self
    }

    /// Instantiates the scenes defined in the manifest into a workspace.
    pub fn build<R>(mut self, renderer: &mut R) -> Result<(SceneWorkspace, bool), ProjectError>
    where
        R: SceneImportDevice,
    {
        if self.manifest.scenes().is_empty() {
            return Err(ProjectError::NoScenes);
        }

        let mut workspace = SceneWorkspace::new();
        let mut textures_changed = false;

        for (index, document) in self.manifest.scenes().iter().enumerate() {
            self.library.register_document(document);

            let asset = if let Some(asset) = self.manifest.scene_asset(&document.id) {
                asset.clone()
            } else {
                self.library
                    .load_document(document, self.project_root)?
                    .clone()
            };

            self.library.insert(document.id.clone(), asset.clone());

            let mut scene = Scene::new();
            if index == 0 {
                scene.set_camera(self.manifest.engine_camera().to_camera());
                let environment = self
                    .manifest
                    .environment()
                    .to_environment(self.project_root)?;
                scene.set_environment(environment);
            }

            let mut bundle = SceneAssetBundle::new(asset, SceneAssetResources::builder().build());
            let registration = bundle.register_resources(renderer, &mut scene.assets);
            textures_changed |= registration.textures_changed;

            if !bundle.asset.entities.is_empty() {
                let main_scene =
                    scene.instantiate_asset_with_renderer(&bundle.asset, None, renderer);
                scene.set_main_scene(main_scene);
            }

            let listener = self
                .dirty_listener_factory
                .as_mut()
                .and_then(|factory| factory(document));

            workspace.open_scene_with_listener(document.id.clone(), scene, listener);
        }

        Ok((workspace, textures_changed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::MaterialAsset;
    use crate::renderer::material::Material;
    use std::path::PathBuf;

    #[test]
    fn open_scene_sets_active_handle() {
        let mut workspace = SceneWorkspace::new();
        let handle = workspace.open_scene("scene_a".to_string(), Scene::new());

        assert_eq!(workspace.active_scene_handle(), Some(handle));
        assert_eq!(workspace.active_document_id(), Some(&"scene_a".to_string()));
    }

    #[test]
    fn closing_active_scene_updates_handle() {
        let mut workspace = SceneWorkspace::new();
        let first = workspace.open_scene("scene_a".to_string(), Scene::new());
        let second = workspace.open_scene("scene_b".to_string(), Scene::new());

        workspace.set_active_scene_by_handle(second);
        let closed = workspace
            .close_scene_by_handle(second)
            .expect("scene removed");
        assert!(closed.assets.meshes.is_empty());

        assert_eq!(workspace.active_scene_handle(), Some(first));
        assert_eq!(workspace.active_document_id(), Some(&"scene_a".to_string()));
    }

    #[test]
    fn switching_active_scene_updates_assets() {
        let mut workspace = SceneWorkspace::new();
        let mut first_scene = Scene::new();
        first_scene
            .assets
            .materials
            .insert(MaterialAsset::from_material(
                Material::pbr(),
                PathBuf::new(),
            ));
        let first_material_count = first_scene.assets.materials.len();
        let first_id = "scene_a".to_string();
        let first = workspace.open_scene(first_id.clone(), first_scene);

        let second_scene = Scene::new();
        let second_material_count = second_scene.assets.materials.len();
        let second_id = "scene_b".to_string();
        let second = workspace.open_scene(second_id.clone(), second_scene);

        workspace.set_active_scene_by_handle(first);
        assert_eq!(workspace.assets().materials.len(), first_material_count);

        workspace.set_active_scene_by_handle(second);
        assert_eq!(workspace.assets().materials.len(), second_material_count);

        workspace.set_active_scene(&first_id).unwrap();
        assert_eq!(workspace.active_scene_handle(), Some(first));
        assert!(workspace.set_active_scene(&second_id).is_ok());
        assert_eq!(workspace.active_scene_handle(), Some(second));
    }
}
