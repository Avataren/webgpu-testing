use rune::runtime::{try_result, VmResult};
use rune::{FromValue, Value};

use crate::scripting::rune::guards::{with_active_entity, with_active_state};

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
pub(crate) fn get_state(key: String) -> VmResult<Value> {
    with_active_entity(move |handle| {
        with_active_state(move |map| {
            let entry_key = (handle, key);
            match map.get(&entry_key) {
                Some(value) => VmResult::Ok(value.clone()),
                None => VmResult::panic("State key not found"),
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
    with_active_entity(move |handle| {
        with_active_state(move |map| {
            let entry_key = (handle, key);
            match map.get(&entry_key) {
                Some(value) => try_result(f64::from_value(value.clone())),
                None => VmResult::panic("State key not found"),
            }
        })
    })
}

#[rune::function]
pub(crate) fn set_f64(key: String, value: f64) -> VmResult<()> {
    with_active_entity(move |handle| {
        with_active_state(move |map| {
            map.insert((handle, key), Value::from(value));
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
    with_active_state(move |map| {
        let entry_key = (handle, key);
        match map.get(&entry_key) {
            Some(value) => try_result(f64::from_value(value.clone())),
            None => VmResult::Ok(default),
        }
    })
}

#[rune::function]
pub(crate) fn set_f64_for(handle: i64, key: String, value: f64) -> VmResult<()> {
    with_active_state(move |map| {
        map.insert((handle, key), Value::from(value));
        VmResult::Ok(())
    })
}
