//! Web output: cubed-mixer in an AudioWorklet (cubed-synth's `cubed-worklet`).
//!
//! On first use the page creates an AudioContext, loads `pkg/cubed-worklet.js`
//! into its worklet and sends the processor `pkg/cubed_worklet.wasm`
//! (build_web.sh stages both next to the player). Sound files are fetched
//! relative to the page and decoded by the browser at the context's rate,
//! then sent to the worklet as PCM once; a play waits for its file to load.
//! Songs (AHX, A2M, SID) are fetched once and sent with each play, and the
//! worklet builds their players; effects are sent as a kind and parameters
//! and built there too. Commands go over the node's port as
//! `cubed_mixer::wire` bytes, in order with the clips, songs and effects,
//! and events and meters come back about every 20 ms.
//! Browsers keep audio suspended until the user first clicks or presses a
//! key; sounds played before that start then. If the worklet cannot load,
//! sounds are recorded but silent. The latest telemetry is also put in
//! `window.cubedAudio` (frame, voices, meters: 5 floats per bus slot), for
//! tests that drive the page.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use cubed_mixer::{
    wire, BusMeter, ClipId, Command, Effect, Event, Play, PlaySource, SourceId, VoiceId,
};
use js_sys::{Array, ArrayBuffer, Float32Array, Object, Reflect, Uint8Array};
use serde_json::{json, Value};
use wasm_bindgen::{closure::Closure, JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AudioBuffer, AudioContext, AudioContextState, AudioWorkletNode, AudioWorkletNodeOptions,
    MessageEvent, Response,
};

use super::layout::{BusLayout, MixerLayout};
use super::mix::{Ambience, Buses, MixOp};
use super::{PlayOptions, AUDIO};

/// The processor script and the mixer's wasm, relative to the page.
const WORKLET_SCRIPT: &str = "pkg/cubed-worklet.js";
const WORKLET_WASM: &str = "pkg/cubed_worklet.wasm";
/// Floats per bus in the worklet's meters.
const METER_FIELDS: usize = 5;
/// A stop fades out over this long, so it never clicks.
const STOP_FADE_SECONDS: f32 = 0.005;

/// A sound file, fetched and decoded once per path.
enum ClipState {
    Loading,
    Ready(ClipId),
    Failed(String),
}

/// A song file, fetched once per path.
enum SongState {
    Loading,
    Ready(ArrayBuffer),
    Failed(String),
}

/// A song play waiting for its file.
struct WaitingSong {
    path: PathBuf,
    format: u32,
    subsong: usize,
    looped: bool,
    play: PlaySource,
}

/// A message for the worklet, kept in order until its node exists.
enum Outgoing {
    Commands(Vec<u8>),
    /// A clip, song or effect.
    Message(JsValue),
}

/// What the worklet last reported.
#[derive(Default)]
struct Telemetry {
    frame: f64,
    voices: u32,
    rejected: u32,
    bad_batches: u32,
    meters: Vec<f32>,
}

/// The audio context and worklet (both started lazily), the clips sent to
/// it and the voices playing on it.
#[derive(Default)]
pub(super) struct Output {
    /// `None` until first used; `Some(None)` if no context could be created.
    context: Option<Option<AudioContext>>,
    /// The worklet's node, once it is loaded.
    node: Option<AudioWorkletNode>,
    /// The worklet could not be loaded: sounds stay silent.
    failed: bool,
    /// The worklet has its wasm running.
    ready: bool,
    /// Messages waiting for the node.
    queue: Vec<Outgoing>,
    /// Encoded commands not yet posted.
    commands: Vec<u8>,
    clips: HashMap<PathBuf, ClipState>,
    next_clip_id: u32,
    /// Plays waiting for their file to load, in order.
    waiting: Vec<(PathBuf, Play)>,
    songs: HashMap<PathBuf, SongState>,
    waiting_songs: Vec<WaitingSong>,
    /// Voices that may still be playing (or waiting to).
    voices: HashSet<u64>,
    buses: Buses,
    telemetry: Telemetry,
    pub(super) finished_captures: Vec<PathBuf>,
}

fn js_error(err: JsValue) -> String {
    err.as_string().unwrap_or_else(|| format!("{err:?}"))
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

async fn fetch_bytes(url: &str) -> Result<ArrayBuffer, String> {
    let window = web_sys::window().ok_or("no window")?;
    let response: Response = JsFuture::from(window.fetch_with_str(url))
        .await
        .and_then(|response| response.dyn_into())
        .map_err(|err| format!("cannot read {url}: {}", js_error(err)))?;
    if !response.ok() {
        return Err(format!("cannot read {url}: HTTP {}", response.status()));
    }
    JsFuture::from(response.array_buffer().map_err(js_error)?)
        .await
        .and_then(|data| data.dyn_into())
        .map_err(|err| format!("cannot read {url}: {}", js_error(err)))
}

async fn fetch_and_decode(context: &AudioContext, path: &Path) -> Result<AudioBuffer, String> {
    let url = crate::io::web_url(path)?;
    let data = fetch_bytes(&url)
        .await
        .map_err(|err| err.replace(&url, &path.display().to_string()))?;
    JsFuture::from(context.decode_audio_data(&data).map_err(js_error)?)
        .await
        .and_then(|buffer| buffer.dyn_into())
        .map_err(|err| format!("cannot decode {}: {}", path.display(), js_error(err)))
}

fn set(object: &Object, key: &str, value: impl Into<JsValue>) {
    let _ = Reflect::set(object, &key.into(), &value.into());
}

fn get(object: &JsValue, key: &str) -> JsValue {
    Reflect::get(object, &key.into()).unwrap_or(JsValue::UNDEFINED)
}

/// Loads the processor and its wasm and starts the node.
async fn start_worklet(context: &AudioContext) -> Result<AudioWorkletNode, String> {
    let worklet = context.audio_worklet().map_err(js_error)?;
    JsFuture::from(worklet.add_module(WORKLET_SCRIPT).map_err(js_error)?)
        .await
        .map_err(|err| format!("cannot load {WORKLET_SCRIPT}: {}", js_error(err)))?;
    let wasm = fetch_bytes(WORKLET_WASM).await?;

    let options = AudioWorkletNodeOptions::new();
    options.set_number_of_inputs(0);
    options.set_number_of_outputs(1);
    options.set_output_channel_count(&Array::of1(&JsValue::from(2)));
    let node =
        AudioWorkletNode::new_with_options(context, "cubed-mixer", &options).map_err(js_error)?;
    node.connect_with_audio_node(&context.destination())
        .map_err(js_error)?;
    let port = node.port().map_err(js_error)?;
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(|event: MessageEvent| {
        AUDIO.with(|cell| {
            if let Ok(mut cell) = cell.try_borrow_mut() {
                if let Some(audio) = cell.as_mut() {
                    audio.output.receive(&event.data());
                }
            }
        });
    });
    port.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    // The handler lives as long as the node, which is the page.
    on_message.forget();

    let seed = || (js_sys::Math::random() * 4_294_967_296.0) as u32;
    let init = Object::new();
    set(&init, "type", "init");
    set(&init, "wasm", wasm.clone());
    set(&init, "maxVoices", 0);
    set(&init, "maxBuses", 0);
    set(&init, "maxClips", 0);
    set(&init, "seedLo", seed());
    set(&init, "seedHi", seed());
    port.post_message_with_transferable(&init, &Array::of1(&wasm))
        .map_err(js_error)?;
    Ok(node)
}

impl Output {
    /// The context, created (with the worklet starting) on first use.
    fn context(&mut self) -> Option<AudioContext> {
        if self.context.is_none() {
            match AudioContext::new() {
                Ok(context) => {
                    resume_on_gesture(&context);
                    let started = context.clone();
                    wasm_bindgen_futures::spawn_local(async move {
                        let result = start_worklet(&started).await;
                        AUDIO.with(|cell| {
                            if let Some(audio) = cell.borrow_mut().as_mut() {
                                audio.output.worklet_started(result);
                            }
                        });
                    });
                    self.context = Some(Some(context));
                    // The mix's buses go first, queued until the node exists.
                    let ops = self.buses.all();
                    self.apply(ops);
                }
                Err(err) => {
                    log::warn!(target: "audio", "No audio output, sounds will be silent: {}", js_error(err));
                    self.context = Some(None);
                }
            }
        }
        self.context.clone().flatten()
    }

    fn worklet_started(&mut self, result: Result<AudioWorkletNode, String>) {
        match result {
            Ok(node) => {
                self.node = Some(node);
                self.flush();
                for message in std::mem::take(&mut self.queue) {
                    self.post(message);
                }
            }
            Err(err) => {
                log::warn!(target: "audio", "No audio mixer, sounds will be silent: {err}");
                self.failed = true;
                self.queue.clear();
                self.commands.clear();
                self.waiting.clear();
                self.waiting_songs.clear();
                self.voices.clear();
            }
        }
    }

    /// Takes a message from the worklet.
    fn receive(&mut self, data: &JsValue) {
        match get(data, "type").as_string().as_deref() {
            Some("telemetry") => {
                // For tests driving the page (web_playtest, CDP scripts).
                if let Some(window) = web_sys::window() {
                    let _ = Reflect::set(&window, &"cubedAudio".into(), data);
                }
                if let Ok(events) = get(data, "events").dyn_into::<ArrayBuffer>() {
                    let bytes = Uint8Array::new(&events).to_vec();
                    for event in wire::decode_events(&bytes) {
                        let (Event::Ended(id) | Event::Stolen(id) | Event::Rejected(id)) = event;
                        self.voices.remove(&id.0);
                    }
                }
                if let Ok(meters) = get(data, "meters").dyn_into::<Float32Array>() {
                    self.telemetry.meters = meters.to_vec();
                }
                let number = |key| get(data, key).as_f64().unwrap_or(0.0);
                self.telemetry.frame = number("frame");
                self.telemetry.voices = number("voices") as u32;
                self.telemetry.rejected = number("rejected") as u32;
                self.telemetry.bad_batches = number("badBatches") as u32;
            }
            Some("ready") => self.ready = true,
            Some("song") => {
                if get(data, "ok").as_bool() != Some(true) {
                    let message = get(data, "error").as_string().unwrap_or_default();
                    log::warn!(target: "audio", "cannot play the song: {message}");
                }
            }
            Some("error") => {
                let message = get(data, "message").as_string().unwrap_or_default();
                log::warn!(target: "audio", "audio worklet: {message}");
            }
            _ => {}
        }
    }

    fn send(&mut self, command: Command) {
        if !self.failed {
            wire::encode(&command, &mut self.commands);
        }
    }

    /// Layout changes, in order: effects go as their own messages.
    fn apply(&mut self, ops: Vec<MixOp>) {
        for op in ops {
            match op {
                MixOp::Command(command) => self.send(command),
                MixOp::Effect { bus, slot, effect } => {
                    // The commands before it (its bus) go first.
                    self.flush();
                    self.post(Outgoing::Message(effect_message(bus.0, slot, effect)));
                }
            }
        }
    }

    /// Changes for a mix that has started; one that starts later gets the
    /// whole mix then.
    fn apply_if_started(&mut self, ops: Vec<MixOp>) {
        if matches!(self.context, Some(Some(_))) {
            self.apply(ops);
            self.flush();
        }
    }

    /// Posts the commands sent so far, as one message.
    pub(super) fn flush(&mut self) {
        if !self.commands.is_empty() {
            let bytes = std::mem::take(&mut self.commands);
            self.post(Outgoing::Commands(bytes));
        }
    }

    fn post(&mut self, message: Outgoing) {
        if self.failed {
            return;
        }
        let Some(port) = self.node.as_ref().and_then(|node| node.port().ok()) else {
            self.queue.push(message);
            return;
        };
        let posted = match message {
            Outgoing::Commands(bytes) => {
                let buffer = Uint8Array::from(bytes.as_slice()).buffer();
                let object = Object::new();
                set(&object, "type", "commands");
                set(&object, "bytes", buffer.clone());
                port.post_message_with_transferable(&object, &Array::of1(&buffer))
            }
            Outgoing::Message(object) => port.post_message(&object),
        };
        if let Err(err) = posted {
            log::warn!(target: "audio", "cannot reach the audio worklet: {}", js_error(err));
        }
    }

    fn start_loading(&mut self, context: AudioContext, path: PathBuf) {
        self.clips.insert(path.clone(), ClipState::Loading);
        wasm_bindgen_futures::spawn_local(async move {
            let result = fetch_and_decode(&context, &path).await;
            AUDIO.with(|cell| {
                if let Some(audio) = cell.borrow_mut().as_mut() {
                    audio.output.loaded(path, result);
                }
            });
        });
    }

    /// Sends a loaded file to the worklet and starts the plays that waited
    /// for it, or drops them if it failed.
    fn loaded(&mut self, path: PathBuf, result: Result<AudioBuffer, String>) {
        let (now, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.waiting)
            .into_iter()
            .partition(|(p, _)| *p == path);
        self.waiting = later;
        let buffer = match result {
            Ok(buffer) => buffer,
            Err(err) => {
                log::warn!(target: "audio", "{err}");
                for (_, play) in now {
                    self.voices.remove(&play.voice.0);
                }
                self.clips.insert(path, ClipState::Failed(err));
                return;
            }
        };
        let id = ClipId(self.next_clip_id);
        self.next_clip_id += 1;
        // Commands sent before this clip go first.
        self.flush();
        self.post(Outgoing::Message(clip_message(id, &buffer)));
        self.clips.insert(path, ClipState::Ready(id));
        for (_, play) in now {
            if self.voices.contains(&play.voice.0) {
                self.send(Command::Play(Play { clip: id, ..play }));
            }
        }
        self.flush();
    }

    /// Returns whether the sound is audible (an audio context exists).
    /// The master volume rides on the master fader (`set_master_volume`).
    pub(super) fn play(
        &mut self,
        id: u64,
        path: &Path,
        options: &PlayOptions,
        spatial: Option<cubed_mixer::Spatial>,
        _master_volume: f32,
    ) -> Result<bool, String> {
        if let Some(ClipState::Failed(err)) = self.clips.get(path) {
            return Err(err.clone());
        }
        let bus = self.buses.for_sound(options.bus.as_deref())?;
        if let Some(format) = song_format(path) {
            return self.play_song(id, path, format, bus, options, spatial);
        }
        let Some(context) = self.context() else {
            return Ok(false);
        };
        if self.failed {
            return Ok(false);
        }
        let play = Play {
            gain: options.volume.max(0.0),
            pitch: options.pitch,
            looped: options.looped,
            priority: options.priority,
            spatial,
            ..Play::new(VoiceId(id), ClipId(0), bus)
        };
        self.voices.insert(id);
        match self.clips.get(path) {
            Some(ClipState::Ready(clip)) => {
                let clip = *clip;
                self.send(Command::Play(Play { clip, ..play }));
            }
            Some(ClipState::Loading) => self.waiting.push((path.to_path_buf(), play)),
            Some(ClipState::Failed(_)) | None => {
                self.waiting.push((path.to_path_buf(), play));
                self.start_loading(context, path.to_path_buf());
            }
        }
        self.flush();
        Ok(true)
    }

    fn play_song(
        &mut self,
        id: u64,
        path: &Path,
        format: u32,
        bus: cubed_mixer::BusId,
        options: &PlayOptions,
        spatial: Option<cubed_mixer::Spatial>,
    ) -> Result<bool, String> {
        if let Some(SongState::Failed(err)) = self.songs.get(path) {
            return Err(err.clone());
        }
        if self.context().is_none() || self.failed {
            return Ok(false);
        }
        let waiting = WaitingSong {
            path: path.to_path_buf(),
            format,
            subsong: options.subsong,
            looped: options.looped,
            play: PlaySource {
                gain: options.volume.max(0.0),
                priority: options.priority,
                spatial,
                ..PlaySource::new(VoiceId(id), SourceId(0), bus)
            },
        };
        self.voices.insert(id);
        match self.songs.get(path) {
            Some(SongState::Ready(bytes)) => {
                let bytes = bytes.clone();
                self.start_song(waiting, &bytes);
            }
            Some(SongState::Loading) => self.waiting_songs.push(waiting),
            Some(SongState::Failed(_)) | None => {
                self.waiting_songs.push(waiting);
                self.songs.insert(path.to_path_buf(), SongState::Loading);
                let path = path.to_path_buf();
                wasm_bindgen_futures::spawn_local(async move {
                    let result = match crate::io::web_url(&path) {
                        Ok(url) => fetch_bytes(&url).await,
                        Err(err) => Err(err),
                    };
                    AUDIO.with(|cell| {
                        if let Some(audio) = cell.borrow_mut().as_mut() {
                            audio.output.song_loaded(path, result);
                        }
                    });
                });
            }
        }
        Ok(true)
    }

    /// Sends the song's file (the worklet builds its player in source slot
    /// 0) and the play that takes it.
    fn start_song(&mut self, song: WaitingSong, bytes: &ArrayBuffer) {
        if !self.voices.contains(&song.play.voice.0) {
            return;
        }
        self.flush();
        let message = Object::new();
        set(&message, "type", "song");
        set(&message, "source", 0);
        set(&message, "format", song.format);
        set(&message, "bytes", bytes.slice(0));
        set(&message, "subsong", song.subsong as u32);
        set(&message, "looped", song.looped);
        self.post(Outgoing::Message(message.into()));
        self.send(Command::PlaySource(song.play));
        self.flush();
    }

    fn song_loaded(&mut self, path: PathBuf, result: Result<ArrayBuffer, String>) {
        let (now, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.waiting_songs)
            .into_iter()
            .partition(|s| s.path == path);
        self.waiting_songs = later;
        match result {
            Ok(bytes) => {
                for song in now {
                    self.start_song(song, &bytes);
                }
                self.songs.insert(path, SongState::Ready(bytes));
            }
            Err(err) => {
                log::warn!(target: "audio", "{err}");
                for song in now {
                    self.voices.remove(&song.play.voice.0);
                }
                self.songs.insert(path, SongState::Failed(err));
            }
        }
    }

    pub(super) fn set_listener(&mut self, listener: cubed_mixer::Listener) {
        if matches!(self.context, Some(Some(_))) {
            self.send(Command::SetListener(listener));
        }
    }

    /// Moves a voice; a play still waiting for its file starts from there.
    pub(super) fn move_voice(&mut self, id: u64, position: [f32; 3], velocity: [f32; 3]) {
        let waiting = self
            .waiting
            .iter_mut()
            .find(|(_, p)| p.voice.0 == id)
            .map(|(_, p)| &mut p.spatial)
            .or_else(|| {
                self.waiting_songs
                    .iter_mut()
                    .find(|s| s.play.voice.0 == id)
                    .map(|s| &mut s.play.spatial)
            });
        if let Some(spatial) = waiting {
            if let Some(spatial) = spatial {
                spatial.position = position;
                spatial.velocity = velocity;
            }
        } else if self.voices.contains(&id) {
            self.send(Command::SetVoicePosition {
                voice: VoiceId(id),
                position,
                velocity,
            });
        }
    }

    pub(super) fn is_playing(&self, id: u64) -> bool {
        self.voices.contains(&id)
    }

    fn stop_fade(&self) -> u32 {
        (STOP_FADE_SECONDS * self.sample_rate()) as u32
    }

    pub(super) fn stop(&mut self, id: u64) -> bool {
        let fade = self.stop_fade();
        self.stop_over(id, fade)
    }

    /// Fades the sound out over `seconds`, then stops it.
    pub(super) fn stop_after(&mut self, id: u64, seconds: f32) -> bool {
        let fade = ((seconds.max(0.0) * self.sample_rate()) as u32).max(self.stop_fade());
        self.stop_over(id, fade)
    }

    fn stop_over(&mut self, id: u64, frames: u32) -> bool {
        if !self.voices.contains(&id) {
            return false;
        }
        let waiting_clip = self.waiting.iter().position(|(_, p)| p.voice.0 == id);
        let waiting_song = self.waiting_songs.iter().position(|s| s.play.voice.0 == id);
        if waiting_clip.is_some() || waiting_song.is_some() {
            if let Some(i) = waiting_clip {
                self.waiting.remove(i);
            }
            if let Some(i) = waiting_song {
                self.waiting_songs.remove(i);
            }
            self.voices.remove(&id);
            return true;
        }
        self.send(Command::Stop(VoiceId(id), frames));
        self.flush();
        true
    }

    /// Glides a playing sound's volume to `volume` over `seconds`.
    pub(super) fn fade(&mut self, id: u64, volume: f32, seconds: f32) -> bool {
        if !self.voices.contains(&id) {
            return false;
        }
        let gain = volume.max(0.0);
        if let Some((_, play)) = self.waiting.iter_mut().find(|(_, p)| p.voice.0 == id) {
            play.gain = gain;
            return true;
        }
        if let Some(song) = self.waiting_songs.iter_mut().find(|s| s.play.voice.0 == id) {
            song.play.gain = gain;
            return true;
        }
        let frames = (seconds.max(0.0) * self.sample_rate()) as u32;
        self.send(Command::FadeVoice {
            voice: VoiceId(id),
            gain,
            frames,
        });
        self.flush();
        true
    }

    fn sample_rate(&self) -> f32 {
        match &self.context {
            Some(Some(context)) => context.sample_rate(),
            _ => 48_000.0,
        }
    }

    pub(super) fn stop_all(&mut self) {
        for (_, play) in self.waiting.drain(..) {
            self.voices.remove(&play.voice.0);
        }
        for song in self.waiting_songs.drain(..) {
            self.voices.remove(&song.play.voice.0);
        }
        if matches!(self.context, Some(Some(_))) {
            let fade = self.stop_fade();
            self.send(Command::StopAll(fade));
            self.flush();
        }
    }

    pub(super) fn set_master_volume(&mut self, volume: f32) {
        let ops = self.buses.set_master_volume(volume).into_iter().collect();
        self.apply_if_started(ops);
    }

    /// The audio volumes' effect at the listener.
    pub(super) fn set_ambience(&mut self, ambience: Ambience) {
        let ops = self.buses.set_ambience(ambience);
        self.apply_if_started(ops);
    }

    pub(super) fn playing(&mut self) -> Vec<u64> {
        let mut ids: Vec<u64> = self.voices.iter().copied().collect();
        ids.sort_unstable();
        ids
    }

    /// Events arrive by themselves on the web.
    pub(super) fn poll(&mut self) {}

    pub(super) fn apply_layout(&mut self, layout: MixerLayout) {
        let ops = self.buses.set_layout(layout);
        self.apply_if_started(ops);
    }

    pub(super) fn layout(&self) -> &MixerLayout {
        self.buses.layout()
    }

    pub(super) fn snapshots_mut(&mut self) -> &mut Vec<super::layout::SnapshotLayout> {
        self.buses.snapshots_mut()
    }

    pub(super) fn update_bus(
        &mut self,
        name: &str,
        change: impl FnOnce(&mut BusLayout),
    ) -> Result<(), String> {
        let ops = self.buses.update(name, change)?;
        self.apply_if_started(ops);
        Ok(())
    }

    pub(super) fn meter(&self, name: &str) -> Option<BusMeter> {
        let at = self.buses.slot(name)?.0 as usize * METER_FIELDS;
        let m = self.telemetry.meters.get(at..at + METER_FIELDS)?;
        (m[0] == 1.0).then(|| BusMeter {
            peak: [m[1], m[2]],
            rms: [m[3], m[4]],
        })
    }

    pub(super) fn status_json(&self) -> Value {
        let state = if self.failed {
            "failed"
        } else if self.ready {
            "running"
        } else if self.context.is_some() {
            "loading"
        } else {
            "not started yet"
        };
        let mut status = json!({
            "backend": "cubed-mixer (AudioWorklet)",
            "worklet": state,
        });
        if let Some(Some(context)) = &self.context {
            let rate = context.sample_rate();
            status["sample_rate"] = json!(rate);
            status["context"] = json!(format!("{:?}", context.state()).to_lowercase());
            status["seconds_rendered"] =
                json!((self.telemetry.frame / rate as f64 * 1000.0).round() / 1000.0);
            status["voices"] = json!(self.telemetry.voices);
            status["rejected_commands"] = json!(self.telemetry.rejected);
            status["bad_command_batches"] = json!(self.telemetry.bad_batches);
        }
        status
    }

    pub(super) fn capture(&mut self, _path: PathBuf, _seconds: f32) -> Result<(), String> {
        Err("recording the mix is not available on the web yet".into())
    }
}

/// The worklet's number for a song file's format (see `cw_prepare_song`).
fn song_format(path: &Path) -> Option<u32> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "ahx" | "hvl" => Some(0),
        "a2m" => Some(1),
        "asid" => Some(2),
        _ => None,
    }
}

/// `{type: 'effect', bus, slot, kind, params}`: kind -1 empties the slot.
fn effect_message(bus: u16, slot: u8, effect: Option<Effect>) -> JsValue {
    let object = Object::new();
    set(&object, "type", "effect");
    set(&object, "bus", bus);
    set(&object, "slot", slot);
    match effect {
        Some(effect) => {
            set(&object, "kind", effect.kind.index());
            set(
                &object,
                "params",
                Float32Array::from(effect.params.as_slice()),
            );
        }
        None => set(&object, "kind", -1),
    }
    object.into()
}

/// `{type: 'clip', id, sampleRate, left, right?}` with the buffer's channels.
fn clip_message(id: ClipId, buffer: &AudioBuffer) -> JsValue {
    let channel = |index: u32| -> JsValue {
        Reflect::get(buffer, &"getChannelData".into())
            .ok()
            .and_then(|f| f.dyn_into::<js_sys::Function>().ok())
            .and_then(|f| f.call1(buffer, &index.into()).ok())
            .unwrap_or(JsValue::UNDEFINED)
    };
    let object = Object::new();
    set(&object, "type", "clip");
    set(&object, "id", id.0);
    set(&object, "sampleRate", buffer.sample_rate());
    set(&object, "left", channel(0));
    if buffer.number_of_channels() >= 2 {
        set(&object, "right", channel(1));
    }
    object.into()
}
