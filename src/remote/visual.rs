//! Visual comparison captures.
//!
//! A [`PostProcessOverride`] changes the post-processing a frame is *rendered* with
//! without touching the scene, so there is nothing to undo afterwards. `visual.capture`
//! uses it to render one frozen view under several variants: for each variant it applies
//! the override, lets temporal anti-aliasing settle, then captures a few consecutive
//! frames and records what each one was rendered with (settings, TAA jitter index, frame
//! number, camera, scene time). The images land in a directory next to a `manifest.json`,
//! ready to be compared; the comparison itself is not done here.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use serde_json::{json, Map, Value};

use crate::app::{GpuUpdateContext, RuntimeMode};
use crate::renderer::postprocess::PostProcessSettings;
use crate::scene::{Camera, Scene};

use super::handlers::post_process_patch;
use super::hub::{CaptureRequest, RemoteHub, Reply};
use super::json::{opt_str, patch_serde, vec3_json};

/// The most variants, frames per variant and settle frames one capture may use.
const MAX_VARIANTS: usize = 16;
const MAX_FRAMES_PER_VARIANT: u64 = 32;
const MAX_SETTLE_FRAMES: u64 = 600;
const DEFAULT_SETTLE_FRAMES: u64 = 16;
const DEFAULT_FRAMES_PER_VARIANT: u64 = 2;
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
    /// Capture this frame as image `index` of `variant`.
    capture: Option<(usize, u64)>,
    finished: bool,
}

/// The order of a capture, independent of the renderer: variant after variant, `settle`
/// frames after applying each before `per_variant` consecutive captures.
#[derive(Debug)]
struct Sequence {
    variants: usize,
    settle: u64,
    per_variant: u64,
    current: usize,
    applied_at: Option<u64>,
    captured: u64,
}

impl Sequence {
    fn new(variants: usize, settle: u64, per_variant: u64) -> Self {
        Self {
            variants,
            settle,
            per_variant,
            current: 0,
            applied_at: None,
            captured: 0,
        }
    }

    /// The step for the frame numbered `frame`, which is about to render. Call once per
    /// frame, after the image requested by the previous step (if any) has completed.
    fn step(&mut self, frame: u64, previous_capture_done: bool) -> Step {
        let mut step = Step::default();
        if previous_capture_done {
            self.captured += 1;
            if self.captured == self.per_variant {
                self.current += 1;
                self.captured = 0;
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
        if frame - applied_at >= self.settle {
            step.capture = Some((self.current, self.captured));
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
    let settle = bounded(params, "settle_frames", DEFAULT_SETTLE_FRAMES, 0, MAX_SETTLE_FRAMES)?;
    let per_variant = bounded(
        params,
        "frames",
        DEFAULT_FRAMES_PER_VARIANT,
        1,
        MAX_FRAMES_PER_VARIANT,
    )?;
    let max_size = bounded(params, "max_size", 8192, 16, 8192)? as u32;

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
        return Err(format!("at most {MAX_VARIANTS} variants (including the baseline)"));
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
        if variants[..index].iter().any(|other| other.name == variant.name) {
            return Err(format!("duplicate variant name {:?}", variant.name));
        }
    }

    let dir = match opt_str(params, "dir") {
        Some(dir) => PathBuf::from(dir),
        None => default_dir(),
    };
    let reports = variants
        .iter()
        .map(|variant| VariantReport {
            name: variant.name.clone(),
            description: variant
                .render_override
                .as_ref()
                .map_or_else(|| json!({"base": "scene", "settings": {}}), |o| o.describe()),
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
        sequence: Sequence::new(variants.len(), settle, per_variant),
        variants,
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

        // The image requested last frame has been rendered and captured by now.
        let mut done = false;
        if let Some(pending) = &self.pending {
            match pending.receiver.try_recv() {
                Ok(result) => {
                    let pending = self.pending.take().expect("checked above");
                    self.record_image(pending, result?, ctx)?;
                    done = true;
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

        let step = self.sequence.step(hub.frame(), done);
        if step.finished {
            return Ok(true);
        }
        if let Some(index) = step.apply {
            hub.post_process_override = self.variants[index].render_override.clone();
        }
        if let Some((variant, index)) = step.capture {
            self.request_image(hub, &ctx.scene, variant, index);
        }
        Ok(false)
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

    fn request_image(&mut self, hub: &mut RemoteHub, scene: &Scene, variant: usize, index: u64) {
        let (sender, receiver) = mpsc::channel();
        let path = self
            .dir
            .join(format!("{}_{:02}.png", self.variants[variant].name, index));
        hub.captures.push(CaptureRequest {
            reply: Reply::new(sender),
            viewport_only: true,
            max_size: self.max_size,
            save_path: Some(path),
            inline: false,
            camera: None,
            view: self.view,
            aspect: None,
        });
        self.pending = Some(PendingImage {
            receiver,
            variant,
            index,
            frame: hub.frame(),
            scene_time: scene.time(),
            requested_at: hub.frame(),
        });
    }

    /// Note what an image was rendered with. Runs on the frame after the capture, before
    /// anything renders, so the renderer still holds that frame's state.
    fn record_image(
        &mut self,
        pending: PendingImage,
        saved: Value,
        ctx: &mut GpuUpdateContext<'_>,
    ) -> Result<(), String> {
        let settings = serde_json::to_value(ctx.renderer.postprocess_settings())
            .map_err(|err| err.to_string())?;
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
        let baseline = self.reports.first().and_then(|report| report.settings.clone());
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

        let mut variants = Vec::new();
        let mut manifest_variants = Vec::new();
        for report in &self.reports {
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
            let mut full = summary.clone();
            full["effective_settings"] = report.settings.clone().unwrap_or(Value::Null);
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
        let report = json!({
            "dir": self.dir.display().to_string(),
            "manifest": manifest_path.display().to_string(),
            "baseline": self.variants.first().is_some_and(|v| v.render_override.is_none()).then_some("baseline"),
            "camera": camera_json(&view),
            "sequence": {
                "settle_frames": self.sequence.settle,
                "frames_per_variant": self.sequence.per_variant,
                "engine_frames": hub.frame() - self.started_frame,
            },
            "simulation": simulation,
            "warnings": warnings,
            "variants": variants,
        });
        let mut manifest = report.clone();
        manifest["variants"] = Value::Array(manifest_variants);
        write_manifest(&manifest_path, &manifest)?;
        Ok(report)
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

    fn run(variants: usize, settle: u64, per_variant: u64) -> Vec<(u64, Step)> {
        let mut sequence = Sequence::new(variants, settle, per_variant);
        let mut log = Vec::new();
        let mut capture_done = false;
        for frame in 100.. {
            let step = sequence.step(frame, capture_done);
            capture_done = step.capture.is_some();
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
        let log = run(2, 3, 2);
        let applied: Vec<_> = log
            .iter()
            .filter_map(|(frame, step)| step.apply.map(|variant| (*frame, variant)))
            .collect();
        let captured: Vec<_> = log
            .iter()
            .filter_map(|(frame, step)| step.capture.map(|(v, i)| (*frame, v, i)))
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
        let log = run(2, 0, 1);
        assert_eq!(log[0].1.apply, Some(0));
        assert_eq!(log[0].1.capture, Some((0, 0)));
        assert_eq!(log[1].1.apply, Some(1));
        assert_eq!(log[1].1.capture, Some((1, 0)));
        assert!(log[2].1.finished);
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
