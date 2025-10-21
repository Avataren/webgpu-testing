use log::info;
use std::collections::HashMap;
use wgpu_cube::app::{GpuUpdateContext, RuntimeMode, UpdateContext};
use wgpu_cube::scene::TransformComponent;

use super::core::{
    EditorApplication, EditorCameraState, EditorTransformState, RuntimeModeTransition,
};

impl EditorApplication {
    pub(super) fn detect_mode_transition(
        &mut self,
        ctx: &mut UpdateContext,
        new_mode: RuntimeMode,
    ) {
        let old_mode = self.last_runtime_mode;

        // Entering play mode from editor
        if matches!(old_mode, RuntimeMode::Editor) && matches!(new_mode, RuntimeMode::Playing) {
            info!("Transitioning from Editor to Play mode");

            // Save camera state
            let camera = ctx.scene.camera();
            self.editor_camera_state = Some(EditorCameraState {
                eye: camera.eye,
                target: camera.target,
                up: camera.up,
                fov_y_radians: camera.fov_y_radians,
            });

            // Save ALL transform states
            let mut transforms = HashMap::new();
            {
                let world = ctx.scene.main_world();
                for (entity, transform) in world.query::<&TransformComponent>().iter() {
                    transforms.insert(entity, transform.0);
                }
            }
            self.editor_transform_state = Some(EditorTransformState { transforms });

            info!(
                "Saved {} entity transforms",
                self.editor_transform_state
                    .as_ref()
                    .unwrap()
                    .transforms
                    .len()
            );

            // Queue the transition
            self.pending_mode_transition = Some(RuntimeModeTransition {
                from: old_mode,
                to: new_mode,
            });

            // Clear interaction state
            self.gizmo_drag = None;
            self.selection.clear_pending_pick();
        }

        // Exiting play mode to editor
        if matches!(old_mode, RuntimeMode::Playing) && matches!(new_mode, RuntimeMode::Editor) {
            info!("Transitioning from Play to Editor mode");

            // Queue the transition
            self.pending_mode_transition = Some(RuntimeModeTransition {
                from: old_mode,
                to: new_mode,
            });
        }

        self.last_runtime_mode = new_mode;
    }

    pub(super) fn process_pending_mode_transition(&mut self, ctx: &mut GpuUpdateContext) {
        let Some(transition) = self.pending_mode_transition.take() else {
            return;
        };

        match (transition.from, transition.to) {
            (RuntimeMode::Editor, RuntimeMode::Playing) => {
                self.enter_play_mode(ctx);
            }
            (RuntimeMode::Playing, RuntimeMode::Editor) => {
                self.exit_play_mode(ctx);
            }
            _ => {}
        }
    }

    fn exit_play_mode(&mut self, ctx: &mut GpuUpdateContext) {
        info!("Exiting play mode - restoring editor state");

        // DO NOT call set_animation_playback(false) here!
        // The animation system may try to restore its own cached state,
        // conflicting with our manual restoration.

        // Restore the saved transforms directly
        if let Some(saved_state) = self.editor_transform_state.take() {
            let world = ctx.scene.main_world_mut();
            let mut restored_count = 0;
            let mut missing_count = 0;

            for (entity, saved_transform) in saved_state.transforms {
                if world.contains(entity) {
                    if let Ok(mut transform) = world.get::<&mut TransformComponent>(entity) {
                        transform.0 = saved_transform;
                        restored_count += 1;
                    }
                } else {
                    missing_count += 1;
                }
            }

            info!(
                "Restored {} entity transforms ({} entities missing)",
                restored_count, missing_count
            );
        }

        // Restore camera state
        if let Some(camera_state) = self.editor_camera_state.take() {
            let camera = ctx.scene.camera_mut();
            camera.eye = camera_state.eye;
            camera.target = camera_state.target;
            camera.up = camera_state.up;
            camera.fov_y_radians = camera_state.fov_y_radians;
        }

        // Propagate the restored transforms
        ctx.scene.propagate_transforms();

        // NOW disable animations after transforms are restored
        // This way the animation system can't interfere with our restoration
        ctx.scene.set_animation_playback(false);

        // Reinitialize editor state
        self.refresh_next_editor_entity_id(ctx.scene);
        self.initialize_history_state(ctx.scene);

        self.selection.set_selected(None);
        self.selection.set_highlighted(None);
        self.selection.request_override(None);

        self.ensure_viewport_tab_for_mode(RuntimeMode::Editor);

        info!("Editor state restored");
    }

    fn enter_play_mode(&mut self, ctx: &mut GpuUpdateContext) {
        info!("Entering play mode");

        // Ensure transforms are fully propagated before starting animations
        ctx.scene.propagate_transforms();

        // Enable animations - this should capture the current state as rest pose
        ctx.scene.set_animation_playback(true);

        // Propagate again after animation system is initialized
        ctx.scene.propagate_transforms();

        // Switch to game viewport
        self.ensure_viewport_tab_for_mode(RuntimeMode::Playing);

        info!("Play mode started");
    }
}
