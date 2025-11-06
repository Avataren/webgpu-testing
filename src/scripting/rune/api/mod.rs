use rune::Module;

use super::error::RuneScriptingError;

mod component;
mod entity;
mod event;
mod hierarchy;
mod input;
mod logging;
mod query;
mod state;
mod transform;
pub mod ui;

pub(crate) use component::{add_component, get_component, has_component, remove_component, set_component};
pub(crate) use entity::{attach_inline_script, attach_script_file, find_entity_by_name, import_gltf, set_name, set_rotation, set_translation, spawn_entity};
pub(crate) use event::{emit_event, subscribe_event, unsubscribe_event};
pub(crate) use hierarchy::{get_children, get_parent, set_parent};
pub(crate) use input::{
    get_mouse_delta, get_mouse_position, get_mouse_scroll_delta, is_key_just_pressed,
    is_key_just_released, is_key_pressed, is_mouse_button_just_pressed,
    is_mouse_button_just_released, is_mouse_button_pressed,
};
pub(crate) use logging::{log_debug, log_error, log_info, log_warn};
pub(crate) use query::{
    get_entities_in_box, get_entities_in_radius, get_nearest_entity,
    get_nearest_entity_with_component, query_entities_with_component,
};
pub(crate) use state::{get_f64, get_state, set_f64, set_state, try_get_state};
pub(crate) use transform::{
    get_world_rotation, get_world_translation, look_at, rotate, set_scale, translate,
};

pub(crate) fn script_module() -> Result<Module, RuneScriptingError> {
    let mut module = Module::new();
    module.function_meta(spawn_entity)?;
    module.function_meta(set_name)?;
    module.function_meta(set_translation)?;
    module.function_meta(set_rotation)?;
    module.function_meta(import_gltf)?;
    module.function_meta(set_state)?;
    module.function_meta(get_state)?;
    module.function_meta(try_get_state)?;
    module.function_meta(get_f64)?;
    module.function_meta(set_f64)?;
    module.function_meta(attach_inline_script)?;
    module.function_meta(attach_script_file)?;
    module.function_meta(log_debug)?;
    module.function_meta(log_info)?;
    module.function_meta(log_warn)?;
    module.function_meta(log_error)?;
    // Component access functions
    module.function_meta(get_component)?;
    module.function_meta(set_component)?;
    module.function_meta(add_component)?;
    module.function_meta(remove_component)?;
    module.function_meta(has_component)?;
    module.function_meta(find_entity_by_name)?;
    // Transform manipulation functions
    module.function_meta(translate)?;
    module.function_meta(rotate)?;
    module.function_meta(set_scale)?;
    module.function_meta(look_at)?;
    module.function_meta(get_world_translation)?;
    module.function_meta(get_world_rotation)?;
    // Hierarchy functions
    module.function_meta(set_parent)?;
    module.function_meta(get_parent)?;
    module.function_meta(get_children)?;
    // Input functions
    module.function_meta(is_key_pressed)?;
    module.function_meta(is_key_just_pressed)?;
    module.function_meta(is_key_just_released)?;
    module.function_meta(is_mouse_button_pressed)?;
    module.function_meta(is_mouse_button_just_pressed)?;
    module.function_meta(is_mouse_button_just_released)?;
    module.function_meta(get_mouse_position)?;
    module.function_meta(get_mouse_delta)?;
    module.function_meta(get_mouse_scroll_delta)?;
    // Entity query functions
    module.function_meta(query_entities_with_component)?;
    // Spatial query functions
    module.function_meta(get_entities_in_radius)?;
    module.function_meta(get_nearest_entity)?;
    module.function_meta(get_nearest_entity_with_component)?;
    module.function_meta(get_entities_in_box)?;
    // Event system functions
    module.function_meta(emit_event)?;
    module.function_meta(subscribe_event)?;
    module.function_meta(unsubscribe_event)?;

    // UI types and functions
    module.ty::<ui::UiContext>()?;
    module.function_meta(ui::UiContext::label)?;
    module.function_meta(ui::UiContext::button)?;
    module.function_meta(ui::UiContext::heading)?;
    module.function_meta(ui::UiContext::separator)?;
    module.function_meta(ui::UiContext::text_edit)?;
    module.function_meta(ui::UiContext::slider)?;
    module.function_meta(ui::UiContext::drag_value)?;
    module.function_meta(ui::UiContext::checkbox)?;
    module.function_meta(ui::UiContext::color_edit)?;

    Ok(module)
}
