use rune::alloc::String as RuneString;
use rune::runtime::{try_result, VmResult};
use rune::{FromValue, Value};

use crate::scripting::rune::guards::{with_active_entity, with_active_state};

// ============================================================================
// Internal helper functions to reduce code duplication
// ============================================================================

/// Generic helper to get a typed value from state with default
fn get_typed<T: FromValue>(key: String, default: T) -> VmResult<T> {
    with_active_entity(move |handle| {
        with_active_state(move |map| {
            match map.get(&(handle, key)) {
                Some(value) => try_result(T::from_value(value.clone())),
                None => VmResult::Ok(default),
            }
        })
    })
}

/// Generic helper to get a typed value from state for a specific entity
fn get_typed_for<T: FromValue>(handle: i64, key: String, default: T) -> VmResult<T> {
    with_active_state(move |map| {
        match map.get(&(handle, key)) {
            Some(value) => try_result(T::from_value(value.clone())),
            None => VmResult::Ok(default),
        }
    })
}

/// Generic helper to set a simple value type (non-string)
fn set_simple<T: Into<Value>>(key: String, value: T) -> VmResult<()> {
    with_active_entity(move |handle| {
        with_active_state(move |map| {
            map.insert((handle, key), value.into());
            VmResult::Ok(())
        })
    })
}

/// Generic helper to set a simple value for a specific entity
fn set_simple_for<T: Into<Value>>(handle: i64, key: String, value: T) -> VmResult<()> {
    with_active_state(move |map| {
        map.insert((handle, key), value.into());
        VmResult::Ok(())
    })
}

// ============================================================================
// Public API functions (exposed to Rune scripts)
// ============================================================================

// Two-parameter versions that use the active entity from EntityGuard
#[rune::function]
pub(crate) fn set_state(key: String, value: Value) -> VmResult<()> {
    with_active_entity(move |handle| {
        with_active_state(move |map| {
            map.insert((handle, key), value);
            VmResult::Ok(())
        })
    })
}

#[rune::function]
pub(crate) fn get_state(key: String, default: Value) -> VmResult<Value> {
    // The issue: returning a captured parameter value creates Rune snapshots
    // Solution: If key doesn't exist, SET it with default, then GET it back
    // This way we always return a value from the map, never the captured parameter
    with_active_entity(move |handle| {
        with_active_state(move |map| {
            let entry_key = (handle, key);
            match map.get(&entry_key) {
                Some(value) => VmResult::Ok(value.clone()),
                None => {
                    // Clone before insert - map.insert() takes ownership
                    // Trying to move the captured parameter directly causes "Cannot take" error
                    map.insert(entry_key.clone(), default.clone());
                    // Now retrieve it from the map
                    match map.get(&entry_key) {
                        Some(value) => VmResult::Ok(value.clone()),
                        None => VmResult::panic("State insertion failed"),
                    }
                }
            }
        })
    })
}

#[rune::function]
pub(crate) fn try_get_state(key: String) -> VmResult<Value> {
    with_active_entity(move |handle| {
        with_active_state(move |map| {
            let entry_key = (handle, key);
            match map.get(&entry_key) {
                Some(value) => VmResult::Ok(value.clone()),
                None => VmResult::Ok(Value::from(())),
            }
        })
    })
}

#[rune::function]
pub(crate) fn get_f64(key: String) -> VmResult<f64> {
    // Panics if not found (original behavior for backward compatibility)
    with_active_entity(move |handle| {
        with_active_state(move |map| {
            match map.get(&(handle, key)) {
                Some(value) => try_result(f64::from_value(value.clone())),
                None => VmResult::panic("State key not found"),
            }
        })
    })
}

#[rune::function]
pub(crate) fn set_f64(key: String, value: f64) -> VmResult<()> {
    set_simple(key, value)
}

#[rune::function]
pub(crate) fn get_bool(key: String, default: bool) -> VmResult<bool> {
    get_typed(key, default)
}

#[rune::function]
pub(crate) fn set_bool(key: String, value: bool) -> VmResult<()> {
    set_simple(key, value)
}

#[rune::function]
pub(crate) fn get_string(key: String, default: String) -> VmResult<String> {
    get_typed(key, default)
}

#[rune::function]
pub(crate) fn set_string(key: String, value: String) -> VmResult<()> {
    with_active_entity(move |handle| {
        with_active_state(move |map| {
            let rune_str = match RuneString::try_from(value.as_str()) {
                Ok(s) => s,
                Err(_) => return VmResult::panic("Failed to allocate string"),
            };
            let val = match Value::try_from(rune_str) {
                Ok(v) => v,
                Err(_) => return VmResult::panic("Failed to create value from string"),
            };
            map.insert((handle, key), val);
            VmResult::Ok(())
        })
    })
}

// Three-parameter versions for setting state on arbitrary entities
// Use these when you need to set/get state on entities other than self
#[rune::function]
pub(crate) fn set_state_for(handle: i64, key: String, value: Value) -> VmResult<()> {
    with_active_state(move |map| {
        map.insert((handle, key), value);
        VmResult::Ok(())
    })
}

#[rune::function]
pub(crate) fn get_state_for(handle: i64, key: String, default: Value) -> VmResult<Value> {
    with_active_state(move |map| {
        let entry_key = (handle, key);
        match map.get(&entry_key) {
            Some(value) => VmResult::Ok(value.clone()),
            None => VmResult::Ok(default),
        }
    })
}

#[rune::function]
pub(crate) fn try_get_state_for(handle: i64, key: String) -> VmResult<Value> {
    with_active_state(move |map| {
        let entry_key = (handle, key);
        match map.get(&entry_key) {
            Some(value) => VmResult::Ok(value.clone()),
            None => VmResult::Ok(Value::from(())),
        }
    })
}

#[rune::function]
pub(crate) fn get_f64_for(handle: i64, key: String, default: f64) -> VmResult<f64> {
    get_typed_for(handle, key, default)
}

#[rune::function]
pub(crate) fn set_f64_for(handle: i64, key: String, value: f64) -> VmResult<()> {
    set_simple_for(handle, key, value)
}
