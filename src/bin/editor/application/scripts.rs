use std::fs;
use std::path::PathBuf;

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
}
