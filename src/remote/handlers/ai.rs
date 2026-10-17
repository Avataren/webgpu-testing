//! AI over remote control: what agents think (states, actions, blackboards, trees,
//! senses), and steering them (events, blackboard values, noises, restarts), defining
//! brains from JSON and drawing their senses. The component goes through `entity.get` /
//! `entity.update` (`ai_agent`).

use std::sync::Arc;

use serde_json::{json, Value};

use crate::ai::context::Recipients;
use crate::ai::{
    parse_brain, split_key, AiCommand, AiDebugDraw, AiEvent, BoardScope, Def, Value as AiValue,
};

use super::super::json::{opt_bool, opt_f64, opt_str, req_entity, req_str, vec3_from_json};
use super::{RemoteContext, RemoteResult};

/// `ai.status`: every agent's brain, state and action, counts by state, the brains.
pub(super) fn status(ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let scene = &ctx.gpu.scene;
    Ok(scene.ai().status_json(scene.main_world()))
}

/// `ai.inspect {entity}`: one agent in depth.
pub(super) fn inspect(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let scene = &ctx.gpu.scene;
    let world = scene.main_world();
    let entity = req_entity(world, params)?;
    scene.ai().inspect(world, entity).ok_or_else(|| {
        "not a running AI agent (it needs an ai_agent component, and play mode)".into()
    })
}

/// `ai.define {name, brain}`: define a brain from JSON (a tree's root node, or a
/// machine with `states`); agents running `name` start it again.
pub(super) fn define(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let name = req_str(params, "name")?;
    let def = params.get("brain").ok_or("missing parameter \"brain\"")?;
    let brain = parse_brain(name, &Def::from_json(def))?;
    let kind = brain.kind();
    ctx.gpu.scene.ai_mut().define(name, brain);
    Ok(json!({ "defined": name, "kind": kind }))
}

/// `ai.validate {brain}`: check a brain without defining it.
pub(super) fn validate(params: &Value) -> RemoteResult {
    let def = params.get("brain").ok_or("missing parameter \"brain\"")?;
    match parse_brain("brain", &Def::from_json(def)) {
        Ok(crate::ai::Brain::Tree(tree)) => Ok(json!({
            "ok": true,
            "kind": "tree",
            "nodes": tree.nodes.iter().map(|n| json!({"label": n.label, "type": n.kind.type_name(), "depth": n.depth})).collect::<Vec<_>>(),
        })),
        Ok(crate::ai::Brain::Machine(machine)) => Ok(json!({
            "ok": true,
            "kind": "machine",
            "initial": machine.states[machine.initial].name,
            "states": machine.states.iter().map(|s| s.name.clone()).collect::<Vec<_>>(),
        })),
        Err(error) => Ok(json!({ "ok": false, "error": error })),
    }
}

/// `ai.send {entity | to: "all", event, data, radius, center, tag}`.
pub(super) fn send(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let event_name = req_str(params, "event")?;
    let data = params
        .get("data")
        .map(AiValue::from_json)
        .unwrap_or_default();
    let event = AiEvent {
        name: Arc::from(event_name),
        data,
        from: None,
        position: None,
    };
    let scene = &ctx.gpu.scene;
    let shared = scene.ai().shared().clone();
    if params.get("entity").is_some() {
        let entity = req_entity(scene.main_world(), params)?;
        if !shared.send(entity, event) {
            return Err("not a running AI agent".into());
        }
        return Ok(json!({ "sent": event_name, "to": entity.to_bits().get() }));
    }
    let to = match opt_f64(params, "radius") {
        Some(radius) => Recipients::Near {
            center: params
                .get("center")
                .map(vec3_from_json)
                .transpose()?
                .unwrap_or_default(),
            radius: radius as f32,
            tag: opt_str(params, "tag").map(str::to_owned),
        },
        None => Recipients::All,
    };
    shared.queue(AiCommand::Send {
        to,
        event,
        group: opt_str(params, "group").map(str::to_owned),
    });
    Ok(json!({ "queued": event_name }))
}

/// `ai.set {entity, key, value}` (or `{group, key, value}`, `{key: "global.x", value}`).
pub(super) fn set(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let key = req_str(params, "key")?;
    let value = params
        .get("value")
        .map(AiValue::from_json)
        .unwrap_or_default();
    let scene = &ctx.gpu.scene;
    let shared = scene.ai().shared().clone();
    let (scope, name) = split_key(key);
    let board = match (scope, opt_str(params, "group")) {
        (BoardScope::Global, _) => shared.global.clone(),
        (_, Some(group)) => shared.group_board(group),
        (BoardScope::Group, None) => {
            let entity = req_entity(scene.main_world(), params)?;
            let group = shared
                .agents
                .borrow()
                .get(&entity)
                .map(|a| a.group.clone())
                .ok_or("not a running AI agent")?;
            shared.group_board(&group)
        }
        (BoardScope::Agent, None) => {
            let entity = req_entity(scene.main_world(), params)?;
            shared.board(entity).ok_or("not a running AI agent")?
        }
    };
    board.set(name, value);
    Ok(json!({ "set": key, "board": board.lock().to_json() }))
}

/// `ai.noise {position, radius, tag, data}`.
pub(super) fn noise(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let position = vec3_from_json(
        params
            .get("position")
            .ok_or("missing parameter \"position\"")?,
    )?;
    let radius = opt_f64(params, "radius").unwrap_or(10.0) as f32;
    let tag = opt_str(params, "tag").unwrap_or("noise");
    let data = params
        .get("data")
        .map(AiValue::from_json)
        .unwrap_or_default();
    ctx.gpu.scene.ai().shared().queue(AiCommand::Noise {
        tag: Arc::from(tag),
        position,
        radius,
        data,
        from: None,
    });
    Ok(json!({ "queued": tag }))
}

/// `ai.restart {entity}`.
pub(super) fn restart(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let scene = &ctx.gpu.scene;
    let entity = req_entity(scene.main_world(), params)?;
    scene.ai().shared().queue(AiCommand::Restart(entity));
    Ok(json!({ "restarting": entity.to_bits().get() }))
}

/// `ai.pause {entity, paused}`.
pub(super) fn pause(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let scene = &ctx.gpu.scene;
    let entity = req_entity(scene.main_world(), params)?;
    let paused = opt_bool(params, "paused").unwrap_or(true);
    let shared = scene.ai().shared().clone();
    let mut agents = shared.agents.borrow_mut();
    let agent = agents.get_mut(&entity).ok_or("not a running AI agent")?;
    agent.paused = paused;
    Ok(json!({ "paused": paused }))
}

/// `ai.debug {senses, goals, percepts, states, all}`.
pub(super) fn debug(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let ai = ctx.gpu.scene.ai_mut();
    let mut debug = ai.debug_draw();
    if let Some(all) = opt_bool(params, "all") {
        debug = if all {
            AiDebugDraw::all()
        } else {
            AiDebugDraw::default()
        };
    }
    if let Some(v) = opt_bool(params, "senses") {
        debug.senses = v;
    }
    if let Some(v) = opt_bool(params, "goals") {
        debug.goals = v;
    }
    if let Some(v) = opt_bool(params, "percepts") {
        debug.percepts = v;
    }
    if let Some(v) = opt_bool(params, "states") {
        debug.states = v;
    }
    ai.set_debug_draw(debug);
    Ok(serde_json::to_value(debug).unwrap_or(Value::Null))
}
