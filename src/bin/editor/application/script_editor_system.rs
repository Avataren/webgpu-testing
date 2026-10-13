use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use hecs::Entity;
use log::warn;

use super::core::PendingScriptAction;
use super::system::{EditorAppAccess, EditorCommand, EditorContext, EditorSystem};
use crate::script_editor::{ScriptEditorEvent, ScriptEditorState};
use wgpu_cube::scene::{Scene, SceneWorkspaceSceneMut};
use wgpu_cube::scripting::LuaScriptComponent;

#[derive(Default)]
pub(crate) struct ScriptEditorSystem {
    editor: ScriptEditorState,
    window_enabled: bool,
    content_root: Option<PathBuf>,
}

impl ScriptEditorSystem {
    /// Open an entity's script: inline scripts edit the component, file scripts the file.
    pub(crate) fn open_script_editor(&mut self, entity: Entity, component: LuaScriptComponent) {
        self.editor.open_inline(entity, &component);
    }

    /// Open any text file in the code editor.
    pub(crate) fn open_file(&mut self, path: &Path) {
        self.editor.open_file(path);
    }

    /// True while the code editor should get keyboard shortcuts such as Ctrl+S.
    pub(crate) fn wants_keyboard(&self) -> bool {
        self.window_enabled && self.editor.wants_keyboard()
    }

    pub(crate) fn set_window_enabled(&mut self, enabled: bool) {
        self.window_enabled = enabled;
    }

    /// The project content folder, so project files show relative paths.
    pub(crate) fn set_content_root(&mut self, content_root: Option<PathBuf>) {
        self.content_root = content_root;
    }

    fn ensure_target_valid(&mut self, scene: &Scene) {
        let world = scene.main_world();
        for entity in self.editor.inline_entities() {
            if !world.contains(entity) {
                self.editor
                    .sync_inline(entity, Err("Entity has been removed."));
                continue;
            }
            match world.get::<&LuaScriptComponent>(entity) {
                Ok(component) => self.editor.sync_inline(entity, Ok(&component)),
                Err(_) => self
                    .editor
                    .sync_inline(entity, Err("Script component removed from entity.")),
            }
        }
    }

    fn notify_saved(&mut self, entity: Entity, message: impl Into<String>) {
        self.editor.finish_save(entity, message);
    }

    fn notify_error(&mut self, entity: Entity, message: impl Into<String>) {
        self.editor.fail_save(entity, message);
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

                    // Replacing the component recompiles just this script.
                    if updated {
                        notifications.push((entity, message));
                        app.record_scene_change(scene);
                    }
                }
            }
        }

        for (entity, message) in notifications {
            self.notify_saved(entity, message);
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

        match self.editor.show(egui_ctx, self.content_root.clone()) {
            ScriptEditorEvent::None => {}
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
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
