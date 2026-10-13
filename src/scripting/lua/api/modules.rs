//! # Modules: `require` and `include`
//!
//! Lets a game be split into several files in the project's scripts folder.
//!
//! - `require(name)` loads `name` as a module and returns what it returns (usually a
//!   table). The module runs once per script instance in its own environment (it sees
//!   the scripting API, and its globals stay its own); later calls return the cached
//!   value.
//! - `include(name)` runs the file inside the calling script's environment, as if its
//!   text were pasted there, so the functions and globals it defines become the
//!   script's. It runs on every call.
//!
//! `name` is a path without `.lua` (`"enemies"`, `"ai.autopilot"` or `"ai/autopilot"`),
//! looked up next to the script first, then under the script root. Editing a required
//! or included file hot-reloads every script that uses it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use mlua::{Lua, Result as LuaResult, Table, Value};

/// Files a script instance loaded through `require` or `include`.
pub(crate) type Dependencies = Arc<Mutex<Vec<PathBuf>>>;

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Map a module name to the candidate files, in search order.
fn candidates(name: &str, search_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let relative = name.strip_suffix(".lua").unwrap_or(name);
    // Dots separate folders unless the name already uses slashes.
    let relative = if relative.contains('/') {
        relative.to_owned()
    } else {
        relative.replace('.', "/")
    };
    search_dirs
        .iter()
        .map(|dir| dir.join(format!("{relative}.lua")))
        .collect()
}

fn find(name: &str, search_dirs: &[PathBuf]) -> LuaResult<(PathBuf, String)> {
    let candidates = candidates(name, search_dirs);
    for path in &candidates {
        if let Ok(contents) = crate::io::read_to_string(path) {
            return Ok((path.clone(), contents));
        }
    }
    Err(mlua::Error::RuntimeError(format!(
        "module {name:?} not found; looked for {}",
        candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

fn compile(lua: &Lua, path: &Path, contents: &str, env: Table) -> LuaResult<mlua::Function> {
    lua.load(contents)
        .set_name(format!("@{}", path.display()))
        .set_environment(env)
        .into_function()
}

/// Define `require` and `include` in a script's environment.
pub(crate) fn install(
    lua: &Lua,
    env: &Table,
    search_dirs: Vec<PathBuf>,
    dependencies: Dependencies,
) -> LuaResult<()> {
    let search_dirs = Arc::new(search_dirs);
    let loaded: Arc<Mutex<HashMap<PathBuf, mlua::RegistryKey>>> = Arc::default();
    // Modules being loaded, to report a cycle instead of overflowing the stack.
    let loading: Arc<Mutex<Vec<PathBuf>>> = Arc::default();

    let require = {
        let (search_dirs, dependencies) = (search_dirs.clone(), dependencies.clone());
        lua.create_function(move |lua, name: String| -> LuaResult<Value> {
            let (path, contents) = find(&name, &search_dirs)?;
            if let Some(key) = lock(&loaded).get(&path) {
                return lua.registry_value(key);
            }
            if lock(&loading).contains(&path) {
                return Err(mlua::Error::RuntimeError(format!(
                    "require cycle: {name:?} is still loading"
                )));
            }
            {
                let mut dependencies = lock(&dependencies);
                if !dependencies.contains(&path) {
                    dependencies.push(path.clone());
                }
            }
            let module_env = lua.create_table()?;
            let metatable = lua.create_table()?;
            metatable.set("__index", lua.globals())?;
            module_env.set_metatable(Some(metatable));
            let function = compile(lua, &path, &contents, module_env)?;
            lock(&loading).push(path.clone());
            let result = function.call::<Value>(name.clone());
            lock(&loading).retain(|loading| loading != &path);
            // A module that returns nothing is cached as `true`, as in standard Lua.
            let value = match result? {
                Value::Nil => Value::Boolean(true),
                value => value,
            };
            let key = lua.create_registry_value(value.clone())?;
            lock(&loaded).insert(path, key);
            Ok(value)
        })?
    };
    env.raw_set("require", require)?;

    let env_key = Arc::new(lua.create_registry_value(env.clone())?);
    let include = lua.create_function(move |lua, name: String| -> LuaResult<mlua::MultiValue> {
        let (path, contents) = find(&name, &search_dirs)?;
        {
            let mut dependencies = lock(&dependencies);
            if !dependencies.contains(&path) {
                dependencies.push(path.clone());
            }
        }
        let env: Table = lua.registry_value(&env_key)?;
        compile(lua, &path, &contents, env)?.call(())
    })?;
    env.raw_set("include", include)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_names_map_to_files() {
        let dirs = vec![PathBuf::from("/game/scripts"), PathBuf::from("/root")];
        assert_eq!(
            candidates("ai.autopilot", &dirs),
            vec![
                PathBuf::from("/game/scripts/ai/autopilot.lua"),
                PathBuf::from("/root/ai/autopilot.lua")
            ]
        );
        assert_eq!(
            candidates("ai/auto.pilot.lua", &dirs)[0],
            PathBuf::from("/game/scripts/ai/auto.pilot.lua")
        );
    }

    #[test]
    fn require_caches_and_include_shares_the_environment() {
        let dir = std::env::temp_dir().join(format!("lua-modules-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(
            dir.join("lib/counter.lua"),
            "local M = { loads = (LOADS or 0) + 1 }\nLOADS = M.loads\nreturn M",
        )
        .unwrap();
        std::fs::write(
            dir.join("helpers.lua"),
            "function double(x) return x * 2 end",
        )
        .unwrap();

        let lua = Lua::new();
        let env = lua.create_table().unwrap();
        let metatable = lua.create_table().unwrap();
        metatable.set("__index", lua.globals()).unwrap();
        env.set_metatable(Some(metatable));
        let dependencies: Dependencies = Arc::default();
        install(&lua, &env, vec![dir.clone()], dependencies.clone()).unwrap();

        let (first, second, doubled, leaked): (i64, bool, i64, Value) = lua
            .load(
                "local a = require('lib.counter')\n\
                 local b = require('lib/counter')\n\
                 include('helpers')\n\
                 return a.loads, a == b, double(21), LOADS",
            )
            .set_environment(env.clone())
            .eval()
            .unwrap();
        assert_eq!(first, 1);
        assert!(second, "the second require returns the cached module");
        assert_eq!(doubled, 42, "include defines functions in the script");
        assert!(leaked.is_nil(), "module globals stay in the module");
        assert_eq!(lock(&dependencies).len(), 2);

        let missing = lua
            .load("require('nope')")
            .set_environment(env)
            .exec()
            .unwrap_err()
            .to_string();
        assert!(missing.contains("module \"nope\" not found"), "{missing}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
