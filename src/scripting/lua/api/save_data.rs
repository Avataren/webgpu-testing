//! # Save data API
//!
//! Small text values a game keeps between runs, such as a high score or settings:
//!
//! - `save_data(key, text)` stores `text` under `key` (`nil` deletes it)
//! - `load_data(key)` returns the stored text, or `nil`
//!
//! Values belong to the open project and the user, not to the project's files, so a
//! game's saves never end up in its repository. Native builds write
//! `<user data dir>/wgpu-cube/saves/<project>/<key>.txt` (`$XDG_DATA_HOME` or
//! `~/.local/share` on Linux, `~/Library/Application Support` on macOS, `%APPDATA%` on
//! Windows); web builds use the browser's `localStorage`. Keys are 1-64 characters of
//! letters, digits, `_`, `-` and `.`; store structured data as JSON or a Lua literal.

use std::sync::{Mutex, PoisonError};

use mlua::{Lua, Result as LuaResult};

/// Largest value `save_data` stores, so a runaway script cannot fill the disk.
const MAX_VALUE_BYTES: usize = 1 << 20;

/// Which project's saves scripts see; set by the host when a project opens.
static NAMESPACE: Mutex<Option<String>> = Mutex::new(None);

/// Keep saves of the project called `name` (usually its directory name) apart from
/// other projects'.
pub fn set_save_namespace(name: &str) {
    let name: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    *NAMESPACE.lock().unwrap_or_else(PoisonError::into_inner) = (!name.is_empty()).then_some(name);
}

/// The project's namespace (`default` before one is set): engine files kept per user
/// and project, such as graphics settings, use it too.
pub(crate) fn namespace() -> String {
    NAMESPACE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .unwrap_or_else(|| "default".to_owned())
}

fn check_key(key: &str) -> LuaResult<()> {
    let valid = (1..=64).contains(&key.len())
        && !key.starts_with('.')
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if valid {
        Ok(())
    } else {
        Err(mlua::Error::RuntimeError(format!(
            "invalid save key {key:?}: use 1-64 letters, digits, _, - or ."
        )))
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod store {
    use std::path::PathBuf;

    fn data_dir() -> Option<PathBuf> {
        if cfg!(test) {
            return Some(
                std::env::temp_dir().join(format!("save-data-test-{}", std::process::id())),
            );
        }
        let env = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
        if cfg!(windows) {
            return env("APPDATA").map(PathBuf::from);
        }
        if cfg!(target_os = "macos") {
            return env("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"));
        }
        env("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| env("HOME").map(|home| PathBuf::from(home).join(".local/share")))
    }

    fn path(area: &str, namespace: &str, key: &str) -> Result<PathBuf, String> {
        let dir = data_dir().ok_or("no user data directory (HOME is not set)")?;
        Ok(dir
            .join("wgpu-cube")
            .join(area)
            .join(namespace)
            .join(format!("{key}.txt")))
    }

    pub fn save(area: &str, namespace: &str, key: &str, value: Option<&str>) -> Result<(), String> {
        let path = path(area, namespace, key)?;
        match value {
            Some(value) => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|err| format!("cannot create {}: {err}", parent.display()))?;
                }
                // Write then rename, so a crash never leaves half a save.
                let partial = path.with_extension("tmp");
                std::fs::write(&partial, value)
                    .and_then(|()| std::fs::rename(&partial, &path))
                    .map_err(|err| format!("cannot write {}: {err}", path.display()))
            }
            None => match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(err) => Err(format!("cannot delete {}: {err}", path.display())),
            },
        }
    }

    pub fn load(area: &str, namespace: &str, key: &str) -> Result<Option<String>, String> {
        let path = path(area, namespace, key)?;
        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(Some(text)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(format!("cannot read {}: {err}", path.display())),
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod store {
    fn storage() -> Result<web_sys::Storage, String> {
        web_sys::window()
            .and_then(|window| window.local_storage().ok().flatten())
            .ok_or_else(|| "localStorage is not available".to_owned())
    }

    // Save data keeps its original item names.
    fn item(area: &str, namespace: &str, key: &str) -> String {
        if area == super::SAVES {
            format!("wgpu-cube/{namespace}/{key}")
        } else {
            format!("wgpu-cube/{area}/{namespace}/{key}")
        }
    }

    pub fn save(area: &str, namespace: &str, key: &str, value: Option<&str>) -> Result<(), String> {
        let storage = storage()?;
        let item = item(area, namespace, key);
        match value {
            Some(value) => storage.set_item(&item, value),
            None => storage.remove_item(&item),
        }
        .map_err(|err| format!("localStorage refused {item}: {err:?}"))
    }

    pub fn load(area: &str, namespace: &str, key: &str) -> Result<Option<String>, String> {
        storage()?
            .get_item(&item(area, namespace, key))
            .map_err(|err| format!("localStorage read failed: {err:?}"))
    }
}

/// Where scripts' save data lives (`<user data dir>/wgpu-cube/saves`).
const SAVES: &str = "saves";

/// A per-user, per-project file the engine keeps outside the save data scripts see,
/// in `area` (`<user data dir>/wgpu-cube/<area>/<project>/<key>.txt`, or `localStorage`
/// on the web). `None` deletes it.
pub(crate) fn save_user_file(area: &str, key: &str, value: Option<&str>) -> Result<(), String> {
    store::save(area, &namespace(), key, value)
}

/// Read a file written by [`save_user_file`]; `None` when there is none.
pub(crate) fn load_user_file(area: &str, key: &str) -> Result<Option<String>, String> {
    store::load(area, &namespace(), key)
}

pub(crate) fn register_save_data_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();
    globals.set(
        "save_data",
        lua.create_function(|_, (key, value): (String, Option<String>)| {
            check_key(&key)?;
            if value
                .as_ref()
                .is_some_and(|value| value.len() > MAX_VALUE_BYTES)
            {
                return Err(mlua::Error::RuntimeError(format!(
                    "save_data values are limited to {MAX_VALUE_BYTES} bytes"
                )));
            }
            store::save(SAVES, &namespace(), &key, value.as_deref())
                .map_err(mlua::Error::RuntimeError)?;
            Ok(true)
        })?,
    )?;
    globals.set(
        "load_data",
        lua.create_function(|_, key: String| {
            check_key(&key)?;
            store::load(SAVES, &namespace(), &key).map_err(mlua::Error::RuntimeError)
        })?,
    )?;
    Ok(())
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn keys_are_checked() {
        assert!(check_key("highscore").is_ok());
        assert!(check_key("level-2.best_time").is_ok());
        for bad in ["", "../x", "a/b", ".hidden", &"x".repeat(65)] {
            assert!(check_key(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn values_round_trip_and_delete() {
        let namespace = format!("test-{}", std::process::id());
        store::save(SAVES, &namespace, "score", Some("1234")).unwrap();
        assert_eq!(
            store::load(SAVES, &namespace, "score").unwrap().as_deref(),
            Some("1234")
        );
        store::save(SAVES, &namespace, "score", None).unwrap();
        assert_eq!(store::load(SAVES, &namespace, "score").unwrap(), None);
        store::save(SAVES, &namespace, "score", None).unwrap();
    }
}
