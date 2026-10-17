//! What a game's export carries: only the content its project uses.
//!
//! [`used_content`] follows references from a saved project, starting at its main
//! scene (the first in `project.json`):
//!
//! - scene, material, glTF and other JSON files: every string naming a project file
//!   (model imports, materials, textures, scripts, clip libraries, baked lighting, HDR
//!   skies...), followed in turn; prefab instances bring their scene documents;
//! - a model import (`*.glb.import`) brings its whole package folder (meshes, textures);
//! - Lua scripts: string literals naming project files (`play_sound("content/audio/x.wav")`)
//!   and modules they `require` or `include`. A literal naming a folder
//!   (`"content/audio/"`, for paths a script builds at run time) brings the whole folder;
//! - the mixer's layout (`content/audio/mixer.json`), which nothing names.
//!
//! Everything else in `content/` (assets imported or taken from the asset library and
//! not used, the documents of models not placed in the scene) stays out.
//! [`export_content`] copies the used files and a `project.json` listing only the used
//! scenes into a build folder.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use crate::project::{CONTENT_DIR, PROJECT_FILE_NAME};

/// The outcome of [`used_content`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsedContent {
    /// Project-relative files the game uses (`project.json` itself not included).
    pub files: BTreeSet<PathBuf>,
    /// The scene documents it uses (ids), main scene first.
    pub scenes: Vec<String>,
    pub bytes: u64,
    /// Files under `content/` it does not use.
    pub unused: Vec<PathBuf>,
    pub unused_bytes: u64,
    /// References to files that are missing, or outside the project (which an export
    /// cannot carry).
    pub problems: Vec<String>,
}

/// Scene documents listed in `project.json`, by id: (relative path, name).
type Documents = BTreeMap<String, (PathBuf, String)>;

struct Walker<'a> {
    root: &'a Path,
    documents: Documents,
    scene_names: BTreeMap<String, String>,
    files: BTreeSet<PathBuf>,
    scenes: Vec<String>,
    queue: VecDeque<PathBuf>,
    problems: BTreeSet<String>,
}

impl Walker<'_> {
    /// Use the file at project-relative `path` (and what it refers to).
    fn add_file(&mut self, path: PathBuf) {
        if self.files.insert(path.clone()) {
            self.queue.push_back(path);
        }
    }

    fn add_dir(&mut self, dir: &Path) {
        let mut stack = vec![self.root.join(dir)];
        while let Some(current) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&current) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if let Ok(relative) = path.strip_prefix(self.root) {
                    self.add_file(relative.to_path_buf());
                }
            }
        }
    }

    fn add_scene(&mut self, id: &str) {
        if self.scenes.iter().any(|scene| scene == id) {
            return;
        }
        match self.documents.get(id).cloned() {
            Some((path, _)) => {
                self.scenes.push(id.to_string());
                self.add_file(path);
            }
            None => {
                self.problems
                    .insert(format!("scene document {id:?} is not in project.json"));
            }
        }
    }

    /// A project-relative path for `text` met in the file `from`, if it names something
    /// in the project (relative to the project, to `from`'s folder, or absolute inside
    /// the project). Paths outside the project are reported.
    fn resolve(&mut self, text: &str, from: &Path) -> Option<PathBuf> {
        if text.is_empty() || text.len() > 512 || text.contains('\n') {
            return None;
        }
        if !(text.contains('/') || text.contains('\\') || text.contains('.')) {
            return None;
        }
        let decoded = percent_decode(text);
        let candidate = PathBuf::from(decoded.replace('\\', "/"));
        if candidate.is_absolute() {
            return match candidate.strip_prefix(self.root) {
                Ok(relative) => normalize(relative),
                Err(_) => {
                    if candidate.is_file() && looks_like_asset(&candidate) {
                        self.problems.insert(format!(
                            "{} refers to {} outside the project",
                            from.display(),
                            candidate.display()
                        ));
                    }
                    None
                }
            };
        }
        let beside = from
            .parent()
            .map(|dir| dir.join(&candidate))
            .and_then(|path| normalize(&path));
        let in_project = normalize(&candidate);
        [in_project, beside]
            .into_iter()
            .flatten()
            .find(|path| self.root.join(path).exists())
    }

    fn use_reference(&mut self, text: &str, from: &Path, folders: bool) {
        let Some(path) = self.resolve(text, from) else {
            return;
        };
        let absolute = self.root.join(&path);
        if absolute.is_file() {
            self.add_file(path);
        } else if folders && absolute.is_dir() && path.starts_with(CONTENT_DIR) {
            // Not content/ itself: that would be everything.
            if path.components().count() > 1 {
                self.add_dir(&path);
            }
        }
    }

    fn visit(&mut self, path: &Path) {
        let absolute = self.root.join(path);
        let name = path.to_string_lossy().to_lowercase();
        if name.ends_with(".import") {
            // A model import: its package folder holds what it lists (meshes, textures).
            if let Some(dir) = path.parent() {
                self.add_dir(dir);
            }
        }
        if name.ends_with(".json") || name.ends_with(".gltf") || name.ends_with(".import") {
            let Ok(text) = std::fs::read_to_string(&absolute) else {
                return;
            };
            let Ok(value) = serde_json::from_str::<Value>(&text) else {
                return;
            };
            let mut strings = Vec::new();
            collect_strings(&value, &mut strings);
            // Prefab instances name their scene document.
            let mut prefabs = Vec::new();
            collect_scene_refs(&value, &mut prefabs);
            for id in prefabs {
                self.add_scene(&id);
            }
            for text in strings {
                self.use_reference(&text, path, false);
            }
        } else if name.ends_with(".lua") {
            let Ok(source) = std::fs::read_to_string(&absolute) else {
                return;
            };
            for literal in lua_strings(&source) {
                self.use_reference(&literal, path, true);
                // Scene documents named by id or name.
                if self.documents.contains_key(&literal) {
                    self.add_scene(&literal);
                } else if let Some(id) = self.scene_names.get(&literal).cloned() {
                    self.add_scene(&id);
                }
                // Modules: "lib.helpers" -> lib/helpers.lua beside the script or under
                // the scripts folder (engine.* ones come with the engine).
                if is_module_name(&literal) {
                    let relative = format!("{}.lua", literal.replace('.', "/"));
                    let scripts = Path::new(CONTENT_DIR).join("scripts");
                    let candidates = [
                        path.parent().map(|dir| dir.join(&relative)),
                        Some(scripts.join(&relative)),
                    ];
                    if let Some(found) = candidates
                        .into_iter()
                        .flatten()
                        .find(|candidate| self.root.join(candidate).is_file())
                    {
                        self.add_file(found);
                    }
                }
            }
        }
    }
}

/// What the saved project at `project_root` uses. See the [module docs](self).
pub fn used_content(project_root: &Path) -> Result<UsedContent, String> {
    let root = project_root
        .canonicalize()
        .map_err(|err| format!("cannot open {}: {err}", project_root.display()))?;
    let manifest = read_manifest(&root)?;
    let documents = documents(&manifest);
    let main = manifest["scenes"]
        .as_array()
        .and_then(|scenes| scenes.first())
        .and_then(|scene| scene["id"].as_str())
        .ok_or("project.json lists no scenes")?
        .to_string();
    let scene_names = documents
        .iter()
        .map(|(id, (_, name))| (name.clone(), id.clone()))
        .collect();
    let mut walker = Walker {
        root: &root,
        documents,
        scene_names,
        files: BTreeSet::new(),
        scenes: Vec::new(),
        queue: VecDeque::new(),
        problems: BTreeSet::new(),
    };
    walker.add_scene(&main);
    // The mixer's buses and effects, which nothing names.
    if root.join(crate::audio::LAYOUT_FILE).is_file() {
        walker.add_file(PathBuf::from(crate::audio::LAYOUT_FILE));
    }
    // project.json's own references (the environment's sky), not its scene list.
    let mut strings = Vec::new();
    for (key, value) in manifest.as_object().into_iter().flatten() {
        if key != "scenes" && key != "imports" {
            collect_strings(value, &mut strings);
        }
    }
    let manifest_path = PathBuf::from(PROJECT_FILE_NAME);
    for text in strings {
        walker.use_reference(&text, &manifest_path, false);
    }
    while let Some(path) = walker.queue.pop_front() {
        walker.visit(&path);
    }

    let mut used = UsedContent {
        scenes: walker.scenes,
        problems: walker.problems.into_iter().collect(),
        ..UsedContent::default()
    };
    for file in walker.files {
        match std::fs::metadata(root.join(&file)) {
            Ok(meta) => {
                used.bytes += meta.len();
                used.files.insert(file);
            }
            Err(_) => used.problems.push(format!("{} is missing", file.display())),
        }
    }
    let mut stack = vec![root.join(CONTENT_DIR)];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(relative) = path.strip_prefix(&root) {
                if !used.files.contains(relative) {
                    used.unused_bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
                    used.unused.push(relative.to_path_buf());
                }
            }
        }
    }
    used.unused.sort();
    Ok(used)
}

/// Copy what the project at `project_root` uses into `output` (keeping its paths), with
/// a `project.json` listing only the used scenes. Returns what was copied.
pub fn export_content(project_root: &Path, output: &Path) -> Result<UsedContent, String> {
    let used = used_content(project_root)?;
    let root = project_root
        .canonicalize()
        .map_err(|err| format!("cannot open {}: {err}", project_root.display()))?;
    for file in &used.files {
        let target = output.join(file);
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
        }
        std::fs::copy(root.join(file), &target)
            .map_err(|err| format!("cannot copy {}: {err}", file.display()))?;
    }
    let mut manifest = read_manifest(&root)?;
    if let Some(scenes) = manifest["scenes"].as_array_mut() {
        scenes.retain(|scene| {
            scene["id"]
                .as_str()
                .is_some_and(|id| used.scenes.iter().any(|used| used == id))
        });
    }
    if let Some(imports) = manifest["imports"].as_array_mut() {
        imports.retain(|import| {
            import["package"]
                .as_str()
                .map(PathBuf::from)
                .is_some_and(|package| used.files.iter().any(|file| file.starts_with(&package)))
        });
    }
    std::fs::create_dir_all(output)
        .map_err(|err| format!("cannot create {}: {err}", output.display()))?;
    let text = serde_json::to_string_pretty(&manifest).map_err(|err| err.to_string())?;
    std::fs::write(output.join(PROJECT_FILE_NAME), text)
        .map_err(|err| format!("cannot write project.json: {err}"))?;
    Ok(used)
}

fn read_manifest(root: &Path) -> Result<Value, String> {
    let path = root.join(PROJECT_FILE_NAME);
    let text = std::fs::read_to_string(&path)
        .map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    serde_json::from_str(&text).map_err(|err| format!("{} is not valid: {err}", path.display()))
}

fn documents(manifest: &Value) -> Documents {
    manifest["scenes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|scene| {
            Some((
                scene["id"].as_str()?.to_string(),
                (
                    PathBuf::from(scene["relative_path"].as_str()?),
                    scene["name"].as_str().unwrap_or_default().to_string(),
                ),
            ))
        })
        .collect()
}

fn collect_strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => out.push(text.clone()),
        Value::Array(items) => items.iter().for_each(|item| collect_strings(item, out)),
        Value::Object(map) => map.values().for_each(|item| collect_strings(item, out)),
        _ => {}
    }
}

/// Scene documents prefab instances point at (`"scene_ref": {"document": id}`).
fn collect_scene_refs(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Array(items) => items.iter().for_each(|item| collect_scene_refs(item, out)),
        Value::Object(map) => {
            if let Some(id) = map
                .get("scene_ref")
                .and_then(|scene_ref| scene_ref.get("document"))
                .and_then(Value::as_str)
            {
                out.push(id.to_string());
            }
            map.values().for_each(|item| collect_scene_refs(item, out));
        }
        _ => {}
    }
}

/// `a/./b/../c` -> `a/c`; `None` for paths leaving their base.
fn normalize(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

fn looks_like_asset(path: &Path) -> bool {
    path.extension().is_some_and(|ext| {
        !matches!(
            ext.to_string_lossy().to_lowercase().as_str(),
            "exe" | "so" | "dll"
        )
    })
}

fn percent_decode(text: &str) -> String {
    if !text.contains('%') {
        return text.to_string();
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(value) = u8::from_str_radix(&text[i + 1..i + 3], 16) {
                out.push(value);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_string())
}

fn is_module_name(text: &str) -> bool {
    !text.is_empty()
        && text.len() < 128
        && !text.starts_with('.')
        && !text.ends_with('.')
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '/')
}

/// The string literals of a Lua source: '...', "..." (with escapes) and [[...]] /
/// [==[...]==] long strings; comments are skipped.
fn lua_strings(source: &str) -> Vec<String> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    // `[==[` at `start`: the closing bracket's level, and where the text begins.
    let long_open = |start: usize| -> Option<(usize, usize)> {
        if chars.get(start) != Some(&'[') {
            return None;
        }
        let mut j = start + 1;
        while chars.get(j) == Some(&'=') {
            j += 1;
        }
        (chars.get(j) == Some(&'[')).then_some((j - start - 1, j + 1))
    };
    let long_close = |from: usize, level: usize| -> (usize, usize) {
        let mut k = from;
        while k < chars.len() {
            if chars[k] == ']' {
                let mut m = k + 1;
                let mut equals = 0;
                while chars.get(m) == Some(&'=') {
                    equals += 1;
                    m += 1;
                }
                if equals == level && chars.get(m) == Some(&']') {
                    return (k, m + 1);
                }
            }
            k += 1;
        }
        (chars.len(), chars.len())
    };
    while i < chars.len() {
        let c = chars[i];
        if c == '-' && chars.get(i + 1) == Some(&'-') {
            if let Some((level, text)) = long_open(i + 2) {
                i = long_close(text, level).1;
            } else {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            continue;
        }
        if c == '"' || c == '\'' {
            let mut text = String::new();
            i += 1;
            while i < chars.len() && chars[i] != c && chars[i] != '\n' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    let next = chars[i + 1];
                    text.push(match next {
                        'n' => '\n',
                        't' => '\t',
                        other => other,
                    });
                    i += 2;
                    continue;
                }
                text.push(chars[i]);
                i += 1;
            }
            out.push(text);
            i += 1;
            continue;
        }
        if let Some((level, text)) = long_open(i) {
            let (end, after) = long_close(text, level);
            out.push(chars[text..end].iter().collect());
            i = after;
            continue;
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lua_literals_skip_comments_and_read_long_strings() {
        let source = r#"
            -- play_sound("content/audio/commented.wav")
            --[[ "also.wav" ]]
            local a = "content/audio/jump.wav"
            local b = 'single\'s.ogg'
            local c = [==[content/levels/one.json]==]
            local m = require("lib.helpers")
        "#;
        assert_eq!(
            lua_strings(source),
            [
                "content/audio/jump.wav",
                "single's.ogg",
                "content/levels/one.json",
                "lib.helpers"
            ]
        );
    }

    fn write(root: &Path, path: &str, contents: &str) {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn a_project_exports_what_its_main_scene_uses() {
        let root = std::env::temp_dir().join(format!("export-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        write(
            &root,
            "project.json",
            r#"{"scenes": [
                {"id": "main", "name": "Main", "relative_path": "content/scenes/main.json"},
                {"id": "robot.scene", "name": "Robot", "relative_path": "content/scenes/robot.scene.json"},
                {"id": "unused.scene", "name": "Unused", "relative_path": "content/scenes/unused.scene.json"}
            ], "imports": [
                {"package": "content/gltf/robot", "metadata": []},
                {"package": "content/gltf/crate", "metadata": []}
            ]}"#,
        );
        write(
            &root,
            "content/scenes/main.json",
            r#"{"entities": [
                {"name": "Robot", "scene_ref": {"document": "robot.scene"},
                 "material": {"path": "content/materials/red.mat.json"}},
                {"name": "Player", "script": {"source": {"File": {"path": "content/scripts/player.lua"}}},
                 "animator": {"libraries": ["content/animations/Walk.glb"]}}
            ]}"#,
        );
        write(
            &root,
            "content/scenes/robot.scene.json",
            r#"{"entities": [{"gltf_source": "content/gltf/robot/robot.glb.import"}]}"#,
        );
        write(
            &root,
            "content/scenes/unused.scene.json",
            r#"{"entities": []}"#,
        );
        write(
            &root,
            "content/gltf/robot/robot.glb.import",
            r#"{"scene": {}}"#,
        );
        write(&root, "content/gltf/robot/meshes/mesh_0000.json", "{}");
        write(&root, "content/gltf/crate/crate.glb.import", "{}");
        write(
            &root,
            "content/materials/red.mat.json",
            r#"{"textures": {"base_color": "content/textures/red.png"}}"#,
        );
        write(&root, "content/textures/red.png", "png");
        write(&root, "content/textures/blue.png", "png");
        write(&root, "content/animations/Walk.glb", "glb");
        write(&root, "content/animations/Run.glb", "glb");
        write(
            &root,
            "content/scripts/player.lua",
            r#"local H = require("lib.helpers")
               play_sound("content/audio/jump.wav")
               local step = "content/audio/steps/" .. n .. ".wav""#,
        );
        write(&root, "content/scripts/lib/helpers.lua", "return {}");
        write(&root, "content/scripts/unused.lua", "");
        write(&root, "content/audio/jump.wav", "wav");
        write(&root, "content/audio/steps/1.wav", "wav");
        write(&root, "content/audio/music.ogg", "ogg");
        write(&root, "content/audio/mixer.json", "{}");

        let used = used_content(&root).unwrap();
        let files: Vec<String> = used
            .files
            .iter()
            .map(|f| f.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            files,
            [
                "content/animations/Walk.glb",
                "content/audio/jump.wav",
                "content/audio/mixer.json",
                "content/audio/steps/1.wav",
                "content/gltf/robot/meshes/mesh_0000.json",
                "content/gltf/robot/robot.glb.import",
                "content/materials/red.mat.json",
                "content/scenes/main.json",
                "content/scenes/robot.scene.json",
                "content/scripts/lib/helpers.lua",
                "content/scripts/player.lua",
                "content/textures/red.png",
            ]
        );
        assert_eq!(used.scenes, ["main", "robot.scene"]);
        assert!(used
            .unused
            .contains(&PathBuf::from("content/animations/Run.glb")));
        assert!(used.problems.is_empty(), "{:?}", used.problems);

        let out = root.join("build");
        export_content(&root, &out).unwrap();
        let manifest: Value =
            serde_json::from_str(&std::fs::read_to_string(out.join("project.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["scenes"].as_array().unwrap().len(), 2);
        assert_eq!(manifest["imports"].as_array().unwrap().len(), 1);
        assert!(out.join("content/audio/steps/1.wav").is_file());
        assert!(!out.join("content/audio/music.ogg").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
