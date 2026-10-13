//! # Audio API
//!
//! Sound effects and music for game scripts. Paths are relative to the project (e.g.
//! `"content/audio/hit.wav"`) or absolute; wav, ogg, mp3 and flac are supported.

use mlua::{Lua, Result as LuaResult, Table};

use crate::audio::{self, PlayOptions};

/// Registers the audio functions:
///
/// - `play_sound(path[, {volume, pitch, loop}])` - start a sound, returns its id
/// - `stop_sound(id)` - stop one sound
/// - `stop_all_sounds()` - stop everything
/// - `set_master_volume(volume)` - gain applied to every sound
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
            }
            audio::play(&path, play).map_err(mlua::Error::RuntimeError)
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
        "set_master_volume",
        lua.create_function(|_, volume: f32| {
            audio::set_master_volume(volume);
            Ok(())
        })?,
    )?;

    Ok(())
}
