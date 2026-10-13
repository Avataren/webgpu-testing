//! Sound playback for games.
//!
//! Scripts call `play_sound` (see the Lua guide); remote clients use `audio.play`.
//! The output device is opened on the first sound, so applications that never play
//! anything never touch the audio system. Decoded files are cached by path. Every
//! play is also recorded in a short history, which lets automated playtests check
//! which sounds a game played without hearing them.
//!
//! On the web, and when no output device is available, sounds are recorded in the
//! history but not heard.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};

/// How many recent plays [`history`] remembers.
const HISTORY_LEN: usize = 64;

/// How a sound is played.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayOptions {
    /// Linear gain; 1 is the file's own level.
    pub volume: f32,
    /// Playback speed, which also shifts the pitch; 1 is unchanged.
    pub pitch: f32,
    /// Repeat until stopped.
    pub looped: bool,
}

impl Default for PlayOptions {
    fn default() -> Self {
        Self {
            volume: 1.0,
            pitch: 1.0,
            looped: false,
        }
    }
}

/// One entry of the play history.
#[derive(Clone, Debug)]
struct Played {
    id: u64,
    path: PathBuf,
    options: PlayOptions,
    /// Seconds since the audio system was first used.
    time: f64,
    audible: bool,
}

struct Audio {
    started: crate::time::Instant,
    next_id: u64,
    master_volume: f32,
    history: VecDeque<Played>,
    #[cfg(not(target_arch = "wasm32"))]
    output: native::Output,
}

impl Audio {
    fn new() -> Self {
        Self {
            started: crate::time::Instant::now(),
            next_id: 1,
            master_volume: 1.0,
            history: VecDeque::new(),
            #[cfg(not(target_arch = "wasm32"))]
            output: native::Output::default(),
        }
    }
}

thread_local! {
    static AUDIO: RefCell<Option<Audio>> = const { RefCell::new(None) };
}

fn with_audio<R>(f: impl FnOnce(&mut Audio) -> R) -> R {
    AUDIO.with(|cell| f(cell.borrow_mut().get_or_insert_with(Audio::new)))
}

/// Resolve a sound path: absolute, relative to the working directory, to the open
/// project, or to its content folder.
pub fn resolve_path(path: &str) -> Result<PathBuf, String> {
    let raw = Path::new(path);
    let mut candidates = vec![raw.to_path_buf()];
    if raw.is_relative() {
        if let Some(root) = crate::project::active_project_root() {
            candidates.push(root.join(raw));
            candidates.push(root.join(crate::project::CONTENT_DIR).join(raw));
        }
    }
    candidates
        .iter()
        .find(|candidate| candidate.is_file())
        .map(|found| found.canonicalize().unwrap_or_else(|_| found.clone()))
        .ok_or_else(|| format!("sound file not found: {path}"))
}

/// Start playing the sound at `path` and return its id (for [`stop`]).
///
/// Fails only when the file cannot be found or decoded. Without an output device the
/// sound is recorded but silent.
pub fn play(path: &str, options: PlayOptions) -> Result<u64, String> {
    let path = resolve_path(path)?;
    with_audio(|audio| {
        let id = audio.next_id;
        #[cfg(not(target_arch = "wasm32"))]
        let audible = audio.output.play(id, &path, options, audio.master_volume)?;
        #[cfg(target_arch = "wasm32")]
        let audible = false;
        audio.next_id += 1;
        let time = audio.started.elapsed().as_secs_f64();
        audio.history.push_back(Played {
            id,
            path,
            options,
            time,
            audible,
        });
        while audio.history.len() > HISTORY_LEN {
            audio.history.pop_front();
        }
        Ok(id)
    })
}

/// Stop one sound. Returns whether it was still playing.
pub fn stop(id: u64) -> bool {
    with_audio(|audio| {
        #[cfg(not(target_arch = "wasm32"))]
        return audio.output.stop(id);
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (audio, id);
            false
        }
    })
}

/// Stop every sound, e.g. when play mode ends.
pub fn stop_all() {
    // Nothing to stop if audio was never used; don't open a device just for this.
    AUDIO.with(|cell| {
        if let Some(_audio) = cell.borrow_mut().as_mut() {
            #[cfg(not(target_arch = "wasm32"))]
            _audio.output.stop_all();
        }
    });
}

/// Set the gain applied to every sound, including those already playing.
pub fn set_master_volume(volume: f32) {
    with_audio(|audio| {
        audio.master_volume = volume.max(0.0);
        #[cfg(not(target_arch = "wasm32"))]
        audio.output.set_master_volume(audio.master_volume);
    });
}

/// Ids of sounds that are still playing.
pub fn playing() -> Vec<u64> {
    AUDIO.with(|cell| {
        cell.borrow_mut()
            .as_mut()
            .map(|_audio| {
                #[cfg(not(target_arch = "wasm32"))]
                return _audio.output.playing();
                #[cfg(target_arch = "wasm32")]
                Vec::new()
            })
            .unwrap_or_default()
    })
}

/// Sequence number the next play will get; pass it to [`history`] later to list only
/// what was played after this point.
pub fn next_id() -> u64 {
    AUDIO.with(|cell| cell.borrow().as_ref().map_or(1, |audio| audio.next_id))
}

/// Recent plays with an id of at least `since`, oldest first, as JSON.
pub fn history(since: u64) -> Value {
    AUDIO.with(|cell| {
        let cell = cell.borrow();
        let Some(audio) = cell.as_ref() else {
            return json!([]);
        };
        let project = crate::project::active_project_root();
        audio
            .history
            .iter()
            .filter(|played| played.id >= since)
            .map(|played| {
                let path = project
                    .as_ref()
                    .and_then(|root| played.path.strip_prefix(root).ok())
                    .unwrap_or(&played.path);
                json!({
                    "id": played.id,
                    "path": path.display().to_string(),
                    "volume": played.options.volume,
                    "pitch": played.options.pitch,
                    "loop": played.options.looped,
                    "time": (played.time * 1000.0).round() / 1000.0,
                    "audible": played.audible,
                })
            })
            .collect()
    })
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use rodio::{Decoder, MixerDeviceSink, Player, Source};
    use std::io::Cursor;

    /// The output device (opened lazily) and the sounds playing on it.
    #[derive(Default)]
    pub(super) struct Output {
        /// `None` until first used; `Some(None)` if no device could be opened.
        device: Option<Option<MixerDeviceSink>>,
        files: HashMap<PathBuf, Arc<[u8]>>,
        players: Vec<(u64, Player, f32)>,
    }

    impl Output {
        fn device(&mut self) -> Option<&MixerDeviceSink> {
            self.device
                .get_or_insert_with(|| match rodio::DeviceSinkBuilder::open_default_sink() {
                    Ok(mut sink) => {
                        sink.log_on_drop(false);
                        Some(sink)
                    }
                    Err(err) => {
                        log::warn!(target: "audio", "No audio output, sounds will be silent: {err}");
                        None
                    }
                })
                .as_ref()
        }

        fn bytes(&mut self, path: &Path) -> Result<Arc<[u8]>, String> {
            if let Some(bytes) = self.files.get(path) {
                return Ok(bytes.clone());
            }
            let bytes: Arc<[u8]> = std::fs::read(path)
                .map_err(|err| format!("cannot read {}: {err}", path.display()))?
                .into();
            self.files.insert(path.to_path_buf(), bytes.clone());
            Ok(bytes)
        }

        /// Returns whether the sound is audible (a device is open).
        pub(super) fn play(
            &mut self,
            id: u64,
            path: &Path,
            options: PlayOptions,
            master_volume: f32,
        ) -> Result<bool, String> {
            let bytes = self.bytes(path)?;
            let decoder = Decoder::try_from(Cursor::new(bytes))
                .map_err(|err| format!("cannot decode {}: {err}", path.display()))?;
            self.players.retain(|(_, player, _)| !player.empty());
            let Some(device) = self.device() else {
                return Ok(false);
            };
            let player = Player::connect_new(device.mixer());
            player.set_volume(options.volume * master_volume);
            player.set_speed(options.pitch.max(0.01));
            if options.looped {
                player.append(decoder.repeat_infinite());
            } else {
                player.append(decoder);
            }
            self.players.push((id, player, options.volume));
            Ok(true)
        }

        pub(super) fn stop(&mut self, id: u64) -> bool {
            let before = self.players.len();
            self.players.retain(|(player_id, player, _)| {
                let keep = *player_id != id;
                if !keep {
                    player.stop();
                }
                keep
            });
            self.players.len() != before
        }

        pub(super) fn stop_all(&mut self) {
            for (_, player, _) in self.players.drain(..) {
                player.stop();
            }
        }

        pub(super) fn set_master_volume(&mut self, master_volume: f32) {
            for (_, player, volume) in &self.players {
                player.set_volume(volume * master_volume);
            }
        }

        pub(super) fn playing(&mut self) -> Vec<u64> {
            self.players.retain(|(_, player, _)| !player.empty());
            self.players.iter().map(|(id, _, _)| *id).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_files_are_an_error() {
        assert!(play("does/not/exist.wav", PlayOptions::default()).is_err());
    }

    #[test]
    fn plays_are_recorded_even_without_a_device() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("beep.wav");
        write_test_wav(&path);
        let since = next_id();
        let id = play(path.to_str().unwrap(), PlayOptions::default()).unwrap();
        let history = history(since);
        assert_eq!(history.as_array().unwrap().len(), 1);
        assert_eq!(history[0]["id"], id);
        stop_all();
    }

    /// A 10 ms, 8 kHz, mono 16-bit silent WAV.
    fn write_test_wav(path: &Path) {
        let samples = 80u32;
        let data_len = samples * 2;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&8000u32.to_le_bytes());
        bytes.extend_from_slice(&16000u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        bytes.resize(bytes.len() + data_len as usize, 0);
        std::fs::write(path, bytes).unwrap();
    }
}
