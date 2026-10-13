use log::info;
use wgpu_cube::app::{GpuUpdateContext, RuntimeMode};

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

    pub fn detect_mode_transition(&mut self, new_mode: RuntimeMode) {
        let old_mode = self.shared.last_runtime_mode;

        // Entering play mode from editor. The scene snapshot that stop restores is taken
        // by the runtime controller before the first play frame's scripts run; by now
        // they already have, so the editor must not take (or restore) its own.
        if matches!(old_mode, RuntimeMode::Editor) && matches!(new_mode, RuntimeMode::Playing) {
            info!("Transitioning from Editor to Play mode");

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
        info!("Exiting play mode");

        // The runtime controller has already restored the pre-play scene.

        // Refresh the hierarchy so the inspector shows the restored components
        // (material handles and the like).
        self.shared.mark_scene_hierarchy_dirty(ctx.scene_handle);

        // UI plugins own their world and Lua runtime, so they are unaffected by the
        // scene restore and keep their state.

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
