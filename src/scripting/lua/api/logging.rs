//! # Logging API
//!
//! This module provides logging functions for Lua scripts to output messages
//! at different severity levels (debug, info, warn, error).
//!
//! All log messages are tagged with the "script" target for filtering.

use log::{debug, error, info, warn};
use mlua::{Lua, Result as LuaResult};

/// Registers logging API functions with the Lua runtime.
///
/// This function exposes four logging functions to Lua scripts:
/// - `log_debug(message)` - Debug level (verbose, detailed information)
/// - `log_info(message)` - Info level (general informational messages)
/// - `log_warn(message)` - Warning level (potential issues)
/// - `log_error(message)` - Error level (serious problems)
///
/// # Example Lua usage
///
/// ```lua
/// log_info("Script initialized")
/// log_debug("Current position: " .. tostring(pos.x) .. ", " .. tostring(pos.y))
/// log_warn("Entity not found, using default")
/// log_error("Failed to load resource")
/// ```
///
/// # Arguments
///
/// * `lua` - The Lua runtime to register functions with
///
/// # Returns
///
/// `Ok(())` on success, or a Lua error if registration fails.
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
