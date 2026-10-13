//! Editor-specific remote-control methods: projects, scene documents, undo history,
//! selection and asset import. Everything else is handled by the engine's built-in
//! handlers (`wgpu_cube::remote::handlers`).

use std::path::PathBuf;

use serde_json::{json, Value};
use wgpu_cube::app::{GpuUpdateContext, RuntimeMode};
use wgpu_cube::remote::{entity_from_json, entity_id, RemoteContext, RemoteResult};
use wgpu_cube::SceneCreationAction;

use super::core::EditorApplication;
use super::EditorCommand;

/// Methods handled here, listed by `engine.methods`.
pub(super) const EDITOR_METHODS: &[&str] = &[
    "editor.state",
    "editor.undo",
    "editor.redo",
    "editor.select",
    "editor.open_file",
    "project.create",
    "project.open",
    "project.save",
    "scene.new",
    "scene.activate",
    "scene.close",
    "scene.rename",
    "scene.instance",
    "asset.import",
];

fn str_param<'a>(params: &'a Value, key: &str) -> Result<&'a str, String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string parameter \"{key}\""))
}

/// Make a client-supplied path absolute against the editor's working directory.
fn absolute(path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(&path))
            .unwrap_or(path)
    }
}

/// Content subfolder for a non-model asset, by file extension.
fn content_folder(extension: &str) -> Option<&'static str> {
    Some(match extension {
        "png" | "jpg" | "jpeg" | "tga" | "bmp" | "hdr" | "exr" => "textures",
        "wav" | "ogg" | "mp3" | "flac" => "audio",
        "ttf" | "otf" => "fonts",
        "lua" => "scripts",
        "wgsl" => "shaders",
        _ => return None,
    })
}

/// Copy an image, sound, font, script or shader into the project's content folder.
/// A texture the scene already uses is reloaded when its file is replaced.
fn import_file(
    ctx: &mut RemoteContext<'_, '_>,
    project_dir: &std::path::Path,
    source: &std::path::Path,
    extension: &str,
    params: &Value,
) -> RemoteResult {
    let content = project_dir.join(wgpu_cube::project::CONTENT_DIR);
    let relative = match params.get("dest").and_then(Value::as_str) {
        Some(dest) => PathBuf::from(dest),
        None => {
            let folder = content_folder(extension).ok_or_else(|| {
                format!(
                    "cannot import .{extension} files: use glTF/GLB models, images (png, jpg, tga, bmp, hdr, exr), \
                     audio (wav, ogg, mp3, flac), fonts (ttf, otf), .lua or .wgsl, or give dest"
                )
            })?;
            let name = source.file_name().ok_or("the path has no file name")?;
            PathBuf::from(folder).join(name)
        }
    };
    if relative.is_absolute() || relative.components().any(|part| part.as_os_str() == "..") {
        return Err("dest must be a path inside the project's content folder".into());
    }
    let target = content.join(&relative);
    let overwrite = params
        .get("overwrite")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let same_file = target
        .canonicalize()
        .is_ok_and(|target| source.canonicalize().is_ok_and(|source| source == target));
    let replaced = target.exists() && !same_file;
    if replaced && !overwrite {
        return Err(format!(
            "{} already exists; pass overwrite: true to replace it",
            target.display()
        ));
    }
    if !same_file {
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("cannot create {}: {err}", parent.display()))?;
        }
        std::fs::copy(source, &target)
            .map_err(|err| format!("cannot copy to {}: {err}", target.display()))?;
    }
    let mut result = json!({
        "imported": target.display().to_string(),
        "project_path": std::path::Path::new(wgpu_cube::project::CONTENT_DIR)
            .join(&relative)
            .display()
            .to_string(),
        "note": "Reference it by project_path (e.g. material base_color_texture) so the project stays portable.",
    });
    if replaced {
        let reloaded = ctx
            .gpu
            .scene
            .assets
            .reload_textures(&mut *ctx.gpu.renderer, Some(&target))
            .into_iter()
            .filter(|(_, outcome)| outcome.is_ok())
            .count();
        if reloaded > 0 {
            ctx.gpu
                .renderer
                .update_texture_bind_group(&ctx.gpu.scene.assets);
        }
        result["textures_reloaded"] = json!(reloaded);
    }
    Ok(result)
}

impl EditorApplication {
    pub(super) fn handle_remote_command(
        &mut self,
        method: &str,
        params: &Value,
        ctx: &mut RemoteContext<'_, '_>,
    ) -> Option<RemoteResult> {
        let result = match method {
            "editor.state" => Ok(self.remote_editor_state(ctx)),
            "editor.undo" | "editor.redo" => {
                let history = self.history();
                let possible = if method == "editor.undo" {
                    history.can_undo()
                } else {
                    history.can_redo()
                };
                if !possible {
                    return Some(Err(format!("nothing to {}", &method[7..])));
                }
                self.enqueue_command(if method == "editor.undo" {
                    EditorCommand::HistoryUndo
                } else {
                    EditorCommand::HistoryRedo
                });
                Ok(json!({ "queued": method }))
            }
            "editor.select" => self.remote_select(params, ctx),
            "editor.open_file" => (|| {
                let path = absolute(str_param(params, "path")?);
                if !path.is_file() {
                    return Err(format!("no file at {}", path.display()));
                }
                self.script_editor_system_mut().open_file(&path);
                Ok(json!({ "opened": path.display().to_string() }))
            })(),
            "project.create" => (|| {
                let name = str_param(params, "name")?.trim().to_owned();
                if name.is_empty() {
                    return Err("project name is empty".to_string());
                }
                let location = absolute(str_param(params, "location")?);
                let directory = location.join(&name);
                if directory.join("project.json").exists() {
                    return Err(format!(
                        "{} already contains a project; use project.open",
                        directory.display()
                    ));
                }
                self.enqueue_command(EditorCommand::CreateProject { name, location });
                Ok(json!({
                    "queued": "project.create",
                    "directory": directory.display().to_string(),
                    "note": "The project is created and opened over the next frames; check editor.state.",
                }))
            })(),
            "project.open" => (|| {
                let path = absolute(str_param(params, "path")?);
                if !path.join("project.json").exists() {
                    return Err(format!("no project.json in {}", path.display()));
                }
                self.enqueue_command(EditorCommand::LoadProject(path.clone()));
                Ok(json!({
                    "queued": "project.open",
                    "path": path.display().to_string(),
                    "note": "The project loads over the next frames; check editor.state.",
                }))
            })(),
            "project.save" => (|| {
                let dir = match params.get("path").and_then(Value::as_str) {
                    Some(path) => absolute(path),
                    None => self
                        .project_system()
                        .controller()
                        .current_dir()
                        .cloned()
                        .ok_or("no project is open; pass \"path\" or create a project first")?,
                };
                if matches!(
                    self.shared.runtime_state.active_mode(),
                    RuntimeMode::Playing
                ) {
                    return Err(
                        "stop play mode before saving; play-mode changes are discarded on stop"
                            .to_string(),
                    );
                }
                self.project_system_mut()
                    .controller_mut()
                    .request_save_project(dir.clone());
                Ok(json!({ "queued": "project.save", "path": dir.display().to_string() }))
            })(),
            "scene.new" => {
                self.shared.runtime_state.request_mode(RuntimeMode::Editor);
                self.enqueue_command(EditorCommand::NewScene);
                Ok(json!({ "queued": "scene.new" }))
            }
            "scene.activate" | "scene.close" => (|| {
                let document_id = str_param(params, "document_id")?.to_owned();
                if method == "scene.activate" {
                    self.shared.runtime_state.request_mode(RuntimeMode::Editor);
                    self.enqueue_command(EditorCommand::ActivateScene(document_id.clone()));
                } else {
                    self.enqueue_command(EditorCommand::CloseScene(document_id.clone()));
                }
                Ok(json!({ "queued": method, "document_id": document_id }))
            })(),
            "scene.rename" => (|| {
                let old_id = str_param(params, "document_id")?.to_owned();
                let new_name = str_param(params, "name")?.to_owned();
                self.enqueue_command(EditorCommand::RenameScene { old_id, new_name });
                Ok(json!({ "queued": "scene.rename" }))
            })(),
            "scene.instance" => (|| {
                let document_id = str_param(params, "document_id")?.to_owned();
                let node_path = params
                    .get("node_path")
                    .and_then(Value::as_array)
                    .map(|path| {
                        path.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                self.enqueue_command(EditorCommand::CreateScene(
                    SceneCreationAction::SceneInstance {
                        document_id,
                        node_path,
                    },
                ));
                Ok(json!({
                    "queued": "scene.instance",
                    "note": "The instance is parented to the selected entity, if any, and selected once created.",
                }))
            })(),
            "asset.import" => (|| {
                let path = absolute(str_param(params, "path")?);
                if !path.exists() {
                    return Err(format!("file not found: {}", path.display()));
                }
                let Some(project_dir) = self.project_system().controller().current_dir().cloned()
                else {
                    return Err("open or create a project before importing".to_string());
                };
                let extension = path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(str::to_ascii_lowercase)
                    .unwrap_or_default();
                if !matches!(extension.as_str(), "gltf" | "glb") {
                    return import_file(ctx, &project_dir, &path, &extension, params);
                }
                self.enqueue_command(EditorCommand::ImportPath(path.clone()));
                Ok(json!({
                    "queued": "asset.import",
                    "path": path.display().to_string(),
                    "note": "The glTF is copied into the project, saved as a scene document and instanced into the active scene.",
                }))
            })(),
            _ => return None,
        };
        Some(result)
    }

    fn remote_editor_state(&self, ctx: &mut RemoteContext<'_, '_>) -> Value {
        let controller = self.project_system().controller();
        let project_dir = controller.current_dir().cloned();
        let content_root = project_dir
            .as_ref()
            .map(|dir| dir.join(wgpu_cube::project::CONTENT_DIR));
        let documents: Vec<Value> = self
            .project_system()
            .scene_documents()
            .iter()
            .map(|doc| {
                json!({
                    "document_id": doc.id,
                    "name": doc.name,
                    "path": doc.relative_path.display().to_string(),
                })
            })
            .collect();
        let tabs: Vec<Value> = self
            .scene_tabs()
            .iter()
            .map(|tab| {
                json!({
                    "document_id": tab.document_id,
                    "title": tab.title,
                    "active": tab.active,
                })
            })
            .collect();
        let world = ctx.gpu.scene.main_world();
        let selected = self.selection_system().selected().map(|entity| {
            json!({
                "id": entity_id(entity),
                "name": world.get::<&wgpu_cube::scene::Name>(entity).ok().map(|name| name.0.clone()),
            })
        });
        let history = self.history();

        json!({
            "project": project_dir.as_ref().map(|dir| json!({
                "name": controller.metadata().name,
                "directory": dir.display().to_string(),
                "content_root": content_root.as_ref().map(|root| root.display().to_string()),
                "scripts_dir": content_root.as_ref().map(|root| root.join("scripts").display().to_string()),
            })),
            "startup_dialog_visible": self.project_system().is_startup_dialog_visible(),
            "runtime_mode": match self.shared.runtime_state.active_mode() {
                RuntimeMode::Editor => "editor",
                RuntimeMode::Playing => "playing",
            },
            "scene_documents": documents,
            "open_scenes": tabs,
            "active_scene": ctx.gpu.scene.document_id(),
            "selected": selected,
            "can_undo": history.can_undo(),
            "can_redo": history.can_redo(),
            "hints": [
                "Create or open a project first; the startup dialog blocks the editor until then.",
                "Write game scripts under scripts_dir and attach them with entity_update {script: {path}}.",
                "Scripts without an annotation run only in play mode; use runtime_set play/stop.",
                "Save with project_save; play-mode changes are discarded when stopping.",
            ],
        })
    }

    fn remote_select(&mut self, params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
        let entity = match params.get("entity") {
            None | Some(Value::Null) => None,
            Some(value) => Some(entity_from_json(ctx.gpu.scene.main_world(), value)?),
        };
        let selection = self.selection_system_mut();
        selection.set_selected(entity);
        selection.set_highlighted(entity);
        selection.request_override(entity);
        Ok(json!({ "selected": entity.map(entity_id) }))
    }

    /// Keep editor state in step after remote requests changed the scene.
    pub(super) fn handle_remote_scene_modified(&mut self, ctx: &mut GpuUpdateContext) {
        self.shared.active_camera_entity = ctx.scene.active_camera_entity();
        if matches!(self.shared.runtime_state.active_mode(), RuntimeMode::Editor) {
            self.record_scene_change(&mut ctx.scene);
            self.history_system_mut().clear_redo();
        } else {
            // Play-mode changes are thrown away on stop, so keep them out of the history.
            self.shared.mark_scene_hierarchy_dirty(ctx.scene_handle);
        }
    }
}
