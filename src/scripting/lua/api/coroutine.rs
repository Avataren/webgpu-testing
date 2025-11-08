//! # Lua Coroutine API
//!
//! This module provides coroutine functionality for Lua scripts, allowing
//! asynchronous-style programming with yielding and resuming execution.
//!
//! ## Available Functions
//!
//! - `coroutine_create(func)` - Create a new coroutine from a function
//! - `coroutine_resume(id, ...)` - Resume a coroutine with optional arguments
//! - `coroutine_yield(...)` - Yield from the current coroutine
//! - `coroutine_status(id)` - Get the status of a coroutine ("running", "suspended", "dead")
//! - `wait(seconds)` - Yield for a specified time duration
//! - `wait_frames(count)` - Yield for a specified number of frames
//!
//! ## Example Usage
//!
//! ```lua
//! function on_created(self_entity)
//!     local coro = coroutine_create(function()
//!         log_info("Starting coroutine")
//!         wait(2.0)  -- Wait 2 seconds
//!         log_info("After 2 seconds")
//!         wait_frames(60)  -- Wait 60 frames
//!         log_info("After 60 frames")
//!     end)
//!
//!     -- Coroutine will be automatically resumed by the scheduler
//! end
//! ```

use mlua::{Lua, Result as LuaResult};
use std::cell::RefCell;
use std::rc::Rc;

use crate::scripting::lua::types::{CoroutineId, CoroutineMap, CoroutineState, CoroutineStatus, WaitState};

thread_local! {
    /// Active coroutine map for the currently executing script instance
    pub(crate) static ACTIVE_COROUTINES: RefCell<Option<Rc<RefCell<CoroutineMap>>>> = const { RefCell::new(None) };

    /// Counter for generating unique coroutine IDs
    static COROUTINE_ID_COUNTER: RefCell<CoroutineId> = const { RefCell::new(1) };

    /// Track the currently executing coroutine ID (for yield)
    static CURRENT_COROUTINE_ID: RefCell<Option<CoroutineId>> = const { RefCell::new(None) };
}

/// Guard that sets up the active coroutine map for a script instance
pub(crate) struct CoroutineGuard {
    _coroutines: Rc<RefCell<CoroutineMap>>,
}

impl CoroutineGuard {
    pub fn enter(coroutines: &Rc<RefCell<CoroutineMap>>) -> Self {
        let coroutines_clone = Rc::clone(coroutines);
        ACTIVE_COROUTINES.with(|cell| *cell.borrow_mut() = Some(Rc::clone(&coroutines_clone)));
        Self {
            _coroutines: coroutines_clone,
        }
    }
}

impl Drop for CoroutineGuard {
    fn drop(&mut self) {
        ACTIVE_COROUTINES.with(|cell| *cell.borrow_mut() = None);
    }
}

/// Helper to access the active coroutine map
fn with_active_coroutines<R>(
    f: impl FnOnce(&mut CoroutineMap) -> Result<R, mlua::Error>,
) -> Result<R, mlua::Error> {
    ACTIVE_COROUTINES.with(|cell| {
        let opt = cell.borrow();
        let Some(rc) = opt.as_ref() else {
            return Err(mlua::Error::RuntimeError("coroutine context missing".into()));
        };
        let mut borrow = rc.borrow_mut();
        f(&mut borrow)
    })
}

/// Generate a unique coroutine ID
fn next_coroutine_id() -> CoroutineId {
    COROUTINE_ID_COUNTER.with(|cell| {
        let mut id = cell.borrow_mut();
        let current = *id;
        *id += 1;
        current
    })
}

/// Set the currently executing coroutine ID
fn set_current_coroutine_id(id: Option<CoroutineId>) {
    CURRENT_COROUTINE_ID.with(|cell| {
        *cell.borrow_mut() = id;
    });
}

/// Get the currently executing coroutine ID
fn get_current_coroutine_id() -> Option<CoroutineId> {
    CURRENT_COROUTINE_ID.with(|cell| *cell.borrow())
}

/// Register all coroutine API functions with the Lua runtime
pub(crate) fn register_coroutine_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    // coroutine_create(func) -> coroutine_id
    globals.set(
        "coroutine_create",
        lua.create_function(|lua, func: mlua::Function| {
            // Create a new Lua thread (coroutine)
            let thread = lua.create_thread(func)?;

            // Generate a unique ID
            let id = next_coroutine_id();

            // Store in the active coroutine map
            with_active_coroutines(|map| {
                map.insert(id, CoroutineState::new(thread));
                Ok(id)
            })
        })?,
    )?;

    // coroutine_resume(id, ...) -> success, ...
    globals.set(
        "coroutine_resume",
        lua.create_function(|_lua, (id, args): (CoroutineId, mlua::MultiValue)| {
            with_active_coroutines(|map| {
                let coro_state = map.get_mut(&id).ok_or_else(|| {
                    mlua::Error::RuntimeError(format!("coroutine {} not found", id))
                })?;

                // Check if coroutine is dead
                if coro_state.status == CoroutineStatus::Dead {
                    return Err(mlua::Error::RuntimeError(
                        "cannot resume dead coroutine".into(),
                    ));
                }

                // Set current coroutine ID for yield
                set_current_coroutine_id(Some(id));

                // Mark as running
                coro_state.status = CoroutineStatus::Running;

                // Resume the coroutine
                let result = coro_state.thread.resume::<mlua::MultiValue>(args);

                // Get the coroutine state again (it may have been modified)
                let coro_state = map.get_mut(&id).ok_or_else(|| {
                    mlua::Error::RuntimeError(format!("coroutine {} disappeared during resume", id))
                })?;

                // Update status based on result
                match result {
                    Ok(values) => {
                        // Check if thread is finished or in error state
                        match coro_state.thread.status() {
                            mlua::ThreadStatus::Error => {
                                coro_state.status = CoroutineStatus::Dead;
                            }
                            mlua::ThreadStatus::Resumable => {
                                coro_state.status = CoroutineStatus::Suspended;
                            }
                            _ => {
                                coro_state.status = CoroutineStatus::Suspended;
                            }
                        }
                        set_current_coroutine_id(None);
                        Ok((true, values))
                    }
                    Err(e) => {
                        coro_state.status = CoroutineStatus::Dead;
                        set_current_coroutine_id(None);
                        Err(e)
                    }
                }
            })
        })?,
    )?;

    // coroutine_yield(...) -> ...
    // Use Lua's built-in coroutine.yield function
    let coroutine_table: mlua::Table = globals.get("coroutine")?;
    let yield_fn: mlua::Function = coroutine_table.get("yield")?;
    globals.set("coroutine_yield", yield_fn)?;

    // coroutine_status(id) -> "running" | "suspended" | "dead"
    globals.set(
        "coroutine_status",
        lua.create_function(|_lua, id: CoroutineId| {
            with_active_coroutines(|map| {
                let coro_state = map.get(&id).ok_or_else(|| {
                    mlua::Error::RuntimeError(format!("coroutine {} not found", id))
                })?;
                Ok(coro_state.status.as_str())
            })
        })?,
    )?;

    // wait(seconds) - Yield for a specified time duration
    globals.set(
        "wait",
        lua.create_function(|lua, seconds: f64| {
            if seconds < 0.0 {
                return Err(mlua::Error::RuntimeError(
                    "wait duration must be >= 0".into(),
                ));
            }

            // Get current coroutine ID
            let coro_id = get_current_coroutine_id().ok_or_else(|| {
                mlua::Error::RuntimeError("wait() can only be called from within a coroutine".into())
            })?;

            // Set wait state in the coroutine
            with_active_coroutines(|map| {
                let coro_state = map.get_mut(&coro_id).ok_or_else(|| {
                    mlua::Error::RuntimeError(format!("coroutine {} not found", coro_id))
                })?;
                coro_state.wait_state = WaitState::Seconds(seconds);
                coro_state.accumulated_time = 0.0;
                Ok(())
            })?;

            // Call Lua's coroutine.yield()
            let coroutine_table: mlua::Table = lua.globals().get("coroutine")?;
            let yield_fn: mlua::Function = coroutine_table.get("yield")?;
            yield_fn.call::<()>(())
        })?,
    )?;

    // wait_frames(count) - Yield for a specified number of frames
    globals.set(
        "wait_frames",
        lua.create_function(|lua, frames: u32| {
            // Get current coroutine ID
            let coro_id = get_current_coroutine_id().ok_or_else(|| {
                mlua::Error::RuntimeError("wait_frames() can only be called from within a coroutine".into())
            })?;

            // Set wait state in the coroutine
            with_active_coroutines(|map| {
                let coro_state = map.get_mut(&coro_id).ok_or_else(|| {
                    mlua::Error::RuntimeError(format!("coroutine {} not found", coro_id))
                })?;
                coro_state.wait_state = WaitState::Frames(frames);
                Ok(())
            })?;

            // Call Lua's coroutine.yield()
            let coroutine_table: mlua::Table = lua.globals().get("coroutine")?;
            let yield_fn: mlua::Function = coroutine_table.get("yield")?;
            yield_fn.call::<()>(())
        })?,
    )?;

    Ok(())
}
