//! Injected keyboard and mouse input.

use glam::Vec2;
use serde_json::{json, Value};

use super::super::hub::InputEvent;
use super::super::json::{opt_str, opt_u64};
use super::{RemoteContext, RemoteResult};

/// Frames a tapped key or button stays down unless the event says otherwise.
const DEFAULT_TAP_FRAMES: u64 = 3;

/// Normalize a key name to what the engine reports for real key presses (see
/// `key_to_string` in the winit platform layer): single characters are upper case and
/// named keys use winit's names.
pub(super) fn normalize_key(key: &str) -> String {
    let trimmed = key.trim();
    if trimmed.chars().count() == 1 {
        return trimmed.to_uppercase();
    }
    match trimmed.to_ascii_lowercase().as_str() {
        "space" | "spacebar" => "Space".into(),
        "enter" | "return" => "Return".into(),
        "esc" | "escape" => "Escape".into(),
        "up" | "arrowup" => "ArrowUp".into(),
        "down" | "arrowdown" => "ArrowDown".into(),
        "left" | "arrowleft" => "ArrowLeft".into(),
        "right" | "arrowright" => "ArrowRight".into(),
        "shift" => "Shift".into(),
        "ctrl" | "control" => "Control".into(),
        "alt" => "Alt".into(),
        "meta" | "super" | "cmd" => "Meta".into(),
        "tab" => "Tab".into(),
        "backspace" => "Backspace".into(),
        _ => trimmed.to_owned(),
    }
}

fn mouse_button(value: &Value) -> Result<u32, String> {
    match value {
        Value::Number(number) => number
            .as_u64()
            .map(|n| n as u32)
            .ok_or_else(|| format!("invalid mouse button {number}")),
        Value::String(name) => match name.to_ascii_lowercase().as_str() {
            "left" => Ok(0),
            "right" => Ok(1),
            "middle" => Ok(2),
            "back" => Ok(3),
            "forward" => Ok(4),
            other => Err(format!("unknown mouse button {other:?}")),
        },
        _ => Err(format!("invalid mouse button {value}")),
    }
}

fn vec2(value: &Value) -> Result<Vec2, String> {
    match value {
        Value::Array(items) if items.len() == 2 => {
            let x = items[0].as_f64().ok_or("expected numbers")?;
            let y = items[1].as_f64().ok_or("expected numbers")?;
            Ok(Vec2::new(x as f32, y as f32))
        }
        _ => Err(format!("expected [x, y], got {value}")),
    }
}

/// Turn one event description into scheduled `(delay, event)` pairs.
fn parse_event(event: &Value) -> Result<Vec<(u64, InputEvent)>, String> {
    let delay = opt_u64(event, "delay").unwrap_or(0);
    let action = opt_str(event, "action").unwrap_or("tap");
    let hold = opt_u64(event, "frames")
        .unwrap_or(DEFAULT_TAP_FRAMES)
        .max(1);

    let press_release =
        |down: InputEvent, up: InputEvent| -> Result<Vec<(u64, InputEvent)>, String> {
            match action {
                "tap" | "press" => Ok(vec![(delay, down), (delay + hold, up)]),
                "down" | "hold" => Ok(vec![(delay, down)]),
                "up" | "release" => Ok(vec![(delay, up)]),
                other => Err(format!("unknown action {other:?}; use tap, down or up")),
            }
        };

    if let Some(key) = opt_str(event, "key") {
        let key = normalize_key(key);
        return press_release(InputEvent::KeyDown(key.clone()), InputEvent::KeyUp(key));
    }
    if let Some(button) = event.get("mouse_button") {
        let button = mouse_button(button)?;
        return press_release(InputEvent::MouseDown(button), InputEvent::MouseUp(button));
    }
    if let Some(position) = event.get("mouse_move") {
        return Ok(vec![(delay, InputEvent::MouseMove(vec2(position)?))]);
    }
    if let Some(delta) = event.get("scroll") {
        return Ok(vec![(delay, InputEvent::Scroll(vec2(delta)?))]);
    }
    Err(format!(
        "event needs one of key, mouse_button, mouse_move or scroll: {event}"
    ))
}

pub(super) fn send(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let events: Vec<&Value> = match params.get("events") {
        Some(Value::Array(events)) => events.iter().collect(),
        Some(other) => return Err(format!("events must be an array, got {other}")),
        None => vec![params],
    };
    if events.is_empty() {
        return Err("no events given".into());
    }

    let mut scheduled = Vec::new();
    for event in events {
        scheduled.extend(parse_event(event)?);
    }
    let last_frame = scheduled.iter().map(|(delay, _)| *delay).max().unwrap_or(0);
    let count = scheduled.len();
    for (delay, event) in scheduled {
        ctx.hub.schedule_input(delay, event);
    }
    Ok(json!({
        "scheduled": count,
        "starts_at_frame": ctx.hub.frame() + 1,
        "completes_after_frames": last_frame + 1,
        "note": "Input is applied at the start of upcoming frames; use wait to let it play out.",
    }))
}

pub(super) fn state(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let input = ctx.gpu.scene.input_state();
    let mut keys_down: Vec<&str> = input.keys_down().collect();
    keys_down.sort_unstable();
    let mut buttons: Vec<u32> = input.mouse_buttons_down().collect();
    buttons.sort_unstable();
    let position = input.mouse_position();

    let mut out = json!({
        "keys_down": keys_down,
        "mouse_buttons_down": buttons,
        "mouse_position": [position.x, position.y],
        "queued_events": ctx.hub.pending_input(),
    });
    if let Some(Value::Array(keys)) = params.get("keys") {
        let states: serde_json::Map<String, Value> = keys
            .iter()
            .filter_map(Value::as_str)
            .map(|key| {
                let key = normalize_key(key);
                let state = json!({
                    "down": input.is_key_pressed(&key),
                    "just_pressed": input.is_key_just_pressed(&key),
                    "just_released": input.is_key_just_released(&key),
                });
                (key, state)
            })
            .collect();
        out["keys"] = Value::Object(states);
    }
    Ok(out)
}

pub(super) fn clear(ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    ctx.hub.clear_input();
    ctx.gpu.scene.input_state_mut().reset();
    Ok(json!({ "cleared": true }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names_match_window_events() {
        assert_eq!(normalize_key("w"), "W");
        assert_eq!(normalize_key("space"), "Space");
        assert_eq!(normalize_key("enter"), "Return");
        assert_eq!(normalize_key("up"), "ArrowUp");
        assert_eq!(normalize_key("F1"), "F1");
    }

    #[test]
    fn tap_schedules_press_and_release() {
        let events = parse_event(&json!({ "key": "d", "frames": 5, "delay": 2 })).unwrap();
        assert_eq!(
            events,
            vec![
                (2, InputEvent::KeyDown("D".into())),
                (7, InputEvent::KeyUp("D".into()))
            ]
        );
        assert!(parse_event(&json!({ "nothing": true })).is_err());
    }
}
