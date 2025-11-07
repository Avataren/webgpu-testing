use std::collections::VecDeque;

use hecs::Entity;
use log::warn;

use super::core::PendingScriptAction;
use super::system::{EditorAppAccess, EditorCommand, EditorContext, EditorSystem};
use crate::script_editor::{ScriptEditorEvent, ScriptEditorState};
use wgpu_cube::scene::{Scene, SceneWorkspaceSceneMut};
use wgpu_cube::scripting::LuaScriptComponent;

#[derive(Default)]
pub(crate) struct ScriptEditorSystem {
    editor: Option<ScriptEditorState>,
    window_enabled: bool,
}

impl ScriptEditorSystem {
    pub(crate) fn open_script_editor(&mut self, entity: Entity, component: LuaScriptComponent) {
        if let Some(editor) = self.editor.as_mut() {
            if editor.entity() == entity {
                editor.sync_with_component(&component);
                return;
            }
        }
        self.editor = Some(ScriptEditorState::new(entity, component));
    }

    pub(crate) fn set_window_enabled(&mut self, enabled: bool) {
        self.window_enabled = enabled;
    }

    fn ensure_target_valid(&mut self, scene: &Scene) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };

        let world = scene.main_world();
        if !world.contains(editor.entity()) {
            editor.mark_target_missing("Entity has been removed.");
            return;
        }

        match world.get::<&LuaScriptComponent>(editor.entity()) {
            Ok(component) => {
                editor.clear_target_missing();
                editor.sync_with_component(&component);
            }
            Err(_) => {
                editor.mark_target_missing("Script component removed from entity.");
            }
        }
    }

    fn notify_saved(&mut self, entity: Entity, message: impl Into<String>) {
        if let Some(editor) = self.editor.as_mut() {
            if editor.entity() == entity {
                editor.finish_save(message);
            }
        }
    }

    fn notify_error(&mut self, entity: Entity, message: impl Into<String>) {
        if let Some(editor) = self.editor.as_mut() {
            if editor.entity() == entity {
                editor.fail_save(message);
            }
        }
    }

    fn drain_script_commands(queue: &mut VecDeque<EditorCommand>) -> Vec<PendingScriptAction> {
        let mut pending = Vec::new();
        let mut remaining = VecDeque::with_capacity(queue.len());

        while let Some(command) = queue.pop_front() {
            match command {
                EditorCommand::Script(action) => pending.push(action),
                other => remaining.push_back(other),
            }
        }

        *queue = remaining;
        pending
    }

    fn process_pending_actions(
        &mut self,
        app: &mut EditorAppAccess<'_>,
        scene: &mut SceneWorkspaceSceneMut<'_>,
        actions: Vec<PendingScriptAction>,
    ) {
        if actions.is_empty() {
            return;
        }

        let mut reload_runtime = false;
        let mut notifications = Vec::new();

        for action in actions {
            match action {
                PendingScriptAction::SaveInline {
                    entity,
                    name,
                    contents,
                    message,
                } => {
                    let mut updated = false;
                    let mut error_message = None;

                    {
                        let world = scene.main_world_mut();
                        match world.get::<&mut LuaScriptComponent>(entity) {
                            Ok(mut component) => {
                                *component = LuaScriptComponent::new_inline(name, contents);
                                updated = true;
                            }
                            Err(err) => {
                                error_message =
                                    Some(format!("Failed to update inline script: {err}"));
                                warn!("Failed to update inline script for {:?}: {err}", entity);
                            }
                        }
                    }

                    if let Some(message) = error_message {
                        self.notify_error(entity, message);
                    }

                    if updated {
                        reload_runtime = true;
                        notifications.push((entity, message));
                        app.record_scene_change(scene);
                    }
                }
                PendingScriptAction::ReloadRuntime { entity, message } => {
                    reload_runtime = true;
                    notifications.push((entity, message));
                }
            }
        }

        if reload_runtime {
            scene.reset_script_runtime();
            for (entity, message) in notifications {
                self.notify_saved(entity, message);
            }
        }
    }
}

impl EditorSystem for ScriptEditorSystem {
    fn update<'app, 'ctx, 'scene>(&mut self, ctx: &mut EditorContext<'app, 'ctx, 'scene>) {
        if ctx.update_context_mut().is_none() {
            return;
        }

        let actions = {
            let queue = ctx.command_queue();
            Self::drain_script_commands(queue)
        };

        let _ = ctx.with_update_app(move |app, update_ctx| {
            self.ensure_target_valid(&update_ctx.scene);
            self.process_pending_actions(app, &mut update_ctx.scene, actions);
        });
    }

    fn ui<'app, 'ctx>(&mut self, ctx: &mut EditorContext<'app, 'ctx, 'ctx>) {
        let egui_ctx = {
            let Some(ui_ctx) = ctx.ui_context() else {
                return;
            };
            ui_ctx.egui()
        };

        if !self.window_enabled {
            return;
        }

        let Some(editor) = self.editor.as_mut() else {
            return;
        };

        let event = editor.show(egui_ctx);

        match event {
            ScriptEditorEvent::None => {}
            ScriptEditorEvent::Closed => {
                self.editor = None;
            }
            ScriptEditorEvent::SaveInline {
                entity,
                name,
                contents,
                message,
            } => {
                ctx.command_queue().push_back(EditorCommand::Script(
                    PendingScriptAction::SaveInline {
                        entity,
                        name,
                        contents,
                        message,
                    },
                ));
            }
            ScriptEditorEvent::SaveFile { entity, message } => {
                ctx.command_queue().push_back(EditorCommand::Script(
                    PendingScriptAction::ReloadRuntime { entity, message },
                ));
            }
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
