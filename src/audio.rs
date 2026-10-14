//! Sound playback for games.
//!
//! Scripts call `play_sound` (see the Lua guide); remote clients use `audio.play`.
//! The output device is opened on the first sound, so applications that never play
//! anything never touch the audio system. Decoded files are cached by path. Every
//! play is also recorded in a short history, which lets automated playtests check
//! which sounds a game played without hearing them.
//!
//! On the web, sounds go through the Web Audio API: files are fetched relative to
//! the page and decoded by the browser, so the first play of a file starts once it
//! has loaded, and a missing or undecodable file is logged rather than returned as an
//! error. Browsers keep audio suspended until the user first clicks or presses a
//! key; sounds played before that start then. Without an output device sounds are
//! recorded in the history but not heard.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

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
    output: Output,
}

impl Audio {
    fn new() -> Self {
        Self {
            started: crate::time::Instant::now(),
            next_id: 1,
            master_volume: 1.0,
            history: VecDeque::new(),
            output: Output::default(),
        }
    }
}

thread_local! {
    static AUDIO: RefCell<Option<Audio>> = const { RefCell::new(None) };
}

fn with_audio<R>(f: impl FnOnce(&mut Audio) -> R) -> R {
    AUDIO.with(|cell| f(cell.borrow_mut().get_or_insert_with(Audio::new)))
}

/// Forget a generated/replaced file without opening a device or interrupting voices.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn invalidate_file(path: &Path) {
    AUDIO.with(|cell| {
        if let Some(audio) = cell.borrow_mut().as_mut() {
            // resolve_path canonicalizes existing files before putting them in the cache.
            audio
                .output
                .files
                .remove(&path.canonicalize().unwrap_or_else(|_| path.to_path_buf()));
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
/// Fails only when the file cannot be found or decoded (on the web, only once a
/// failed load is known). Without an output device the sound is recorded but silent.
pub fn play(path: &str, options: PlayOptions) -> Result<u64, String> {
    let path = resolve_path(path)?;
    with_audio(|audio| {
        let id = audio.next_id;
        let audible = audio.output.play(id, &path, options, audio.master_volume)?;
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

/// Set the gain applied to every sound, including those already playing.
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
use native::Output;
#[cfg(target_arch = "wasm32")]
use web::Output;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use rodio::{Decoder, MixerDeviceSink, Player, Source};
    use std::io::Cursor;
    use std::sync::Arc;

    /// The output device (opened lazily) and the sounds playing on it.
    #[derive(Default)]
    pub(super) struct Output {
        /// `None` until first used; `Some(None)` if no device could be opened.
        device: Option<Option<MixerDeviceSink>>,
        pub(super) files: HashMap<PathBuf, Arc<[u8]>>,
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

#[cfg(target_arch = "wasm32")]
mod web {
    use super::*;
    use js_sys::ArrayBuffer;
    use wasm_bindgen::{closure::Closure, JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{
        AudioBuffer, AudioBufferSourceNode, AudioContext, AudioContextState,
        AudioScheduledSourceNode, GainNode, Response,
    };

    /// A sound file, fetched and decoded once per path.
    enum Clip {
        Loading,
        Ready(AudioBuffer),
        Failed(String),
    }

    struct Voice {
        id: u64,
        path: PathBuf,
        options: PlayOptions,
        gain: GainNode,
        /// `None` while the clip is still loading.
        source: Option<AudioBufferSourceNode>,
        /// Context time at which the sound ends; infinite for loops and pending sounds.
        ends_at: f64,
    }

    impl Voice {
        fn silence(&self) {
            if let Some(source) = &self.source {
                let _ = AudioScheduledSourceNode::stop(source);
            }
            let _ = self.gain.disconnect();
        }
    }

    #[derive(Clone)]
    struct Device {
        context: AudioContext,
        master: GainNode,
    }

    /// The audio context (opened lazily), the loaded clips and the sounds playing.
    #[derive(Default)]
    pub(super) struct Output {
        /// `None` until first used; `Some(None)` if no context could be created.
        device: Option<Option<Device>>,
        clips: HashMap<PathBuf, Clip>,
        voices: Vec<Voice>,
    }

    fn js_error(err: JsValue) -> String {
        err.as_string().unwrap_or_else(|| format!("{err:?}"))
    }

    fn open_device() -> Result<Device, JsValue> {
        let context = AudioContext::new()?;
        let master = context.create_gain()?;
        master.connect_with_audio_node(&context.destination())?;
        resume_on_gesture(&context);
        Ok(Device { context, master })
    }

    /// Browsers keep a page's audio suspended until the user interacts with it, so
    /// resume the context on clicks, taps and key presses.
    fn resume_on_gesture(context: &AudioContext) {
        if context.state() == AudioContextState::Running {
            return;
        }
        let Some(window) = web_sys::window() else {
            return;
        };
        let context = context.clone();
        let resume = Closure::<dyn FnMut()>::new(move || {
            if context.state() == AudioContextState::Suspended {
                let _ = context.resume();
            }
        });
        for event in ["pointerdown", "keydown", "touchend"] {
            let _ = window.add_event_listener_with_callback(event, resume.as_ref().unchecked_ref());
        }
        // The listeners live as long as the page.
        resume.forget();
    }

    fn start(
        context: &AudioContext,
        voice: &mut Voice,
        buffer: &AudioBuffer,
    ) -> Result<(), String> {
        let source = context.create_buffer_source().map_err(js_error)?;
        source.set_buffer(Some(buffer));
        let speed = voice.options.pitch.max(0.01);
        source.playback_rate().set_value(speed);
        source.set_loop(voice.options.looped);
        source
            .connect_with_audio_node(&voice.gain)
            .map_err(js_error)?;
        source.start().map_err(js_error)?;
        if !voice.options.looped {
            // While the context is suspended its clock stands still, and so does the sound.
            voice.ends_at = context.current_time() + buffer.duration() / speed as f64;
        }
        voice.source = Some(source);
        Ok(())
    }

    async fn fetch_and_decode(context: &AudioContext, path: &Path) -> Result<AudioBuffer, String> {
        let url = crate::io::web_url(path)?;
        let window = web_sys::window().ok_or("no window")?;
        let response: Response = JsFuture::from(window.fetch_with_str(&url))
            .await
            .and_then(|response| response.dyn_into())
            .map_err(|err| format!("cannot read {}: {}", path.display(), js_error(err)))?;
        if !response.ok() {
            return Err(format!(
                "cannot read {}: HTTP {}",
                path.display(),
                response.status()
            ));
        }
        let data: ArrayBuffer = JsFuture::from(response.array_buffer().map_err(js_error)?)
            .await
            .and_then(|data| data.dyn_into())
            .map_err(|err| format!("cannot read {}: {}", path.display(), js_error(err)))?;
        JsFuture::from(context.decode_audio_data(&data).map_err(js_error)?)
            .await
            .and_then(|buffer| buffer.dyn_into())
            .map_err(|err| format!("cannot decode {}: {}", path.display(), js_error(err)))
    }

    fn load(context: AudioContext, path: PathBuf) {
        wasm_bindgen_futures::spawn_local(async move {
            let result = fetch_and_decode(&context, &path).await;
            AUDIO.with(|cell| {
                if let Some(audio) = cell.borrow_mut().as_mut() {
                    audio.output.loaded(&context, path, result);
                }
            });
        });
    }

    impl Output {
        fn device(&mut self) -> Option<Device> {
            self.device
                .get_or_insert_with(|| match open_device() {
                    Ok(device) => Some(device),
                    Err(err) => {
                        log::warn!(target: "audio", "No audio output, sounds will be silent: {}", js_error(err));
                        None
                    }
                })
                .clone()
        }

        /// Start the sounds that waited for `path` to load, or drop them if it failed.
        fn loaded(
            &mut self,
            context: &AudioContext,
            path: PathBuf,
            result: Result<AudioBuffer, String>,
        ) {
            let clip = match result {
                Ok(buffer) => Clip::Ready(buffer),
                Err(err) => {
                    log::warn!(target: "audio", "{err}");
                    Clip::Failed(err)
                }
            };
            self.voices.retain_mut(|voice| {
                if voice.source.is_some() || voice.path != path {
                    return true;
                }
                let started = match &clip {
                    Clip::Ready(buffer) => match start(context, voice, buffer) {
                        Ok(()) => true,
                        Err(err) => {
                            log::warn!(target: "audio", "cannot play {}: {err}", path.display());
                            false
                        }
                    },
                    _ => false,
                };
                if !started {
                    voice.silence();
                }
                started
            });
            self.clips.insert(path, clip);
        }

        /// Forget sounds that have finished.
        fn prune(&mut self) {
            let Some(Some(device)) = &self.device else {
                return;
            };
            let now = device.context.current_time();
            self.voices.retain(|voice| {
                let playing = voice.ends_at > now;
                if !playing {
                    voice.silence();
                }
                playing
            });
        }

        /// Returns whether the sound is audible (an audio context exists).
        pub(super) fn play(
            &mut self,
            id: u64,
            path: &Path,
            options: PlayOptions,
            master_volume: f32,
        ) -> Result<bool, String> {
            if let Some(Clip::Failed(err)) = self.clips.get(path) {
                return Err(err.clone());
            }
            self.prune();
            let Some(device) = self.device() else {
                return Ok(false);
            };
            device.master.gain().set_value(master_volume);
            let gain = device.context.create_gain().map_err(js_error)?;
            gain.gain().set_value(options.volume);
            gain.connect_with_audio_node(&device.master)
                .map_err(js_error)?;
            let mut voice = Voice {
                id,
                path: path.to_path_buf(),
                options,
                gain,
                source: None,
                ends_at: f64::INFINITY,
            };
            match self.clips.get(path) {
                Some(Clip::Ready(buffer)) => {
                    if let Err(err) = start(&device.context, &mut voice, buffer) {
                        voice.silence();
                        return Err(format!("cannot play {}: {err}", path.display()));
                    }
                }
                Some(Clip::Loading) => {}
                Some(Clip::Failed(_)) | None => {
                    self.clips.insert(path.to_path_buf(), Clip::Loading);
                    load(device.context.clone(), path.to_path_buf());
                }
            }
            self.voices.push(voice);
            Ok(true)
        }

        pub(super) fn stop(&mut self, id: u64) -> bool {
            self.prune();
            let Some(index) = self.voices.iter().position(|voice| voice.id == id) else {
                return false;
            };
            self.voices.remove(index).silence();
            true
        }

        pub(super) fn stop_all(&mut self) {
            for voice in self.voices.drain(..) {
                voice.silence();
            }
        }

        pub(super) fn set_master_volume(&mut self, master_volume: f32) {
            if let Some(Some(device)) = &self.device {
                device.master.gain().set_value(master_volume);
            }
        }

        pub(super) fn playing(&mut self) -> Vec<u64> {
            self.prune();
            self.voices.iter().map(|voice| voice.id).collect()
        }
    }
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
        std::fs::write(&path, [1]).unwrap();
        let canonical = path.canonicalize().unwrap();
        with_audio(|audio| {
            audio
                .output
                .files
                .insert(canonical.clone(), std::sync::Arc::from([1u8]));
            audio
                .output
                .files
                .insert(other.clone(), std::sync::Arc::from([2u8]));
        });
        invalidate_file(&path);
        with_audio(|audio| {
            assert!(!audio.output.files.contains_key(&canonical));
            assert!(audio.output.files.contains_key(&other));
            assert!(audio.output.playing().is_empty());
        });
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
