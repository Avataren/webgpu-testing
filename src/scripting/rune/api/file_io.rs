/// File I/O API for RuneScript
///
/// Provides sandboxed file operations. Only allows access to specific directories
/// to prevent scripts from accessing sensitive system files.
use std::path::{Component, Path, PathBuf};

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

/// Read text from a file.
/// Only allows reading from allowed directories (examples/scripts, scripts).
/// Returns empty string on error.
#[rune::function]
pub fn read_file(path: String) -> String {
    let path_buf = PathBuf::from(&path);

    if !is_path_allowed(&path_buf) {
        log::error!(
            "Script attempted to read file outside allowed directories: {}",
            path
        );
        return format!("ERROR: Access denied to '{}'", path);
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        match std::fs::read_to_string(&path_buf) {
            Ok(contents) => contents,
            Err(err) => {
                log::error!("Failed to read file '{}': {}", path, err);
                format!("ERROR: Failed to read file: {}", err)
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    {
        log::error!("File I/O not supported on WASM");
        format!("ERROR: File I/O not supported on WASM")
    }
}

/// Write text to a file.
/// Only allows writing to allowed directories (examples/scripts, scripts).
/// Returns empty string on success, error message on failure.
#[rune::function]
pub fn write_file(path: String, contents: String) -> String {
    let path_buf = PathBuf::from(&path);

    if !is_path_allowed(&path_buf) {
        log::error!(
            "Script attempted to write file outside allowed directories: {}",
            path
        );
        return format!("ERROR: Access denied to '{}'", path);
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        // Create parent directories if they don't exist
        if let Some(parent) = path_buf.parent() {
            if let Err(err) = std::fs::create_dir_all(parent) {
                log::error!("Failed to create directories for '{}': {}", path, err);
                return format!("ERROR: Failed to create directories: {}", err);
            }
        }

        match std::fs::write(&path_buf, contents) {
            Ok(()) => String::new(), // Empty string means success
            Err(err) => {
                log::error!("Failed to write file '{}': {}", path, err);
                format!("ERROR: Failed to write file: {}", err)
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    {
        log::error!("File I/O not supported on WASM");
        format!("ERROR: File I/O not supported on WASM")
    }
}

/// Check if a file exists.
/// Only allows checking files in allowed directories.
#[rune::function]
pub fn file_exists(path: String) -> bool {
    let path_buf = PathBuf::from(&path);

    if !is_path_allowed(&path_buf) {
        return false;
    }

    path_buf.exists() && path_buf.is_file()
}

/// List files in a directory.
/// Only allows listing in allowed directories.
/// Returns a vector of file names (not full paths).
#[rune::function]
pub fn list_files(dir_path: String) -> Vec<String> {
    let path_buf = PathBuf::from(&dir_path);

    if !is_path_allowed(&path_buf) {
        log::error!(
            "Script attempted to list directory outside allowed directories: {}",
            dir_path
        );
        return vec![];
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
                files
            }
            Err(err) => {
                log::error!("Failed to list directory '{}': {}", dir_path, err);
                vec![]
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    {
        vec![]
    }
}
