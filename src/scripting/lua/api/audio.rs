//! # Audio API
//!
//! Sound effects and music for game scripts. Paths are relative to the project (e.g.
//! `"content/audio/hit.wav"`) or absolute; wav, ogg, mp3 and flac are supported.
//! Sounds play on the project's mixer buses (`content/audio/mixer.json`): `sfx`
//! unless they name another, such as `music`, `ambience`, `ui` or `voice`.

use mlua::{Lua, Result as LuaResult, Table, Value};

use crate::audio::{self, AudioSource, PlayOptions, SpatialOptions};
use crate::scene::components::WorldTransform;
use crate::scripting::lua::guards::{resolve_entity, with_active_world};

/// Registers the audio functions:
///
/// - `play_sound(path[, {volume, pitch, loop, bus, priority, entity, position, spatial,
///   min_distance, max_distance, rolloff, doppler}])` - start a sound, returns its id.
///   With `entity` it plays from that entity and follows it; with `position` it
///   plays from there. `spatial` is "auto" (2D under an orthographic camera, else 3D),
///   "2d", "3d" or "off"; `rolloff` "inverse" or "linear"; `doppler` scales the pitch
///   shift of motion (0 = none).
/// - `move_sound(id, position[, velocity])` - move a positioned sound
/// - `play_audio_source(entity)` / `stop_audio_source(entity)` - start or stop an
///   entity's `AudioSource` (for one that does not play on start)
/// - `stop_sound(id)` - stop one sound
/// - `stop_all_sounds()` - stop everything
/// - `set_master_volume(volume)` - gain applied to every sound
/// - `set_bus_volume(bus, db)` - a bus's fader in dB (0 = unchanged, -80 = silent)
/// - `set_bus_muted(bus, muted)` - mute or unmute a bus
///
/// Bus changes from scripts last until the project reopens; the editor's mixer
/// window saves the layout.
///
/// Sounds also stop when play mode ends.
pub(crate) fn register_audio_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    globals.set(
        "play_sound",
        lua.create_function(|_, (path, options): (String, Option<Table>)| {
            let mut play = PlayOptions::default();
            if let Some(options) = options {
                if let Some(volume) = options.get::<Option<f32>>("volume")? {
                    play.volume = volume.max(0.0);
                }
                if let Some(pitch) = options.get::<Option<f32>>("pitch")? {
                    play.pitch = pitch;
                }
                if let Some(looped) = options.get::<Option<bool>>("loop")? {
                    play.looped = looped;
                }
                if let Some(bus) = options.get::<Option<String>>("bus")? {
                    play.bus = Some(bus);
                }
                if let Some(priority) = options.get::<Option<u8>>("priority")? {
                    play.priority = priority;
                }
                let entity =
                    match options.get::<Option<i64>>("entity")? {
                        Some(handle) => Some(resolve_entity(handle).ok_or_else(|| {
                            mlua::Error::RuntimeError(format!("no entity {handle}"))
                        })?),
                        None => None,
                    };
                let position = match (entity, options.get::<Value>("position")?) {
                    (Some(entity), _) => Some(entity_position(entity)?),
                    (None, Value::Table(table)) => Some(vec3(&table)?),
                    _ => None,
                };
                if let Some(position) = position {
                    let mut spatial = SpatialOptions::at(position);
                    if let Some(mode) = options.get::<Option<String>>("spatial")? {
                        spatial.mode = parse(&mode, "spatial", "auto, 2d, 3d or off")?;
                    }
                    if let Some(rolloff) = options.get::<Option<String>>("rolloff")? {
                        spatial.rolloff = parse(&rolloff, "rolloff", "inverse or linear")?;
                    }
                    if let Some(v) = options.get::<Option<f32>>("min_distance")? {
                        spatial.min_distance = v;
                    }
                    if let Some(v) = options.get::<Option<f32>>("max_distance")? {
                        spatial.max_distance = v;
                    }
                    if let Some(v) = options.get::<Option<f32>>("doppler")? {
                        spatial.doppler = v;
                    }
                    play.spatial = Some(spatial);
                }
                if let (Some(entity), Some(_)) = (entity, play.spatial) {
                    return audio::play_attached(&path, play, entity)
                        .map_err(mlua::Error::RuntimeError);
                }
            }
            audio::play(&path, play).map_err(mlua::Error::RuntimeError)
        })?,
    )?;

    globals.set(
        "move_sound",
        lua.create_function(|_, (id, position, velocity): (u64, Table, Option<Table>)| {
            let velocity = match velocity {
                Some(table) => vec3(&table)?,
                None => [0.0; 3],
            };
            audio::move_sound(id, vec3(&position)?, velocity);
            Ok(())
        })?,
    )?;

    globals.set(
        "play_audio_source",
        lua.create_function(|_, handle: i64| {
            let entity = resolve_entity(handle)
                .ok_or_else(|| mlua::Error::RuntimeError(format!("no entity {handle}")))?;
            let source = with_active_world(|world| {
                world
                    .get::<&AudioSource>(entity)
                    .map(|source| (*source).clone())
                    .map_err(|_| {
                        mlua::Error::RuntimeError(format!("entity {handle} has no AudioSource"))
                    })
            })?;
            let position = entity_position(entity)?;
            audio::play_source(entity, &source, position).map_err(mlua::Error::RuntimeError)
        })?,
    )?;

    globals.set(
        "stop_audio_source",
        lua.create_function(|_, handle: i64| {
            Ok(resolve_entity(handle).is_some_and(audio::stop_source))
        })?,
    )?;

    globals.set(
        "stop_sound",
        lua.create_function(|_, id: u64| Ok(audio::stop(id)))?,
    )?;

    globals.set(
        "stop_all_sounds",
        lua.create_function(|_, ()| {
            audio::stop_all();
            Ok(())
        })?,
    )?;

    globals.set(
        "set_bus_volume",
        lua.create_function(|_, (bus, db): (String, f32)| {
            audio::update_bus(&bus, |b| b.gain_db = db.clamp(cubed_mixer::MIN_DB, 24.0))
                .map_err(mlua::Error::RuntimeError)
        })?,
    )?;

    globals.set(
        "set_bus_muted",
        lua.create_function(|_, (bus, muted): (String, bool)| {
            audio::update_bus(&bus, |b| b.mute = muted).map_err(mlua::Error::RuntimeError)
        })?,
    )?;

    globals.set(
        "set_master_volume",
        lua.create_function(|_, volume: f32| {
            audio::set_master_volume(volume);
            Ok(())
        })?,
    )?;

    Ok(())
}

/// An entity's position in the world.
fn entity_position(entity: hecs::Entity) -> LuaResult<[f32; 3]> {
    with_active_world(|world| {
        world
            .get::<&WorldTransform>(entity)
            .map(|t| t.0.translation.to_array())
            .map_err(|_| mlua::Error::RuntimeError("the entity has no position".into()))
    })
}

/// `{x, y, z}` or `{1, 2, 3}`.
fn vec3(table: &Table) -> LuaResult<[f32; 3]> {
    let named = (|| -> LuaResult<Option<[f32; 3]>> {
        Ok(
            match (
                table.get::<Option<f32>>("x")?,
                table.get::<Option<f32>>("y")?,
                table.get::<Option<f32>>("z")?,
            ) {
                (Some(x), Some(y), Some(z)) => Some([x, y, z]),
                _ => None,
            },
        )
    })()?;
    if let Some(v) = named {
        return Ok(v);
    }
    Ok([
        table.get::<f32>(1)?,
        table.get::<f32>(2)?,
        table.get::<f32>(3)?,
    ])
}

/// A setting by its name in saved scenes ("2d", "linear", ...).
fn parse<T: serde::de::DeserializeOwned>(value: &str, what: &str, choices: &str) -> LuaResult<T> {
    serde_json::from_value(serde_json::Value::from(value))
        .map_err(|_| mlua::Error::RuntimeError(format!("{what} must be {choices}, not {value:?}")))
}
