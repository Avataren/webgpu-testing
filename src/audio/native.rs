//! Native output: the cubed-mixer core in a cpal callback.
//!
//! Files are decoded to PCM with symphonia the first time they play and
//! sent to the mixer once; voices then refer to them by clip id. Songs
//! (AHX, A2M, SID) are read and built into a player for each play, which
//! the mixer runs as a source, and streamed sounds (music) are decoded by a
//! thread of their own a couple of seconds ahead of playback. The mixer
//! reports finished voices as events, which keep [`Output::playing`] up to
//! date.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cubed_mixer::native::{HostConfig, HostStatus, NativeHost};
use cubed_mixer::songs::{open_song, SongFormat, SongOptions};
use cubed_mixer::{
    BusMeter, Capture, Clip, ClipId, Command, Event, MixerConfig, Play, PlaySource, SourceId,
    VoiceId,
};

use super::layout::{BusLayout, MixerLayout};
use super::mix::{Ambience, Buses, MixOp};
use super::PlayOptions;

/// A stop fades out over this long, so it never clicks.
const STOP_FADE_SECONDS: f32 = 0.005;
/// How far ahead of playback a stream is decoded.
const STREAM_AHEAD_SECONDS: f32 = 2.0;

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
    /// Voices that may still be playing, with their clip (`None`: a song).
    voices: HashMap<u64, Option<ClipId>>,
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
            let ops = self.buses.all();
            self.apply(ops);
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

    /// Sends layout changes: effects are built here, off the audio thread.
    fn apply(&mut self, ops: Vec<MixOp>) {
        let rate = self.sample_rate() as f32;
        for op in ops {
            match op {
                MixOp::Command(command) => self.send(command),
                MixOp::Effect { bus, slot, effect } => self.send(Command::SetInsert {
                    bus,
                    slot,
                    insert: effect.map(|e| e.build(rate, MixerConfig::default().max_block)),
                }),
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
            if let Some(Some(clip)) = self.voices.remove(&id) {
                if self.retired.contains(&clip) && !self.voices.values().any(|&c| c == Some(clip)) {
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
        if self.voices.values().any(|&c| c == Some(clip.id)) {
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
        spatial: Option<cubed_mixer::Spatial>,
        _master_volume: f32,
    ) -> Result<bool, String> {
        self.poll();
        let bus = self.buses.for_sound(options.bus.as_deref())?;
        if let Some(format) = song_format(path) {
            return self.play_song(id, path, format, bus, options, spatial);
        }
        if options.stream {
            return self.play_stream(id, path, bus, options, spatial);
        }
        let clip = self.clip(path)?;
        if self.host().is_none() {
            return Ok(false);
        }
        self.voices.insert(id, Some(clip));
        self.send(Command::Play(Play {
            gain: options.volume.max(0.0),
            pitch: options.pitch,
            looped: options.looped,
            priority: options.priority,
            spatial,
            ..Play::new(VoiceId(id), clip, bus)
        }));
        Ok(true)
    }

    /// A song: its player is built here and the mixer runs it.
    fn play_song(
        &mut self,
        id: u64,
        path: &Path,
        format: SongFormat,
        bus: cubed_mixer::BusId,
        options: &PlayOptions,
        spatial: Option<cubed_mixer::Spatial>,
    ) -> Result<bool, String> {
        let bytes =
            std::fs::read(path).map_err(|err| format!("cannot read {}: {err}", path.display()))?;
        let song_options = SongOptions {
            subsong: options.subsong,
            looped: options.looped,
        };
        // A song that cannot be read is an error, with or without a device.
        let rate = match self.host() {
            Some(host) => host.sample_rate() as f32,
            None => {
                open_song(format, &bytes, 48_000.0, song_options)
                    .map_err(|err| format!("cannot play {}: {err}", path.display()))?;
                return Ok(false);
            }
        };
        let song = open_song(format, &bytes, rate, song_options)
            .map_err(|err| format!("cannot play {}: {err}", path.display()))?;
        self.voices.insert(id, None);
        // One slot is enough: the play right after takes the song out of it.
        self.send(Command::PrepareSource(SourceId(0), song));
        self.send(Command::PlaySource(PlaySource {
            gain: options.volume.max(0.0),
            priority: options.priority,
            spatial,
            ..PlaySource::new(VoiceId(id), SourceId(0), bus)
        }));
        Ok(true)
    }

    /// A streamed file: a thread decodes it into the voice as it plays.
    fn play_stream(
        &mut self,
        id: u64,
        path: &Path,
        bus: cubed_mixer::BusId,
        options: &PlayOptions,
        spatial: Option<cubed_mixer::Spatial>,
    ) -> Result<bool, String> {
        let bytes: Arc<[u8]> = std::fs::read(path)
            .map_err(|err| format!("cannot read {}: {err}", path.display()))?
            .into();
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_string);
        let mut decoder = StreamDecoder::open(bytes, extension, options.looped)
            .map_err(|err| format!("cannot decode {}: {err}", path.display()))?;
        let Some(host) = self.host() else {
            return Ok(false);
        };
        let mixer_rate = host.sample_rate() as f32;
        let rate = decoder.sample_rate as f32;
        let (mut writer, stream) =
            cubed_mixer::stream::stream((STREAM_AHEAD_SECONDS * rate) as usize, rate, mixer_rate);
        // The first part decodes here, so the sound starts at once.
        let mut pending = Vec::new();
        let ended = decoder.fill(&mut writer, &mut pending, 0.5);
        let name = path.display().to_string();
        let spawned = std::thread::Builder::new()
            .name("audio-stream".into())
            .spawn(move || {
                if !ended {
                    decoder.run(&mut writer, pending, &name);
                }
                writer.finish();
            });
        if let Err(err) = spawned {
            return Err(format!("cannot start streaming {}: {err}", path.display()));
        }
        self.voices.insert(id, None);
        self.send(Command::PrepareSource(SourceId(0), Box::new(stream)));
        self.send(Command::PlaySource(PlaySource {
            gain: options.volume.max(0.0),
            priority: options.priority,
            spatial,
            ..PlaySource::new(VoiceId(id), SourceId(0), bus)
        }));
        Ok(true)
    }

    pub(super) fn set_listener(&mut self, listener: cubed_mixer::Listener) {
        self.send_if_open([Command::SetListener(listener)]);
    }

    pub(super) fn move_voice(&mut self, id: u64, position: [f32; 3], velocity: [f32; 3]) {
        if self.voices.contains_key(&id) {
            self.send_if_open([Command::SetVoicePosition {
                voice: VoiceId(id),
                position,
                velocity,
            }]);
        }
    }

    /// Whether the voice may still be playing (as of the last poll).
    pub(super) fn is_playing(&self, id: u64) -> bool {
        self.voices.contains_key(&id)
    }

    /// Commands go out as they are sent natively.
    pub(super) fn flush(&mut self) {
        self.poll();
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
        let fade = self.stop_fade();
        self.stop_over(id, fade)
    }

    /// Fades the sound out over `seconds`, then stops it.
    pub(super) fn stop_after(&mut self, id: u64, seconds: f32) -> bool {
        let fade = ((seconds.max(0.0) * self.sample_rate() as f32) as u32).max(self.stop_fade());
        self.stop_over(id, fade)
    }

    fn stop_over(&mut self, id: u64, frames: u32) -> bool {
        self.poll();
        if !self.voices.contains_key(&id) {
            return false;
        }
        self.send(Command::Stop(VoiceId(id), frames));
        true
    }

    /// Glides a playing sound's volume to `volume` over `seconds`.
    pub(super) fn fade(&mut self, id: u64, volume: f32, seconds: f32) -> bool {
        self.poll();
        if !self.voices.contains_key(&id) {
            return false;
        }
        let frames = (seconds.max(0.0) * self.sample_rate() as f32) as u32;
        self.send_if_open([Command::FadeVoice {
            voice: VoiceId(id),
            gain: volume.max(0.0),
            frames,
        }]);
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

    /// Sends changes only to a mixer that is running (one that opens later
    /// gets the whole mix then).
    fn apply_if_open(&mut self, ops: Vec<MixOp>) {
        if matches!(self.host, Some(Some(_))) {
            self.apply(ops);
        }
    }

    /// Makes `layout` the mix: buses created, changed or removed to match.
    pub(super) fn apply_layout(&mut self, layout: MixerLayout) {
        let ops = self.buses.set_layout(layout);
        self.apply_if_open(ops);
    }

    /// The gain scripts set on top of the master fader (linear).
    pub(super) fn set_master_volume(&mut self, volume: f32) {
        let ops = self.buses.set_master_volume(volume).into_iter().collect();
        self.apply_if_open(ops);
    }

    /// The audio volumes' effect at the listener.
    pub(super) fn set_ambience(&mut self, ambience: Ambience) {
        let ops = self.buses.set_ambience(ambience);
        self.apply_if_open(ops);
    }

    pub(super) fn layout(&self) -> &MixerLayout {
        self.buses.layout()
    }

    pub(super) fn snapshots_mut(&mut self) -> &mut Vec<super::layout::SnapshotLayout> {
        self.buses.snapshots_mut()
    }

    /// Changes one bus in the layout and the mix.
    pub(super) fn update_bus(
        &mut self,
        name: &str,
        change: impl FnOnce(&mut BusLayout),
    ) -> Result<(), String> {
        let ops = self.buses.update(name, change)?;
        self.apply_if_open(ops);
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

/// A compressed file decoded packet by packet into a stream, from its bytes in
/// memory (a few MB for minutes of music); looping starts it over.
struct StreamDecoder {
    bytes: Arc<[u8]>,
    extension: Option<String>,
    looped: bool,
    sample_rate: u32,
    format: Box<dyn symphonia::core::formats::FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    track: u32,
    buffer: Option<symphonia::core::audio::SampleBuffer<f32>>,
}

impl StreamDecoder {
    fn open(bytes: Arc<[u8]>, extension: Option<String>, looped: bool) -> Result<Self, String> {
        let (format, decoder, track, sample_rate) = Self::start(&bytes, extension.as_deref())?;
        Ok(Self {
            bytes,
            extension,
            looped,
            sample_rate,
            format,
            decoder,
            track,
            buffer: None,
        })
    }

    #[allow(clippy::type_complexity)]
    fn start(
        bytes: &Arc<[u8]>,
        extension: Option<&str>,
    ) -> Result<
        (
            Box<dyn symphonia::core::formats::FormatReader>,
            Box<dyn symphonia::core::codecs::Decoder>,
            u32,
            u32,
        ),
        String,
    > {
        use symphonia::core::codecs::DecoderOptions;
        use symphonia::core::formats::FormatOptions;
        use symphonia::core::io::MediaSourceStream;
        use symphonia::core::meta::MetadataOptions;
        use symphonia::core::probe::Hint;

        let source = std::io::Cursor::new(bytes.clone());
        let stream = MediaSourceStream::new(Box::new(source), Default::default());
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
        let format = probed.format;
        let track = format.default_track().ok_or("no audio track")?;
        let rate = track
            .codec_params
            .sample_rate
            .ok_or("unknown sample rate")?;
        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(|e| e.to_string())?;
        let track = track.id;
        Ok((format, decoder, track, rate))
    }

    /// The next packet's frames, interleaved stereo (mono doubled, beyond two
    /// channels the first two), appended to `out`. False at the end (a
    /// looped file starts over instead).
    fn next(&mut self, out: &mut Vec<f32>) -> Result<bool, String> {
        use symphonia::core::audio::SampleBuffer;
        use symphonia::core::errors::Error;
        loop {
            let packet = match self.format.next_packet() {
                Ok(packet) => packet,
                Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    if !self.looped {
                        return Ok(false);
                    }
                    let (format, decoder, track, _) =
                        Self::start(&self.bytes, self.extension.as_deref())?;
                    (self.format, self.decoder, self.track) = (format, decoder, track);
                    continue;
                }
                Err(Error::ResetRequired) => return Ok(false),
                Err(e) => return Err(e.to_string()),
            };
            if packet.track_id() != self.track {
                continue;
            }
            let decoded = match self.decoder.decode(&packet) {
                Ok(decoded) => decoded,
                Err(Error::DecodeError(_)) => continue,
                Err(e) => return Err(e.to_string()),
            };
            let spec = *decoded.spec();
            let channels = spec.channels.count().max(1);
            let capacity = decoded.capacity() as u64;
            if !self
                .buffer
                .as_ref()
                .is_some_and(|b| b.capacity() as u64 >= capacity * channels as u64)
            {
                self.buffer = Some(SampleBuffer::new(capacity, spec));
            }
            let buffer = self.buffer.as_mut().expect("set above");
            buffer.copy_interleaved_ref(decoded);
            for frame in buffer.samples().chunks(channels) {
                let left = frame[0];
                out.extend_from_slice(&[left, *frame.get(1).unwrap_or(&left)]);
            }
            return Ok(true);
        }
    }

    /// Decodes until the stream holds `seconds` or the file ends (true).
    fn fill(
        &mut self,
        writer: &mut cubed_mixer::stream::StreamWriter,
        pending: &mut Vec<f32>,
        seconds: f32,
    ) -> bool {
        let target = (seconds * self.sample_rate as f32) as usize;
        let mut pushed = 0;
        while pushed < target {
            if pending.is_empty() {
                match self.next(pending) {
                    Ok(true) => {}
                    Ok(false) | Err(_) => return true,
                }
            }
            let frames = writer.push(pending);
            pending.drain(..frames * 2);
            pushed += frames;
            if frames == 0 {
                break;
            }
        }
        false
    }

    /// Keeps the stream fed until the file ends or the voice does.
    fn run(
        &mut self,
        writer: &mut cubed_mixer::stream::StreamWriter,
        mut pending: Vec<f32>,
        name: &str,
    ) {
        loop {
            if writer.is_abandoned() {
                return;
            }
            if pending.is_empty() {
                match self.next(&mut pending) {
                    Ok(true) => {}
                    Ok(false) => return,
                    Err(err) => {
                        log::warn!(target: "audio", "{name}: {err}; the stream stops");
                        return;
                    }
                }
            }
            let frames = writer.push(&pending);
            pending.drain(..frames * 2);
            if !pending.is_empty() {
                // Full: playback takes a block every few milliseconds.
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    }
}

/// The song format a file's extension names.
fn song_format(path: &Path) -> Option<SongFormat> {
    SongFormat::from_extension(path.extension()?.to_str()?)
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
