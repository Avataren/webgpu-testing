//! Editor-specific remote-control methods: projects, scene documents, undo history,
//! selection and asset import. Everything else is handled by the engine's built-in
//! handlers (`wgpu_cube::remote::handlers`).

use std::path::PathBuf;

use serde_json::{json, Value};
use wgpu_cube::app::{GpuUpdateContext, RuntimeMode};
use wgpu_cube::io::JsonLayout;
use wgpu_cube::remote::{entity_from_json, entity_id, RemoteContext, RemoteResult};
use wgpu_cube::scene::{TransformGizmoMode, TransformGizmoSpace};
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
    "prefab.create",
    "prefab.inspect",
    "prefab.update",
];

fn gizmo_mode_name(mode: TransformGizmoMode) -> &'static str {
    match mode {
        TransformGizmoMode::Translate => "translate",
        TransformGizmoMode::Rotate => "rotate",
        TransformGizmoMode::Scale => "scale",
    }
}

fn gizmo_space_name(space: TransformGizmoSpace) -> &'static str {
    match space {
        TransformGizmoSpace::Local => "local",
        TransformGizmoSpace::World => "world",
    }
}

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
/// Whether a glTF should be imported as a clip library (its animations, to play on
/// characters) rather than as a model: asked for with `kind: "animation"`, or a file
/// without meshes that has animations.
fn is_animation_library(path: &std::path::Path, params: &Value) -> Result<bool, String> {
    match params.get("kind").and_then(Value::as_str) {
        Some("animation") => return Ok(true),
        Some("model") | None => {}
        Some(other) => return Err(format!("unknown kind {other:?}; use model or animation")),
    }
    if params.get("kind").is_some() {
        return Ok(false);
    }
    let gltf = gltf::Gltf::open(path).map_err(|err| format!("{}: {err}", path.display()))?;
    Ok(gltf.meshes().len() == 0 && gltf.animations().len() > 0)
}

/// Copy a clip library (and a .gltf's external buffers) into content/animations and
/// report its clips.
pub(super) fn import_animation_library(
    project_dir: &std::path::Path,
    source: &std::path::Path,
    params: &Value,
) -> RemoteResult {
    let name = source.file_name().ok_or("the path has no file name")?;
    let relative = PathBuf::from(wgpu_cube::project::CONTENT_DIR)
        .join("animations")
        .join(name);
    let target = project_dir.join(&relative);
    let overwrite = params
        .get("overwrite")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if target.exists() && !overwrite {
        return Err(format!(
            "{} already exists; pass overwrite: true to replace it",
            target.display()
        ));
    }
    let parent = target.parent().ok_or("bad target path")?;
    std::fs::create_dir_all(parent)
        .map_err(|err| format!("cannot create {}: {err}", parent.display()))?;
    std::fs::copy(source, &target)
        .map_err(|err| format!("cannot copy to {}: {err}", target.display()))?;
    // A .gltf may keep its buffers in files next to it.
    let gltf = gltf::Gltf::open(source).map_err(|err| err.to_string())?;
    for buffer in gltf.buffers() {
        if let gltf::buffer::Source::Uri(uri) = buffer.source() {
            if uri.starts_with("data:") || uri.contains("..") || uri.contains("://") {
                continue;
            }
            let from = source
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join(uri);
            let to = parent.join(uri);
            if let Some(dir) = to.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            std::fs::copy(&from, &to)
                .map_err(|err| format!("cannot copy buffer {}: {err}", from.display()))?;
        }
    }
    let library = wgpu_cube::scene::skeletal::ClipLibrary::load(&target)?;
    Ok(json!({
        "imported": target.display().to_string(),
        "project_path": relative.display().to_string(),
        "clips": library.clips.iter().map(|clip| json!({
            "name": clip.name,
            "duration": clip.duration,
            "tracks": clip.tracks.len(),
        })).collect::<Vec<_>>(),
        "note": "Add it to a character with entity_update {entity, animator: {add_clips: project_path}}; \
                 a file with one clip plays under its file name (e.g. \"walk\" for walk.glb).",
    }))
}

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
            "prefab.create" | "prefab.update" => self.remote_save_prefab(method, params, ctx),
            "prefab.inspect" => self.remote_inspect_prefab(params, ctx),
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
                let after_seq = self
                    .project_system()
                    .controller()
                    .last_save()
                    .map_or(0, |save| save.seq);
                self.project_system_mut()
                    .controller_mut()
                    .request_save_project_with_options(
                        dir.clone(),
                        match params["compact"].as_bool() {
                            Some(true) => JsonLayout::Compact,
                            Some(false) => JsonLayout::Pretty,
                            None => JsonLayout::Preserve,
                        },
                    );
                Ok(json!({
                    "queued": "project.save",
                    "path": dir.display().to_string(),
                    "save_seq": after_seq + 1,
                    "note": "Saved on the next frame; editor.state's last_save reports the outcome.",
                }))
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
                // Animations can go straight onto a character's animator.
                let attach = params
                    .get("entity")
                    .map(|value| entity_from_json(ctx.gpu.scene.main_world(), value))
                    .transpose()?;
                if super::blender_convert::CONVERTIBLE.contains(&extension.as_str()) {
                    let clips = attach.is_some()
                        || params.get("kind").and_then(Value::as_str) == Some("animation");
                    if clips {
                        self.enqueue_command(EditorCommand::ImportClips {
                            path: path.clone(),
                            attach,
                        });
                    } else {
                        self.enqueue_command(EditorCommand::ImportPath(path.clone()));
                    }
                    return Ok(json!({
                        "queued": "asset.import",
                        "path": path.display().to_string(),
                        "converting": true,
                        "note": "Blender converts the file in the background (a few seconds). A model \
                                 then imports and appears in the scene like a glTF; a file without \
                                 meshes (or kind: animation) becomes a clip library in \
                                 content/animations named after the file. Watch logs for the result.",
                    }));
                }
                if !matches!(extension.as_str(), "gltf" | "glb") {
                    return import_file(ctx, &project_dir, &path, &extension, params);
                }
                if attach.is_some() || is_animation_library(&path, params)? {
                    let result = import_animation_library(&project_dir, &path, params)?;
                    if let (Some(entity), Some(library)) = (attach, result["project_path"].as_str())
                    {
                        self.enqueue_command(EditorCommand::Inspector(
                            crate::inspector::InspectorAction::AddAnimationLibrary {
                                entity,
                                path: PathBuf::from(library),
                            },
                        ));
                    }
                    return Ok(result);
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

    fn remote_save_prefab(
        &mut self,
        method: &str,
        params: &Value,
        ctx: &mut RemoteContext<'_, '_>,
    ) -> RemoteResult {
        if ctx.runtime_mode() != Some(RuntimeMode::Editor) {
            return Err("prefab authoring requires editor mode".into());
        }
        let root = self
            .project_system()
            .controller()
            .current_dir()
            .cloned()
            .ok_or("open a project before authoring prefabs")?;
        let entity = entity_from_json(
            ctx.gpu.scene.main_world(),
            params.get("entity").ok_or("entity required")?,
        )?;
        let (id, name) = if method == "prefab.create" {
            let name = str_param(params, "name")?;
            if name.is_empty()
                || name.len() > 100
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return Err("prefab name needs 1..=100 letters, digits, _ or -".into());
            }
            let id = format!("prefab_{name}");
            if self.project_system().scene_document(&id).is_some()
                || root
                    .join("content/scenes")
                    .join(format!("{id}.json"))
                    .exists()
            {
                return Err("prefab already exists; use prefab_update".into());
            }
            (id, name.to_string())
        } else {
            let id = str_param(params, "document_id")?;
            let document = self
                .project_system()
                .scene_document(id)
                .ok_or("unknown prefab document")?;
            if !id.starts_with("prefab_") {
                return Err("prefab_update only updates documents created by prefab_create".into());
            }
            (id.to_string(), document.name.clone())
        };
        let mut asset = ctx
            .gpu
            .scene
            .export_entity_subtree(entity, name)
            .ok_or("cannot export this hierarchy")?;
        if method == "prefab.update" {
            // Linked instances address the source root by name. Reauthoring from a
            // differently named selection must keep that existing reference valid.
            let root_name = self
                .project_system_mut()
                .scene_library_mut()
                .asset(&id)
                .and_then(|source| source.entities.iter().find(|row| row.parent.is_none()))
                .and_then(|row| row.name.clone());
            if let Some(root) = asset.entities.first_mut() {
                if root_name.is_some() {
                    root.name = root_name;
                }
            }
        }
        asset.relativize_script_paths(&root);
        asset
            .persist_material_assets(&root)
            .map_err(|e| e.to_string())?;
        let entities = asset.entities.len();
        self.project_system_mut()
            .save_scene_to_library(id.clone(), asset, &root)?;
        Ok(
            json!({"document_id":id,"path":format!("content/scenes/{id}.json"),"entities":entities,
            "note":"Source saved and registered for scene_instance. project_save persists the document registration. Existing instances retain their current values; instantiate again or reopen to load the updated source."}),
        )
    }

    fn remote_inspect_prefab(
        &mut self,
        params: &Value,
        ctx: &mut RemoteContext<'_, '_>,
    ) -> RemoteResult {
        let id = str_param(params, "document_id")?;
        let document = self
            .project_system()
            .scene_document(id)
            .cloned()
            .ok_or("unknown document")?;
        let asset = self
            .project_system_mut()
            .scene_library_mut()
            .asset(id)
            .cloned()
            .ok_or("source document is not loaded")?;
        let instances = ctx
            .gpu
            .scene
            .prefab_entity_metadata()
            .into_iter()
            .filter(|(_, meta)| meta.document_id == id)
            .map(|(entity, meta)| json!({"id":entity_id(entity),"node_path":meta.node_path}))
            .collect::<Vec<_>>();
        // Instance transform/component overrides are serialized in scene_ref on proxy entities.
        let tree = ctx.gpu.scene.export_main_asset("PrefabInspection");
        fn collect(value: &Value, id: &str, out: &mut Vec<Value>) {
            match value {
                Value::Object(object) => {
                    if let Some(reference) = object.get("scene_ref").filter(|r| r["document"] == id)
                    {
                        out.push(reference.clone());
                    }
                    for value in object.values() {
                        collect(value, id, out);
                    }
                }
                Value::Array(array) => {
                    for value in array {
                        collect(value, id, out);
                    }
                }
                _ => {}
            }
        }
        let mut overrides = Vec::new();
        collect(
            &serde_json::to_value(tree).map_err(|e| e.to_string())?,
            id,
            &mut overrides,
        );
        Ok(
            json!({"document":document,"source":asset,"instance_entities":instances,"instance_overrides":overrides}),
        )
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
        let tool = self.history_system().transform_tool();

        json!({
            "project": project_dir.as_ref().map(|dir| json!({
                "name": controller.metadata().name,
                "directory": dir.display().to_string(),
                "content_root": content_root.as_ref().map(|root| root.display().to_string()),
                "scripts_dir": content_root.as_ref().map(|root| root.join("scripts").display().to_string()),
            })),
            "last_save": controller.last_save().map(|save| json!({
                "seq": save.seq,
                "directory": save.dir.display().to_string(),
                "error": save.error,
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
            "gizmo": gizmo_mode_name(tool.gizmo_mode),
            "gizmo_space": gizmo_space_name(tool.gizmo_space),
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
        // The transform tool, as the W/E/R shortcuts and the toolbar set it.
        let mode = match params.get("gizmo").and_then(Value::as_str) {
            None => None,
            Some("translate") => Some(TransformGizmoMode::Translate),
            Some("rotate") => Some(TransformGizmoMode::Rotate),
            Some("scale") => Some(TransformGizmoMode::Scale),
            Some(other) => {
                return Err(format!(
                    "unknown gizmo \"{other}\" (translate, rotate or scale)"
                ))
            }
        };
        let space = match params.get("gizmo_space").and_then(Value::as_str) {
            None => None,
            Some("local") => Some(TransformGizmoSpace::Local),
            Some("world") => Some(TransformGizmoSpace::World),
            Some(other) => return Err(format!("unknown gizmo_space \"{other}\" (local or world)")),
        };
        if mode.is_some() || space.is_some() {
            let tool = self.history_system_mut().transform_tool_mut();
            if let Some(mode) = mode {
                tool.gizmo_mode = mode;
            }
            if let Some(space) = space {
                tool.gizmo_space = space;
            }
            if params.get("entity").is_none() && params.get("uv").is_none() {
                let tool = self.history_system().transform_tool();
                return Ok(json!({
                    "gizmo": gizmo_mode_name(tool.gizmo_mode),
                    "gizmo_space": gizmo_space_name(tool.gizmo_space),
                }));
            }
        }
        // A click in the Scene view: the GPU pick a mouse click makes.
        if let Some(uv) = params.get("uv").and_then(Value::as_array) {
            let coordinate = |index: usize| {
                uv.get(index)
                    .and_then(Value::as_f64)
                    .filter(|value| (0.0..=1.0).contains(value))
                    .ok_or_else(|| "uv must be [u, v] in 0..1".to_string())
            };
            let uv = glam::Vec2::new(coordinate(0)? as f32, coordinate(1)? as f32);
            self.selection_system_mut()
                .set_pending_pick(super::core::ViewportPick { uv });
            return Ok(json!({
                "queued": "pick",
                "uv": [uv.x, uv.y],
                "note": "The selection follows in a few frames (editor_state.selected)",
            }));
        }
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
