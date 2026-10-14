//! Read-only validation of saved projects without instantiating or migrating them.
use crate::{project::ProjectManifest, scene::SceneAsset, scripting::LuaScriptingRuntime};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
};

struct Validator {
    root: PathBuf,
    errors: Vec<Value>,
    warnings: Vec<Value>,
    checked: BTreeSet<PathBuf>,
    lua: LuaScriptingRuntime,
    document_ids: BTreeSet<String>,
}
impl Validator {
    fn issue(&mut self, error: bool, file: &Path, location: &str, message: impl Into<String>) {
        let row =
            json!({"file":file.display().to_string(),"location":location,"message":message.into()});
        if error {
            self.errors.push(row);
        } else {
            self.warnings.push(row);
        }
    }
    fn reference(&mut self, file: &Path, location: &str, value: &str, required: bool) {
        if value.is_empty() || value.starts_with("data:") {
            return;
        }
        let path = Path::new(value);
        if path.is_absolute() || path.components().any(|c| c == Component::ParentDir) {
            self.issue(false, file, location, format!("nonportable path {value:?}"));
        }
        let resolved = self.root.join(path);
        if !resolved.is_file() {
            self.issue(required, file, location, format!("missing asset {value:?}"));
            return;
        }
        if let Ok(canonical) = resolved.canonicalize() {
            if !canonical.starts_with(&self.root) {
                self.issue(
                    false,
                    file,
                    location,
                    format!("asset resolves outside the project: {value:?}"),
                );
            }
        }
        if resolved
            .extension()
            .is_some_and(|e| e == "json" || e == "lua")
        {
            self.check_file(&resolved);
        }
    }
    fn walk(&mut self, file: &Path, location: &str, value: &Value) {
        match value {
            Value::Object(object) => {
                for (key, value) in object {
                    let location = format!("{location}/{key}");
                    if let Some(path) = value.as_str().filter(|_| {
                        matches!(
                            key.as_str(),
                            "path"
                                | "relative_path"
                                | "package"
                                | "gltf_source"
                                | "material"
                                | "hdr_path"
                        )
                    }) {
                        self.reference(file, &location, path, key != "gltf_source");
                    }
                    if key == "Inline" {
                        if let Some(source) = value["source"].as_str() {
                            self.compile(file, &location, source);
                        }
                    }
                    self.walk(file, &location, value);
                }
            }
            Value::Array(array) => {
                for (i, value) in array.iter().enumerate() {
                    self.walk(file, &format!("{location}/{i}"), value);
                }
            }
            _ => {}
        }
    }
    fn compile(&mut self, file: &Path, location: &str, source: &str) {
        if let Err(error) = self
            .lua
            .lua()
            .load(source)
            .set_name(file.to_string_lossy())
            .into_function()
        {
            self.issue(true, file, location, error.to_string());
        }
    }
    fn check_file(&mut self, file: &Path) {
        let normalized = file.canonicalize().unwrap_or_else(|_| file.to_path_buf());
        if !self.checked.insert(normalized) {
            return;
        }
        let text = match std::fs::read_to_string(file) {
            Ok(text) => text,
            Err(e) => {
                self.issue(true, file, "", e.to_string());
                return;
            }
        };
        if file.extension().is_some_and(|e| e == "lua") {
            self.compile(file, "", &text);
            return;
        }
        let value = match serde_json::from_str::<Value>(&text) {
            Ok(v) => v,
            Err(e) => {
                self.issue(true, file, "", e.to_string());
                return;
            }
        };
        if value.get("entities").is_some() {
            match serde_json::from_value::<SceneAsset>(value.clone()) {
                Ok(asset) => {
                    let mut ids = BTreeSet::new();
                    let mut names = BTreeSet::new();
                    for (i, entity) in asset.entities.iter().enumerate() {
                        let loc = format!("/entities/{i}");
                        if let Some(reference) = &entity.scene_ref {
                            if !self.document_ids.contains(&reference.document) {
                                self.issue(
                                    true,
                                    file,
                                    &loc,
                                    format!(
                                        "prefab document {:?} is not registered",
                                        reference.document
                                    ),
                                );
                            }
                        }
                        if let Some(id) = entity.editor_id {
                            if !ids.insert(id) {
                                self.issue(true, file, &loc, "duplicate editor entity id");
                            }
                        }
                        if let Some(name) = &entity.name {
                            if !names.insert(name) {
                                self.issue(false,file,&loc,format!("duplicate name {name:?}; MCP name lookup will be ambiguous"));
                            }
                        }
                        if entity
                            .parent
                            .is_some_and(|index| index >= asset.entities.len())
                            || entity
                                .children
                                .iter()
                                .any(|index| *index >= asset.entities.len())
                        {
                            self.issue(true, file, &loc, "hierarchy index is out of range");
                        }
                        let mut current = Some(i);
                        let mut visited = BTreeSet::new();
                        while let Some(index) = current {
                            if !visited.insert(index) {
                                self.issue(true, file, &loc, "hierarchy has a parent cycle");
                                break;
                            }
                            current = asset.entities.get(index).and_then(|entity| entity.parent);
                        }
                        if let Some(camera) = &entity.camera {
                            use crate::scene::CameraProjection;
                            let valid = match camera.projection {
                                CameraProjection::Perspective {
                                    fov_y_radians,
                                    near,
                                    far,
                                } => {
                                    fov_y_radians.is_finite()
                                        && fov_y_radians > 0.0
                                        && fov_y_radians < std::f32::consts::PI
                                        && near > 0.0
                                        && far > near
                                }
                                CameraProjection::Orthographic {
                                    left,
                                    right,
                                    bottom,
                                    top,
                                    near,
                                    far,
                                } => right > left && top > bottom && far > near,
                            };
                            if !valid {
                                self.issue(
                                    true,
                                    file,
                                    &loc,
                                    "invalid camera projection or clipping range",
                                );
                            }
                        }
                    }
                    if asset
                        .active_camera
                        .is_some_and(|i| asset.entities.get(i).is_none_or(|e| e.camera.is_none()))
                    {
                        self.issue(
                            true,
                            file,
                            "/active_camera",
                            "active camera does not reference a camera entity",
                        );
                    }
                    if let Some(mesh) = asset.mesh_data.iter().find(|m| {
                        m.indices
                            .iter()
                            .any(|index| *index as usize >= m.vertices.len())
                    }) {
                        let _ = mesh;
                        self.issue(true, file, "/mesh_data", "mesh index is out of range");
                    }
                }
                Err(e) => self.issue(true, file, "", format!("invalid scene asset: {e}")),
            }
        }
        self.walk(file, "", &value);
    }
}

/// Validate serialized assets and Lua syntax without running scripts or writing files.
pub(crate) fn validate(root: &Path) -> Result<Value, String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let file = root.join("project.json");
    let text = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
    let manifest: ProjectManifest =
        serde_json::from_str(&text).map_err(|e| format!("invalid project.json: {e}"))?;
    let mut validator = Validator {
        root: root.clone(),
        errors: Vec::new(),
        warnings: Vec::new(),
        checked: BTreeSet::new(),
        lua: LuaScriptingRuntime::new().map_err(|e| e.to_string())?,
        document_ids: manifest
            .scenes
            .iter()
            .map(|document| document.id.clone())
            .collect(),
    };
    validator.check_file(&file);
    if manifest.scenes.is_empty() {
        validator.issue(true, &file, "/scenes", "project has no scene documents");
    }
    let mut ids = BTreeSet::new();
    for document in &manifest.scenes {
        if !ids.insert(&document.id) {
            validator.issue(
                true,
                &file,
                "/scenes",
                format!("duplicate scene document {}", document.id),
            );
        }
    }
    let mut stack = vec![root.join("content")];
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let kind = entry.file_type().map_err(|e| e.to_string())?;
                let path = entry.path();
                if kind.is_dir() {
                    stack.push(path);
                } else if kind.is_file()
                    && path.extension().is_some_and(|e| e == "lua" || e == "json")
                {
                    validator.check_file(&path);
                }
            }
        }
    }
    Ok(
        json!({"ok":validator.errors.is_empty(),"project":root.display().to_string(),"files_checked":validator.checked.len(),"errors":validator.errors,"warnings":validator.warnings,"scope":"Saved scene structure, asset paths, cameras and Lua syntax. Dynamic require/audio paths and gameplay behavior need playtests."}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validation_reports_missing_assets_and_lua_syntax_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("content")).unwrap();
        let manifest = crate::project::ProjectManifest::new_empty(Default::default());
        manifest.save_to_dir(dir.path()).unwrap();
        std::fs::write(dir.path().join("content/bad.lua"), "function broken(").unwrap();
        let before = std::fs::read(dir.path().join("project.json")).unwrap();
        let result = validate(dir.path()).unwrap();
        assert_eq!(result["ok"], false);
        assert!(result["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["file"].as_str().unwrap().ends_with("bad.lua")));
        assert_eq!(
            before,
            std::fs::read(dir.path().join("project.json")).unwrap()
        );
    }
}
