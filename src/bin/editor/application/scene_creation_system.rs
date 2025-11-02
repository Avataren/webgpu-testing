use std::collections::VecDeque;

use hecs::Entity;

use super::system::{EditorCommand, EditorContext, EditorSystem};
use wgpu_cube::SceneCreationAction;

#[derive(Default)]
pub(crate) struct SceneCreationSystem;

impl SceneCreationSystem {
    fn drain_scene_commands(queue: &mut VecDeque<EditorCommand>) -> Vec<SceneCreationAction> {
        let mut pending = Vec::new();
        let mut remaining = VecDeque::with_capacity(queue.len());

        while let Some(command) = queue.pop_front() {
            match command {
                EditorCommand::CreateScene(action) => pending.push(action),
                other => remaining.push_back(other),
            }
        }

        *queue = remaining;
        pending
    }

    fn process_pending_creations(
        &mut self,
        ctx: &mut EditorContext<'_, '_, '_>,
        actions: Vec<SceneCreationAction>,
    ) {
        if actions.is_empty() {
            return;
        }

        let Some(()) = ctx.with_gpu_app(|app, gpu_ctx| {
            let mut last_created: Option<Entity> = None;

            for action in actions {
                let created = match action {
                    SceneCreationAction::Primitive(preset) => app.create_primitive(gpu_ctx, preset),
                    SceneCreationAction::ParticleSystem(preset) => {
                        app.create_particle_system(gpu_ctx, preset)
                    }
                    SceneCreationAction::PointLight => app.create_point_light(gpu_ctx),
                    SceneCreationAction::DirectionalLight => app.create_directional_light(gpu_ctx),
                    SceneCreationAction::SpotLight => app.create_spot_light(gpu_ctx),
                    SceneCreationAction::Camera => app.create_camera(gpu_ctx),
                    SceneCreationAction::Environment => app.create_environment(gpu_ctx),
                };

                if let Some(entity) = created {
                    last_created = Some(entity);
                }
            }

            if let Some(entity) = last_created {
                let selection = app.selection_system_mut();
                selection.set_selected(Some(entity));
                selection.set_highlighted(Some(entity));
                selection.request_override(Some(entity));
                app.record_scene_change(&mut gpu_ctx.scene);
                app.history_system_mut().clear_redo();

                if let Some(handle) = app.scene_hierarchy_handle_for_scene(gpu_ctx.scene_handle) {
                    if let Ok(mut state) = handle.lock() {
                        let info = state.workspace_info();
                        state.refresh_from_scene(&gpu_ctx.scene, info);
                    }
                }
            }
        }) else {
            return;
        };
    }
}

impl EditorSystem for SceneCreationSystem {
    fn gpu_update<'app, 'ctx, 'scene>(&mut self, ctx: &mut EditorContext<'app, 'ctx, 'scene>) {
        let actions = Self::drain_scene_commands(ctx.command_queue());
        self.process_pending_creations(ctx, actions);
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
