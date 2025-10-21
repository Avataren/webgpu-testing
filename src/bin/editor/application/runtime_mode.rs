use log::info;
use wgpu_cube::app::{GpuUpdateContext, RuntimeMode, UpdateContext};
use wgpu_cube::scene::SceneStateSnapshot;

use super::core::{EditorApplication, RuntimeModeTransition};

impl EditorApplication {
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

    pub fn detect_mode_transition(&mut self, ctx: &mut UpdateContext, new_mode: RuntimeMode) {
        let old_mode = self.last_runtime_mode;

        // Entering play mode from editor
        if matches!(old_mode, RuntimeMode::Editor) && matches!(new_mode, RuntimeMode::Playing) {
            info!("Transitioning from Editor to Play mode - capturing scene snapshot");

            // Capture the ENTIRE scene state
            self.editor_scene_snapshot = Some(SceneStateSnapshot::capture(ctx.scene));

            self.pending_mode_transition = Some(RuntimeModeTransition {
                from: old_mode,
                to: new_mode,
            });

            self.transform_tool.gizmo_drag = None;
            self.selection.clear_pending_pick();
        }

        // Exiting play mode to editor
        if matches!(old_mode, RuntimeMode::Playing) && matches!(new_mode, RuntimeMode::Editor) {
            info!("Transitioning from Play to Editor mode");
            self.pending_mode_transition = Some(RuntimeModeTransition {
                from: old_mode,
                to: new_mode,
            });
        }

        self.last_runtime_mode = new_mode;
    }

    fn exit_play_mode(&mut self, ctx: &mut GpuUpdateContext) {
        info!("Exiting play mode - restoring editor scene");

        // Completely restore the scene from the snapshot
        if let Some(snapshot) = self.editor_scene_snapshot.take() {
            snapshot.restore(ctx.scene); // Changed from restore_without_assets
        }

        // Reinitialize editor state
        self.refresh_next_editor_entity_id(ctx.scene);
        self.initialize_history_state(ctx.scene);

        self.selection.set_selected(None);
        self.selection.set_highlighted(None);
        self.selection.request_override(None);

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
