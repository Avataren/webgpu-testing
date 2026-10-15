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

/// Change the given settings (`reset: true` first restores the defaults), and open or
/// close the page (`open`).
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
    let mut merged = serde_json::to_value(&base).map_err(|err| err.to_string())?;
    let mut unknown = Vec::new();
    for (key, value) in object {
        if matches!(key.as_str(), "reset" | "open") {
            continue;
        }
        match merged.get_mut(key) {
            Some(slot) => *slot = value.clone(),
            None => unknown.push(key.clone()),
        }
    }
    if !unknown.is_empty() {
        return Err(format!(
            "unknown graphics settings: {} (see graphics.get)",
            unknown.join(", ")
        ));
    }
    let settings: GraphicsSettings = serde_json::from_value(merged)
        .map_err(|err| format!("invalid graphics settings: {err}"))?;
    graphics.set(settings);
    if let Some(open) = object.get("open").and_then(Value::as_bool) {
        graphics.open = open;
    }
    drop(graphics);
    get()
}
