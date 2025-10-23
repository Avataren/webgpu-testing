use std::fs;
use std::path::{Path, PathBuf};

use hecs::Entity;
use log::{error, warn};
use wgpu_cube::app::UpdateContext;
use wgpu_cube::scripting::RuneScriptComponent;
use wgpu_cube::scripting::RuneScriptSource;

use super::core::{EditorApplication, PendingScriptAction};

impl EditorApplication {
    pub(super) fn load_script_text(path: &str) -> Option<String> {
        let full_path = PathBuf::from("scripts").join(path);
        match fs::read_to_string(&full_path) {
            Ok(src) => Some(src),
            Err(err) => {
                error!("Failed to read script {:?}: {err}", full_path);
                None
            }
        }
    }

    pub(super) fn load_script(path: &str) -> Option<RuneScriptSource> {
        Self::load_script_text(path).map(|src| RuneScriptSource::inline(path, src))
    }

    pub(super) fn create_import_script(path: &Path) -> Option<RuneScriptSource> {
        let template = Self::load_script_text("editor_import_gltf.rn")?;
        let raw_path = path.to_string_lossy();
        let mut path_string = raw_path.replace('\\', "/");
        if std::path::MAIN_SEPARATOR != '/' {
            path_string = path_string.replace(std::path::MAIN_SEPARATOR, "/");
        }

        let encoded_path = match serde_json::to_string(&path_string) {
            Ok(value) => value,
            Err(err) => {
                error!("Failed to encode glTF path {path:?}: {err}");
                return None;
            }
        };

        let script_source = template.replace("__GLTF_PATH__", &encoded_path);

        let script_name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "imported_gltf".to_string());

        Some(RuneScriptSource::inline(
            format!("editor_import_gltf::{script_name}"),
            script_source,
        ))
    }

    pub(super) fn apply_pending_script_actions(
        &mut self,
        ctx: &mut UpdateContext,
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
                        let world = ctx.scene.main_world_mut();
                        match world.get::<&mut RuneScriptComponent>(entity) {
                            Ok(mut component) => {
                                *component = RuneScriptComponent::new_inline(name, contents);
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
                        self.notify_script_editor_error(entity, message);
                    }

                    if updated {
                        reload_runtime = true;
                        notifications.push((entity, message));
                        self.record_scene_change(ctx.scene);
                    }
                }
                PendingScriptAction::ReloadRuntime { entity, message } => {
                    reload_runtime = true;
                    notifications.push((entity, message));
                }
            }
        }

        if reload_runtime {
            ctx.scene.reset_script_runtime();
            for (entity, message) in notifications {
                self.notify_script_editor_saved(entity, message);
            }
        }
    }

    fn notify_script_editor_saved(&mut self, entity: Entity, message: impl Into<String>) {
        if let Some(editor) = self.script_editor.as_mut() {
            if editor.entity() == entity {
                editor.finish_save(message);
            }
        }
    }

    fn notify_script_editor_error(&mut self, entity: Entity, message: impl Into<String>) {
        if let Some(editor) = self.script_editor.as_mut() {
            if editor.entity() == entity {
                editor.fail_save(message);
            }
        }
    }

    pub(super) fn ensure_script_editor_target_valid(&mut self, scene: &wgpu_cube::scene::Scene) {
        if let Some(editor) = self.script_editor.as_mut() {
            let world = scene.main_world();
            if !world.contains(editor.entity()) {
                editor.mark_target_missing("Entity has been removed.");
                return;
            }

            match world.get::<&RuneScriptComponent>(editor.entity()) {
                Ok(component) => {
                    editor.clear_target_missing();
                    let component_ref: &RuneScriptComponent = &component;
                    editor.sync_with_component(component_ref);
                }
                Err(_) => {
                    editor.mark_target_missing("Script component removed from entity.");
                }
            }
        }
    }
}
