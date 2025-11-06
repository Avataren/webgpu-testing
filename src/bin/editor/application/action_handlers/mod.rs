// Action handlers for InspectorAction
//
// This module extracts the massive 946-line match statement from
// apply_pending_inspector_actions() into separate, focused handler functions.
//
// Each handler takes a context and returns a result indicating what changed.

pub mod transform;
pub mod camera;
pub mod lights;
pub mod misc;
pub mod components;
pub mod materials;
pub mod dispatch;

pub use dispatch::dispatch_action;

use wgpu_cube::scene::Scene;
use crate::application::EditorApplication;

/// Context passed to action handlers
pub struct ActionContext<'a> {
    pub scene: &'a mut Scene,
    pub app: &'a mut EditorApplication,
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
