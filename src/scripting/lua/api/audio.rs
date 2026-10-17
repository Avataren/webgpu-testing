//! # Audio API
//!
//! Sound effects and music for game scripts. Paths are relative to the project (e.g.
//! `"content/audio/hit.wav"`) or absolute; wav, ogg, mp3 and flac are supported, and
//! chip-music songs (AHX/HVL, A2M, SID `.asid`) play as they are. Sounds play on the
//! project's mixer buses (`content/audio/mixer.json`): `sfx` unless they name another,
//! such as `music`, `ambience`, `ui` or `voice`.

use mlua::{Lua, Result as LuaResult, Table, Value};

use crate::audio::{self, AudioSource, MusicOptions, PlayOptions, SpatialOptions};
use crate::scene::components::WorldTransform;
use crate::scripting::lua::guards::{resolve_entity, with_active_world};

/// Registers the audio functions:
///
/// - `play_sound(path[, {volume, pitch, loop, bus, priority, subsong, stream, entity,
///   position, spatial, min_distance, max_distance, rolloff, doppler}])` - start a sound,
///   returns its id. `stream` decodes a long file as it plays (music does by itself).
///   With `entity` it plays from that entity and follows it; with `position` it
///   plays from there. `spatial` is "auto" (2D under an orthographic camera, else 3D),
///   "2d", "3d" or "off"; `rolloff` "inverse" or "linear"; `doppler` scales the pitch
///   shift of motion (0 = none).
/// - `move_sound(id, position[, velocity])` - move a positioned sound
/// - `play_audio_source(entity)` / `stop_audio_source(entity)` - start or stop an
///   entity's `AudioSource` (for one that does not play on start)
/// - `stop_sound(id[, fade])` - stop one sound, fading out over `fade` seconds
/// - `fade_sound(id, volume, seconds)` - glide a sound's volume
/// - `play_music(path[, {volume, bus, crossfade, loop, subsong}])` - play a track on the
///   `music` bus, looping, crossfading from the one before over `crossfade` seconds
///   (default 1); returns its id
/// - `stop_music([fade])` - fade the music out (default 1 second)
/// - `stop_all_sounds()` - stop everything
/// - `set_master_volume(volume)` - gain applied to every sound
/// - `set_bus_volume(bus, db)` - a bus's fader in dB (0 = unchanged, -80 = silent)
/// - `set_bus_muted(bus, muted)` - mute or unmute a bus
/// - `set_audio_snapshot(name[, fade])` - fade the mix to one of the project's
///   snapshots over `fade` seconds (default 0.5); `nil` fades back to none
/// - `set_bus_effect(bus, index, {param = value, ..., bypass})` - change a bus effect's
///   parameters (`index` 1 is the first effect), e.g. `set_bus_effect("music", 1,
///   {cutoff_hz = 600})` for a low-pass
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
                if let Some(subsong) = options.get::<Option<usize>>("subsong")? {
                    play.subsong = subsong;
                }
                if let Some(stream) = options.get::<Option<bool>>("stream")? {
                    play.stream = stream;
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
        lua.create_function(|_, (id, fade): (u64, Option<f32>)| {
            Ok(match fade {
                Some(seconds) => audio::stop_after(id, seconds),
                None => audio::stop(id),
            })
        })?,
    )?;

    globals.set(
        "fade_sound",
        lua.create_function(|_, (id, volume, seconds): (u64, f32, f32)| {
            Ok(audio::fade(id, volume, seconds))
        })?,
    )?;

    globals.set(
        "play_music",
        lua.create_function(|_, (path, options): (String, Option<Table>)| {
            let mut music = MusicOptions::default();
            if let Some(options) = options {
                if let Some(volume) = options.get::<Option<f32>>("volume")? {
                    music.volume = volume.max(0.0);
                }
                music.bus = options.get::<Option<String>>("bus")?;
                if let Some(crossfade) = options.get::<Option<f32>>("crossfade")? {
                    music.crossfade = crossfade.max(0.0);
                }
                if let Some(looped) = options.get::<Option<bool>>("loop")? {
                    music.looped = looped;
                }
                if let Some(subsong) = options.get::<Option<usize>>("subsong")? {
                    music.subsong = subsong;
                }
            }
            audio::play_music(&path, music).map_err(mlua::Error::RuntimeError)
        })?,
    )?;

    globals.set(
        "stop_music",
        lua.create_function(|_, fade: Option<f32>| Ok(audio::stop_music(fade.unwrap_or(1.0))))?,
    )?;

    globals.set(
        "set_audio_snapshot",
        lua.create_function(|_, (name, fade): (Option<String>, Option<f32>)| {
            audio::set_snapshot(name.as_deref(), fade.unwrap_or(0.5))
                .map_err(mlua::Error::RuntimeError)
        })?,
    )?;

    globals.set(
        "set_bus_effect",
        lua.create_function(|_, (bus, index, changes): (String, usize, Table)| {
            let mut problem = None;
            audio::update_bus(&bus, |b| {
                let Some(effect) = index.checked_sub(1).and_then(|i| b.effects.get_mut(i)) else {
                    problem = Some(format!("bus {bus} has no effect {index}"));
                    return;
                };
                let kind = cubed_mixer::EffectKind::from_name(&effect.kind);
                for pair in changes.pairs::<String, Value>() {
                    let Ok((name, value)) = pair else { continue };
                    match (name.as_str(), value) {
                        ("bypass", Value::Boolean(on)) => effect.bypass = on,
                        (name, Value::Number(v)) => {
                            set_param(effect, kind, name, v as f32, &mut problem)
                        }
                        (name, Value::Integer(v)) => {
                            set_param(effect, kind, name, v as f32, &mut problem)
                        }
                        (name, _) => problem = Some(format!("{name} must be a number")),
                    }
                }
            })
            .map_err(mlua::Error::RuntimeError)?;
            match problem {
                Some(problem) => Err(mlua::Error::RuntimeError(problem)),
                None => Ok(()),
            }
        })?,
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

/// Sets a parameter of a bus effect by name (clamped to its range).
fn set_param(
    effect: &mut audio::EffectLayout,
    kind: Option<cubed_mixer::EffectKind>,
    name: &str,
    value: f32,
    problem: &mut Option<String>,
) {
    let info = kind.and_then(|k| k.params().iter().find(|p| p.name == name));
    match info {
        Some(info) => {
            effect
                .params
                .insert(name.to_string(), value.clamp(info.min, info.max));
        }
        None => *problem = Some(format!("{} has no parameter {name}", effect.kind)),
    }
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
