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
//! Coroutines created with `coroutine_create` are driven by the engine: they start on
//! the next scheduler tick and are resumed automatically once their wait completes.
//!
//! ## Example Usage
//!
//! ```lua
//! function on_created(self_entity)
//!     coroutine_create(function()
//!         log_info("Starting coroutine")
//!         wait(2.0)  -- Wait 2 seconds
//!         log_info("After 2 seconds")
//!         wait_frames(60)  -- Wait 60 frames
//!         log_info("After 60 frames")
//!     end)
//! end
//! ```

use mlua::{Lua, MultiValue, Result as LuaResult, Value};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::scripting::lua::types::{
    CoroutineId, CoroutineMap, CoroutineState, CoroutineStatus, WaitState,
};

/// Tag yielded by `wait(seconds)`; the scheduler turns it into [`WaitState::Seconds`].
const WAIT_SECONDS_TAG: &str = "__engine_wait_seconds";
/// Tag yielded by `wait_frames(count)`; the scheduler turns it into [`WaitState::Frames`].
const WAIT_FRAMES_TAG: &str = "__engine_wait_frames";

/// `wait` and `wait_frames` must yield from Lua: a Rust callback cannot yield
/// across the C-call boundary. They hand the request to the scheduler as yield values.
const WAIT_HELPERS: &str = r#"
local yield, isyieldable = coroutine.yield, coroutine.isyieldable

function wait(seconds)
    if type(seconds) ~= "number" or seconds < 0 then
        error("wait duration must be a number >= 0", 2)
    end
    if not isyieldable() then
        error("wait() can only be called from within a coroutine", 2)
    end
    return yield("__engine_wait_seconds", seconds)
end

function wait_frames(frames)
    if math.type(frames) ~= "integer" or frames < 0 then
        error("wait_frames count must be an integer >= 0", 2)
    end
    if not isyieldable() then
        error("wait_frames() can only be called from within a coroutine", 2)
    end
    return yield("__engine_wait_frames", frames)
end
"#;

thread_local! {
    /// Active coroutine map for the currently executing script instance
    pub(crate) static ACTIVE_COROUTINES: RefCell<Option<Rc<RefCell<CoroutineMap>>>> = const { RefCell::new(None) };

    /// Counter for generating unique coroutine IDs
    static COROUTINE_ID_COUNTER: Cell<CoroutineId> = const { Cell::new(1) };
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

/// Returns the coroutine map of the script instance that is currently executing.
fn active_coroutines() -> LuaResult<Rc<RefCell<CoroutineMap>>> {
    ACTIVE_COROUTINES.with(|cell| {
        cell.borrow()
            .clone()
            .ok_or_else(|| mlua::Error::RuntimeError("coroutine context missing".into()))
    })
}

/// Generate a unique coroutine ID
fn next_coroutine_id() -> CoroutineId {
    COROUTINE_ID_COUNTER.with(|counter| {
        let id = counter.get();
        counter.set(id + 1);
        id
    })
}

/// Interpret the values a coroutine yielded as a scheduler wait request.
fn wait_state_from_yield(values: &MultiValue) -> WaitState {
    let mut iter = values.iter();
    let (Some(Value::String(tag)), Some(amount)) = (iter.next(), iter.next()) else {
        return WaitState::None;
    };
    let amount = match amount {
        Value::Integer(value) => *value as f64,
        Value::Number(value) => *value,
        _ => return WaitState::None,
    };

    match tag.to_str().as_deref() {
        Ok(WAIT_SECONDS_TAG) => WaitState::Seconds(amount),
        Ok(WAIT_FRAMES_TAG) => WaitState::Frames(amount as u32),
        _ => WaitState::None,
    }
}

/// Resume coroutine `id` from `coroutines`.
///
/// The map is not borrowed while the coroutine runs, so the coroutine body is free to
/// create, query or resume other coroutines.
pub(crate) fn resume_coroutine(
    coroutines: &RefCell<CoroutineMap>,
    id: CoroutineId,
    args: MultiValue,
) -> LuaResult<MultiValue> {
    let thread = {
        let mut map = coroutines.borrow_mut();
        let state = map
            .get_mut(&id)
            .ok_or_else(|| mlua::Error::RuntimeError(format!("coroutine {id} not found")))?;
        match state.status {
            CoroutineStatus::Dead => {
                return Err(mlua::Error::RuntimeError(
                    "cannot resume dead coroutine".into(),
                ))
            }
            CoroutineStatus::Running => {
                return Err(mlua::Error::RuntimeError(
                    "cannot resume running coroutine".into(),
                ))
            }
            CoroutineStatus::Suspended => {}
        }
        state.status = CoroutineStatus::Running;
        state.reset_wait();
        state.thread.clone()
    };

    let result = thread.resume::<MultiValue>(args);

    if let Some(state) = coroutines.borrow_mut().get_mut(&id) {
        match &result {
            Ok(values) if thread.status() == mlua::ThreadStatus::Resumable => {
                state.status = CoroutineStatus::Suspended;
                state.wait_state = wait_state_from_yield(values);
            }
            _ => state.status = CoroutineStatus::Dead,
        }
    }

    result
}

/// Register all coroutine API functions with the Lua runtime
pub(crate) fn register_coroutine_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    // coroutine_create(func) -> coroutine_id
    globals.set(
        "coroutine_create",
        lua.create_function(|lua, func: mlua::Function| {
            let thread = lua.create_thread(func)?;
            let id = next_coroutine_id();
            active_coroutines()?
                .borrow_mut()
                .insert(id, CoroutineState::new(thread));
            Ok(id)
        })?,
    )?;

    // coroutine_resume(id, ...) -> success, ...
    globals.set(
        "coroutine_resume",
        lua.create_function(|_lua, (id, args): (CoroutineId, MultiValue)| {
            let coroutines = active_coroutines()?;
            let values = resume_coroutine(&coroutines, id, args)?;
            Ok((true, values))
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
            let coroutines = active_coroutines()?;
            let map = coroutines.borrow();
            // Finished coroutines are pruned by the scheduler, so a missing id is dead.
            Ok(map
                .get(&id)
                .map_or(CoroutineStatus::Dead, |state| state.status)
                .as_str())
        })?,
    )?;

    lua.load(WAIT_HELPERS)
        .set_name("=engine/coroutine")
        .exec()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_outside_coroutine_is_an_error() {
        let lua = Lua::new();
        register_coroutine_api(&lua).unwrap();
        let err = lua.load("wait(1.0)").exec().unwrap_err();
        assert!(err.to_string().contains("within a coroutine"), "{err}");
    }

    #[test]
    fn resume_reports_wait_request() {
        let lua = Lua::new();
        register_coroutine_api(&lua).unwrap();
        let func: mlua::Function = lua
            .load("return function() wait(0.5) wait_frames(3) end")
            .eval()
            .unwrap();

        let map = RefCell::new(CoroutineMap::new());
        map.borrow_mut()
            .insert(1, CoroutineState::new(lua.create_thread(func).unwrap()));

        resume_coroutine(&map, 1, MultiValue::new()).unwrap();
        assert!(matches!(map.borrow()[&1].wait_state, WaitState::Seconds(s) if s == 0.5));

        resume_coroutine(&map, 1, MultiValue::new()).unwrap();
        assert!(matches!(map.borrow()[&1].wait_state, WaitState::Frames(3)));

        resume_coroutine(&map, 1, MultiValue::new()).unwrap();
        assert_eq!(map.borrow()[&1].status, CoroutineStatus::Dead);
    }
}
