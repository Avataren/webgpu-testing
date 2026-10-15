//! # Process API
//!
//! Talk to a program over its stdin and stdout, a line at a time, without blocking the
//! frame: a UCI chess engine, a language server, a simulation. Native builds only; the
//! program must be inside the working directory or the open project. Processes are
//! killed when play mode stops.

use mlua::{Lua, Result as LuaResult, Value};

use crate::process;

/// Registers the process functions:
///
/// - `process_spawn(path[, args])` → `id`, or `nil, message` when it cannot start
/// - `process_write(id, line)` → `boolean` - send a line (a newline is added)
/// - `process_read_lines(id)` → `{line, ...}` - what it printed since the last call
/// - `process_running(id)` → `boolean`
/// - `process_kill(id)` → `boolean`
pub(crate) fn register_process_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    globals.set(
        "process_spawn",
        lua.create_function(
            |lua, (path, args): (String, Option<Vec<String>>)| match process::spawn(
                &path,
                &args.unwrap_or_default(),
            ) {
                Ok(id) => Ok((Value::Number(id as f64), Value::Nil)),
                Err(message) => Ok((Value::Nil, Value::String(lua.create_string(message)?))),
            },
        )?,
    )?;

    globals.set(
        "process_write",
        lua.create_function(|_, (id, line): (u64, String)| Ok(process::write_line(id, &line)))?,
    )?;

    globals.set(
        "process_read_lines",
        lua.create_function(|_, id: u64| Ok(process::read_lines(id)))?,
    )?;

    globals.set(
        "process_running",
        lua.create_function(|_, id: u64| Ok(process::running(id)))?,
    )?;

    globals.set(
        "process_kill",
        lua.create_function(|_, id: u64| Ok(process::kill(id)))?,
    )?;

    Ok(())
}
