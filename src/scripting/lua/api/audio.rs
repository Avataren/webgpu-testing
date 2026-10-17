//! # Audio API
//!
//! Sound effects and music for game scripts. Paths are relative to the project (e.g.
//! `"content/audio/hit.wav"`) or absolute; wav, ogg, mp3 and flac are supported.
//! Sounds play on the project's mixer buses (`content/audio/mixer.json`): `sfx`
//! unless they name another, such as `music`, `ambience`, `ui` or `voice`.

use mlua::{Lua, Result as LuaResult, Table};

use crate::audio::{self, PlayOptions};

/// Registers the audio functions:
///
/// - `play_sound(path[, {volume, pitch, loop, bus}])` - start a sound, returns its id
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
