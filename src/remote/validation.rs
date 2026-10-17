//! Read-only validation of saved projects without instantiating or migrating them.
use crate::{project::ProjectManifest, scene::SceneAsset, scripting::LuaScriptingRuntime};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};

/// Lua functions that do nothing useful in a browser build (no file system there).
const WEB_UNSUPPORTED_LUA: &[&str] = &[
    "read_file",
    "write_file",
    "file_exists",
    "list_files",
    "open_file_dialog",
    "save_file_dialog",
    "open_folder_dialog",
    "io.open",
    "io.lines",
    "io.popen",
    "os.getenv",
    "os.execute",
    "os.remove",
    "os.rename",
    "os.tmpname",
];

/// Extensions of files a script loads by path.
const ASSET_EXTENSIONS: &[&str] = &[
    "wav", "ogg", "mp3", "flac", "ahx", "hvl", "a2m", "asid", "png", "jpg", "jpeg", "hdr", "json",
    "lua", "gltf", "glb",
];

/// Whether `line` calls `name(` (not a longer identifier, field or method that ends in it).
fn calls_function(line: &str, name: &str) -> bool {
    let mut from = 0;
    while let Some(found) = line[from..].find(name) {
        let start = from + found;
        let end = start + name.len();
        let before = line[..start].chars().next_back();
        let boundary =
            before.is_none_or(|c| !(c.is_alphanumeric() || matches!(c, '_' | '.' | ':')));
        if boundary && line[end..].trim_start().starts_with('(') {
            return true;
        }
        from = end;
    }
    false
}

/// The contents of the single- or double-quoted string literals on one line.
fn quoted_strings(line: &str) -> Vec<&str> {
    let bytes = line.as_bytes();
    let mut found = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let quote = bytes[i];
        if quote != b'"' && quote != b'\'' {
            i += 1;
            continue;
        }
        let start = i + 1;
        let mut end = start;
        while end < bytes.len() && bytes[end] != quote {
            end += if bytes[end] == b'\\' { 2 } else { 1 };
        }
        if end >= bytes.len() {
            break;
        }
        found.push(&line[start..end]);
        i = end + 1;
    }
    found
}

struct Validator {
    root: PathBuf,
    errors: Vec<Value>,
    warnings: Vec<Value>,
    /// What a browser build of the project would trip over.
    browser_errors: Vec<Value>,
    browser_warnings: Vec<Value>,
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
    fn browser_issue(
        &mut self,
        error: bool,
        file: &Path,
        location: &str,
        message: impl Into<String>,
    ) {
        let row =
            json!({"file":file.display().to_string(),"location":location,"message":message.into()});
        if error {
            self.browser_errors.push(row);
        } else {
            self.browser_warnings.push(row);
        }
    }
    /// A referenced asset path, as the browser will request it: `web_build` serves only
    /// `project.json` and `content/`, so anything else is a 404 there.
    fn browser_reference(&mut self, file: &Path, location: &str, value: &str, required: bool) {
        let path = Path::new(value);
        let leaves_project =
            path.is_absolute() || path.components().any(|c| c == Component::ParentDir);
        let staged = path
            .components()
            .next()
            .is_some_and(|c| c.as_os_str() == "content");
        if leaves_project {
            self.browser_issue(
                required,
                file,
                location,
                format!("{value:?} is outside the project, so a web build cannot serve it"),
            );
        } else if !staged && value != "project.json" {
            self.browser_issue(
                required,
                file,
                location,
                format!("{value:?} is not under content/, so web_build does not stage it"),
            );
        }
    }
    /// Lua source facts that matter in a browser: no file system, and asset paths that
    /// are written out in full (dynamically built paths are not seen).
    fn browser_lua_scan(&mut self, file: &Path, location: &str, source: &str) {
        let mut reported = BTreeSet::new();
        for line in source.lines() {
            if line.trim_start().starts_with("--") {
                continue;
            }
            for name in WEB_UNSUPPORTED_LUA {
                if calls_function(line, name) && reported.insert(*name) {
                    self.browser_issue(
                        false,
                        file,
                        location,
                        format!(
                            "{name}() has no file system in a browser (use save_data/load_data for saved state)"
                        ),
                    );
                }
            }
            for literal in quoted_strings(line) {
                let Some(extension) = literal
                    .strip_prefix("content/")
                    .and_then(|rest| Path::new(rest).extension())
                    .and_then(|e| e.to_str())
                else {
                    continue;
                };
                if !ASSET_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()) {
                    continue;
                }
                if !self.root.join(literal).is_file() && reported.insert(literal) {
                    self.issue(
                        false,
                        file,
                        location,
                        format!("script refers to {literal:?}, which does not exist"),
                    );
                }
            }
        }
    }
    fn reference(&mut self, file: &Path, location: &str, value: &str, required: bool) {
        if value.is_empty() || value.starts_with("data:") {
            return;
        }
        self.browser_reference(file, location, value, required);
        let path = Path::new(value);
        if path.is_absolute() || path.components().any(|c| c == Component::ParentDir) {
            self.issue(false, file, location, format!("nonportable path {value:?}"));
        }
        let resolved = self.root.join(path);
        // An imported glTF package is the folder its meshes and textures were unpacked to.
        if location.ends_with("/package") && resolved.is_dir() {
            return;
        }
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
                                | "baked"
                        )
                    }) {
                        // A missing glTF source or probe bake only loses detail: the
                        // scene keeps its meshes, the probes the environment's light.
                        let required = !matches!(key.as_str(), "gltf_source" | "baked");
                        self.reference(file, &location, path, required);
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
            return;
        }
        self.browser_lua_scan(file, location, source);
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
        browser_errors: Vec::new(),
        browser_warnings: Vec::new(),
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
    let mut lowercase_names: BTreeMap<String, PathBuf> = BTreeMap::new();
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let kind = entry.file_type().map_err(|e| e.to_string())?;
                let path = entry.path();
                if kind.is_dir() {
                    stack.push(path);
                } else if kind.is_file() {
                    let relative = path.strip_prefix(&root).unwrap_or(&path);
                    let key = relative.to_string_lossy().to_lowercase();
                    if let Some(other) = lowercase_names.insert(key, relative.to_path_buf()) {
                        validator.browser_issue(
                            false,
                            &path,
                            "",
                            format!(
                                "{} and {} differ only in letter case; a case-insensitive file system or host serves just one",
                                other.display(),
                                relative.display()
                            ),
                        );
                    }
                    if path.extension().is_some_and(|e| e == "lua" || e == "json") {
                        validator.check_file(&path);
                    }
                }
            }
        }
    }
    let browser = json!({
        "ok": validator.browser_errors.is_empty(),
        "errors": validator.browser_errors,
        "warnings": validator.browser_warnings,
        "scope": "What web_build serves (project.json and content/ only), asset files that differ only in case, and Lua calls that need a file system. A passing preflight is not proof the project runs in a browser: use web_build and web_playtest.",
    });
    Ok(
        json!({"ok":validator.errors.is_empty(),"project":root.display().to_string(),"files_checked":validator.checked.len(),"errors":validator.errors,"warnings":validator.warnings,"browser":browser,"scope":"Saved scene structure, asset paths, cameras and Lua syntax, plus a browser preflight. Dynamic require/audio paths and gameplay behavior need playtests."}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lua_scanning_finds_calls_and_quoted_paths() {
        assert!(calls_function("local t = read_file('x')", "read_file"));
        assert!(calls_function("  io.open ( path )", "io.open"));
        assert!(!calls_function("my_read_file(x)", "read_file"));
        assert!(!calls_function("obj.read_file(x)", "read_file"));
        assert!(!calls_function("local read_file = nil", "read_file"));
        assert_eq!(
            quoted_strings(r#"play_sound("content/a.wav", {x='b'}) -- 'c'"#),
            ["content/a.wav", "b", "c"]
        );
        assert_eq!(quoted_strings(r#"s = "a\"b" .. 'c'"#), [r#"a\"b"#, "c"]);
    }

    #[test]
    fn browser_preflight_reports_unserved_assets_case_clashes_and_file_system_use() {
        let dir = tempfile::tempdir().unwrap();
        let content = dir.path().join("content");
        for sub in ["scripts", "scenes", "audio"] {
            std::fs::create_dir_all(content.join(sub)).unwrap();
        }
        let manifest = crate::project::ProjectManifest::new_empty(Default::default());
        manifest.save_to_dir(dir.path()).unwrap();
        std::fs::write(
            content.join("scripts/game.lua"),
            "local s = read_file('save.txt')\nplay_sound(\"content/audio/gone.wav\")\n-- write_file('x')\n",
        )
        .unwrap();
        std::fs::write(content.join("audio/Hit.wav"), b"").unwrap();
        std::fs::write(content.join("audio/hit.wav"), b"").unwrap();
        std::fs::write(
            content.join("scenes/a.json"),
            r#"{"material": "textures/m.json", "path": "content/audio/hit.wav"}"#,
        )
        .unwrap();

        let result = validate(dir.path()).unwrap();
        let browser = &result["browser"];
        assert_eq!(browser["ok"], false);
        let errors = browser["errors"].to_string();
        assert!(
            errors.contains("textures/m.json") && errors.contains("not under content/"),
            "{errors}"
        );
        assert!(!errors.contains("hit.wav"), "{errors}");
        let warnings = browser["warnings"].to_string();
        assert!(warnings.contains("read_file()"), "{warnings}");
        assert!(
            !warnings.contains("write_file"),
            "a comment is not a call: {warnings}"
        );
        assert!(
            warnings.contains("differ only in letter case"),
            "{warnings}"
        );
        // The missing literal is a plain validation warning, native or not.
        assert!(result["warnings"]
            .to_string()
            .contains("content/audio/gone.wav"));
    }

    #[test]
    fn import_package_folders_count_and_missing_bakes_only_warn() {
        let dir = tempfile::tempdir().unwrap();
        let content = dir.path().join("content");
        std::fs::create_dir_all(content.join("gltf/chair")).unwrap();
        std::fs::create_dir_all(content.join("scenes")).unwrap();
        let manifest = crate::project::ProjectManifest::new_empty(Default::default());
        manifest.save_to_dir(dir.path()).unwrap();
        std::fs::write(
            content.join("scenes/a.json"),
            r#"{"imports": [{"package": "content/gltf/chair"}, {"package": "content/gltf/gone"}],
                "light_probe_volume": {"baked": "content/lighting/gone.probes"}}"#,
        )
        .unwrap();

        let result = validate(dir.path()).unwrap();
        let errors = result["errors"].to_string();
        assert!(!errors.contains("gltf/chair"), "{errors}");
        assert!(errors.contains("gltf/gone"), "{errors}");
        assert!(!errors.contains("gone.probes"), "{errors}");
        assert!(result["warnings"].to_string().contains("gone.probes"));
    }

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
