//! Lua console, script diagnostics and runtime mode.

use serde_json::{json, Value};

use crate::app::RuntimeMode;
use crate::scripting::LuaScriptComponent;

use super::super::json::{entity_from_json, entity_id, entity_name, opt_bool, opt_str, req_str};
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
    }))
}

pub(super) fn runtime_set(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let handle = ctx
        .runtime_handle()
        .ok_or("runtime mode is not available in this application")?;
    let mode = match req_str(params, "mode")? {
        "play" | "playing" => RuntimeMode::Playing,
        "editor" | "edit" | "stop" => RuntimeMode::Editor,
        other => return Err(format!("unknown mode {other:?}; use play or stop")),
    };
    let previous = handle.active_mode();
    handle.request_mode(mode);
    Ok(json!({
        "previous": runtime_mode_name(previous),
        "requested": runtime_mode_name(mode),
        "note": "The switch happens at the start of the next frame. Stopping restores the scene as it was before play.",
    }))
}
