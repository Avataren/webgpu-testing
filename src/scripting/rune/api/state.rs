use rune::runtime::{try_result, VmResult};
use rune::{FromValue, Value};

use crate::scripting::rune::guards::with_active_state;

#[rune::function]
pub(crate) fn set_state(handle: i64, key: String, value: Value) -> VmResult<()> {
    with_active_state(move |map| {
        map.insert((handle, key), value);
        VmResult::Ok(())
    })
}

#[rune::function]
pub(crate) fn get_state(handle: i64, key: String, default: Value) -> VmResult<Value> {
    with_active_state(move |map| {
        let entry_key = (handle, key);
        match map.get(&entry_key) {
            Some(value) => VmResult::Ok(value.clone()),
            None => VmResult::Ok(default),
        }
    })
}

#[rune::function]
pub(crate) fn try_get_state(handle: i64, key: String) -> VmResult<Option<Value>> {
    with_active_state(move |map| {
        let entry_key = (handle, key);
        let value = map.get(&entry_key).cloned();
        VmResult::Ok(value)
    })
}

#[rune::function]
pub(crate) fn get_f64(handle: i64, key: String, default: f64) -> VmResult<f64> {
    with_active_state(move |map| {
        let entry_key = (handle, key);
        match map.get(&entry_key) {
            Some(value) => try_result(f64::from_value(value.clone())),
            None => VmResult::Ok(default),
        }
    })
}

#[rune::function]
pub(crate) fn set_f64(handle: i64, key: String, value: f64) -> VmResult<()> {
    with_active_state(move |map| {
        map.insert((handle, key), Value::from(value));
        VmResult::Ok(())
    })
}
