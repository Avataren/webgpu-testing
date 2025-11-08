// Action handlers for InspectorAction
//
// This module extracts the massive 946-line match statement from
// apply_pending_inspector_actions() into separate, focused handler functions.
//
// Each handler takes a context and returns a result indicating what changed.

pub mod camera;
pub mod components;
pub mod dispatch;
pub mod environment;
pub mod lights;
pub mod materials;
pub mod misc;
pub mod particles;
pub mod scripts;
pub mod shader;
pub mod transform;

pub use dispatch::dispatch_action;

use crate::application::EditorApplication;
use wgpu_cube::scene::SceneWorkspaceSceneMut;

/// Context passed to action handlers
pub struct ActionContext<'scene, 'app> {
    pub scene: &'scene mut SceneWorkspaceSceneMut<'scene>,
    pub app: &'app mut EditorApplication,
}

/// Result from an action handler indicating what changed
#[derive(Default)]
pub struct ActionResult {
    pub transforms_changed: bool,
    pub scene_changed: bool,
}

impl ActionResult {
    pub fn no_change() -> Self {
        Self::default()
    }

    pub fn transforms_changed() -> Self {
        Self {
            transforms_changed: true,
            scene_changed: true,
        }
    }

    pub fn scene_changed() -> Self {
        Self {
            transforms_changed: false,
            scene_changed: true,
        }
    }
}
