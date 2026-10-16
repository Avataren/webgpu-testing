//! # Modules: `require` and `include`
//!
//! Lets a game be split into several files in the project's scripts folder.
//!
//! - `require(name)` loads `name` as a module and returns what it returns (usually a
//!   table). The module runs once per script instance in its own environment (it sees
//!   the scripting API, and its globals stay its own); later calls return the cached
//!   value. Modules can `require` other modules too, sharing the script's cache, so a
//!   module two others require is one instance (a natural place for shared game state).
//! - `include(name)` runs the file inside the calling script's environment, as if its
//!   text were pasted there, so the functions and globals it defines become the
//!   script's. It runs on every call.
//!
//! `name` is a path without `.lua` (`"enemies"`, `"ai.autopilot"` or `"ai/autopilot"`),
//! looked up next to the script first, then under the script root (also from inside a
//! module, so a module in `ai/` requires its neighbour as `"ai.helpers"`). Editing a required
//! or included file hot-reloads every script that uses it.
//!
//! The engine brings modules of its own under `engine.`, used when the project has no file
//! of that name (so copying one into `scripts/engine/` overrides it):
//!
//! - `engine.third_person`: keyboard and mouse controls for a character with a
//!   `CharacterController` and a spring camera (see "Third-person controller").

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

/// Modules built into the engine, by their path under `engine/`.
const BUILT_IN: [(&str, &str); 1] = [("third_person", include_str!("../modules/third_person.lua"))];

/// A built-in module: `engine.third_person` or `engine/third_person`.
fn built_in(name: &str) -> Option<(PathBuf, String)> {
    let relative = name.strip_suffix(".lua").unwrap_or(name).replace('.', "/");
    let module = relative.strip_prefix("engine/")?;
    BUILT_IN
        .iter()
        .find(|(name, _)| *name == module)
        .map(|(name, source)| {
            (
                PathBuf::from(format!("engine/{name}.lua")),
                (*source).to_owned(),
            )
        })
}

fn find(name: &str, search_dirs: &[PathBuf]) -> LuaResult<(PathBuf, String)> {
    let candidates = candidates(name, search_dirs);
    for path in &candidates {
        if let Ok(contents) = crate::io::read_to_string(path) {
            return Ok((path.clone(), contents));
        }
    }
    if let Some(found) = built_in(name) {
        return Ok(found);
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
    // The same modules by the name they were required as, so a `require` inside a function
    // that runs every frame does not touch the disk.
    let by_name: Arc<Mutex<HashMap<String, mlua::RegistryKey>>> = Arc::default();
    // Modules being loaded, to report a cycle instead of overflowing the stack.
    let loading: Arc<Mutex<Vec<PathBuf>>> = Arc::default();
    // What every module's environment falls back to: the API globals plus this
    // script's `require`.
    let module_base = lua.create_table()?;
    let base_metatable = lua.create_table()?;
    base_metatable.set("__index", lua.globals())?;
    module_base.set_metatable(Some(base_metatable));
    let base_key = Arc::new(lua.create_registry_value(module_base.clone())?);

    let require = {
        let (search_dirs, dependencies) = (search_dirs.clone(), dependencies.clone());
        lua.create_function(move |lua, name: String| -> LuaResult<Value> {
            if let Some(key) = lock(&by_name).get(&name) {
                return lua.registry_value(key);
            }
            let (path, contents) = find(&name, &search_dirs)?;
            if let Some(key) = lock(&loaded).get(&path) {
                let value: Value = lua.registry_value(key)?;
                lock(&by_name).insert(name, lua.create_registry_value(value.clone())?);
                return Ok(value);
            }
            if lock(&loading).contains(&path) {
                return Err(mlua::Error::RuntimeError(format!(
                    "require cycle: {name:?} is still loading (move what both modules need into a \
                     third, or require it inside a function instead of at the top)"
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
            metatable.set("__index", lua.registry_value::<Table>(&base_key)?)?;
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
            lock(&loaded).insert(path, lua.create_registry_value(value.clone())?);
            lock(&by_name).insert(name, lua.create_registry_value(value.clone())?);
            Ok(value)
        })?
    };
    module_base.raw_set("require", require.clone())?;
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
    fn modules_require_modules_from_the_shared_cache() {
        let dir = std::env::temp_dir().join(format!("lua-nested-modules-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("game")).unwrap();
        std::fs::write(dir.join("game/state.lua"), "return { n = 0 }").unwrap();
        std::fs::write(
            dir.join("game/a.lua"),
            "local S = require('game.state')\nreturn { bump = function() S.n = S.n + 1 end }",
        )
        .unwrap();
        std::fs::write(
            dir.join("game/b.lua"),
            "local S = require('game.state')\nreturn { read = function() return S.n end }",
        )
        .unwrap();
        std::fs::write(dir.join("game/loop1.lua"), "return require('game.loop2')").unwrap();
        std::fs::write(dir.join("game/loop2.lua"), "return require('game.loop1')").unwrap();

        let lua = Lua::new();
        let env = lua.create_table().unwrap();
        let metatable = lua.create_table().unwrap();
        metatable.set("__index", lua.globals()).unwrap();
        env.set_metatable(Some(metatable));
        let dependencies: Dependencies = Arc::default();
        install(&lua, &env, vec![dir.clone()], dependencies.clone()).unwrap();

        let (n, same): (i64, bool) = lua
            .load(
                "local a, b = require('game.a'), require('game.b')\n\
                 a.bump() a.bump()\n\
                 return b.read(), require('game.state') == require('game/state')",
            )
            .set_environment(env.clone())
            .eval()
            .unwrap();
        assert_eq!(n, 2, "both modules share one state module");
        assert!(same);
        assert_eq!(
            lock(&dependencies).len(),
            3,
            "nested modules are dependencies too"
        );

        let cycle = lua
            .load("require('game.loop1')")
            .set_environment(env)
            .exec()
            .unwrap_err()
            .to_string();
        assert!(cycle.contains("require cycle"), "{cycle}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn engine_modules_are_built_in() {
        let lua = Lua::new();
        let env = lua.create_table().unwrap();
        let metatable = lua.create_table().unwrap();
        metatable.set("__index", lua.globals()).unwrap();
        env.set_metatable(Some(metatable));
        install(&lua, &env, vec![std::env::temp_dir()], Arc::default()).unwrap();
        let (has_new, same): (bool, bool) = lua
            .load(
                "local T = require('engine.third_person')\n\
                 return type(T.new) == 'function', T == require('engine/third_person')",
            )
            .set_environment(env)
            .eval()
            .unwrap();
        assert!(has_new && same);
        assert!(built_in("engine.nothing").is_none());
        assert!(built_in("third_person").is_none(), "only under engine.");
    }

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
