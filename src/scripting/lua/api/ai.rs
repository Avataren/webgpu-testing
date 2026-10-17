//! # AI API
//!
//! Brains for `AiAgent`s (see `docs/ai.md`): behaviour trees and state machines written
//! as Lua tables, whose functions run in the context of the script that defined them.
//!
//! ## Brains
//! - `ai_tree(name, root)` defines (or redefines) a behaviour tree; agents running it
//!   start it again. A node is `{ "type", children or arguments..., param = value }`,
//!   e.g. `{ "sequence", { "move_to", "spot" }, { "play", "waving" } }`; a bare function
//!   is a Lua leaf.
//! - `ai_machine(name, { initial = "idle", states = {...}, transitions = {...} })`
//!   defines a state machine. A state has `enter`, `update`, `exit` (functions or
//!   names), `tree` (a node or a tree's name), `machine`, `set = {key = value}`,
//!   `final` and `transitions = { { "to_state", event = ..., when = ..., after = ...,
//!   on = "success" } }`.
//! - `ai_define(name, def)` is either (a machine when `def` has `states`); returns
//!   "tree" or "machine".
//! - `ai_register(name, function)` names a function brains loaded from JSON can call.
//! - `ai_brains()` lists `{name, kind}`.
//! - `ai_set_brain(entity, name[, options])` gives an entity an `AiAgent` running
//!   `name`; `options` are the component's fields (`tags`, `group`, `tick_rate`,
//!   `run_speed`, `blackboard`, `perception = {sight_range, sight_angle, ...}`).
//!
//! Lua functions in brains get one argument, `ctx`: `ctx.agent` (the entity),
//! `ctx.bb` (its blackboard: read and assign keys), `ctx.group`, `ctx.global` (shared
//! boards), `ctx.dt`, `ctx.time`, `ctx.node`, `ctx.phase`, `ctx.state`,
//! `ctx.state_time`, `ctx.memory` (a `lua` leaf's table while it runs), `ctx.event`
//! (`{name, data, from, position}` for a state entered by an event) and `ctx.result`
//! (how a leaf ended, in `exit`). A leaf's `run` returns `true`/nil (success), `false`
//! (failure) or `"running"`.
//!
//! ## Blackboards
//! - `ai_get(entity, key)` / `ai_set(entity, key, value)`: one value (`group.key`
//!   and `global.key` reach the shared boards). Tables `{x, y, z}` are stored as
//!   positions; other tables and functions by reference.
//! - `ai_blackboard(entity)`, `ai_group_board(name)`, `ai_global()` return a board to
//!   index and assign (`pairs` lists it).
//!
//! ## Events and senses
//! - `ai_send(entity, event[, data[, from]])`: the agent's brain gets `event` on its next
//!   tick (transitions with `event = ...`, `wait_event` leaves); `from` is the sender
//!   (default: the script's entity; a brain's function passes `ctx.agent`).
//! - `ai_broadcast(event[, data[, {x, y, z, radius, tag, group}]])`: to every agent, to
//!   those within `radius` of a point (with `tag`), or to a group.
//! - `ai_noise(x, y, z, radius, tag[, data[, from]])`: agents that hear it get `hear`
//!   and `hear:<tag>` events and remember it (`heard('tag')`).
//! - `ai_senses(entity)` returns `{percepts = {{entity, tags, visible, position,
//!   seen_ago}}, noises = {...}}`; `ai_sees(entity[, tag])`.
//!
//! ## Inspecting and steering
//! - `ai_state(entity)`: its state path (`"work/serve"`), or nil; `ai_action(entity)`:
//!   what its tree is doing; `ai_status(entity)`: `{brain, state, action, tags, group,
//!   error, paused}`.
//! - `ai_agents([{brain, tag, state, group}])` lists agents (entity ids).
//! - `ai_restart(entity)`, `ai_pause(entity, paused)`.
//! - `ai_debug_draw(enabled | {senses, goals, percepts, states})`.
//! - Scripts on an agent get `on_ai_state(self, {from, to})` when its state changes.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use glam::Vec3;
use hecs::Entity;
use mlua::{
    Function, Lua, LuaSerdeExt, MetaMethod, Result as LuaResult, Table, UserData, UserDataMethods,
    Value as LuaValue,
};

use crate::ai::context::{AiEvent, LuaCall, Recipients};
use crate::ai::{
    parse_brain, parse_machine, parse_tree, split_key, AiCommand, AiDebugDraw, AiShared,
    BlackboardRef, BoardScope, Brain, Def, LuaFn, LuaRef, Value,
};
use crate::scripting::lua::commands::entity_bits;
use crate::scripting::lua::guards::{resolve_entity, with_active_commands, ACTIVE_ENTITY};

thread_local! {
    static ACTIVE_AI: RefCell<Option<Rc<AiShared>>> = const { RefCell::new(None) };
}

/// Makes a scene's AI reachable from `ai_*` functions while its scripts (and brains)
/// run.
pub(crate) struct AiGuard {
    previous: Option<Rc<AiShared>>,
}

impl AiGuard {
    pub fn enter(shared: &Rc<AiShared>) -> Self {
        let previous = ACTIVE_AI.with(|cell| cell.replace(Some(shared.clone())));
        Self { previous }
    }
}

impl Drop for AiGuard {
    fn drop(&mut self) {
        let previous = self.previous.take();
        ACTIVE_AI.with(|cell| *cell.borrow_mut() = previous);
    }
}

fn error(message: impl Into<String>) -> mlua::Error {
    mlua::Error::RuntimeError(message.into())
}

/// Run `f` with the scene's AI (no borrow of the guard is held while it runs).
fn with_ai<R>(f: impl FnOnce(&AiShared) -> LuaResult<R>) -> LuaResult<R> {
    let shared = ACTIVE_AI
        .with(|cell| cell.borrow().clone())
        .ok_or_else(|| error("the AI is not available here (it runs with the scene's scripts)"))?;
    f(&shared)
}

fn entity_of(handle: i64) -> LuaResult<Entity> {
    resolve_entity(handle).ok_or_else(|| error(format!("{handle} is not an entity (yet)")))
}

/// The script running now, which owns functions it defines.
fn active_owner() -> Option<Entity> {
    ACTIVE_ENTITY
        .with(|cell| *cell.borrow())
        .filter(|bits| *bits > 0)
        .and_then(|bits| Entity::from_bits(bits as u64))
}

// ---------------------------------------------------------------------- values

/// A Lua table `{x = .., y = .., z = ..}` (and nothing else) as a position.
fn table_vec3(table: &Table) -> Option<Vec3> {
    if table.raw_len() != 0 {
        return None;
    }
    let mut count = 0;
    for pair in table.clone().pairs::<LuaValue, LuaValue>() {
        let (key, _) = pair.ok()?;
        match key {
            LuaValue::String(s) if matches!(s.to_str().ok()?.as_ref(), "x" | "y" | "z") => {
                count += 1
            }
            _ => return None,
        }
    }
    if count != 3 {
        return None;
    }
    let get = |k: &str| table.raw_get::<Option<f64>>(k).ok().flatten();
    Some(Vec3::new(
        get("x")? as f32,
        get("y")? as f32,
        get("z")? as f32,
    ))
}

fn by_reference(lua: &Lua, value: LuaValue) -> Value {
    lua.create_registry_value(value)
        .map(|key| Value::Lua(Arc::new(LuaRef(key))))
        .unwrap_or_default()
}

/// A Lua value as a blackboard value: `{x, y, z}` tables become positions, other tables
/// and functions are kept by reference.
pub(crate) fn value_from_lua(lua: &Lua, value: LuaValue) -> Value {
    match value {
        LuaValue::Nil => Value::Nil,
        LuaValue::Boolean(b) => Value::Bool(b),
        LuaValue::Integer(i) => Value::Int(i),
        LuaValue::Number(n) => Value::Num(n),
        LuaValue::String(s) => Value::str(s.to_string_lossy()),
        LuaValue::Table(table) => match table_vec3(&table) {
            Some(v) => Value::Vec3(v),
            None => by_reference(lua, LuaValue::Table(table)),
        },
        other @ (LuaValue::Function(_) | LuaValue::UserData(_)) => by_reference(lua, other),
        _ => Value::Nil,
    }
}

pub(crate) fn value_to_lua(lua: &Lua, value: &Value) -> LuaResult<LuaValue> {
    Ok(match value {
        Value::Nil => LuaValue::Nil,
        Value::Bool(b) => LuaValue::Boolean(*b),
        Value::Int(i) => LuaValue::Integer(*i),
        Value::Num(n) => LuaValue::Number(*n),
        Value::Str(s) => LuaValue::String(lua.create_string(&**s)?),
        Value::Vec3(v) => {
            let table = lua.create_table()?;
            table.set("x", v.x)?;
            table.set("y", v.y)?;
            table.set("z", v.z)?;
            LuaValue::Table(table)
        }
        Value::Json(json) => json_to_lua(lua, json)?,
        Value::Lua(lua_ref) => lua.registry_value(&lua_ref.0)?,
    })
}

fn json_to_lua(lua: &Lua, json: &serde_json::Value) -> LuaResult<LuaValue> {
    let options = mlua::SerializeOptions::new()
        .serialize_none_to_null(false)
        .serialize_unit_to_null(false);
    lua.to_value_with(json, options)
}

pub(crate) fn lua_field(lua: &Lua, lua_ref: &LuaRef, name: &str) -> Value {
    let Ok(LuaValue::Table(table)) = lua.registry_value::<LuaValue>(&lua_ref.0) else {
        return Value::Nil;
    };
    let value = match name.parse::<i64>() {
        Ok(index) => table.get::<LuaValue>(index),
        Err(_) => table.get::<LuaValue>(name),
    };
    value.map(|v| value_from_lua(lua, v)).unwrap_or_default()
}

pub(crate) fn lua_list(lua: &Lua, lua_ref: &LuaRef) -> Vec<Value> {
    let Ok(LuaValue::Table(table)) = lua.registry_value::<LuaValue>(&lua_ref.0) else {
        return Vec::new();
    };
    table
        .sequence_values::<LuaValue>()
        .filter_map(Result::ok)
        .map(|v| value_from_lua(lua, v))
        .collect()
}

pub(crate) fn lua_vec3(lua: &Lua, lua_ref: &LuaRef) -> Option<Vec3> {
    let Ok(LuaValue::Table(table)) = lua.registry_value::<LuaValue>(&lua_ref.0) else {
        return None;
    };
    let get = |k: LuaValue| table.get::<Option<f64>>(k).ok().flatten();
    let named = (|| {
        Some(Vec3::new(
            get(LuaValue::String(lua.create_string("x").ok()?))? as f32,
            get(LuaValue::String(lua.create_string("y").ok()?))? as f32,
            get(LuaValue::String(lua.create_string("z").ok()?))? as f32,
        ))
    })();
    named.or_else(|| {
        Some(Vec3::new(
            get(LuaValue::Integer(1))? as f32,
            get(LuaValue::Integer(2))? as f32,
            get(LuaValue::Integer(3))? as f32,
        ))
    })
}

pub(crate) fn lua_len(lua: &Lua, lua_ref: &LuaRef) -> Option<usize> {
    match lua.registry_value::<LuaValue>(&lua_ref.0).ok()? {
        LuaValue::Table(table) => Some(table.raw_len()),
        LuaValue::String(s) => Some(s.as_bytes().len()),
        _ => None,
    }
}

fn function_label(function: &Function) -> String {
    let info = function.info();
    format!(
        "{}:{}",
        info.short_src.unwrap_or_else(|| "?".into()),
        info.line_defined.unwrap_or(0)
    )
}

/// A brain definition from Lua: tables keep their named fields and list items,
/// functions are kept (owned by the running script).
fn def_from_lua(lua: &Lua, value: LuaValue, owner: Option<Entity>, depth: usize) -> LuaResult<Def> {
    if depth > 96 {
        return Err(error(
            "the definition is nested too deeply (a table inside itself?)",
        ));
    }
    Ok(match value {
        LuaValue::Nil => Def::Nil,
        LuaValue::Boolean(b) => Def::Bool(b),
        LuaValue::Integer(i) => Def::Int(i),
        LuaValue::Number(n) => Def::Num(n),
        LuaValue::String(s) => Def::Str(s.to_string_lossy()),
        LuaValue::Function(f) => Def::Func(Arc::new(LuaFn {
            label: function_label(&f),
            key: lua.create_registry_value(f)?,
            owner,
        })),
        LuaValue::Table(table) => {
            let len = table.raw_len();
            let mut items = Vec::with_capacity(len);
            for i in 1..=len {
                items.push(def_from_lua(lua, table.raw_get(i)?, owner, depth + 1)?);
            }
            let mut named = Vec::new();
            for pair in table.pairs::<LuaValue, LuaValue>() {
                let (key, value) = pair?;
                match key {
                    LuaValue::String(s) => named.push((
                        s.to_string_lossy(),
                        def_from_lua(lua, value, owner, depth + 1)?,
                    )),
                    LuaValue::Integer(i) if i >= 1 && (i as usize) <= len => {}
                    other => {
                        return Err(error(format!(
                            "brain tables use names or list positions as keys, not {}",
                            other.type_name()
                        )))
                    }
                }
            }
            // Lua's order is arbitrary; sorting keeps messages and states stable.
            named.sort_by(|a, b| a.0.cmp(&b.0));
            Def::Table { named, items }
        }
        other => {
            return Err(error(format!(
                "a brain cannot hold a {}",
                other.type_name()
            )))
        }
    })
}

// ---------------------------------------------------------------------- boards

/// A blackboard as Lua sees it: index and assign its keys; `pairs` lists them.
#[derive(Clone)]
pub(crate) struct BoardProxy(pub BlackboardRef);

impl UserData for BoardProxy {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::Index, |lua, this, key: String| {
            let value = this.0.get(&key);
            value_to_lua(lua, &value)
        });
        methods.add_meta_method(
            MetaMethod::NewIndex,
            |lua, this, (key, value): (String, LuaValue)| {
                let value = value_from_lua(lua, value);
                this.0.set(key, value);
                Ok(())
            },
        );
        methods.add_meta_method(MetaMethod::Pairs, |lua, this, ()| {
            let entries: Vec<(String, Value)> = this
                .0
                .lock()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let table = lua.create_table()?;
            for (key, value) in entries {
                table.set(key, value_to_lua(lua, &value)?)?;
            }
            let next: Function = lua.globals().get("next")?;
            Ok((next, table, LuaValue::Nil))
        });
        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            Ok(format!("blackboard ({} keys)", this.0.lock().len()))
        });
    }
}

fn event_table(lua: &Lua, event: &AiEvent) -> LuaResult<Table> {
    let table = lua.create_table()?;
    table.set("name", &*event.name)?;
    table.set("data", value_to_lua(lua, &event.data)?)?;
    if let Some(from) = event.from {
        table.set("from", entity_bits(from))?;
    }
    if let Some(p) = event.position {
        table.set("position", value_to_lua(lua, &Value::Vec3(p))?)?;
    }
    Ok(table)
}

/// The `ctx` a brain's Lua function gets.
pub(crate) fn make_ctx(lua: &Lua, call: &LuaCall) -> LuaResult<Table> {
    let ctx = lua.create_table()?;
    ctx.set("agent", entity_bits(call.agent))?;
    ctx.set("bb", BoardProxy(call.board.clone()))?;
    ctx.set("group", BoardProxy(call.group.clone()))?;
    ctx.set("global", BoardProxy(call.global.clone()))?;
    ctx.set("dt", call.dt)?;
    ctx.set("time", call.time)?;
    ctx.set("node", call.node)?;
    ctx.set("phase", call.phase.name())?;
    if let Some(memory) = call.memory {
        ctx.set("memory", value_to_lua(lua, memory)?)?;
    }
    if let Some(event) = call.event {
        ctx.set("event", event_table(lua, event)?)?;
    }
    if let Some(result) = call.result {
        ctx.set("result", result)?;
    }
    if let Some(state) = call.state {
        ctx.set("state", state)?;
        ctx.set("state_time", call.state_time)?;
    }
    Ok(ctx)
}

/// The board a key names for an agent: its own, its group's or the scene's.
fn board_for(ai: &AiShared, entity: Entity, key: &str) -> LuaResult<(BlackboardRef, String)> {
    let (scope, name) = split_key(key);
    let agents = ai.agents.borrow();
    let agent = agents
        .get(&entity)
        .ok_or_else(|| error("not an AI agent (it needs an AiAgent component, in play)"))?;
    let board = match scope {
        BoardScope::Agent => agent.board.clone(),
        BoardScope::Group => {
            let group = agent.group.clone();
            drop(agents);
            ai.group_board(&group)
        }
        BoardScope::Global => ai.global.clone(),
    };
    Ok((board, name.to_owned()))
}

fn debug_from(value: LuaValue) -> LuaResult<AiDebugDraw> {
    Ok(match value {
        LuaValue::Boolean(true) => AiDebugDraw::all(),
        LuaValue::Boolean(false) | LuaValue::Nil => AiDebugDraw::default(),
        LuaValue::Table(t) => AiDebugDraw {
            senses: t.get::<Option<bool>>("senses")?.unwrap_or(false),
            goals: t.get::<Option<bool>>("goals")?.unwrap_or(false),
            percepts: t.get::<Option<bool>>("percepts")?.unwrap_or(false),
            states: t.get::<Option<bool>>("states")?.unwrap_or(false),
        },
        _ => return Err(error("ai_debug_draw takes a boolean or a table")),
    })
}

fn define(lua: &Lua, name: String, def: LuaValue, kind: Option<&str>) -> LuaResult<&'static str> {
    let def = def_from_lua(lua, def, active_owner(), 0)?;
    let brain = match kind {
        Some("tree") => Brain::Tree(Arc::new(parse_tree(&name, &def).map_err(error)?)),
        Some(_) => Brain::Machine(Arc::new(parse_machine(&name, &def).map_err(error)?)),
        None => parse_brain(&name, &def).map_err(error)?,
    };
    let kind = brain.kind();
    with_ai(|ai| {
        ai.library
            .try_borrow_mut()
            .map_err(|_| error("cannot define brains from inside a brain's condition"))?
            .define(name, brain);
        Ok(kind)
    })
}

pub(crate) fn register_ai_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    globals.set(
        "ai_tree",
        lua.create_function(|lua, (name, def): (String, LuaValue)| {
            define(lua, name, def, Some("tree")).map(|_| true)
        })?,
    )?;
    globals.set(
        "ai_machine",
        lua.create_function(|lua, (name, def): (String, LuaValue)| {
            define(lua, name, def, Some("machine")).map(|_| true)
        })?,
    )?;
    globals.set(
        "ai_define",
        lua.create_function(|lua, (name, def): (String, LuaValue)| define(lua, name, def, None))?,
    )?;

    globals.set(
        "ai_register",
        lua.create_function(|lua, (name, function): (String, Function)| {
            let f = Arc::new(LuaFn {
                label: format!("{name} ({})", function_label(&function)),
                key: lua.create_registry_value(function)?,
                owner: active_owner(),
            });
            with_ai(|ai| {
                ai.library.borrow_mut().register_function(name, f);
                Ok(())
            })
        })?,
    )?;

    globals.set(
        "ai_brains",
        lua.create_function(|lua, ()| {
            let names = with_ai(|ai| Ok(ai.library.borrow().names()))?;
            let list = lua.create_table()?;
            for (i, (name, kind)) in names.into_iter().enumerate() {
                let entry = lua.create_table()?;
                entry.set("name", name)?;
                entry.set("kind", kind)?;
                list.raw_set(i + 1, entry)?;
            }
            Ok(list)
        })?,
    )?;

    globals.set(
        "ai_set_brain",
        lua.create_function(
            |lua, (handle, brain, options): (i64, String, Option<LuaValue>)| {
                let mut value = match options {
                    Some(options) => lua.from_value::<serde_json::Value>(options)?,
                    None => serde_json::json!({}),
                };
                if !value.is_object() {
                    return Err(error("ai_set_brain's options must be a table"));
                }
                value["brain"] = serde_json::Value::String(brain);
                with_active_commands(|commands| {
                    commands.set_component(handle, "ai_agent".into(), value)
                })
            },
        )?,
    )?;

    globals.set(
        "ai_get",
        lua.create_function(|lua, (handle, key): (i64, String)| {
            let entity = entity_of(handle)?;
            let value = with_ai(|ai| {
                let (board, name) = board_for(ai, entity, &key)?;
                let value = board.get(&name);
                Ok(value)
            })?;
            value_to_lua(lua, &value)
        })?,
    )?;

    globals.set(
        "ai_set",
        lua.create_function(|lua, (handle, key, value): (i64, String, LuaValue)| {
            let entity = entity_of(handle)?;
            let value = value_from_lua(lua, value);
            with_ai(|ai| {
                let (board, name) = board_for(ai, entity, &key)?;
                board.set(name, value);
                Ok(())
            })
        })?,
    )?;

    globals.set(
        "ai_blackboard",
        lua.create_function(|_, handle: i64| {
            let Some(entity) = resolve_entity(handle) else {
                return Ok(None);
            };
            with_ai(|ai| Ok(ai.board(entity).map(BoardProxy)))
        })?,
    )?;
    globals.set(
        "ai_group_board",
        lua.create_function(|_, name: String| with_ai(|ai| Ok(BoardProxy(ai.group_board(&name)))))?,
    )?;
    globals.set(
        "ai_global",
        lua.create_function(|_, ()| with_ai(|ai| Ok(BoardProxy(ai.global.clone()))))?,
    )?;

    globals.set(
        "ai_send",
        lua.create_function(
            |lua, (handle, event, data, from): (i64, String, Option<LuaValue>, Option<i64>)| {
                let Some(entity) = resolve_entity(handle) else {
                    return Ok(false);
                };
                let event = AiEvent {
                    name: Arc::from(event),
                    data: data.map(|d| value_from_lua(lua, d)).unwrap_or_default(),
                    // A brain's function sends as its agent: `ai_send(e, name, data, ctx.agent)`.
                    from: from.and_then(resolve_entity).or_else(active_owner),
                    position: None,
                };
                with_ai(|ai| Ok(ai.send(entity, event)))
            },
        )?,
    )?;

    globals.set(
        "ai_broadcast",
        lua.create_function(
            |lua, (event, data, options): (String, Option<LuaValue>, Option<Table>)| {
                let event = AiEvent {
                    name: Arc::from(event),
                    data: data.map(|d| value_from_lua(lua, d)).unwrap_or_default(),
                    from: active_owner(),
                    position: None,
                };
                let mut group = None;
                let to = match options {
                    Some(options) => {
                        group = options.get::<Option<String>>("group")?;
                        match options.get::<Option<f32>>("radius")? {
                            Some(radius) => {
                                let center = Vec3::new(
                                    options.get::<Option<f32>>("x")?.unwrap_or(0.0),
                                    options.get::<Option<f32>>("y")?.unwrap_or(0.0),
                                    options.get::<Option<f32>>("z")?.unwrap_or(0.0),
                                );
                                Recipients::Near {
                                    center,
                                    radius,
                                    tag: options.get::<Option<String>>("tag")?,
                                }
                            }
                            None if group.is_some() => Recipients::Group,
                            None => Recipients::All,
                        }
                    }
                    None => Recipients::All,
                };
                with_ai(|ai| {
                    ai.queue(AiCommand::Send { to, event, group });
                    Ok(())
                })
            },
        )?,
    )?;

    globals.set(
        "ai_noise",
        lua.create_function(
            |lua,
             (x, y, z, radius, tag, data, from): (
                f32,
                f32,
                f32,
                f32,
                String,
                Option<LuaValue>,
                Option<i64>,
            )| {
                let data = data.map(|d| value_from_lua(lua, d)).unwrap_or_default();
                let from = from.and_then(resolve_entity);
                with_ai(|ai| {
                    ai.queue(AiCommand::Noise {
                        tag: Arc::from(tag),
                        position: Vec3::new(x, y, z),
                        radius,
                        data,
                        from,
                    });
                    Ok(())
                })
            },
        )?,
    )?;

    globals.set(
        "ai_state",
        lua.create_function(|_, handle: i64| {
            let Some(entity) = resolve_entity(handle) else {
                return Ok(None);
            };
            with_ai(|ai| Ok(ai.agents.borrow().get(&entity).map(|a| a.state_path())))
        })?,
    )?;

    globals.set(
        "ai_action",
        lua.create_function(|_, handle: i64| {
            let Some(entity) = resolve_entity(handle) else {
                return Ok(None);
            };
            with_ai(|ai| {
                Ok(ai
                    .agents
                    .borrow()
                    .get(&entity)
                    .and_then(|a| a.action.clone()))
            })
        })?,
    )?;

    globals.set(
        "ai_status",
        lua.create_function(|lua, handle: i64| {
            let Some(entity) = resolve_entity(handle) else {
                return Ok(None);
            };
            let info = with_ai(|ai| Ok(ai.agents.borrow().get(&entity).cloned()))?;
            let Some(info) = info else {
                return Ok(None);
            };
            let table = lua.create_table()?;
            table.set("brain", info.brain.clone())?;
            table.set("state", info.state_path())?;
            table.set("action", info.action.clone())?;
            table.set("status", info.status.map(|s| s.name()))?;
            let tags = lua.create_table()?;
            for (i, tag) in info.tags.iter().enumerate() {
                tags.raw_set(i + 1, &**tag)?;
            }
            table.set("tags", tags)?;
            table.set("group", info.group.clone())?;
            table.set("error", info.error.clone())?;
            table.set("paused", info.paused)?;
            Ok(Some(table))
        })?,
    )?;

    globals.set(
        "ai_senses",
        lua.create_function(|lua, handle: i64| {
            let Some(entity) = resolve_entity(handle) else {
                return Ok(LuaValue::Nil);
            };
            let found = with_ai(|ai| {
                Ok(ai
                    .agents
                    .borrow()
                    .get(&entity)
                    .map(|a| (a.senses.clone(), ai.time.get())))
            })?;
            let Some((senses, now)) = found else {
                return Ok(LuaValue::Nil);
            };
            json_to_lua(lua, &senses.to_json(now))
        })?,
    )?;

    globals.set(
        "ai_sees",
        lua.create_function(|_, (handle, tag): (i64, Option<String>)| {
            let Some(entity) = resolve_entity(handle) else {
                return Ok(false);
            };
            with_ai(|ai| {
                Ok(ai
                    .agents
                    .borrow()
                    .get(&entity)
                    .is_some_and(|a| a.senses.sees(tag.as_deref())))
            })
        })?,
    )?;

    globals.set(
        "ai_restart",
        lua.create_function(|_, handle: i64| {
            let entity = entity_of(handle)?;
            with_ai(|ai| {
                ai.queue(AiCommand::Restart(entity));
                Ok(())
            })
        })?,
    )?;

    globals.set(
        "ai_pause",
        lua.create_function(|_, (handle, paused): (i64, Option<bool>)| {
            let entity = entity_of(handle)?;
            with_ai(|ai| {
                let mut agents = ai.agents.borrow_mut();
                match agents.get_mut(&entity) {
                    Some(agent) => {
                        agent.paused = paused.unwrap_or(true);
                        Ok(true)
                    }
                    None => Ok(false),
                }
            })
        })?,
    )?;

    globals.set(
        "ai_agents",
        lua.create_function(|lua, filter: Option<Table>| {
            let (brain, tag, state, group) = match &filter {
                Some(f) => (
                    f.get::<Option<String>>("brain")?,
                    f.get::<Option<String>>("tag")?,
                    f.get::<Option<String>>("state")?,
                    f.get::<Option<String>>("group")?,
                ),
                None => (None, None, None, None),
            };
            let mut found: Vec<i64> = with_ai(|ai| {
                Ok(ai
                    .agents
                    .borrow()
                    .iter()
                    .filter(|(_, a)| {
                        brain.as_ref().is_none_or(|b| a.brain == *b)
                            && group.as_ref().is_none_or(|g| a.group == *g)
                            && tag
                                .as_ref()
                                .is_none_or(|t| a.tags.iter().any(|x| **x == **t))
                            && state
                                .as_ref()
                                .is_none_or(|s| a.states.iter().any(|x| **x == **s))
                    })
                    .map(|(e, _)| entity_bits(*e))
                    .collect())
            })?;
            found.sort_unstable();
            let list = lua.create_table()?;
            for (i, e) in found.into_iter().enumerate() {
                list.raw_set(i + 1, e)?;
            }
            Ok(list)
        })?,
    )?;

    globals.set(
        "ai_debug_draw",
        lua.create_function(|_, value: LuaValue| {
            let debug = debug_from(value)?;
            if let Ok(mut view) = crate::ai::scene_ai_view().lock() {
                *view = debug;
            }
            Ok(())
        })?,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_cross_into_lua_and_back() {
        let lua = Lua::new();
        let table = lua.create_table().unwrap();
        table.set("x", 1.0).unwrap();
        table.set("y", 2.0).unwrap();
        table.set("z", 3.0).unwrap();
        let v = value_from_lua(&lua, LuaValue::Table(table));
        assert_eq!(v, Value::Vec3(Vec3::new(1.0, 2.0, 3.0)));

        let other = lua.create_table().unwrap();
        other.set("kind", "stall").unwrap();
        let v = value_from_lua(&lua, LuaValue::Table(other.clone()));
        let Value::Lua(r) = &v else {
            panic!("kept by reference: {v:?}")
        };
        assert_eq!(lua_field(&lua, r, "kind"), Value::str("stall"));
        let back = value_to_lua(&lua, &v).unwrap();
        assert_eq!(back, LuaValue::Table(other));

        let json = Value::from_json(&serde_json::json!({"a": [1, 2], "b": null}));
        let LuaValue::Table(t) = value_to_lua(&lua, &json).unwrap() else {
            panic!()
        };
        assert_eq!(t.get::<LuaValue>("b").unwrap(), LuaValue::Nil);
    }

    #[test]
    fn definitions_keep_names_items_and_functions() {
        let lua = Lua::new();
        let def: LuaValue = lua
            .load(r#"return { "sequence", name = "s", { "wait", 1 }, function() end }"#)
            .eval()
            .unwrap();
        let def = def_from_lua(&lua, def, None, 0).unwrap();
        let Def::Table { named, items } = &def else {
            panic!()
        };
        assert_eq!(named.len(), 1);
        assert_eq!(items.len(), 3);
        assert!(matches!(items[2], Def::Func(_)));
        let tree = parse_tree("t", &def).unwrap();
        assert_eq!(tree.nodes.len(), 3);
    }
}
