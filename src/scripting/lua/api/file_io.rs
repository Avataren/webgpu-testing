use std::path::{Path, PathBuf, Component};

use mlua::{Lua, Result as LuaResult};

/// Allowed directories for file operations
const ALLOWED_DIRS: &[&str] = &["examples/scripts", "scripts"];

/// Normalize a path by resolving . and .. components without requiring the file to exist.
/// This prevents path traversal attacks.
fn normalize_path(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {
                // Skip "." components
            }
            Component::ParentDir => {
                // For "..", pop the last component if possible
                // If we can't pop (we're at root), the path is trying to escape - reject it
                if !normalized.pop() {
                    return None;
                }
            }
            Component::Normal(part) => {
                normalized.push(part);
            }
            Component::RootDir => {
                // Absolute paths are not allowed in our sandboxed environment
                return None;
            }
            Component::Prefix(_) => {
                // Windows prefixes (C:\, \\server\, etc.) are not allowed
                return None;
            }
        }
    }

    Some(normalized)
}

/// Check if a path is within allowed directories
fn is_path_allowed(path: &Path) -> bool {
    // If the path is absolute, allow it (user selected it via file dialog)
    if path.is_absolute() {
        return true;
    }

    // First, get the current directory
    let Some(base) = std::env::current_dir().ok() else {
        return false;
    };

    // Normalize the input path to resolve . and .. without requiring file existence
    let Some(normalized_relative) = normalize_path(path) else {
        log::warn!("Rejected path with invalid traversal: {:?}", path);
        return false;
    };

    // Construct the full path
    let full_path = base.join(&normalized_relative);

    // Now try to canonicalize (this will work if the file exists)
    let canonical = full_path.canonicalize().unwrap_or(full_path);

    // Check if the canonical path starts with any allowed directory
    for allowed in ALLOWED_DIRS {
        let allowed_path = base.join(allowed);

        // Try to canonicalize the allowed path
        let allowed_canonical = allowed_path.canonicalize().unwrap_or(allowed_path);

        if canonical.starts_with(&allowed_canonical) {
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

    // get_file_directory(path: string) -> string
    // Returns the directory portion of a file path
    globals.set(
        "get_file_directory",
        lua.create_function(|_, path: String| {
            let path_buf = PathBuf::from(&path);
            if let Some(parent) = path_buf.parent() {
                Ok(parent.to_string_lossy().to_string())
            } else {
                Ok(String::new())
            }
        })?,
    )?;

    // get_filename(path: string) -> string
    // Returns just the filename portion of a file path
    globals.set(
        "get_filename",
        lua.create_function(|_, path: String| {
            let path_buf = PathBuf::from(&path);
            if let Some(filename) = path_buf.file_name() {
                Ok(filename.to_string_lossy().to_string())
            } else {
                Ok(String::new())
            }
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

    // open_file_dialog(extensions: table, start_dir: string) -> string | nil
    // Opens a native file picker dialog and returns the selected file path
    // extensions should be a table of extension strings like {"lua", "wgsl"}
    // start_dir is optional - the directory to start in
    globals.set(
        "open_file_dialog",
        lua.create_function(|_, (extensions, start_dir): (Option<mlua::Table>, Option<String>)| {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let mut dialog = rfd::FileDialog::new();

                // Set starting directory if provided
                if let Some(dir) = start_dir {
                    let path = PathBuf::from(&dir);
                    if path.exists() && path.is_dir() {
                        dialog = dialog.set_directory(&path);
                    }
                }

                // Add file filters if provided
                if let Some(exts) = extensions {
                    let mut ext_vec = Vec::new();
                    for pair in exts.pairs::<mlua::Value, String>() {
                        if let Ok((_, ext)) = pair {
                            ext_vec.push(ext);
                        }
                    }
                    if !ext_vec.is_empty() {
                        dialog = dialog.add_filter("Files", &ext_vec);
                    }
                }

                match dialog.pick_file() {
                    Some(path) => Ok(Some(path.to_string_lossy().to_string())),
                    None => Ok(None),
                }
            }

            #[cfg(target_arch = "wasm32")]
            {
                log::error!("File dialogs not supported on WASM");
                Ok(None)
            }
        })?,
    )?;

    // save_file_dialog(extensions: table, default_name: string, start_dir: string) -> string | nil
    // Opens a native save file dialog and returns the selected file path
    // extensions should be a table of extension strings like {"lua", "wgsl"}
    // default_name is the suggested filename
    // start_dir is optional - the directory to start in
    globals.set(
        "save_file_dialog",
        lua.create_function(|_, (extensions, default_name, start_dir): (Option<mlua::Table>, Option<String>, Option<String>)| {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let mut dialog = rfd::FileDialog::new();

                // Set starting directory if provided
                if let Some(dir) = start_dir {
                    let path = PathBuf::from(&dir);
                    if path.exists() && path.is_dir() {
                        dialog = dialog.set_directory(&path);
                    }
                }

                // Set default filename if provided
                if let Some(name) = default_name {
                    dialog = dialog.set_file_name(name);
                }

                // Add file filters if provided
                if let Some(exts) = extensions {
                    let mut ext_vec = Vec::new();
                    for pair in exts.pairs::<mlua::Value, String>() {
                        if let Ok((_, ext)) = pair {
                            ext_vec.push(ext);
                        }
                    }
                    if !ext_vec.is_empty() {
                        dialog = dialog.add_filter("Files", &ext_vec);
                    }
                }

                match dialog.save_file() {
                    Some(path) => Ok(Some(path.to_string_lossy().to_string())),
                    None => Ok(None),
                }
            }

            #[cfg(target_arch = "wasm32")]
            {
                log::error!("File dialogs not supported on WASM");
                Ok(None)
            }
        })?,
    )?;

    Ok(())
}
