use log::info;
use wgpu_cube::app::{GpuUpdateContext, RuntimeMode, UpdateContext};
use wgpu_cube::scene::SceneStateSnapshot;

use super::core::{EditorApplication, RuntimeModeTransition};

impl EditorApplication {
    pub(super) fn process_pending_mode_transition(&mut self, ctx: &mut GpuUpdateContext) {
        let Some(transition) = self.shared.pending_mode_transition.take() else {
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

    pub fn detect_mode_transition(&mut self, ctx: &mut UpdateContext, new_mode: RuntimeMode) {
        let old_mode = self.shared.last_runtime_mode;

        // Entering play mode from editor
        if matches!(old_mode, RuntimeMode::Editor) && matches!(new_mode, RuntimeMode::Playing) {
            info!("Transitioning from Editor to Play mode - capturing scene snapshot");

            // Capture the ENTIRE scene state
            self.shared.editor_scene_snapshot = Some(SceneStateSnapshot::capture(&ctx.scene));

            self.shared.pending_mode_transition = Some(RuntimeModeTransition {
                from: old_mode,
                to: new_mode,
            });

            self.history_system_mut().clear_gizmo_drag();
            self.selection_system_mut().clear_pending_pick();
        }

        // Exiting play mode to editor
        if matches!(old_mode, RuntimeMode::Playing) && matches!(new_mode, RuntimeMode::Editor) {
            info!("Transitioning from Play to Editor mode");
            self.shared.pending_mode_transition = Some(RuntimeModeTransition {
                from: old_mode,
                to: new_mode,
            });
        }

        self.shared.last_runtime_mode = new_mode;
    }

    fn exit_play_mode(&mut self, ctx: &mut GpuUpdateContext) {
        info!("Exiting play mode - restoring editor scene");

        // Completely restore the scene from the snapshot
        if let Some(snapshot) = self.shared.editor_scene_snapshot.take() {
            snapshot.restore(&mut ctx.scene); // Changed from restore_without_assets
        }

        // NOTE: UI plugins are now in a separate world and persist across scene changes.
        // No need to reload them after scene restore.
        // Just re-initialize them to ensure they're in a good state.
        if self.shared.ui_plugins_loaded {
            if let Some(ref mut manager) = self.shared.ui_plugin_manager {
                info!("Re-initializing UI plugins after scene restore");
                ctx.scene
                    .run_scripts_for_world(manager.world_mut(), 0.0, true);
            }
        }

        // Reinitialize editor state
        {
            self.history_system_mut()
                .initialize_state(&mut ctx.scene, None, None);
        }

        let selection = self.selection_system_mut();
        selection.set_selected(None);
        selection.set_highlighted(None);
        selection.request_override(None);

        self.ensure_viewport_tab_for_mode(RuntimeMode::Editor);

        info!("Editor scene fully restored");
    }

    fn enter_play_mode(&mut self, ctx: &mut GpuUpdateContext) {
        info!("Entering play mode");

        ctx.scene.propagate_transforms();
        ctx.scene.set_animation_playback(true);

        self.ensure_viewport_tab_for_mode(RuntimeMode::Playing);

        info!("Play mode started");
    }
}
