use std::fs;
use std::path::{Path, PathBuf};

use log::error;
use wgpu_cube::scripting::RuneScriptSource;

use super::core::EditorApplication;

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
}
