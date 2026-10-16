//! # Skeletal animation API
//!
//! Plays clips on characters that have an `Animator` (skinned glTF imports get one on
//! their root). Every function takes the character's root entity or any entity below it
//! (its mesh, a bone): the nearest animator up the hierarchy is used, else the nearest
//! below (a character made with "Make Character" has its model, and animator, below).
//!
//! - `animator_play(entity, clip[, options])` cross-fades to `clip`. Options:
//!   `fade` (seconds, default 0.2), `loop` (default true), `speed` (default 1, negative
//!   plays backwards), `layer` (default 0), `start` (time in seconds). Calling it every
//!   frame with the same clip keeps it playing; a clip that ran to its end restarts.
//! - `animator_stop(entity[, options])` fades out `layer` (default: every layer) over
//!   `fade` seconds, back to the rest pose.
//! - `animator_set_speed(entity, speed)` scales the speed of every clip (0 pauses).
//! - `animator_set_layer(entity, layer, options)` sets a layer's `weight` (0..1, faded
//!   over `fade` seconds) and `mask`: a bone name, so the layer only moves that bone and
//!   the bones below it (e.g. `"Spine"` for an upper-body wave over a walk), or `false`
//!   to clear it.
//! - `animator_add_clips(entity, path)` makes the clips of a glTF/GLB file playable
//!   (project-relative path); the entity gets an animator if it has none.
//! - `animator_set_root_motion(entity, mode)`: `"keep"`, `"in_place"` or `"apply"` (move
//!   the entity by the clip's travel instead of the hips).
//! - `animator_state(entity[, layer])` returns `{clip, time, duration, playing, looping,
//!   finished, fading, speed}` for the layer's current clip, or nil without an animator.
//! - `animator_clips(entity)` returns `{ {name, duration, tracks, bound, missing,
//!   root_speed}, ... }` once the animator has run a frame (in play mode); `root_speed`
//!   is how fast the clip travels over the ground (m/s, 0 for clips made in place).
//! - `find_bone(entity, name)` returns the bone entity called `name` below `entity`, or
//!   nil. Names match without case or a `mixamorig:` style prefix, so `"RightHand"`
//!   finds `mixamorig:RightHand`. Parent a sword to it to hold it.
//!
//! Changes are queued like other script commands and take effect when the animator next
//! updates.
//!
//! ## Animation trees
//!
//! A character with an `AnimationTree` component picks its clips itself from parameters
//! (states, blend spaces and transitions, like Godot's AnimationTree). With
//! `from_character` its CharacterController feeds `speed`, `forward_speed`, `side_speed`,
//! `vertical_speed`, `grounded`, `air_time`, `landing_speed` and the triggers `jumped` and
//! `landed`; scripts add their own. These take the character's root, its model or a bone:
//!
//! - `animation_set(entity, name, value)` sets a parameter (a number, or true/false).
//! - `animation_trigger(entity, name)` fires a trigger: true for the next evaluation only.
//! - `animation_get(entity, name)` reads a parameter (as a number).
//! - `animation_state(entity)` returns `{state, previous, time, progress}` (the state's
//!   name, the one before, seconds in it, and how far its motion has played, 0..1).
//! - `animation_travel(entity, state[, fade])` goes to a state now.
//!
//! The tree plays on layer 0; `animator_play(e, clip, { layer = 1 })` still overlays a
//! clip (an upper-body wave with `animator_set_layer(e, 1, { mask = "Spine" })`).

use std::path::PathBuf;

use hecs::Entity;
use mlua::{Lua, Result as LuaResult, Table, Value};

use crate::scene::skeletal::{
    animator_entity, find_bone, tree_entity, AnimationTree, Animator, AnimatorCommand, PlayOptions,
    RootMotion, TreeCommand,
};
use crate::scripting::lua::commands::entity_bits;
use crate::scripting::lua::guards::{with_active_commands, with_active_world};

fn error(message: impl Into<String>) -> mlua::Error {
    mlua::Error::RuntimeError(message.into())
}

fn options(value: Option<Table>) -> LuaResult<PlayOptions> {
    let mut options = PlayOptions::default();
    let Some(table) = value else {
        return Ok(options);
    };
    if let Some(fade) = table.get::<Option<f32>>("fade")? {
        options.fade = fade.max(0.0);
    }
    if let Some(looping) = table.get::<Option<bool>>("loop")? {
        options.looping = looping;
    }
    if let Some(speed) = table.get::<Option<f32>>("speed")? {
        options.speed = speed;
    }
    if let Some(layer) = table.get::<Option<i64>>("layer")? {
        if !(0..crate::scene::skeletal::MAX_LAYERS as i64).contains(&layer) {
            return Err(error(format!(
                "layer must be 0..{}",
                crate::scene::skeletal::MAX_LAYERS - 1
            )));
        }
        options.layer = layer as usize;
    }
    options.start = table.get::<Option<f32>>("start")?;
    Ok(options)
}

fn queue(entity: i64, command: AnimatorCommand) -> LuaResult<()> {
    with_active_commands(|commands| commands.animator(entity, command))
}

fn queue_tree(entity: i64, command: TreeCommand) -> LuaResult<()> {
    with_active_commands(|commands| commands.animation_tree(entity, command))
}

/// Read the animation tree that drives `entity`, if any.
fn with_tree<R>(entity: i64, f: impl FnOnce(&AnimationTree) -> R) -> LuaResult<Option<R>> {
    with_active_world(|world| {
        let Some(entity) = Entity::from_bits(entity as u64) else {
            return Ok(None);
        };
        let Some(owner) = tree_entity(world, entity) else {
            return Ok(None);
        };
        Ok(world.get::<&AnimationTree>(owner).ok().map(|tree| f(&tree)))
    })
}

/// Read the animator that drives `entity`, if any.
fn with_animator<R>(entity: i64, f: impl FnOnce(&Animator) -> R) -> LuaResult<Option<R>> {
    with_active_world(|world| {
        let Some(entity) = Entity::from_bits(entity as u64) else {
            return Ok(None);
        };
        let Some(owner) = animator_entity(world, entity) else {
            return Ok(None);
        };
        Ok(world
            .get::<&Animator>(owner)
            .ok()
            .map(|animator| f(&animator)))
    })
}

pub(crate) fn register_animation_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    globals.set(
        "animator_play",
        lua.create_function(|_, (entity, clip, opts): (i64, String, Option<Table>)| {
            let options = options(opts)?;
            queue(entity, AnimatorCommand::Play { clip, options })
        })?,
    )?;

    globals.set(
        "animator_stop",
        lua.create_function(|_, (entity, opts): (i64, Option<Table>)| {
            let (layer, fade) = match opts {
                Some(table) => (
                    table
                        .get::<Option<i64>>("layer")?
                        .map(|l| l.max(0) as usize),
                    table.get::<Option<f32>>("fade")?.unwrap_or(0.2),
                ),
                None => (None, 0.2),
            };
            queue(entity, AnimatorCommand::Stop { layer, fade })
        })?,
    )?;

    globals.set(
        "animator_set_speed",
        lua.create_function(|_, (entity, speed): (i64, f32)| {
            queue(entity, AnimatorCommand::SetSpeed(speed))
        })?,
    )?;

    globals.set(
        "animator_set_layer",
        lua.create_function(|_, (entity, layer, opts): (i64, i64, Table)| {
            if !(0..crate::scene::skeletal::MAX_LAYERS as i64).contains(&layer) {
                return Err(error("layer out of range"));
            }
            let mask = match opts.get::<Value>("mask")? {
                Value::Nil => None,
                Value::Boolean(false) => Some(None),
                Value::String(name) => Some(Some(name.to_str()?.to_string())),
                _ => return Err(error("mask must be a bone name or false")),
            };
            queue(
                entity,
                AnimatorCommand::SetLayer {
                    layer: layer as usize,
                    weight: opts.get::<Option<f32>>("weight")?.unwrap_or(1.0),
                    fade: opts.get::<Option<f32>>("fade")?.unwrap_or(0.0),
                    mask,
                },
            )
        })?,
    )?;

    globals.set(
        "animator_add_clips",
        lua.create_function(|_, (entity, path): (i64, String)| {
            queue(entity, AnimatorCommand::AddLibrary(PathBuf::from(path)))
        })?,
    )?;

    globals.set(
        "animator_set_root_motion",
        lua.create_function(|_, (entity, mode): (i64, String)| {
            let mode = RootMotion::parse(&mode)
                .ok_or_else(|| error("root motion is \"keep\", \"in_place\" or \"apply\""))?;
            queue(entity, AnimatorCommand::SetRootMotion(mode))
        })?,
    )?;

    globals.set(
        "animator_state",
        lua.create_function(|lua, (entity, layer): (i64, Option<i64>)| {
            let status = with_animator(entity, |animator| {
                animator.status(layer.unwrap_or(0).max(0) as usize)
            })?;
            let Some(status) = status else {
                return Ok(Value::Nil);
            };
            let table = lua.create_table()?;
            table.set("clip", status.clip)?;
            table.set("time", status.time)?;
            table.set("duration", status.duration)?;
            table.set("playing", status.playing)?;
            table.set("looping", status.looping)?;
            table.set("finished", status.finished)?;
            table.set("fading", status.fading)?;
            table.set("speed", status.speed)?;
            Ok(Value::Table(table))
        })?,
    )?;

    globals.set(
        "animator_clips",
        lua.create_function(|lua, entity: i64| {
            let clips = with_animator(entity, Animator::clips)?.unwrap_or_default();
            let list = lua.create_table()?;
            for (index, clip) in clips.into_iter().enumerate() {
                let entry = lua.create_table()?;
                entry.set("name", clip.name)?;
                entry.set("duration", clip.duration)?;
                entry.set("tracks", clip.tracks)?;
                entry.set("bound", clip.bound)?;
                entry.set("missing", clip.missing.len())?;
                entry.set("root_speed", clip.root_speed)?;
                list.raw_set(index + 1, entry)?;
            }
            Ok(list)
        })?,
    )?;

    globals.set(
        "animation_set",
        lua.create_function(|_, (entity, name, value): (i64, String, Value)| {
            let value = match value {
                Value::Boolean(flag) => f32::from(u8::from(flag)),
                Value::Integer(n) => n as f32,
                Value::Number(n) => n as f32,
                _ => return Err(error("an animation parameter is a number or true/false")),
            };
            queue_tree(entity, TreeCommand::Set(name, value))
        })?,
    )?;

    globals.set(
        "animation_trigger",
        lua.create_function(|_, (entity, name): (i64, String)| {
            queue_tree(entity, TreeCommand::Trigger(name))
        })?,
    )?;

    globals.set(
        "animation_travel",
        lua.create_function(|_, (entity, state, fade): (i64, String, Option<f32>)| {
            queue_tree(
                entity,
                TreeCommand::Travel {
                    state,
                    fade: fade.unwrap_or(0.2),
                },
            )
        })?,
    )?;

    globals.set(
        "animation_get",
        lua.create_function(|_, (entity, name): (i64, String)| {
            with_tree(entity, |tree| tree.value(&name))
        })?,
    )?;

    globals.set(
        "animation_state",
        lua.create_function(|lua, entity: i64| {
            let state = with_tree(entity, |tree| {
                (
                    tree.runtime.state.clone(),
                    tree.runtime.previous.clone(),
                    tree.runtime.time_in_state,
                    tree.runtime.progress,
                )
            })?;
            let Some((state, previous, time, progress)) = state else {
                return Ok(Value::Nil);
            };
            let table = lua.create_table()?;
            table.set("state", state)?;
            table.set("previous", previous)?;
            table.set("time", time)?;
            table.set("progress", progress)?;
            Ok(Value::Table(table))
        })?,
    )?;

    globals.set(
        "find_bone",
        lua.create_function(|_, (entity, name): (i64, String)| {
            with_active_world(|world| {
                let Some(root) = Entity::from_bits(entity as u64) else {
                    return Ok(None);
                };
                Ok(find_bone(world, root, &name).map(entity_bits))
            })
        })?,
    )?;

    Ok(())
}
