use log::{debug, error, info, warn};
use mlua::{Lua, Result as LuaResult};

/// Register logging API functions with the Lua runtime.
pub(crate) fn register_logging_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    // log_debug(message: string)
    globals.set(
        "log_debug",
        lua.create_function(|_, message: String| {
            debug!(target: "script", "{}", message);
            Ok(())
        })?,
    )?;

    // log_info(message: string)
    globals.set(
        "log_info",
        lua.create_function(|_, message: String| {
            info!(target: "script", "{}", message);
            Ok(())
        })?,
    )?;

    // log_warn(message: string)
    globals.set(
        "log_warn",
        lua.create_function(|_, message: String| {
            warn!(target: "script", "{}", message);
            Ok(())
        })?,
    )?;

    // log_error(message: string)
    globals.set(
        "log_error",
        lua.create_function(|_, message: String| {
            error!(target: "script", "{}", message);
            Ok(())
        })?,
    )?;

    Ok(())
}
