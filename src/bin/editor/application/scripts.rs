use std::fs;
use std::path::PathBuf;

use log::{debug, error};
use wgpu_cube::scripting::LuaScriptSource;

use super::core::EditorApplication;

impl EditorApplication {
    pub(super) fn load_script_text(path: &str) -> Option<String> {
        // Try multiple locations to find the script
        let search_paths = Self::get_script_search_paths(path);

        for full_path in &search_paths {
            debug!("Trying to load script from: {:?}", full_path);
            match fs::read_to_string(full_path) {
                Ok(src) => {
                    debug!("Successfully loaded script from: {:?}", full_path);
                    return Some(src);
                }
                Err(_) => continue,
            }
        }

        error!(
            "Failed to read script '{}'. Tried paths: {:?}",
            path, search_paths
        );
        None
    }

    fn get_script_search_paths(path: &str) -> Vec<PathBuf> {
        let mut paths = Vec::new();

        // 1. Try relative to current working directory (for development)
        paths.push(PathBuf::from("scripts").join(path));

        // 2. Try relative to executable location (for deployed builds)
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                paths.push(exe_dir.join("scripts").join(path));
            }
        }

        // 3. In debug builds, also try CARGO_MANIFEST_DIR
        #[cfg(debug_assertions)]
        {
            let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            paths.push(manifest_dir.join("scripts").join(path));
        }

        paths
    }

    pub(super) fn load_script(path: &str) -> Option<LuaScriptSource> {
        Self::load_script_text(path).map(|src| LuaScriptSource::inline(path, src))
    }
}
