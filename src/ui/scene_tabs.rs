use std::sync::{Arc, Mutex};

use crate::scene::workspace::SceneDocumentId;
use crate::scene::{SceneHandle, SceneWorkspace};

/// Metadata describing a single tab in the scene switcher.
///
/// The descriptor captures the [`SceneHandle`] and corresponding document
/// identifier together with a pre-formatted title ready for display.
///
/// # Examples
///
/// ```
/// # use wgpu_cube::scene::{SceneWorkspace, Scene};
/// # use wgpu_cube::ui::SceneTabsState;
/// let mut workspace = SceneWorkspace::new();
/// let handle = workspace.open_scene("demo.scene".into(), Scene::new());
/// let mut state = SceneTabsState::new();
/// state.refresh_from_workspace(&workspace);
/// assert_eq!(state.tabs()[0].handle, handle);
/// assert_eq!(state.tabs()[0].document_id, "demo.scene");
/// assert_eq!(state.tabs()[0].title, "demo");
/// assert!(state.tabs()[0].active);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SceneTabDescriptor {
    /// Handle of the scene represented by this tab.
    pub handle: SceneHandle,
    /// Identifier of the scene document on disk.
    pub document_id: SceneDocumentId,
    /// Display title shown in the UI for this tab.
    pub title: String,
    /// Indicates whether this tab represents the active scene.
    pub active: bool,
}

/// Shared state that tracks the list of open scenes to drive the tab UI.
#[derive(Default)]
pub struct SceneTabsState {
    tabs: Vec<SceneTabDescriptor>,
}

/// Reference-counted handle that allows UI code to read or refresh the tab list.
pub type SceneTabsHandle = Arc<Mutex<SceneTabsState>>;

impl SceneTabsState {
    /// Creates a new state object with no tabs.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a reference-counted handle wrapping a fresh [`SceneTabsState`].
    pub fn handle() -> SceneTabsHandle {
        Arc::new(Mutex::new(Self::default()))
    }

    /// Synchronises the tab list with the current contents of the
    /// [`SceneWorkspace`].
    pub fn refresh_from_workspace(&mut self, workspace: &SceneWorkspace) {
        let active = workspace.active_scene_handle();
        self.tabs.clear();

        for handle in workspace.scene_handles() {
            let Some(document_id) = workspace.document_id_for_handle(handle) else {
                continue;
            };

            let title = prettify_document_id(document_id);
            self.tabs.push(SceneTabDescriptor {
                handle,
                document_id: document_id.clone(),
                title,
                active: Some(handle) == active,
            });
        }
    }

    /// Returns the current set of tab descriptors ready for rendering.
    pub fn tabs(&self) -> &[SceneTabDescriptor] {
        &self.tabs
    }
}

fn prettify_document_id(document_id: &str) -> String {
    use std::path::Path;

    let display = Path::new(document_id)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(document_id);

    if display.is_empty() {
        document_id.to_string()
    } else {
        display.replace('_', " ")
    }
}
