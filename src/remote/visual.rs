//! Visual comparison captures.
//!
//! A [`PostProcessOverride`] changes the post-processing a frame is *rendered* with
//! without touching the scene, so there is nothing to undo afterwards. `visual.capture`
//! uses it to render one frozen view under several variants: for each variant it applies
//! the override, lets temporal anti-aliasing settle, then captures a few consecutive
//! frames and records what each one was rendered with (settings, TAA jitter index, frame
//! number, camera, scene time). The images land in a directory next to a `manifest.json`.
//!
//! Unless told not to, the capture also compares the frames as they arrive (see
//! [`super::visual_compare`]): each variant against the first, a variant's own consecutive
//! frames against each other (temporal anti-aliasing jitter), and the last frames rendered
//! while a variant settled (does temporal history converge in time?). That takes a readback
//! of each of those settle frames, which a capture without comparison does not need.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use image::RgbaImage;

use serde_json::{json, Map, Value};

use crate::app::{GpuUpdateContext, RuntimeMode};
use crate::renderer::postprocess::{AntiAliasing, PostProcessSettings};
use crate::renderer::TAA_JITTER_FRAMES;
use crate::scene::{Camera, Scene};

use super::handlers::post_process_patch;
use super::hub::{CaptureRequest, RemoteHub, Reply};
use super::json::{opt_str, patch_serde, vec3_json};
use super::visual_compare::{FrameKind, Options, Recorder};

/// The most variants, frames per variant and settle frames one capture may use.
const MAX_VARIANTS: usize = 16;
const MAX_FRAMES_PER_VARIANT: u64 = 32;
const MAX_SETTLE_FRAMES: u64 = 600;
const DEFAULT_SETTLE_FRAMES: u64 = 16;
const DEFAULT_FRAMES_PER_VARIANT: u64 = 2;
/// Comparison defaults: a 4x4 grid of regions, pixels differing by more than 4 levels, and
/// difference images amplified 16 times (3 levels show as 48).
const DEFAULT_GRID: u32 = 4;
const MAX_GRID: u32 = 16;
const DEFAULT_THRESHOLD: u64 = 4;
const DEFAULT_DIFF_GAIN: f64 = 16.0;
const MAX_DIFF_GAIN: f64 = 255.0;
/// How many of the last frames rendered while a variant settles are read back for the
/// settle check.
const SETTLE_TRACE_FRAMES: u64 = 16;
/// Frames a capture may wait for one image before giving up.
const CAPTURE_STALL_FRAMES: u64 = 600;

/// How the frames of a capture differ in post-processing from the scene's own settings.
///
/// Applied after the scene's volumes are blended at the camera each frame. Cloning it is
/// cheap enough to hand one to every frame.
#[derive(Clone, Debug, PartialEq)]
pub struct PostProcessOverride {
    /// Start from the engine defaults instead of the scene's blended settings.
    from_defaults: bool,
    /// Settings groups to change, in the layout of `postprocess_set`.
    patch: Value,
}

impl PostProcessOverride {
    /// An override changing the settings named in `patch` (the `postprocess_set` layout:
    /// `{"bloom": {"enabled": false}, "anti_aliasing": "none"}`). With `from_defaults` the
    /// rest comes from the engine defaults rather than the scene's volumes.
    pub fn new(patch: &Value, from_defaults: bool) -> Result<Self, String> {
        Ok(Self {
            from_defaults,
            patch: post_process_patch(patch)?,
        })
    }

    /// The settings to render with, given what the scene's volumes blend to.
    pub fn apply(&self, scene_settings: PostProcessSettings) -> PostProcessSettings {
        let base = if self.from_defaults {
            PostProcessSettings::default()
        } else {
            scene_settings
        };
        // `new` checked the patch against the settings, so this does not fail.
        patch_serde(&base, &self.patch).unwrap_or(base)
    }

    /// The override as JSON, for reports.
    pub fn describe(&self) -> Value {
        json!({
            "base": if self.from_defaults { "defaults" } else { "scene" },
            "settings": self.patch,
        })
    }
}

/// One post-processing configuration of a capture.
struct Variant {
    name: String,
    /// `None` renders with the scene's own post-processing.
    render_override: Option<PostProcessOverride>,
}

/// What a variant was actually rendered with, read back from the renderer.
struct VariantReport {
    name: String,
    description: Value,
    /// Effective settings of the first captured frame.
    settings: Option<Value>,
    /// Whether every captured frame of the variant used the same settings.
    settings_stable: bool,
    images: Vec<Value>,
}

/// An image requested from the renderer, waiting for its frame to be presented.
struct PendingImage {
    receiver: Receiver<Result<Value, String>>,
    /// The frame's pixels, when the capture compares frames.
    pixels: Option<Receiver<RgbaImage>>,
    kind: FrameKind,
    variant: usize,
    index: u64,
    /// Hub frame the image was rendered on.
    frame: u64,
    scene_time: f64,
    requested_at: u64,
}

/// What the capture has to do at the start of a frame.
#[derive(Debug, Default, PartialEq)]
struct Step {
    /// Apply this variant's override from this frame on.
    apply: Option<usize>,
    /// Capture this frame.
    capture: Option<Capture>,
    finished: bool,
}

/// A frame to read back.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Capture {
    variant: usize,
    kind: FrameKind,
    /// Which saved image of the variant it is; for a settle frame, how many frames after
    /// applying the variant it is.
    index: u64,
}

/// The order of a capture, independent of the renderer: variant after variant, `settle`
/// frames after applying each before `per_variant` consecutive captures. The last `trace`
/// of the settle frames are read back too (but not saved) for the settle check.
///
/// The first saved image of a variant waits (up to one jitter period, as further settle
/// frames) until the renderer says its frame is aligned: at the same temporal
/// anti-aliasing jitter phase as the reference's, so that identical settings give identical
/// frames rather than frames that differ by the phase.
#[derive(Debug)]
struct Sequence {
    variants: usize,
    settle: u64,
    per_variant: u64,
    trace: u64,
    current: usize,
    applied_at: Option<u64>,
    captured: u64,
    /// Frames the current variant has waited for alignment.
    waited: u64,
    /// The most frames a variant waits to be at the reference's jitter phase: one jitter
    /// period less one (upscaling lengthens it, see `Renderer::jitter_period`).
    max_wait: u64,
}

impl Sequence {
    fn new(variants: usize, settle: u64, per_variant: u64, trace: u64) -> Self {
        Self {
            variants,
            settle,
            per_variant,
            trace,
            current: 0,
            applied_at: None,
            captured: 0,
            waited: 0,
            max_wait: TAA_JITTER_FRAMES as u64 - 1,
        }
    }

    /// The step for the frame numbered `frame`, which is about to render. Call once per
    /// frame, after the frame requested by the previous step (if any) has completed;
    /// `previous_image_done` says whether that was one of the variant's saved images, and
    /// `aligned` whether this frame may be the variant's first saved image.
    fn step(&mut self, frame: u64, previous_image_done: bool, aligned: bool) -> Step {
        let mut step = Step::default();
        if previous_image_done {
            self.captured += 1;
            if self.captured == self.per_variant {
                self.current += 1;
                self.captured = 0;
                self.waited = 0;
                self.applied_at = None;
            }
        }
        if self.current >= self.variants {
            step.finished = true;
            return step;
        }
        let applied_at = *self.applied_at.get_or_insert_with(|| {
            step.apply = Some(self.current);
            frame
        });
        let age = frame - applied_at;
        let capture = |kind, index| Capture {
            variant: self.current,
            kind,
            index,
        };
        let waiting = self.captured == 0 && !aligned && self.waited < self.max_wait;
        if age >= self.settle && waiting {
            // More settle frames, read back like the last ones before them.
            self.waited += 1;
            if self.trace > 0 {
                step.capture = Some(capture(FrameKind::Settle, age));
            }
        } else if age >= self.settle {
            step.capture = Some(capture(FrameKind::Image, self.captured));
        } else if self.trace > 0 && age + self.trace >= self.settle {
            step.capture = Some(capture(FrameKind::Settle, age));
        }
        step
    }
}

/// A running `visual.capture`.
pub(crate) struct VisualCapture {
    reply: Option<Reply>,
    dir: PathBuf,
    camera_spec: Option<Value>,
    max_size: u32,
    variants: Vec<Variant>,
    sequence: Sequence,
    /// Compares the frames as they arrive; `None` when the capture does not compare.
    recorder: Option<Recorder>,
    /// The TAA jitter phase (index modulo the sequence length) of the reference's first
    /// saved image, once known and when it was rendered with TAA.
    target_phase: Option<u32>,
    /// The view every frame of the capture renders through, once resolved.
    view: Option<Camera>,
    reports: Vec<VariantReport>,
    pending: Option<PendingImage>,
    started_frame: u64,
    scene_time_at_start: f64,
    /// What to put back when the capture ends.
    previous_override: Option<PostProcessOverride>,
    previous_time_scale: Option<(RuntimeMode, f64)>,
    mode: RuntimeMode,
    time_scale: f64,
    manual: bool,
}

impl VisualCapture {
    /// The camera the capture renders through.
    pub(crate) fn view(&self) -> Option<Camera> {
        self.view
    }
}

/// Turn the `visual.capture` parameters into a capture, or say what is wrong with them.
pub(crate) fn parse(params: &Value, reply: Reply) -> Result<VisualCapture, (Reply, String)> {
    match parse_inner(params) {
        Ok(parsed) => Ok(VisualCapture {
            reply: Some(reply),
            ..parsed
        }),
        Err(message) => Err((reply, message)),
    }
}

fn parse_inner(params: &Value) -> Result<VisualCapture, String> {
    let settle = bounded(
        params,
        "settle_frames",
        DEFAULT_SETTLE_FRAMES,
        0,
        MAX_SETTLE_FRAMES,
    )?;
    let per_variant = bounded(
        params,
        "frames",
        DEFAULT_FRAMES_PER_VARIANT,
        1,
        MAX_FRAMES_PER_VARIANT,
    )?;
    let max_size = bounded(params, "max_size", 8192, 16, 8192)? as u32;
    let compare = flag(params, "compare", true)?;
    let settle_check = flag(params, "settle_check", true)?;
    let (cols, rows) = parse_grid(params)?;
    let threshold = bounded(params, "threshold", DEFAULT_THRESHOLD, 0, 255)? as u8;
    let gain = parse_gain(params)?;

    let mut variants = Vec::new();
    if params
        .get("include_baseline")
        .and_then(Value::as_bool)
        .unwrap_or(true)
    {
        variants.push(Variant {
            name: "baseline".into(),
            render_override: None,
        });
    }
    let given = match params.get("variants") {
        Some(Value::Array(given)) => given.as_slice(),
        Some(_) => return Err("variants must be an array of {name, postprocess}".into()),
        None => &[],
    };
    for (index, variant) in given.iter().enumerate() {
        let name = opt_str(variant, "name")
            .ok_or_else(|| format!("variant {index} needs a name (letters, digits, - and _)"))?;
        let base = opt_str(variant, "base").unwrap_or("scene");
        let from_defaults = match base {
            "scene" => false,
            "defaults" => true,
            other => {
                return Err(format!(
                    "variant {name:?}: base must be \"scene\" or \"defaults\", got {other:?}"
                ))
            }
        };
        let patch = variant
            .get("postprocess")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let render_override = PostProcessOverride::new(&patch, from_defaults)
            .map_err(|err| format!("variant {name:?}: {err}"))?;
        variants.push(Variant {
            name: name.to_owned(),
            render_override: Some(render_override),
        });
    }
    if variants.len() < 2 {
        return Err(
            "give at least one variant to compare with the baseline (variants: [{name, \
             postprocess}]), or at least two variants without the baseline"
                .into(),
        );
    }
    if variants.len() > MAX_VARIANTS {
        return Err(format!(
            "at most {MAX_VARIANTS} variants (including the baseline)"
        ));
    }
    for (index, variant) in variants.iter().enumerate() {
        let valid = !variant.name.is_empty()
            && variant
                .name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !valid {
            return Err(format!(
                "variant name {:?} must be letters, digits, - and _ (it names the image files)",
                variant.name
            ));
        }
        if variants[..index]
            .iter()
            .any(|other| other.name == variant.name)
        {
            return Err(format!("duplicate variant name {:?}", variant.name));
        }
    }

    let dir = match opt_str(params, "dir") {
        Some(dir) => PathBuf::from(dir),
        None => default_dir(),
    };
    let trace = if compare && settle_check {
        SETTLE_TRACE_FRAMES
    } else {
        0
    };
    let options = Options {
        cols,
        rows,
        threshold,
        gain,
        frames: per_variant,
        settle_frames: trace.min(settle),
    };
    let recorder = compare.then(|| {
        let names = variants
            .iter()
            .map(|variant| variant.name.clone())
            .collect();
        Recorder::new(options, names, Some(dir.clone()))
    });
    let reports = variants
        .iter()
        .map(|variant| VariantReport {
            name: variant.name.clone(),
            description: variant.render_override.as_ref().map_or_else(
                || json!({"base": "scene", "settings": {}}),
                |o| o.describe(),
            ),
            settings: None,
            settings_stable: true,
            images: Vec::new(),
        })
        .collect();
    Ok(VisualCapture {
        reply: None,
        dir,
        camera_spec: params.get("camera").filter(|v| !v.is_null()).cloned(),
        max_size,
        sequence: Sequence::new(variants.len(), settle, per_variant, trace),
        recorder,
        variants,
        target_phase: None,
        view: None,
        reports,
        pending: None,
        started_frame: 0,
        scene_time_at_start: 0.0,
        previous_override: None,
        previous_time_scale: None,
        mode: RuntimeMode::Editor,
        time_scale: 1.0,
        manual: false,
    })
}

fn bounded(params: &Value, key: &str, default: u64, min: u64, max: u64) -> Result<u64, String> {
    let Some(value) = params.get(key).filter(|value| !value.is_null()) else {
        return Ok(default);
    };
    value
        .as_u64()
        .filter(|value| (min..=max).contains(value))
        .ok_or_else(|| format!("{key} must be an integer from {min} to {max}"))
}

fn flag(params: &Value, key: &str, default: bool) -> Result<bool, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(format!("{key} must be true or false")),
    }
}

/// `grid`: a number for that many columns and rows, or `[columns, rows]`.
fn parse_grid(params: &Value) -> Result<(u32, u32), String> {
    let error = || format!("grid must be a number or [columns, rows], each from 1 to {MAX_GRID}");
    let side = |value: &Value| {
        value
            .as_u64()
            .filter(|side| (1..=u64::from(MAX_GRID)).contains(side))
            .map(|side| side as u32)
            .ok_or_else(error)
    };
    match params.get("grid") {
        None | Some(Value::Null) => Ok((DEFAULT_GRID, DEFAULT_GRID)),
        Some(Value::Array(pair)) if pair.len() == 2 => Ok((side(&pair[0])?, side(&pair[1])?)),
        Some(value) => side(value).map(|side| (side, side)),
    }
}

fn parse_gain(params: &Value) -> Result<f32, String> {
    match params.get("diff_gain") {
        None | Some(Value::Null) => Ok(DEFAULT_DIFF_GAIN as f32),
        Some(value) => value
            .as_f64()
            .filter(|gain| (1.0..=MAX_DIFF_GAIN).contains(gain))
            .map(|gain| gain as f32)
            .ok_or_else(|| format!("diff_gain must be a number from 1 to {MAX_DIFF_GAIN}")),
    }
}

fn default_dir() -> PathBuf {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    std::env::temp_dir()
        .join("engine-visual")
        .join(seconds.to_string())
}

impl RemoteHub {
    /// Start a visual capture, or answer why it cannot run.
    pub(super) fn begin_visual_capture(&mut self, params: &Value, reply: Reply) {
        if self.visual.is_some() {
            reply.send(Err("a visual capture is already running".into()));
            return;
        }
        if !self.captures.is_empty() {
            reply.send(Err("a screenshot is pending; try again".into()));
            return;
        }
        match parse(params, reply) {
            Ok(capture) => self.visual = Some(Box::new(capture)),
            Err((reply, message)) => reply.send(Err(message)),
        }
    }

    /// Advance a running capture by one frame. Call once per frame, before rendering.
    pub(super) fn step_visual_capture(&mut self, ctx: &mut GpuUpdateContext<'_>) {
        let Some(mut capture) = self.visual.take() else {
            return;
        };
        match capture.advance(self, ctx) {
            Ok(false) => self.visual = Some(capture),
            Ok(true) => {
                capture.restore(self);
                let result = capture.finish(self, ctx);
                if let Some(reply) = capture.reply.take() {
                    reply.send(result);
                }
            }
            Err(message) => {
                capture.restore(self);
                if let Some(reply) = capture.reply.take() {
                    reply.send(Err(message));
                }
            }
        }
    }
}

impl VisualCapture {
    /// Returns whether the capture is finished.
    fn advance(
        &mut self,
        hub: &mut RemoteHub,
        ctx: &mut GpuUpdateContext<'_>,
    ) -> Result<bool, String> {
        if self.view.is_none() {
            self.begin(hub, ctx)?;
        }

        // The frame requested last frame has been rendered and captured by now.
        let mut image_done = false;
        if let Some(pending) = &self.pending {
            match pending.receiver.try_recv() {
                Ok(result) => {
                    let pending = self.pending.take().expect("checked above");
                    image_done = pending.kind == FrameKind::Image;
                    self.accept(&pending, result?, ctx)?;
                }
                Err(TryRecvError::Empty) => {
                    if hub.frame().saturating_sub(pending.requested_at) > CAPTURE_STALL_FRAMES {
                        return Err("the engine stopped producing frames for the capture".into());
                    }
                    return Ok(false);
                }
                Err(TryRecvError::Disconnected) => {
                    return Err("a captured frame was dropped by the engine".into())
                }
            }
        }

        let aligned = self.jitter_aligned(ctx);
        self.sequence.max_wait = u64::from(ctx.renderer.jitter_period().saturating_sub(1));
        let step = self.sequence.step(hub.frame(), image_done, aligned);
        if step.finished {
            return Ok(true);
        }
        if let Some(index) = step.apply {
            hub.post_process_override = self.variants[index].render_override.clone();
        }
        if let Some(capture) = step.capture {
            self.request_image(hub, &ctx.scene, capture);
        }
        Ok(false)
    }

    /// Whether the frame about to render is at the jitter phase of the reference's first
    /// saved image. Always true when there is no such phase (the reference had no TAA) or
    /// the variant has no TAA, whose frames are not jittered.
    fn jitter_aligned(&self, ctx: &GpuUpdateContext<'_>) -> bool {
        let Some(target) = self.target_phase else {
            return true;
        };
        let renderer = &ctx.renderer;
        if renderer.postprocess_settings().anti_aliasing != AntiAliasing::Taa {
            return true;
        }
        renderer.jitter_index().wrapping_add(1) % renderer.jitter_period() == target
    }

    /// Resolve the view, freeze the simulation and remember what to restore.
    fn begin(&mut self, hub: &mut RemoteHub, ctx: &mut GpuUpdateContext<'_>) -> Result<(), String> {
        let scene: &Scene = &ctx.scene;
        self.view = Some(match &self.camera_spec {
            Some(spec) => super::hub::resolve_view(scene, spec)?,
            None => *scene.camera(),
        });
        self.started_frame = hub.frame();
        self.scene_time_at_start = scene.time();
        self.previous_override = hub.post_process_override.clone();
        if let Some(runtime) = hub.runtime_handle() {
            self.mode = runtime.active_mode();
            self.time_scale = runtime.time_scale();
            self.manual = runtime.manual_simulation();
            // Play mode runs at its time scale; zero freezes the game on this frame.
            if self.mode == RuntimeMode::Playing && !self.manual && self.time_scale != 0.0 {
                runtime.set_time_scale(0.0);
                self.previous_time_scale = Some((self.mode, self.time_scale));
            }
        }
        std::fs::create_dir_all(&self.dir)
            .map_err(|err| format!("failed to create {}: {err}", self.dir.display()))?;
        Ok(())
    }

    fn request_image(&mut self, hub: &mut RemoteHub, scene: &Scene, capture: Capture) {
        let Capture {
            variant,
            kind,
            index,
        } = capture;
        let (sender, receiver) = mpsc::channel();
        // Settle frames are only compared, never saved.
        let save_path = (kind == FrameKind::Image).then(|| {
            self.dir
                .join(format!("{}_{:02}.png", self.variants[variant].name, index))
        });
        let (pixel_sender, pixels) = match self.recorder {
            Some(_) => {
                let (sender, receiver) = mpsc::channel();
                (Some(sender), Some(receiver))
            }
            None => (None, None),
        };
        hub.captures.push(CaptureRequest {
            reply: Reply::new(sender),
            viewport_only: true,
            max_size: self.max_size,
            save_path,
            inline: false,
            camera: None,
            view: self.view,
            aspect: None,
            pixels: pixel_sender,
        });
        self.pending = Some(PendingImage {
            receiver,
            pixels,
            kind,
            variant,
            index,
            frame: hub.frame(),
            scene_time: scene.time(),
            requested_at: hub.frame(),
        });
    }

    /// Take a frame that has been rendered and captured: note what a saved image was
    /// rendered with, and hand its pixels to the comparison.
    fn accept(
        &mut self,
        pending: &PendingImage,
        saved: Value,
        ctx: &mut GpuUpdateContext<'_>,
    ) -> Result<(), String> {
        if pending.kind == FrameKind::Image {
            self.record_image(pending, saved, ctx)?;
        }
        let pixels = pending
            .pixels
            .as_ref()
            .and_then(|pixels| pixels.try_recv().ok());
        if let (Some(recorder), Some(pixels)) = (&mut self.recorder, pixels) {
            recorder.push(pending.variant, pending.kind, pending.frame, pixels);
        }
        Ok(())
    }

    /// Note what an image was rendered with. Runs on the frame after the capture, before
    /// anything renders, so the renderer still holds that frame's state.
    fn record_image(
        &mut self,
        pending: &PendingImage,
        saved: Value,
        ctx: &mut GpuUpdateContext<'_>,
    ) -> Result<(), String> {
        let settings = serde_json::to_value(ctx.renderer.postprocess_settings())
            .map_err(|err| err.to_string())?;
        if pending.variant == 0
            && pending.index == 0
            && ctx.renderer.postprocess_settings().anti_aliasing == AntiAliasing::Taa
        {
            self.target_phase = Some(ctx.renderer.jitter_index() % ctx.renderer.jitter_period());
        }
        let report = &mut self.reports[pending.variant];
        match &report.settings {
            None => report.settings = Some(settings),
            Some(first) if *first != settings => report.settings_stable = false,
            Some(_) => {}
        }
        let mut image = saved;
        if let Some(object) = image.as_object_mut() {
            object.insert("index".into(), json!(pending.index));
            object.insert("frame".into(), json!(pending.frame));
            object.insert("jitter_index".into(), json!(ctx.renderer.jitter_index()));
            object.insert("scene_time".into(), json!(pending.scene_time));
        }
        report.images.push(image);
        Ok(())
    }

    /// Put back the override and time scale the capture replaced.
    fn restore(&mut self, hub: &mut RemoteHub) {
        hub.post_process_override = self.previous_override.take();
        if let (Some((mode, scale)), Some(runtime)) =
            (self.previous_time_scale.take(), hub.runtime_handle())
        {
            // Stopping play resets the scale; do not reapply an old one to a new session.
            if runtime.active_mode() == mode {
                runtime.set_time_scale(scale);
            }
        }
    }

    fn finish(&mut self, hub: &RemoteHub, ctx: &mut GpuUpdateContext<'_>) -> Result<Value, String> {
        let scene: &Scene = &ctx.scene;
        let view = self.view.expect("resolved when the capture began");
        let baseline = self
            .reports
            .first()
            .and_then(|report| report.settings.clone());
        let scene_time_end = scene.time();
        let frozen = (scene_time_end - self.scene_time_at_start).abs() < 1e-9;
        let times: Vec<f64> = self
            .reports
            .iter()
            .flat_map(|report| report.images.iter())
            .filter_map(|image| image.get("scene_time").and_then(Value::as_f64))
            .collect();
        let scene_time_constant = times.windows(2).all(|pair| pair[0] == pair[1]);

        let mut warnings = Vec::new();
        if !scene_time_constant {
            warnings.push(
                "the scene was not frozen: its time advanced between images, so differences \
                 can come from the scene rather than the post-processing"
                    .to_string(),
            );
        }
        if self.mode == RuntimeMode::Editor {
            warnings.push(
                "edit mode: tool scripts and editor animation are not paused; check scene_time"
                    .into(),
            );
        }
        let unstable: Vec<&str> = self
            .reports
            .iter()
            .filter(|report| !report.settings_stable)
            .map(|report| report.name.as_str())
            .collect();
        if !unstable.is_empty() {
            warnings.push(format!(
                "post-processing changed between frames of variant(s) {}",
                unstable.join(", ")
            ));
        }

        let compared = self.recorder.take().map(Recorder::finish);
        if let Some(compared) = &compared {
            warnings.extend(compared.warnings.iter().cloned());
        }

        let mut variants = Vec::new();
        let mut manifest_variants = Vec::new();
        for (index, report) in self.reports.iter().enumerate() {
            let changed = match (&baseline, &report.settings) {
                (Some(base), Some(this)) => changed_settings(base, this),
                _ => Value::Object(Map::new()),
            };
            let summary = json!({
                "name": report.name,
                "override": report.description,
                "changed_from_baseline": changed,
                "settings_stable": report.settings_stable,
                "images": report.images,
            });
            let (mut summary, mut full) = (summary.clone(), summary);
            full["effective_settings"] = report.settings.clone().unwrap_or(Value::Null);
            if let Some((short, detailed)) = compared.as_ref().and_then(|c| c.variants.get(index)) {
                merge_entries(&mut summary, short);
                merge_entries(&mut full, detailed);
            }
            variants.push(summary);
            manifest_variants.push(full);
        }

        let manifest_path = self.dir.join("manifest.json");
        let simulation = json!({
            "mode": match self.mode {
                RuntimeMode::Editor => "editor",
                RuntimeMode::Playing => "playing",
            },
            "time_scale_before": self.time_scale,
            "manual": self.manual,
            "frozen_by_capture": self.mode == RuntimeMode::Playing,
            "scene_time_start": self.scene_time_at_start,
            "scene_time_end": scene_time_end,
            "scene_unchanged": frozen && scene_time_constant,
        });
        let mut report = json!({
            "dir": self.dir.display().to_string(),
            "manifest": manifest_path.display().to_string(),
            "baseline": self.variants.first().is_some_and(|v| v.render_override.is_none()).then_some("baseline"),
            "camera": camera_json(&view),
            "sequence": {
                "settle_frames": self.sequence.settle,
                "frames_per_variant": self.sequence.per_variant,
                "settle_frames_compared": self.sequence.trace.min(self.sequence.settle),
                "jitter_phase": self.target_phase,
                "engine_frames": hub.frame() - self.started_frame,
            },
            "simulation": simulation,
            "warnings": warnings,
            "variants": variants,
        });
        if let Some(compared) = &compared {
            report["compare"] = compared.options.clone();
        }
        let mut manifest = report.clone();
        manifest["variants"] = Value::Array(manifest_variants);
        write_manifest(&manifest_path, &manifest)?;
        Ok(report)
    }
}

/// Add the keys of the object `extra` to the object `target`.
fn merge_entries(target: &mut Value, extra: &Value) {
    if let (Some(target), Some(extra)) = (target.as_object_mut(), extra.as_object()) {
        target.extend(extra.clone());
    }
}

fn write_manifest(path: &Path, manifest: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(manifest).map_err(|err| err.to_string())?;
    std::fs::write(path, text).map_err(|err| format!("failed to write {}: {err}", path.display()))
}

fn camera_json(camera: &Camera) -> Value {
    json!({
        "position": vec3_json(camera.position()),
        "target": vec3_json(camera.target),
        "up": vec3_json(camera.up),
        "fov_deg": camera.fov_y_radians().to_degrees(),
        "near": camera.near(),
        "far": camera.far(),
    })
}

/// The settings that differ between two settings documents, by dotted path, as
/// `[baseline, variant]` pairs.
fn changed_settings(baseline: &Value, variant: &Value) -> Value {
    let mut changed = Map::new();
    collect_changes(baseline, variant, String::new(), &mut changed);
    Value::Object(changed)
}

fn collect_changes(baseline: &Value, variant: &Value, path: String, out: &mut Map<String, Value>) {
    match (baseline, variant) {
        (Value::Object(base), Value::Object(this)) => {
            for (key, value) in this {
                let child = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                match base.get(key) {
                    Some(base_value) => collect_changes(base_value, value, child, out),
                    None => {
                        out.insert(child, json!([Value::Null, value]));
                    }
                }
            }
        }
        (base, this) if base != this => {
            out.insert(path, json!([base, this]));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(variants: usize, settle: u64, per_variant: u64, trace: u64) -> Vec<(u64, Step)> {
        let mut sequence = Sequence::new(variants, settle, per_variant, trace);
        let mut log = Vec::new();
        let mut image_done = false;
        for frame in 100.. {
            let step = sequence.step(frame, image_done, true);
            image_done = step
                .capture
                .is_some_and(|capture| capture.kind == FrameKind::Image);
            let finished = step.finished;
            log.push((frame, step));
            if finished {
                return log;
            }
            assert!(frame < 10_000, "the sequence never finished");
        }
        unreachable!()
    }

    #[test]
    fn sequence_settles_then_captures_consecutive_frames_per_variant() {
        let log = run(2, 3, 2, 0);
        let applied: Vec<_> = log
            .iter()
            .filter_map(|(frame, step)| step.apply.map(|variant| (*frame, variant)))
            .collect();
        let captured: Vec<_> = log
            .iter()
            .filter_map(|(frame, step)| {
                step.capture.map(|capture| {
                    assert_eq!(
                        capture.kind,
                        FrameKind::Image,
                        "no settle frames without a trace"
                    );
                    (*frame, capture.variant, capture.index)
                })
            })
            .collect();
        // Variant 0 applies on frame 100, settles on 100..=102 and captures 103 and 104;
        // variant 1 applies on 105, settles to 107, captures 108 and 109.
        assert_eq!(applied, vec![(100, 0), (105, 1)]);
        assert_eq!(
            captured,
            vec![(103, 0, 0), (104, 0, 1), (108, 1, 0), (109, 1, 1)]
        );
        assert!(log.last().unwrap().1.finished);
    }

    #[test]
    fn sequence_without_settling_captures_on_the_frame_it_applies() {
        let log = run(2, 0, 1, 0);
        let image = |variant, index| {
            Some(Capture {
                variant,
                kind: FrameKind::Image,
                index,
            })
        };
        assert_eq!(log[0].1.apply, Some(0));
        assert_eq!(log[0].1.capture, image(0, 0));
        assert_eq!(log[1].1.apply, Some(1));
        assert_eq!(log[1].1.capture, image(1, 0));
        assert!(log[2].1.finished);
    }

    #[test]
    fn sequence_traces_the_last_settle_frames_without_counting_them_as_images() {
        // Settle 6 frames, trace the last 4 of them, then save 2 images.
        let log = run(2, 6, 2, 4);
        let captures: Vec<(u64, usize, FrameKind, u64)> = log
            .iter()
            .filter_map(|(frame, step)| step.capture.map(|c| (*frame, c.variant, c.kind, c.index)))
            .collect();
        use FrameKind::{Image, Settle};
        // Variant 0 applies on 100: settle frames are ages 0..=5, traced from age 2.
        // Variant 1 applies on 108 after two saved images (106, 107).
        assert_eq!(
            captures,
            vec![
                (102, 0, Settle, 2),
                (103, 0, Settle, 3),
                (104, 0, Settle, 4),
                (105, 0, Settle, 5),
                (106, 0, Image, 0),
                (107, 0, Image, 1),
                (110, 1, Settle, 2),
                (111, 1, Settle, 3),
                (112, 1, Settle, 4),
                (113, 1, Settle, 5),
                (114, 1, Image, 0),
                (115, 1, Image, 1),
            ]
        );
        // Every traced frame is followed directly by the next one, so differences between
        // consecutive captures are differences between adjacent frames.
        for pair in captures.windows(2).filter(|pair| pair[0].1 == pair[1].1) {
            assert_eq!(pair[0].0 + 1, pair[1].0);
        }
    }

    #[test]
    fn the_first_image_waits_for_alignment_and_the_wait_is_traced() {
        use FrameKind::{Image, Settle};
        // Applied on 100 and settled after 2 frames, tracing them. A frame is aligned when
        // its number is 4 modulo 8, so the first image waits from 102 to 108 (100 is aligned
        // too, but the variant is still settling); the later images follow without waiting.
        let mut sequence = Sequence::new(1, 2, 3, 2);
        let mut log = Vec::new();
        let mut image_done = false;
        for frame in 100.. {
            let step = sequence.step(frame, image_done, frame % 8 == 4);
            image_done = step.capture.is_some_and(|c| c.kind == Image);
            if let Some(capture) = step.capture {
                log.push((frame, capture.kind, capture.index));
            }
            if step.finished {
                break;
            }
            assert!(frame < 200, "never finished");
        }
        let expected: Vec<_> = (100..108)
            .map(|frame| (frame, Settle, frame - 100))
            .chain([(108, Image, 0), (109, Image, 1), (110, Image, 2)])
            .collect();
        assert_eq!(
            log, expected,
            "the waiting frames are traced like settle frames"
        );
    }

    #[test]
    fn waiting_for_alignment_gives_up_after_a_jitter_period() {
        // Never aligned (say the index stopped advancing): capture after at most 7 waits.
        let mut sequence = Sequence::new(2, 1, 1, 0);
        let mut image_frames = Vec::new();
        let mut image_done = false;
        for frame in 100..140 {
            let step = sequence.step(frame, image_done, false);
            image_done = step.capture.is_some();
            if step.capture.is_some() {
                image_frames.push(frame);
            }
            if step.finished {
                break;
            }
        }
        // Variant 0: applied on 100, settled after 1 frame, waits 7 frames, captures on
        // 108. Variant 1: applied on 109, captures on 117.
        assert_eq!(image_frames, vec![108, 117]);
    }

    #[test]
    fn a_trace_longer_than_the_settle_period_starts_when_the_variant_is_applied() {
        let log = run(1, 3, 1, 16);
        let kinds: Vec<_> = log
            .iter()
            .filter_map(|(_, step)| step.capture.map(|c| c.kind))
            .collect();
        assert_eq!(
            kinds,
            [
                FrameKind::Settle,
                FrameKind::Settle,
                FrameKind::Settle,
                FrameKind::Image
            ]
        );
    }

    #[test]
    fn override_applies_over_the_scene_or_over_the_defaults() {
        let mut scene = PostProcessSettings::default();
        scene.bloom.intensity = 3.0;
        scene.vignette.enabled = true;

        let over_scene = PostProcessOverride::new(&json!({"bloom": {"threshold": 0.2}}), false)
            .unwrap()
            .apply(scene.clone());
        assert_eq!(over_scene.bloom.threshold, 0.2);
        assert_eq!(over_scene.bloom.intensity, 3.0, "scene value kept");
        assert!(over_scene.vignette.enabled);

        let over_defaults = PostProcessOverride::new(&json!({"bloom": {"threshold": 0.2}}), true)
            .unwrap()
            .apply(scene);
        assert_eq!(over_defaults.bloom.threshold, 0.2);
        assert_eq!(
            over_defaults.bloom.intensity,
            PostProcessSettings::default().bloom.intensity
        );
        assert!(!over_defaults.vignette.enabled);
    }

    #[test]
    fn override_rejects_unknown_names_and_wrong_types() {
        let unknown = PostProcessOverride::new(&json!({"bloom": {"intensty": 1}}), false);
        assert!(unknown.unwrap_err().contains("bloom.intensty"));
        let wrong_type = PostProcessOverride::new(&json!({"tonemapping": 5}), false);
        assert!(wrong_type.is_err());
        // The tool's older parameter names still work.
        assert!(PostProcessOverride::new(&json!({"bloom": false, "fxaa": true}), false).is_ok());
    }

    fn parse_error(params: Value) -> String {
        let (sender, _receiver) = mpsc::channel();
        match parse(&params, Reply::new(sender)) {
            Ok(_) => panic!("expected an error"),
            Err((_, message)) => message,
        }
    }

    #[test]
    fn parameters_are_validated() {
        assert!(parse_error(json!({})).contains("at least one variant"));
        let one = json!({"variants": [{"name": "a b", "postprocess": {}}]});
        assert!(parse_error(one).contains("letters, digits"));
        let duplicate = json!({"variants": [
            {"name": "x", "postprocess": {}}, {"name": "x", "postprocess": {}}]});
        assert!(parse_error(duplicate).contains("duplicate"));
        let baseline_name = json!({"variants": [{"name": "baseline", "postprocess": {}}]});
        assert!(parse_error(baseline_name).contains("duplicate"));
        let typo = json!({"variants": [{"name": "t", "postprocess": {"bloom": {"x": 1}}}]});
        assert!(parse_error(typo).contains("variant \"t\""));
        let frames = json!({"frames": 0, "variants": [{"name": "t"}]});
        assert!(parse_error(frames).contains("frames must be"));
        let base = json!({"variants": [{"name": "t", "base": "other"}]});
        assert!(parse_error(base).contains("base must be"));

        let (sender, _receiver) = mpsc::channel();
        let ok = parse(
            &json!({"variants": [{"name": "taa_off", "postprocess": {"anti_aliasing": "none"}}]}),
            Reply::new(sender),
        );
        let capture = ok.ok().expect("valid parameters");
        assert_eq!(capture.variants.len(), 2, "the baseline is added");
        assert_eq!(capture.variants[0].name, "baseline");
    }

    fn parsed(params: Value) -> VisualCapture {
        let (sender, _receiver) = mpsc::channel();
        parse(&params, Reply::new(sender))
            .ok()
            .expect("valid parameters")
    }

    #[test]
    fn comparison_settings_default_and_validate() {
        let variants = json!([{"name": "t"}]);
        let default = parsed(json!({"variants": variants}));
        assert!(default.recorder.is_some(), "comparing is the default");
        assert_eq!(default.sequence.trace, SETTLE_TRACE_FRAMES);

        let off = parsed(json!({"variants": variants, "compare": false}));
        assert!(off.recorder.is_none());
        assert_eq!(off.sequence.trace, 0, "no settle frames are read back");

        let no_check = parsed(json!({"variants": variants, "settle_check": false}));
        assert!(no_check.recorder.is_some());
        assert_eq!(no_check.sequence.trace, 0);

        assert_eq!(parse_grid(&json!({})), Ok((4, 4)));
        assert_eq!(parse_grid(&json!({"grid": 6})), Ok((6, 6)));
        assert_eq!(parse_grid(&json!({"grid": [8, 2]})), Ok((8, 2)));
        for bad in [
            json!(0),
            json!(17),
            json!([4]),
            json!([4, 0]),
            json!("4"),
            json!(2.5),
        ] {
            let err = parse_grid(&json!({"grid": bad})).unwrap_err();
            assert!(err.contains("from 1 to 16"), "{bad}: {err}");
        }

        let threshold = parse_error(json!({"variants": variants, "threshold": 256}));
        assert!(threshold.contains("threshold must be"), "{threshold}");
        let gain = parse_error(json!({"variants": variants, "diff_gain": 0.5}));
        assert!(gain.contains("diff_gain must be"), "{gain}");
        let flag = parse_error(json!({"variants": variants, "compare": "yes"}));
        assert!(flag.contains("compare must be true or false"), "{flag}");
    }

    #[test]
    fn changed_settings_lists_only_differences_by_path() {
        let base = serde_json::to_value(PostProcessSettings::default()).unwrap();
        let mut other = PostProcessSettings::default();
        other.bloom.intensity += 1.0;
        other.vignette.enabled = !other.vignette.enabled;
        let other = serde_json::to_value(other).unwrap();
        let changed = changed_settings(&base, &other);
        let keys: Vec<&String> = changed.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["bloom.intensity", "vignette.enabled"]);
        assert_eq!(changed_settings(&base, &base), json!({}));
    }
}
