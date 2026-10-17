//! `graphics.get` / `graphics.set`: the player's graphics settings (see
//! [`crate::graphics_settings`]) and the settings page.

use serde_json::{json, Value};

use super::RemoteResult;
use crate::graphics_settings::{graphics_settings, GraphicsSettings};

pub(super) fn get() -> RemoteResult {
    let graphics = graphics_settings();
    Ok(json!({
        "settings": serde_json::to_value(&graphics.settings).map_err(|err| err.to_string())?,
        "page_open": graphics.open,
        "escape_opens_page": graphics.hotkey_enabled,
        "applies_to_window": graphics.manage_window,
        "exit_button": graphics.exit_button,
        "capabilities": {
            "ray_tracing": graphics.capabilities.ray_tracing,
            "path_tracing": graphics.capabilities.path_tracing,
            "global_illumination": graphics.capabilities.path_tracing,
            "upscaling": graphics.capabilities.upscaling,
            "vsync_optional": graphics.capabilities.vsync_optional,
        },
        "note": format!(
            "The player's settings, saved per game and user; they replace the scene volumes' {}.",
            GraphicsSettings::CONTROLLED_PATHS.join(", ")
        ),
    }))
}

/// Change the given settings (`reset: true` first restores the defaults; `save: false`
/// keeps the change to this run), and open or close the page (`open`).
pub(super) fn set(params: &Value) -> RemoteResult {
    let Some(object) = params.as_object() else {
        return Err("graphics.set expects an object of settings".into());
    };
    let mut graphics = graphics_settings();
    let base = if object.get("reset").and_then(Value::as_bool) == Some(true) {
        GraphicsSettings::default()
    } else {
        graphics.settings.clone()
    };
    let mut patch = object.clone();
    patch.remove("reset");
    patch.remove("open");
    patch.remove("save");
    let settings = base
        .patched(&patch)
        .map_err(|err| format!("{err} (see graphics.get)"))?;
    if object.get("save").and_then(Value::as_bool) == Some(false) {
        graphics.set_unsaved(settings);
    } else {
        graphics.set(settings);
    }
    if let Some(open) = object.get("open").and_then(Value::as_bool) {
        graphics.open = open;
    }
    drop(graphics);
    get()
}
