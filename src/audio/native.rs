//! Native output: the cubed-mixer core in a cpal callback.
//!
//! Files are decoded to PCM with symphonia the first time they play and
//! sent to the mixer once; voices then refer to them by clip id. The mixer
//! reports finished voices as events, which keep [`Output::playing`] up to
//! date.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cubed_mixer::native::{HostConfig, HostStatus, NativeHost};
use cubed_mixer::{BusMeter, Capture, Clip, ClipId, Command, Event, Play, VoiceId};

use super::layout::{BusLayout, MixerLayout};
use super::mix::Buses;
use super::PlayOptions;

/// A stop fades out over this long, so it never clicks.
const STOP_FADE_SECONDS: f32 = 0.005;

/// A decoded file's mixer slot.
struct LoadedClip {
    id: ClipId,
}

/// A recording in progress: where to write it when the mixer hands it back.
struct PendingCapture {
    path: PathBuf,
}

/// The output device (opened lazily), the clips sent to it and the voices
/// playing on it.
#[derive(Default)]
pub(super) struct Output {
    /// `None` until first used; `Some(None)` if no device could be opened.
    host: Option<Option<NativeHost>>,
    /// Decoded clips by canonical path.
    clips: HashMap<PathBuf, LoadedClip>,
    /// Clips replaced on disk while voices still play them: removed from the
    /// mixer once their last voice ends.
    retired: HashSet<ClipId>,
    free_clip_ids: Vec<u32>,
    next_clip_id: u32,
    /// Voices that may still be playing, with their clip.
    voices: HashMap<u64, ClipId>,
    buses: Buses,
    capture: Option<PendingCapture>,
    /// Files the last recording was written to, newest last.
    pub(super) finished_captures: Vec<PathBuf>,
}

impl Output {
    fn host(&mut self) -> Option<&mut NativeHost> {
        if self.host.is_none() {
            let opened = match NativeHost::open(HostConfig::default()) {
                Ok(host) => {
                    log::info!(target: "audio", "Audio output: {} at {} Hz", host.device_name(), host.sample_rate());
                    Some(host)
                }
                Err(err) => {
                    log::warn!(target: "audio", "No audio output, sounds will be silent: {err}");
                    None
                }
            };
            self.host = Some(opened);
            for command in self.buses.all() {
                self.send(command);
            }
        }
        self.host.as_mut().and_then(Option::as_mut)
    }

    /// Sends a command; a full queue drops it (the mixer is minutes behind,
    /// or gone).
    fn send(&mut self, command: Command) {
        if let Some(host) = self.host() {
            if host.send(command).is_err() {
                log::warn!(target: "audio", "audio command queue full; a command was dropped");
            }
        }
    }

    /// Applies the mixer's events and frees what it handed back.
    pub(super) fn poll(&mut self) {
        let Some(Some(host)) = self.host.as_mut() else {
            return;
        };
        let mut ended = Vec::new();
        let mut captures = Vec::new();
        host.poll(
            |event| match event {
                Event::Ended(id) | Event::Stolen(id) | Event::Rejected(id) => ended.push(id.0),
            },
            |capture| captures.push(capture),
        );
        for id in ended {
            if let Some(clip) = self.voices.remove(&id) {
                if self.retired.contains(&clip) && !self.voices.values().any(|&c| c == clip) {
                    self.retired.remove(&clip);
                    self.send(Command::RemoveClip(clip));
                    self.free_clip_ids.push(clip.0);
                }
            }
        }
        for capture in captures {
            if let Some(pending) = self.capture.take() {
                match write_wav(&pending.path, &capture, self.sample_rate()) {
                    Ok(()) => self.finished_captures.push(pending.path),
                    Err(err) => log::warn!(target: "audio", "{err}"),
                }
            }
        }
    }

    /// Decodes the file (once) and returns its clip.
    fn clip(&mut self, path: &Path) -> Result<ClipId, String> {
        if let Some(clip) = self.clips.get(path) {
            return Ok(clip.id);
        }
        let bytes =
            std::fs::read(path).map_err(|err| format!("cannot read {}: {err}", path.display()))?;
        let extension = path.extension().and_then(|e| e.to_str());
        let clip = decode(bytes, extension)
            .map_err(|err| format!("cannot decode {}: {err}", path.display()))?;
        let id = ClipId(self.free_clip_ids.pop().unwrap_or_else(|| {
            self.next_clip_id += 1;
            self.next_clip_id - 1
        }));
        self.send(Command::AddClip(id, Arc::new(clip)));
        self.clips.insert(path.to_path_buf(), LoadedClip { id });
        Ok(id)
    }

    /// Forgets a file that changed on disk without cutting off its voices:
    /// its clip leaves the mixer when the last of them ends.
    pub(super) fn invalidate(&mut self, path: &Path) {
        let Some(clip) = self.clips.remove(path) else {
            return;
        };
        if self.voices.values().any(|&c| c == clip.id) {
            self.retired.insert(clip.id);
        } else {
            self.send(Command::RemoveClip(clip.id));
            self.free_clip_ids.push(clip.id.0);
        }
    }

    /// Returns whether the sound is audible (a device is open).
    /// The master volume rides on the master fader (`set_master_volume`).
    pub(super) fn play(
        &mut self,
        id: u64,
        path: &Path,
        options: &PlayOptions,
        _master_volume: f32,
    ) -> Result<bool, String> {
        self.poll();
        let bus = self.buses.for_sound(options.bus.as_deref())?;
        let clip = self.clip(path)?;
        if self.host().is_none() {
            return Ok(false);
        }
        self.voices.insert(id, clip);
        self.send(Command::Play(Play {
            gain: options.volume.max(0.0),
            pitch: options.pitch,
            looped: options.looped,
            ..Play::new(VoiceId(id), clip, bus)
        }));
        Ok(true)
    }

    fn stop_fade(&self) -> u32 {
        (STOP_FADE_SECONDS * self.sample_rate() as f32) as u32
    }

    fn sample_rate(&self) -> u32 {
        match &self.host {
            Some(Some(host)) => host.sample_rate(),
            _ => 48_000,
        }
    }

    pub(super) fn stop(&mut self, id: u64) -> bool {
        self.poll();
        if !self.voices.contains_key(&id) {
            return false;
        }
        let fade = self.stop_fade();
        self.send(Command::Stop(VoiceId(id), fade));
        true
    }

    pub(super) fn stop_all(&mut self) {
        if matches!(self.host, Some(Some(_))) {
            let fade = self.stop_fade();
            self.send(Command::StopAll(fade));
        }
        self.poll();
    }

    pub(super) fn playing(&mut self) -> Vec<u64> {
        self.poll();
        let mut ids: Vec<u64> = self.voices.keys().copied().collect();
        ids.sort_unstable();
        ids
    }

    /// Sends commands only to a mixer that is running: one that opens later
    /// gets the whole layout then.
    fn send_if_open(&mut self, commands: impl IntoIterator<Item = Command>) {
        if matches!(self.host, Some(Some(_))) {
            for command in commands {
                self.send(command);
            }
        }
    }

    /// Makes `layout` the mix: buses created, changed or removed to match.
    pub(super) fn apply_layout(&mut self, layout: MixerLayout) {
        let commands = self.buses.set_layout(layout);
        self.send_if_open(commands);
    }

    /// The gain scripts set on top of the master fader (linear).
    pub(super) fn set_master_volume(&mut self, volume: f32) {
        let command = self.buses.set_master_volume(volume);
        self.send_if_open(command);
    }

    pub(super) fn layout(&self) -> &MixerLayout {
        self.buses.layout()
    }

    /// Changes one bus in the layout and the mix.
    pub(super) fn update_bus(
        &mut self,
        name: &str,
        change: impl FnOnce(&mut BusLayout),
    ) -> Result<(), String> {
        let commands = self.buses.update(name, change)?;
        self.send_if_open(commands);
        Ok(())
    }

    pub(super) fn meter(&self, name: &str) -> Option<BusMeter> {
        let id = self.buses.slot(name)?;
        match &self.host {
            Some(Some(host)) => host.meter(id),
            _ => None,
        }
    }

    /// The device and the audio thread's timing and counters.
    pub(super) fn status_json(&self) -> serde_json::Value {
        let Some(Some(host)) = &self.host else {
            return serde_json::json!({
                "backend": "cubed-mixer",
                "device": if self.host.is_some() { "none" } else { "not opened yet" },
            });
        };
        let s: HostStatus = host.status();
        serde_json::json!({
            "backend": "cubed-mixer",
            "device": host.device_name(),
            "sample_rate": s.sample_rate,
            "seconds_rendered": (s.frame as f64 / s.sample_rate.max(1) as f64 * 1000.0).round() / 1000.0,
            "voices": s.voices,
            "callback_frames": s.callback_frames,
            "callback_us": (s.callback_us * 10.0).round() / 10.0,
            "callback_max_us": (s.callback_max_us * 10.0).round() / 10.0,
            "load": (s.load * 1000.0).round() / 1000.0,
            "rejected_commands": s.rejected_commands,
            "dropped_events": s.dropped_events,
            "freed_on_audio_thread": s.freed_on_audio_thread,
            "warnings": s.warnings,
            "failed": s.failed,
        })
    }

    #[cfg(test)]
    pub(super) fn has_clip(&self, path: &Path) -> bool {
        self.clips.contains_key(path)
    }

    /// Records the next `seconds` of the master output to a WAV at `path`.
    pub(super) fn capture(&mut self, path: PathBuf, seconds: f32) -> Result<(), String> {
        let rate = self
            .host()
            .map(|h| h.sample_rate())
            .ok_or("no audio output")?;
        let frames = (seconds.clamp(0.01, 600.0) * rate as f32) as usize;
        self.capture = Some(PendingCapture { path });
        self.send(Command::Capture(Box::new(Capture::new(frames))));
        Ok(())
    }
}

/// Decodes a wav, ogg/vorbis, mp3 or flac file to PCM.
fn decode(bytes: Vec<u8>, extension: Option<&str>) -> Result<Clip, String> {
    let (channels, sample_rate, samples) = decode_pcm(bytes, extension)?;
    Ok(Clip::from_interleaved(
        sample_rate as f32,
        channels,
        &samples,
    ))
}

/// Decodes a wav, ogg/vorbis, mp3 or flac file to interleaved samples:
/// `(channels, sample rate, samples)`.
pub(crate) fn decode_pcm(
    bytes: Vec<u8>,
    extension: Option<&str>,
) -> Result<(usize, u32, Vec<f32>), String> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::errors::Error;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let stream = MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes)), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = extension {
        hint.with_extension(extension);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| e.to_string())?;
    let mut format = probed.format;
    let track = format.default_track().ok_or("no audio track")?;
    let track_id = track.id;
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or("unknown sample rate")?;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| e.to_string())?;

    let mut samples = Vec::new();
    let mut channels = 0;
    let mut buffer: Option<SampleBuffer<f32>> = None;
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(Error::ResetRequired) => break,
            Err(e) => return Err(e.to_string()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            // A damaged packet: skip it, as players do.
            Err(Error::DecodeError(_)) => continue,
            Err(e) => return Err(e.to_string()),
        };
        let spec = *decoded.spec();
        channels = spec.channels.count();
        let capacity = decoded.capacity() as u64;
        let fits = buffer
            .as_ref()
            .is_some_and(|b| b.capacity() as u64 >= capacity * channels as u64);
        if !fits {
            buffer = Some(SampleBuffer::new(capacity, spec));
        }
        let buffer = buffer.as_mut().expect("set above");
        buffer.copy_interleaved_ref(decoded);
        samples.extend_from_slice(buffer.samples());
    }
    if channels == 0 || samples.is_empty() {
        return Err("no audio in the file".into());
    }
    Ok((channels, sample_rate, samples))
}

/// Writes a recording as a 32-bit float stereo WAV.
fn write_wav(path: &Path, capture: &Capture, sample_rate: u32) -> Result<(), String> {
    let frames = capture.filled;
    let mut bytes = Vec::with_capacity(44 + frames * 8);
    let data_len = (frames * 8) as u32;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * 8).to_le_bytes());
    bytes.extend_from_slice(&8u16.to_le_bytes());
    bytes.extend_from_slice(&32u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..frames {
        bytes.extend_from_slice(&capture.left[i].to_le_bytes());
        bytes.extend_from_slice(&capture.right[i].to_le_bytes());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))
}
