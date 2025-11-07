use std::path::{Path, PathBuf};

use mlua::{Lua, Result as LuaResult};

/// Allowed directories for file operations
const ALLOWED_DIRS: &[&str] = &["examples/scripts", "scripts"];

/// Check if a path is within allowed directories
fn is_path_allowed(path: &Path) -> bool {
    // Canonicalize the path to resolve any .. or . components
    let canonical = match path.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            // If canonicalization fails, try to construct it manually
            // This handles the case where the file doesn't exist yet
            let Some(base) = std::env::current_dir().ok() else {
                return false;
            };
            let full_path = base.join(path);
            full_path
        }
    };

    // Check if the canonical path starts with any allowed directory
    let Some(base) = std::env::current_dir().ok() else {
        return false;
    };

    for allowed in ALLOWED_DIRS {
        let allowed_path = base.join(allowed);

        // Try to canonicalize the allowed path
        if let Ok(allowed_canonical) = allowed_path.canonicalize() {
            if canonical.starts_with(&allowed_canonical) {
                return true;
            }
        }

        // Also check if the path would be within the allowed directory
        // (for files that don't exist yet)
        if canonical.starts_with(&allowed_path) {
            return true;
        }
    }

    false
}

/// Register file I/O API functions with the Lua runtime.
pub(crate) fn register_file_io_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    // read_file(path: string) -> string
    globals.set(
        "read_file",
        lua.create_function(|_, path: String| {
            let path_buf = PathBuf::from(&path);

            if !is_path_allowed(&path_buf) {
                log::error!("Script attempted to read file outside allowed directories: {}", path);
                return Ok(format!("ERROR: Access denied to '{}'", path));
            }

            #[cfg(not(target_arch = "wasm32"))]
            {
                match std::fs::read_to_string(&path_buf) {
                    Ok(contents) => Ok(contents),
                    Err(err) => {
                        log::error!("Failed to read file '{}': {}", path, err);
                        Ok(format!("ERROR: Failed to read file: {}", err))
                    }
                }
            }

            #[cfg(target_arch = "wasm32")]
            {
                log::error!("File I/O not supported on WASM");
                Ok("ERROR: File I/O not supported on WASM".to_string())
            }
        })?,
    )?;

    // write_file(path: string, contents: string) -> string
    globals.set(
        "write_file",
        lua.create_function(|_, (path, contents): (String, String)| {
            let path_buf = PathBuf::from(&path);

            if !is_path_allowed(&path_buf) {
                log::error!(
                    "Script attempted to write file outside allowed directories: {}",
                    path
                );
                return Ok(format!("ERROR: Access denied to '{}'", path));
            }

            #[cfg(not(target_arch = "wasm32"))]
            {
                // Create parent directories if they don't exist
                if let Some(parent) = path_buf.parent() {
                    if let Err(err) = std::fs::create_dir_all(parent) {
                        log::error!("Failed to create directories for '{}': {}", path, err);
                        return Ok(format!("ERROR: Failed to create directories: {}", err));
                    }
                }

                match std::fs::write(&path_buf, contents) {
                    Ok(()) => Ok(String::new()), // Empty string means success
                    Err(err) => {
                        log::error!("Failed to write file '{}': {}", path, err);
                        Ok(format!("ERROR: Failed to write file: {}", err))
                    }
                }
            }

            #[cfg(target_arch = "wasm32")]
            {
                log::error!("File I/O not supported on WASM");
                Ok("ERROR: File I/O not supported on WASM".to_string())
            }
        })?,
    )?;

    // file_exists(path: string) -> boolean
    globals.set(
        "file_exists",
        lua.create_function(|_, path: String| {
            let path_buf = PathBuf::from(&path);

            if !is_path_allowed(&path_buf) {
                return Ok(false);
            }

            Ok(path_buf.exists() && path_buf.is_file())
        })?,
    )?;

    // list_files(dir_path: string) -> table
    globals.set(
        "list_files",
        lua.create_function(|lua, dir_path: String| {
            let path_buf = PathBuf::from(&dir_path);

            if !is_path_allowed(&path_buf) {
                log::error!(
                    "Script attempted to list directory outside allowed directories: {}",
                    dir_path
                );
                return Ok(lua.create_table()?);
            }

            #[cfg(not(target_arch = "wasm32"))]
            {
                match std::fs::read_dir(&path_buf) {
                    Ok(entries) => {
                        let mut files = Vec::new();
                        for entry in entries.flatten() {
                            if let Ok(metadata) = entry.metadata() {
                                if metadata.is_file() {
                                    if let Some(name) = entry.file_name().to_str() {
                                        files.push(name.to_string());
                                    }
                                }
                            }
                        }
                        files.sort();

                        let table = lua.create_table()?;
                        for (i, file) in files.iter().enumerate() {
                            table.raw_set(i + 1, file.as_str())?;
                        }
                        Ok(table)
                    }
                    Err(err) => {
                        log::error!("Failed to list directory '{}': {}", dir_path, err);
                        Ok(lua.create_table()?)
                    }
                }
            }

            #[cfg(target_arch = "wasm32")]
            {
                Ok(lua.create_table()?)
            }
        })?,
    )?;

    Ok(())
}
