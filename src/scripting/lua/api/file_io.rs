//! # File I/O API
//!
//! This module provides sandboxed file system access for Lua scripts.
//!
//! ## Security & Sandboxing
//!
//! For security, scripts can only access files within allowed directories:
//! - `scripts/`
//! - `examples/scripts/`
//! - paths the user picked in a file dialog during this session
//! - paths the host granted with [`grant_path_access`] (e.g. the open project)
//!
//! Paths are resolved (including `..` and symlinks) before the check, so traversal
//! out of an allowed directory is blocked.
//!
//! ## Features
//!
//! - **Read/Write** - Access text files in allowed directories
//! - **File queries** - Check existence, list files
//! - **Path utilities** - Extract directory/filename from paths
//! - **File dialogs** - Native OS file pickers (not on WASM)
//!
//! ## Platform Support
//!
//! - **Native** - Full support for file operations and dialogs
//! - **WASM** - File I/O operations return errors (not supported)

use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use mlua::{Lua, Result as LuaResult};

/// Allowed directories for file operations.
///
/// Scripts can only read/write files within these directories relative
/// to the current working directory. This prevents unauthorized access
/// to system files.
const ALLOWED_DIRS: &[&str] = &["examples/scripts", "scripts"];

/// Paths (files or directories) the user or host explicitly granted access to.
/// Stored fully resolved.
static GRANTED_PATHS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Allow scripts to access `path`: a file, or a directory and everything below it.
///
/// File dialogs grant their result automatically. Hosts call this for locations
/// scripts legitimately need, such as the open project's directory.
pub fn grant_path_access(path: impl AsRef<Path>) {
    let Some(resolved) = resolve_path(path.as_ref()) else {
        return;
    };
    let mut granted = GRANTED_PATHS.lock().unwrap_or_else(PoisonError::into_inner);
    if !granted.contains(&resolved) {
        granted.push(resolved);
    }
}

/// Resolve `.` and `..` lexically. Returns `None` if `..` would climb above the start.
fn normalize_components(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            Component::Normal(part) => normalized.push(part),
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(normalized)
}

/// Resolve `path` to an absolute path with symlinks resolved, even if it does not
/// exist yet: the nearest existing ancestor is canonicalized and the remaining
/// components are appended. Relative paths are taken relative to the working directory.
fn resolve_path(path: &Path) -> Option<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };

    let mut existing = absolute.as_path();
    let mut missing = Vec::new();
    let canonical = loop {
        match existing.canonicalize() {
            Ok(canonical) => break canonical,
            Err(_) => {
                missing.push(existing.file_name()?);
                existing = existing.parent()?;
            }
        }
    };

    let rest: PathBuf = missing.into_iter().rev().collect();
    let rest = normalize_components(&rest)?;
    Some(canonical.join(rest))
}

/// Check if a path is within an allowed directory or an explicitly granted path.
fn is_path_allowed(path: &Path) -> bool {
    let Some(target) = resolve_path(path) else {
        log::warn!("Rejected unresolvable script path: {:?}", path);
        return false;
    };

    let in_allowed_dir = ALLOWED_DIRS
        .iter()
        .filter_map(|dir| resolve_path(Path::new(dir)))
        .any(|root| target.starts_with(root));
    let granted = GRANTED_PATHS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .any(|root| target.starts_with(root));

    if !(in_allowed_dir || granted) {
        log::warn!("Script file access denied: {}", target.display());
    }
    in_allowed_dir || granted
}

/// Registers file I/O API functions with the Lua runtime.
///
/// This function exposes sandboxed file system access to Lua scripts.
///
/// ## Available Functions
///
/// ### File Operations
/// - `read_file(path)` - Read text file, returns contents or error string
/// - `write_file(path, contents)` - Write text file, creates directories if needed
/// - `file_exists(path)` - Returns true if file exists in allowed directory
/// - `list_files(dir_path)` - Returns 1-indexed table of filenames
///
/// ### Path Utilities
/// - `get_file_directory(path)` - Extract directory portion of path
/// - `get_filename(path)` - Extract filename portion of path
///
/// ### File Dialogs (Native only)
/// - `open_file_dialog(extensions, start_dir)` - Pick file to open
/// - `save_file_dialog(extensions, default_name, start_dir)` - Pick save location
/// - `open_folder_dialog(start_dir)` - Pick folder
///
/// # Example Lua usage
///
/// ```lua
/// -- Read a script file
/// local code = read_file("scripts/my_script.lua")
/// if not code:match("^ERROR:") then
///     log_info("Loaded " .. #code .. " bytes")
/// end
///
/// -- Write configuration
/// write_file("scripts/config.txt", "speed=10\njump_height=5")
///
/// -- List all scripts
/// local files = list_files("scripts")
/// for i = 1, #files do
///     log_info("Found: " .. files[i])
/// end
///
/// -- File dialog (editor scripts)
/// local path = open_file_dialog({"lua", "txt"}, "scripts")
/// if path then
///     log_info("User selected: " .. path)
/// end
/// ```
///
/// # Arguments
///
/// * `lua` - The Lua runtime to register functions with
///
/// # Returns
///
/// `Ok(())` on success, or a Lua error if registration fails.
pub(crate) fn register_file_io_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    // read_file(path: string) -> string
    globals.set(
        "read_file",
        lua.create_function(|_, path: String| {
            let path_buf = PathBuf::from(&path);

            if !is_path_allowed(&path_buf) {
                log::error!(
                    "Script attempted to read file outside allowed directories: {}",
                    path
                );
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
                return lua.create_table();
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
                        lua.create_table()
                    }
                }
            }

            #[cfg(target_arch = "wasm32")]
            {
                lua.create_table()
            }
        })?,
    )?;

    // open_file_dialog(extensions: table, start_dir: string) -> string | nil
    // Opens a native file picker dialog and returns the selected file path
    // extensions should be a table of extension strings like {"lua", "wgsl"}
    // start_dir is optional - the directory to start in
    globals.set(
        "open_file_dialog",
        lua.create_function(
            |_, (extensions, start_dir): (Option<mlua::Table>, Option<String>)| {
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
                        for (_, ext) in exts.pairs::<mlua::Value, String>().flatten() {
                            ext_vec.push(ext);
                        }
                        if !ext_vec.is_empty() {
                            dialog = dialog.add_filter("Files", &ext_vec);
                        }
                    }

                    match dialog.pick_file() {
                        Some(path) => {
                            grant_path_access(&path);
                            Ok(Some(path.to_string_lossy().to_string()))
                        }
                        None => Ok(None),
                    }
                }

                #[cfg(target_arch = "wasm32")]
                {
                    log::error!("File dialogs not supported on WASM");
                    Ok(None::<String>)
                }
            },
        )?,
    )?;

    // save_file_dialog(extensions: table, default_name: string, start_dir: string) -> string | nil
    // Opens a native save file dialog and returns the selected file path
    // extensions should be a table of extension strings like {"lua", "wgsl"}
    // default_name is the suggested filename
    // start_dir is optional - the directory to start in
    globals.set(
        "save_file_dialog",
        lua.create_function(
            |_,
             (extensions, default_name, start_dir): (
                Option<mlua::Table>,
                Option<String>,
                Option<String>,
            )| {
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
                        for (_, ext) in exts.pairs::<mlua::Value, String>().flatten() {
                            ext_vec.push(ext);
                        }
                        if !ext_vec.is_empty() {
                            dialog = dialog.add_filter("Files", &ext_vec);
                        }
                    }

                    match dialog.save_file() {
                        Some(path) => {
                            grant_path_access(&path);
                            Ok(Some(path.to_string_lossy().to_string()))
                        }
                        None => Ok(None),
                    }
                }

                #[cfg(target_arch = "wasm32")]
                {
                    log::error!("File dialogs not supported on WASM");
                    Ok(None::<String>)
                }
            },
        )?,
    )?;

    // open_folder_dialog(start_dir: string) -> string | nil
    // Opens a native folder picker dialog and returns the selected folder path
    // start_dir is optional - the directory to start in
    globals.set(
        "open_folder_dialog",
        lua.create_function(|_, start_dir: Option<String>| {
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

                match dialog.pick_folder() {
                    Some(path) => {
                        grant_path_access(&path);
                        Ok(Some(path.to_string_lossy().to_string()))
                    }
                    None => Ok(None),
                }
            }

            #[cfg(target_arch = "wasm32")]
            {
                log::error!("Folder dialogs not supported on WASM");
                Ok(None::<String>)
            }
        })?,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{grant_path_access, is_path_allowed};

    #[test]
    fn absolute_paths_outside_sandbox_are_denied() {
        let dir = tempfile::tempdir().unwrap();
        let secret = dir.path().join("secret.txt");
        std::fs::write(&secret, "x").unwrap();

        assert!(!is_path_allowed(&secret));
        assert!(!is_path_allowed(std::path::Path::new("/etc/passwd")));
    }

    #[test]
    fn granted_paths_are_allowed_but_siblings_are_not() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        grant_path_access(&project);

        assert!(is_path_allowed(&project.join("data/new_file.json")));
        assert!(!is_path_allowed(&dir.path().join("other.txt")));
        assert!(!is_path_allowed(&project.join("../other.txt")));
    }

    #[test]
    fn relative_traversal_out_of_allowed_dirs_is_denied() {
        assert!(is_path_allowed(std::path::Path::new("scripts/notes.txt")));
        assert!(!is_path_allowed(std::path::Path::new(
            "scripts/../Cargo.toml"
        )));
        assert!(!is_path_allowed(std::path::Path::new("../outside.txt")));
    }
}
