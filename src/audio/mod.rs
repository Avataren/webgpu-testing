//! Sound playback for games, through a mixer of named buses.
//!
//! Scripts call `play_sound` (see the Lua guide); remote clients use `audio.play`.
//! Every sound plays on a bus: `sfx` unless it names another. A project's buses
//! (master, music, sfx, ambience, ui, voice by default) and their faders live in
//! `content/audio/mixer.json`, edited in the editor's mixer window.
//!
//! The output device is opened on the first sound, so applications that never play
//! anything never touch the audio system. Decoded files are cached by path. Every
//! play is also recorded in a short history, which lets automated playtests check
//! which sounds a game played without hearing them.
//!
//! Natively, sounds are mixed by `cubed-mixer` in the audio device's callback, so
//! bus faders, mute and solo apply to everything playing. On the web, sounds still
//! go through the Web Audio API: files are fetched relative to the page and decoded
//! by the browser, so the first play of a file starts once it has loaded, and a
//! missing or undecodable file is logged rather than returned as an error. Browsers
//! keep audio suspended until the user first clicks or presses a key; sounds played
//! before that start then. The web does not apply bus settings yet. Without an
//! output device sounds are recorded in the history but not heard.

mod layout;
#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

use std::cell::RefCell;
#[cfg(target_arch = "wasm32")]
use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

pub use layout::{BusLayout, MixerLayout, DEFAULT_BUS, LAYOUT_FILE, MASTER};

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::decode_pcm;
#[cfg(not(target_arch = "wasm32"))]
use native::Output;
#[cfg(target_arch = "wasm32")]
use web::Output;

/// How many recent plays [`history`] remembers.
const HISTORY_LEN: usize = 64;

/// How a sound is played.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayOptions {
    /// Linear gain; 1 is the file's own level.
    pub volume: f32,
    /// Playback speed, which also shifts the pitch; 1 is unchanged.
    pub pitch: f32,
    /// Repeat until stopped.
    pub looped: bool,
    /// The bus to play on; `None` is [`DEFAULT_BUS`].
    pub bus: Option<String>,
}

impl Default for PlayOptions {
    fn default() -> Self {
        Self {
            volume: 1.0,
            pitch: 1.0,
            looped: false,
            bus: None,
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
    output: Output,
    /// The project whose bus layout is applied (`Some(None)`: no project).
    layout_root: Option<Option<PathBuf>>,
}

impl Audio {
    fn new() -> Self {
        Self {
            started: crate::time::Instant::now(),
            next_id: 1,
            master_volume: 1.0,
            history: VecDeque::new(),
            output: Output::default(),
            layout_root: None,
        }
    }

    /// Loads the open project's bus layout if it is not the one applied.
    fn follow_project(&mut self) {
        let root = crate::project::active_project_root();
        if self.layout_root.as_ref() != Some(&root) {
            self.output.apply_layout(MixerLayout::load(root.as_deref()));
            self.layout_root = Some(root);
        }
    }
}

thread_local! {
    static AUDIO: RefCell<Option<Audio>> = const { RefCell::new(None) };
}

fn with_audio<R>(f: impl FnOnce(&mut Audio) -> R) -> R {
    AUDIO.with(|cell| {
        let mut cell = cell.borrow_mut();
        let audio = cell.get_or_insert_with(Audio::new);
        audio.follow_project();
        f(audio)
    })
}

/// Forget a generated/replaced file without opening a device or interrupting voices.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn invalidate_file(path: &Path) {
    AUDIO.with(|cell| {
        if let Some(audio) = cell.borrow_mut().as_mut() {
            // resolve_path canonicalizes existing files before putting them in the cache.
            audio
                .output
                .invalidate(&path.canonicalize().unwrap_or_else(|_| path.to_path_buf()));
        }
    });
}

/// Resolve a sound path: absolute, relative to the working directory, to the open
/// project, or to its content folder.
pub fn resolve_path(path: &str) -> Result<PathBuf, String> {
    let raw = Path::new(path);
    if cfg!(target_arch = "wasm32") {
        // The web has no filesystem to search; the path is fetched relative to the
        // page, where the project is served.
        return Ok(raw.to_path_buf());
    }
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
/// Fails when the file cannot be found or decoded (on the web, only once a failed
/// load is known), or names a bus that does not exist. Without an output device the
/// sound is recorded but silent.
pub fn play(path: &str, options: PlayOptions) -> Result<u64, String> {
    let path = resolve_path(path)?;
    with_audio(|audio| {
        let id = audio.next_id;
        let audible = audio
            .output
            .play(id, &path, &options, audio.master_volume)?;
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

/// Stop one sound (with a few milliseconds' fade). Returns whether it was still playing.
pub fn stop(id: u64) -> bool {
    with_audio(|audio| audio.output.stop(id))
}

/// Stop every sound, e.g. when play mode ends.
pub fn stop_all() {
    // Nothing to stop if audio was never used; don't open a device just for this.
    AUDIO.with(|cell| {
        if let Some(audio) = cell.borrow_mut().as_mut() {
            audio.output.stop_all();
        }
    });
}

/// Set a gain applied to every sound, including those already playing, on top of
/// the master bus's fader.
pub fn set_master_volume(volume: f32) {
    with_audio(|audio| {
        audio.master_volume = volume.max(0.0);
        audio.output.set_master_volume(audio.master_volume);
    });
}

/// Ids of sounds that are still playing.
pub fn playing() -> Vec<u64> {
    AUDIO.with(|cell| {
        cell.borrow_mut()
            .as_mut()
            .map(|audio| audio.output.playing())
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
                    "bus": played.options.bus.as_deref().unwrap_or(DEFAULT_BUS),
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

/// A bus as the mixer window and `audio.status` show it.
#[derive(Clone, Debug, PartialEq)]
pub struct BusState {
    pub layout: BusLayout,
    /// The output level after the fader, per channel, as linear amplitudes:
    /// the slowly falling peak and the RMS over the last ~300 ms. `None`
    /// before the device opens, and on the web.
    pub peak: Option<[f32; 2]>,
    pub rms: Option<[f32; 2]>,
}

/// The buses of the open project's mix, master first.
pub fn buses() -> Vec<BusState> {
    with_audio(|audio| {
        audio.output.poll();
        audio
            .output
            .layout()
            .buses
            .iter()
            .map(|bus| {
                let meter = audio.output.meter(&bus.name);
                BusState {
                    layout: bus.clone(),
                    peak: meter.map(|m| m.peak),
                    rms: meter.map(|m| m.rms),
                }
            })
            .collect()
    })
}

/// Changes a bus's fader (dB), balance, mute or solo for what is playing and
/// everything played after. [`save_layout`] keeps the change in the project.
pub fn update_bus(name: &str, change: impl FnOnce(&mut BusLayout)) -> Result<(), String> {
    with_audio(|audio| audio.output.update_bus(name, change))
}

/// Writes the mix's buses to the open project's `content/audio/mixer.json`.
pub fn save_layout() -> Result<PathBuf, String> {
    let root = crate::project::active_project_root().ok_or("no project is open")?;
    with_audio(|audio| audio.output.layout().save(&root))
}

/// The mixer's device, timing and buses as JSON, for `audio.status`.
pub fn status() -> Value {
    let buses: Vec<Value> = buses()
        .into_iter()
        .map(|bus| {
            json!({
                "name": bus.layout.name,
                "output": bus.layout.output.as_deref().unwrap_or(if bus.layout.name == MASTER { "" } else { MASTER }),
                "gain_db": bus.layout.gain_db,
                "pan": bus.layout.pan,
                "mute": bus.layout.mute,
                "solo": bus.layout.solo,
                "peak_db": bus.peak.map(|p| [to_db(p[0]), to_db(p[1])]),
                "rms_db": bus.rms.map(|r| [to_db(r[0]), to_db(r[1])]),
            })
        })
        .collect();
    with_audio(|audio| {
        let mut status = audio.output.status_json();
        status["buses"] = Value::from(buses);
        status["master_volume"] = json!(audio.master_volume);
        status
    })
}

/// A linear amplitude in dBFS, rounded to 0.1 dB; silence is -120.
fn to_db(amplitude: f32) -> f32 {
    if amplitude <= 1e-6 {
        -120.0
    } else {
        (20.0 * amplitude.log10() * 10.0).round() / 10.0
    }
}

/// Records the next `seconds` of the mixed output to a 32-bit float WAV at
/// `path` (relative paths are under the open project). The file appears once
/// the time has passed; [`captures`] lists the finished ones.
pub fn capture(path: &str, seconds: f32) -> Result<PathBuf, String> {
    let raw = Path::new(path);
    let path = match crate::project::active_project_root() {
        Some(root) if raw.is_relative() => root.join(raw),
        _ => raw.to_path_buf(),
    };
    with_audio(|audio| audio.output.capture(path.clone(), seconds))?;
    Ok(path)
}

/// Recordings written since the last call, oldest first.
pub fn captures() -> Vec<PathBuf> {
    with_audio(|audio| {
        audio.output.poll();
        std::mem::take(&mut audio.output.finished_captures)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn invalidating_a_replaced_file_clears_only_that_cached_clip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("generated.wav");
        let other = dir.path().join("other.wav");
        write_test_wav(&path);
        write_test_wav(&other);
        let canonical = path.canonicalize().unwrap();
        let other = other.canonicalize().unwrap();
        play(canonical.to_str().unwrap(), PlayOptions::default()).unwrap();
        play(other.to_str().unwrap(), PlayOptions::default()).unwrap();
        invalidate_file(&path);
        with_audio(|audio| {
            assert!(!audio.output.has_clip(&canonical));
            assert!(audio.output.has_clip(&other));
        });
        stop_all();
    }

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
        assert_eq!(history[0]["bus"], DEFAULT_BUS);
        stop_all();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn unknown_buses_are_an_error_and_known_ones_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("beep.wav");
        write_test_wav(&path);
        let options = PlayOptions {
            bus: Some("nope".into()),
            ..PlayOptions::default()
        };
        assert!(play(path.to_str().unwrap(), options).is_err());
        update_bus("music", |bus| bus.gain_db = -6.0).unwrap();
        let music = buses()
            .into_iter()
            .find(|b| b.layout.name == "music")
            .unwrap();
        assert_eq!(music.layout.gain_db, -6.0);
        assert!(update_bus("nope", |_| {}).is_err());
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
