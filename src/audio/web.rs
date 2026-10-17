//! Web output: the Web Audio API, a source and gain node per sound. Bus
//! settings are kept but not applied yet; the AudioWorklet mixer replaces this.

use super::*;
use js_sys::ArrayBuffer;
use wasm_bindgen::{closure::Closure, JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AudioBuffer, AudioBufferSourceNode, AudioContext, AudioContextState, AudioScheduledSourceNode,
    GainNode, Response,
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
    /// The project's buses: shown and saved, not applied yet on the web.
    layout: MixerLayout,
    pub(super) finished_captures: Vec<PathBuf>,
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

fn start(context: &AudioContext, voice: &mut Voice, buffer: &AudioBuffer) -> Result<(), String> {
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
        options: &PlayOptions,
        master_volume: f32,
    ) -> Result<bool, String> {
        if let Some(bus) = options.bus.as_deref() {
            if self.layout.bus(bus).is_none() {
                return Err(format!("no audio bus named \"{bus}\""));
            }
        }
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
            options: options.clone(),
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

    pub(super) fn poll(&mut self) {
        self.prune();
    }

    pub(super) fn apply_layout(&mut self, layout: MixerLayout) {
        self.layout = layout;
    }

    pub(super) fn layout(&self) -> &MixerLayout {
        &self.layout
    }

    pub(super) fn update_bus(
        &mut self,
        name: &str,
        change: impl FnOnce(&mut BusLayout),
    ) -> Result<(), String> {
        let bus = self
            .layout
            .bus_mut(name)
            .ok_or_else(|| format!("no audio bus named \"{name}\""))?;
        change(bus);
        Ok(())
    }

    pub(super) fn meter(&self, _bus: &str) -> Option<cubed_mixer::BusMeter> {
        None
    }

    pub(super) fn status_json(&self) -> Value {
        json!({ "backend": "web-audio", "voices": self.voices.len() })
    }

    pub(super) fn capture(&mut self, _path: PathBuf, _seconds: f32) -> Result<(), String> {
        Err("recording the mix is not available on the web yet".into())
    }
}
