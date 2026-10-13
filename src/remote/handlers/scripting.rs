//! Lua console, script diagnostics and runtime mode.

use serde_json::{json, Value};

use crate::app::core::MAX_TIME_SCALE;
use crate::app::RuntimeMode;
use crate::scripting::LuaScriptComponent;

use super::super::json::{
    entity_from_json, entity_id, entity_name, opt_bool, opt_f64, opt_str, req_str,
};
use super::entity::resolve_existing_path;
use super::info::runtime_mode_name;
use super::{RemoteContext, RemoteResult};

pub(super) fn eval(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let code = req_str(params, "code")?;
    let entity = match params.get("entity") {
        None | Some(Value::Null) => None,
        Some(value) => Some(entity_from_json(ctx.gpu.scene.main_world(), value)?),
    };
    let output = ctx
        .gpu
        .scene
        .eval_lua(code, entity)
        .map_err(|err| err.to_string())?;
    ctx.gpu
        .scene
        .apply_script_asset_commands(&mut *ctx.gpu.renderer);
    if !opt_bool(params, "read_only").unwrap_or(false) {
        ctx.gpu.scene.propagate_transforms();
        ctx.mark_modified();
    }
    let result = match output.values.as_slice() {
        [] => Value::Null,
        [single] => single.clone(),
        many => Value::Array(many.to_vec()),
    };
    Ok(json!({ "result": result, "printed": output.printed }))
}

pub(super) fn status(ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let scene = &ctx.gpu.scene;
    let world = scene.main_world();
    let scripting = scene.lua_scripting();
    let loaded = scripting.script_entities();

    let mut scripts: Vec<Value> = world
        .query::<&LuaScriptComponent>()
        .iter()
        .map(|(entity, component)| {
            let instance = loaded.iter().find(|(loaded, ..)| *loaded == entity);
            json!({
                "entity": entity_id(entity),
                "name": entity_name(world, entity),
                "source": component.source().display_name(),
                "loaded": instance.is_some(),
                "compiled": instance.is_some_and(|(_, _, compiled)| *compiled),
                "last_error": scripting.last_error(entity),
            })
        })
        .collect();
    scripts.sort_by_key(|script| script["entity"].as_u64());
    let failing = scripts
        .iter()
        .filter(|script| !script["last_error"].is_null())
        .count();
    Ok(json!({
        "count": scripts.len(),
        "failing": failing,
        "note": "Scripts compile on the next update after they are attached; game scripts (no annotation) only run on_created/update in play mode.",
        "scripts": scripts,
    }))
}

pub(super) fn reload(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    if let Some(value) = params.get("entity") {
        let entity = entity_from_json(ctx.gpu.scene.main_world(), value)?;
        let reloaded = ctx.gpu.scene.reload_entity_script(entity);
        if !reloaded {
            return Err("that entity has no loaded script".into());
        }
        return Ok(json!({ "reloaded": 1 }));
    }
    if let Some(path) = opt_str(params, "path") {
        let path = resolve_existing_path(path)?;
        let count = ctx.gpu.scene.reload_script_path(&path);
        return Ok(json!({ "reloaded": count, "path": path.display().to_string() }));
    }
    ctx.gpu.scene.reset_script_runtime();
    Ok(json!({ "reloaded": "all" }))
}

pub(super) fn runtime_get(ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let handle = ctx
        .runtime_handle()
        .ok_or("runtime mode is not available in this application")?;
    Ok(json!({
        "mode": runtime_mode_name(handle.active_mode()),
        "requested": runtime_mode_name(handle.desired_mode()),
        "time_scale": handle.time_scale(),
    }))
}

pub(super) fn runtime_set(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let handle = ctx
        .runtime_handle()
        .ok_or("runtime mode is not available in this application")?;
    let mode = match opt_str(params, "mode") {
        None => None,
        Some("play" | "playing") => Some(RuntimeMode::Playing),
        Some("editor" | "edit" | "stop") => Some(RuntimeMode::Editor),
        Some(other) => return Err(format!("unknown mode {other:?}; use play or stop")),
    };
    let time_scale = opt_f64(params, "time_scale");
    if mode.is_none() && time_scale.is_none() {
        return Err("give mode (play or stop) and/or time_scale".into());
    }
    if let Some(scale) = time_scale {
        if !(0.0..=MAX_TIME_SCALE).contains(&scale) {
            return Err(format!("time_scale must be between 0 and {MAX_TIME_SCALE}"));
        }
        handle.set_time_scale(scale);
    }
    let previous = handle.active_mode();
    let mut result = json!({
        "mode": runtime_mode_name(previous),
        "time_scale": handle.time_scale(),
    });
    if let Some(mode) = mode {
        handle.request_mode(mode);
        result["previous"] = result["mode"].take();
        result["requested"] = json!(runtime_mode_name(mode));
        result["note"] = json!("The switch happens at the start of the next frame. Stopping restores the scene as it was before play and resets time_scale to 1.");
    }
    Ok(result)
}
