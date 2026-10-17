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
//! Sounds are mixed by `cubed-mixer` (from cubed-synth), so bus faders, mute and
//! solo apply to everything playing: natively in the audio device's callback, on
//! the web in an AudioWorklet running the same mixer as wasm. On the web, files are
//! fetched relative to the page and decoded by the browser, so the first play of a
//! file starts once it has loaded, and a missing or undecodable file is logged
//! rather than returned as an error. Browsers keep audio suspended until the user
//! first clicks or presses a key; sounds played before that start then. Without an
//! output device sounds are recorded in the history but not heard.

pub mod components;
pub mod edit;
mod layout;
mod mix;
#[cfg(not(target_arch = "wasm32"))]
mod native;
mod scene;
#[cfg(target_arch = "wasm32")]
mod web;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

pub use components::{AudioListener, AudioSource, RolloffSetting, SpatialSetting};
pub use layout::{BusLayout, MixerLayout, SpatialLayout, DEFAULT_BUS, LAYOUT_FILE, MASTER};

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
    /// Who loses when a bus runs out of voices: lower priorities first.
    pub priority: u8,
    /// Where it plays from; `None` plays it as it is.
    pub spatial: Option<SpatialOptions>,
}

/// Where a sound plays from, and how it fades with distance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpatialOptions {
    /// `Off` plays it as it is; `Auto` is 2D or 3D after the listener's camera.
    pub mode: SpatialSetting,
    pub position: [f32; 3],
    /// World units per second, for doppler.
    pub velocity: [f32; 3],
    pub min_distance: f32,
    pub max_distance: f32,
    pub rolloff: RolloffSetting,
    /// Scales the doppler shift; 0 turns it off for this sound.
    pub doppler: f32,
}

impl SpatialOptions {
    /// At `position`, with an `AudioSource`'s defaults.
    pub fn at(position: [f32; 3]) -> SpatialOptions {
        let defaults = AudioSource::default();
        SpatialOptions {
            mode: defaults.spatial,
            position,
            velocity: [0.0; 3],
            min_distance: defaults.min_distance,
            max_distance: defaults.max_distance,
            rolloff: defaults.rolloff,
            doppler: defaults.doppler,
        }
    }
}

impl PlayOptions {
    /// How an `AudioSource` at `position` plays.
    pub fn from_source(source: &AudioSource, position: [f32; 3]) -> PlayOptions {
        PlayOptions {
            volume: source.volume.max(0.0),
            pitch: source.pitch,
            looped: source.looped,
            bus: (!source.bus.is_empty()).then(|| source.bus.clone()),
            priority: source.priority,
            spatial: (source.spatial != SpatialSetting::Off).then_some(SpatialOptions {
                mode: source.spatial,
                position,
                velocity: [0.0; 3],
                min_distance: source.min_distance,
                max_distance: source.max_distance,
                rolloff: source.rolloff,
                doppler: source.doppler,
            }),
        }
    }
}

impl Default for PlayOptions {
    fn default() -> Self {
        Self {
            volume: 1.0,
            pitch: 1.0,
            looped: false,
            bus: None,
            priority: 128,
            spatial: None,
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
    /// The listener's camera is orthographic: `Auto` sounds play in 2D.
    listener_planar: bool,
    /// Play mode's listener, sources and entity-attached sounds.
    scene: scene::SceneAudio,
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
            listener_planar: false,
            scene: scene::SceneAudio::default(),
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
    with_audio(|audio| audio.play(path, options))
}

/// Like [`play`], and the sound follows `entity` as it moves (in play mode).
pub fn play_attached(
    path: &str,
    options: PlayOptions,
    entity: hecs::Entity,
) -> Result<u64, String> {
    let path = resolve_path(path)?;
    with_audio(|audio| {
        let position = options.spatial.map(|s| s.position).unwrap_or_default();
        let id = audio.play(path, options)?;
        audio.scene.attach(id, entity, position);
        Ok(id)
    })
}

/// Starts an entity's `AudioSource` at `position` (scripts' `play_audio_source`);
/// a spatial one follows the entity.
pub fn play_source(
    entity: hecs::Entity,
    source: &AudioSource,
    position: [f32; 3],
) -> Result<u64, String> {
    let path = resolve_path(&source.clip)?;
    with_audio(|audio| {
        let options = PlayOptions::from_source(source, position);
        let spatial = options.spatial.is_some();
        let id = audio.play(path, options)?;
        audio.scene.started(entity, id, spatial.then_some(position));
        Ok(id)
    })
}

/// Stops the sound an entity's `AudioSource` is playing. Returns whether it was.
pub fn stop_source(entity: hecs::Entity) -> bool {
    with_audio(|audio| match audio.scene.voice_of(entity) {
        Some(id) => audio.output.stop(id),
        None => false,
    })
}

/// Moves a playing spatial sound (with its velocity, for doppler).
pub fn move_sound(id: u64, position: [f32; 3], velocity: [f32; 3]) {
    with_audio(|audio| {
        audio.output.move_voice(id, position, velocity);
        audio.output.flush();
    });
}

/// Play mode's sound for this frame: the listener follows an `AudioListener`
/// (or `camera`), `AudioSource`s start, and sounds attached to entities follow
/// them.
pub(crate) fn update_scene(world: &hecs::World, camera: Option<hecs::Entity>, dt: f32) {
    // Nothing to do (and no device to open) for a scene without sound.
    let idle = AUDIO.with(|cell| cell.borrow().is_none())
        && world.query::<&AudioSource>().iter().next().is_none();
    if idle {
        return;
    }
    with_audio(|audio| scene::update(audio, world, camera, dt));
}

/// The range rings of the selected entities' `AudioSource`s, for the editor.
pub(crate) fn debug_geometry(
    world: &hecs::World,
    selected: &[hecs::Entity],
) -> Option<crate::renderer::debug_draw::DebugGeometry> {
    scene::debug_geometry(world, selected)
}

impl Audio {
    fn play(&mut self, path: PathBuf, options: PlayOptions) -> Result<u64, String> {
        let id = self.next_id;
        let spatial = self.spatial(options.spatial.as_ref());
        let audible = self
            .output
            .play(id, &path, &options, spatial, self.master_volume)?;
        self.next_id += 1;
        let time = self.started.elapsed().as_secs_f64();
        self.history.push_back(Played {
            id,
            path,
            options,
            time,
            audible,
        });
        while self.history.len() > HISTORY_LEN {
            self.history.pop_front();
        }
        Ok(id)
    }

    /// The mixer's placement for a sound: `Auto` becomes 2D or 3D after the
    /// listener's camera.
    fn spatial(&self, options: Option<&SpatialOptions>) -> Option<cubed_mixer::Spatial> {
        let o = options?;
        let mode = match o.mode {
            SpatialSetting::Off => return None,
            SpatialSetting::Planar => cubed_mixer::SpatialMode::Planar,
            SpatialSetting::Full => cubed_mixer::SpatialMode::Full,
            SpatialSetting::Auto if self.listener_planar => cubed_mixer::SpatialMode::Planar,
            SpatialSetting::Auto => cubed_mixer::SpatialMode::Full,
        };
        Some(cubed_mixer::Spatial {
            mode,
            position: o.position,
            velocity: o.velocity,
            min_distance: o.min_distance.max(0.001),
            max_distance: o.max_distance.max(o.min_distance),
            rolloff: match o.rolloff {
                RolloffSetting::Inverse => cubed_mixer::Rolloff::Inverse,
                RolloffSetting::Linear => cubed_mixer::Rolloff::Linear,
            },
            doppler: o.doppler.max(0.0),
        })
    }
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
            audio.scene.clear();
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
                    "spatial": played.options.spatial.map(|s| json!({
                        "mode": s.mode,
                        "position": s.position,
                    })),
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
