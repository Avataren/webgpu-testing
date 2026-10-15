//! Per-application remote-control state shared by the frame loop and request handlers.

use std::cell::RefCell;
use std::io::Cursor;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;

use glam::Vec2;
use serde_json::{json, Value};

use crate::app::{GpuUpdateContext, RuntimeMode, RuntimeStateHandle};
use crate::renderer::RenderRegion;
use crate::scene::{Camera, CameraComponent, InputState, Scene};

use super::capture::FrameCapture;
use super::handlers::{self, RemoteContext, SceneBaseline};
use super::json::{entity_from_json, entity_id, opt_bool, opt_f64, opt_str, opt_u64};
use super::visual::{PostProcessOverride, VisualCapture};
use super::RemoteApp;

/// Shared handle to a [`RemoteHub`]; the frame loop and the dispatch system both hold one.
pub type RemoteHubHandle = Rc<RefCell<RemoteHub>>;

/// Default longest side of screenshots returned to clients, in pixels.
const DEFAULT_SCREENSHOT_MAX_SIZE: u32 = 1280;

/// Sends the answer to one request back to the connection that made it.
pub(crate) struct Reply(Sender<Result<Value, String>>);

impl Reply {
    pub(crate) fn new(sender: Sender<Result<Value, String>>) -> Self {
        Self(sender)
    }

    pub(crate) fn send(self, result: Result<Value, String>) {
        // The client may have disconnected; there is nobody left to tell.
        let _ = self.0.send(result);
    }
}

/// A request received from a client, waiting to be handled on the main thread.
pub(crate) struct RemoteRequest {
    pub(crate) method: String,
    pub(crate) params: Value,
    pub(crate) reply: Reply,
}

/// An input event injected by a client.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum InputEvent {
    KeyDown(String),
    KeyUp(String),
    MouseDown(u32),
    MouseUp(u32),
    MouseMove(Vec2),
    Scroll(Vec2),
}

struct ScheduledInput {
    /// Frame (see [`RemoteHub::frame`]) at whose start the event is applied.
    frame: u64,
    event: InputEvent,
}

pub(super) struct CaptureRequest {
    pub(super) reply: Reply,
    pub(super) viewport_only: bool,
    pub(super) max_size: u32,
    pub(super) save_path: Option<PathBuf>,
    pub(super) inline: bool,
    /// The `camera` parameter: render through this camera entity (or `"game"`).
    pub(super) camera: Option<Value>,
    /// `camera` resolved to a view, once the scene has been consulted.
    pub(super) view: Option<Camera>,
    /// Aspect ratio (width / height) to render at instead of the viewport's.
    pub(super) aspect: Option<f32>,
    /// Receives the captured pixels, as they were cropped and scaled for the reply, just
    /// before the reply is sent. For in-process users that compare frames.
    pub(super) pixels: Option<Sender<image::RgbaImage>>,
    /// Frames still to render through `view` / `aspect` before capturing, so temporal
    /// effects (TAA, the path tracer's denoiser) converge on the held view.
    pub(super) settle_frames: u32,
    /// Displayed 8-bit values to report from the captured image.
    pub(super) probe: PixelProbe,
}

/// Pixels of a screenshot to report as numbers: the final 8-bit values as displayed, after
/// anti-aliasing, grain and dither. Coordinates are in the captured image before it is
/// shrunk to `max_size` (the window, or the viewport for viewport captures).
#[derive(Debug, Default)]
pub(super) struct PixelProbe {
    points: Vec<[u32; 2]>,
    /// [x, y, width, height] to summarize per channel.
    rect: Option<[u32; 4]>,
}

impl PixelProbe {
    fn parse(params: &Value) -> Result<Self, String> {
        let pixels = |value: &Value, len: usize, what: &str| -> Result<Vec<u32>, String> {
            value
                .as_array()
                .filter(|items| items.len() == len)
                .and_then(|items| {
                    items
                        .iter()
                        .map(|item| item.as_u64().and_then(|n| u32::try_from(n).ok()))
                        .collect()
                })
                .ok_or_else(|| format!("{what} must be {len} whole pixels"))
        };
        let mut probe = Self::default();
        if let Some(points) = params.get("points").filter(|value| !value.is_null()) {
            let points = points
                .as_array()
                .ok_or("points must be a list of [x, y] pixels")?;
            if points.len() > 1024 {
                return Err("at most 1024 points".into());
            }
            for point in points {
                let xy = pixels(point, 2, "each point")?;
                probe.points.push([xy[0], xy[1]]);
            }
        }
        if let Some(rect) = params.get("rect").filter(|value| !value.is_null()) {
            let rect = pixels(rect, 4, "rect [x, y, width, height]")?;
            if rect[2] == 0 || rect[3] == 0 {
                return Err("rect needs a positive width and height".into());
            }
            probe.rect = Some([rect[0], rect[1], rect[2], rect[3]]);
        }
        Ok(probe)
    }

    fn is_empty(&self) -> bool {
        self.points.is_empty() && self.rect.is_none()
    }

    /// The values at the points and the rectangle's per-channel statistics.
    fn read(&self, image: &image::RgbaImage, result: &mut Value) -> Result<(), String> {
        let (width, height) = image.dimensions();
        let outside = |what: String| {
            format!("{what} is outside the {width}x{height} captured image (source_width x source_height)")
        };
        if !self.points.is_empty() {
            let mut values = Vec::with_capacity(self.points.len());
            for &[x, y] in &self.points {
                if x >= width || y >= height {
                    return Err(outside(format!("point [{x}, {y}]")));
                }
                values.push(json!({ "x": x, "y": y, "rgba": image.get_pixel(x, y).0 }));
            }
            result["points"] = Value::Array(values);
        }
        if let Some([x, y, w, h]) = self.rect {
            if x.saturating_add(w) > width || y.saturating_add(h) > height {
                return Err(outside(format!("rect [{x}, {y}, {w}, {h}]")));
            }
            let mut min = [u8::MAX; 4];
            let mut max = [0u8; 4];
            let mut sum = [0f64; 4];
            let mut sum_sq = [0f64; 4];
            for py in y..y + h {
                for px in x..x + w {
                    for (channel, &value) in image.get_pixel(px, py).0.iter().enumerate() {
                        min[channel] = min[channel].min(value);
                        max[channel] = max[channel].max(value);
                        sum[channel] += value as f64;
                        sum_sq[channel] += (value as f64).powi(2);
                    }
                }
            }
            let count = (w as f64) * (h as f64);
            let mean = sum.map(|total| total / count);
            let round = |value: f64| (value * 1000.0).round() / 1000.0;
            let stddev: Vec<f64> = (0..4)
                .map(|channel| {
                    round(
                        (sum_sq[channel] / count - mean[channel].powi(2))
                            .max(0.0)
                            .sqrt(),
                    )
                })
                .collect();
            result["rect"] = json!({
                "rect": [x, y, w, h],
                "min": min,
                "max": max,
                "mean": mean.map(round),
                "stddev": stddev,
            });
        }
        Ok(())
    }
}

impl CaptureRequest {
    /// Whether the capture needs a frame of its own: one rendered through another
    /// camera or at another aspect ratio than what the window shows.
    fn needs_own_frame(&self) -> bool {
        self.view.is_some() || self.aspect.is_some()
    }
}

/// How the next frame must differ from the normal one for a pending screenshot.
#[derive(Clone, Copy, Debug)]
pub struct CaptureOverride {
    /// Render through this camera instead of the scene's.
    pub camera: Option<Camera>,
    /// Render the scene at this aspect ratio, in the largest region of that shape that
    /// fits the viewport.
    pub aspect: Option<f32>,
}

struct Waiter {
    reply: Reply,
    until_frame: u64,
    deadline: Option<Instant>,
    start_frame: u64,
    start: Instant,
}

struct ProfileCapture {
    reply: Reply,
    frames: usize,
    samples: Vec<Value>,
    baseline: Option<Value>,
}

/// Remote-control state of one running application.
pub struct RemoteHub {
    port: u16,
    requests: Option<Receiver<RemoteRequest>>,
    runtime: Option<RuntimeStateHandle>,
    /// Number of frames completed since the hub was created.
    frame: u64,
    started: Instant,
    last_dt: f64,
    input: Vec<ScheduledInput>,
    simulation_input: Vec<(u64, InputEvent)>,
    pub(super) captures: Vec<CaptureRequest>,
    /// Whether the frame being rendered is the first capture's own frame.
    capturing_own_frame: bool,
    /// Whether the frame being rendered only settles the first capture's view, so no
    /// capture is due.
    settling: bool,
    waiters: Vec<Waiter>,
    scene_baseline: Option<SceneBaseline>,
    profile: Option<ProfileCapture>,
    /// A `visual.capture` in progress.
    pub(super) visual: Option<Box<VisualCapture>>,
    /// Post-processing every frame is rendered with, over what the scene's volumes blend
    /// to. Render-only: it never touches the scene.
    pub(super) post_process_override: Option<PostProcessOverride>,
}

impl RemoteHub {
    /// Start a server on `127.0.0.1:port` and return the hub that receives its requests.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn start(port: u16) -> std::io::Result<RemoteHubHandle> {
        let requests = super::server::start(port)?;
        install_panic_logger();
        log::info!(target: "remote", "Remote control listening on 127.0.0.1:{port}");
        let mut hub = Self::detached();
        hub.port = port;
        hub.requests = Some(requests);
        Ok(Rc::new(RefCell::new(hub)))
    }

    /// A hub without a server, for tests and embedding.
    pub fn detached() -> Self {
        Self {
            port: 0,
            requests: None,
            runtime: None,
            frame: 0,
            started: Instant::now(),
            last_dt: 0.0,
            input: Vec::new(),
            simulation_input: Vec::new(),
            captures: Vec::new(),
            capturing_own_frame: false,
            settling: false,
            waiters: Vec::new(),
            scene_baseline: None,
            profile: None,
            visual: None,
            post_process_override: None,
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Frames completed since the hub was created.
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// Seconds since the hub was created.
    pub fn uptime(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    /// Duration of the last frame, in seconds.
    pub fn last_dt(&self) -> f64 {
        self.last_dt
    }

    pub fn set_runtime_handle(&mut self, handle: RuntimeStateHandle) {
        self.runtime = Some(handle);
    }

    pub fn runtime_handle(&self) -> Option<&RuntimeStateHandle> {
        self.runtime.as_ref()
    }

    pub(crate) fn set_scene_baseline(&mut self, baseline: SceneBaseline) {
        self.scene_baseline = Some(baseline);
    }

    pub(crate) fn scene_baseline(&self) -> Option<&SceneBaseline> {
        self.scene_baseline.as_ref()
    }

    /// Queue `event` to be applied at the start of the frame `delay` frames after the
    /// next one.
    pub(crate) fn schedule_input(&mut self, delay: u64, event: InputEvent) {
        self.input.push(ScheduledInput {
            frame: self.frame + 1 + delay,
            event,
        });
    }

    /// Release every key and button and drop queued input.
    pub(crate) fn clear_input(&mut self) {
        self.input.clear();
        self.simulation_input.clear();
    }

    pub(crate) fn schedule_simulation_input(&mut self, step: u64, event: InputEvent) {
        self.simulation_input.push((step, event));
    }
    pub(crate) fn apply_simulation_input(&mut self, step: u64, input: &mut InputState) {
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.simulation_input)
            .into_iter()
            .partition(|(at, _)| *at <= step);
        self.simulation_input = later;
        for (_, event) in due {
            handlers::apply_simulation_event(input, &event);
        }
    }

    pub(crate) fn pending_input(&self) -> usize {
        self.input.len() + self.simulation_input.len()
    }

    /// Apply injected input that is due. Call at the start of a frame, before the
    /// update stage, like window events.
    pub fn apply_input(&mut self, input: &mut InputState) {
        if self.input.is_empty() {
            return;
        }
        let frame = self.frame;
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.input)
            .into_iter()
            .partition(|scheduled| scheduled.frame <= frame);
        self.input = later;
        for scheduled in due {
            match scheduled.event {
                InputEvent::KeyDown(key) => input.press_key(key),
                InputEvent::KeyUp(key) => input.release_key(&key),
                InputEvent::MouseDown(button) => input.press_mouse_button(button),
                InputEvent::MouseUp(button) => input.release_mouse_button(button),
                InputEvent::MouseMove(position) => {
                    let previous = input.mouse_position();
                    input.add_mouse_delta(position - previous);
                    input.set_mouse_position(position);
                }
                InputEvent::Scroll(delta) => input.add_scroll_delta(delta),
            }
        }
    }

    /// Whether a screenshot is waiting for the current frame.
    pub fn has_pending_captures(&self) -> bool {
        !self.captures.is_empty() && !self.settling
    }

    /// Resolve the `camera` parameter of new screenshot requests against the scene,
    /// answering requests whose camera does not exist.
    fn resolve_capture_cameras(&mut self, scene: &Scene) {
        let mut index = 0;
        while index < self.captures.len() {
            let request = &mut self.captures[index];
            if let (Some(spec), None) = (&request.camera, request.view) {
                match resolve_view(scene, spec) {
                    Ok(view) => request.view = Some(view),
                    Err(message) => {
                        self.captures.remove(index).reply.send(Err(message));
                        continue;
                    }
                }
            }
            index += 1;
        }
    }

    /// How the next frame must be rendered for a pending screenshot, if it differs from a
    /// normal frame.
    ///
    /// Call once per frame before rendering. A screenshot with its own camera or aspect
    /// ratio gets a frame to itself; [`Self::complete_captures`] then answers only that one.
    pub fn capture_frame_override(&mut self) -> Option<CaptureOverride> {
        // A held view renders through the capture's camera for its settle frames first,
        // capturing nothing (later screenshots wait so they never see that camera).
        self.settling = false;
        if let Some(request) = self
            .captures
            .first_mut()
            .filter(|request| request.settle_frames > 0)
        {
            request.settle_frames -= 1;
            self.settling = true;
            self.capturing_own_frame = false;
            if request.needs_own_frame() {
                return Some(CaptureOverride {
                    camera: request.view,
                    aspect: request.aspect,
                });
            }
        }
        let request = self
            .captures
            .first()
            .filter(|request| request.needs_own_frame());
        self.capturing_own_frame = request.is_some();
        match request {
            Some(request) => Some(CaptureOverride {
                camera: request.view,
                aspect: request.aspect,
            }),
            // A visual capture renders every frame through its locked view, so the
            // temporal anti-aliasing history never sees another camera.
            None => self
                .visual
                .as_ref()
                .and_then(|capture| capture.view())
                .map(|view| CaptureOverride {
                    camera: Some(view),
                    aspect: None,
                }),
        }
    }

    /// The render-only post-processing override in effect, if any.
    pub fn post_process_override(&self) -> Option<&PostProcessOverride> {
        self.post_process_override.as_ref()
    }

    /// Answer pending screenshot requests with the frame that is about to be presented.
    ///
    /// `region` is the scene viewport within the window, used by viewport-only captures.
    pub fn complete_captures(
        &mut self,
        capture: Result<FrameCapture, String>,
        region: Option<RenderRegion>,
    ) {
        let due: Vec<CaptureRequest> = if std::mem::take(&mut self.capturing_own_frame) {
            vec![self.captures.remove(0)]
        } else {
            let (due, later) = std::mem::take(&mut self.captures)
                .into_iter()
                .partition(|request| !request.needs_own_frame());
            self.captures = later;
            due
        };
        for request in due {
            let result = capture
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|capture| encode_capture(capture, region, &request));
            request.reply.send(result);
        }
    }

    /// Finish a frame: advance the frame counter and answer waits that are due.
    pub fn end_frame(&mut self, dt: f64) {
        self.frame += 1;
        self.last_dt = dt;
        if self.waiters.is_empty() {
            return;
        }
        let now = Instant::now();
        let frame = self.frame;
        let (done, pending): (Vec<_>, Vec<_>) = std::mem::take(&mut self.waiters)
            .into_iter()
            .partition(|waiter| {
                waiter.until_frame <= frame
                    && waiter.deadline.is_none_or(|deadline| now >= deadline)
            });
        self.waiters = pending;
        for waiter in done {
            waiter.reply.send(Ok(json!({
                "frames_waited": frame - waiter.start_frame,
                "seconds_waited": now.duration_since(waiter.start).as_secs_f64(),
                "frame": frame,
            })));
        }
    }

    fn drain_requests(&mut self) -> Vec<RemoteRequest> {
        self.requests
            .as_ref()
            .map(|requests| requests.try_iter().collect())
            .unwrap_or_default()
    }

    /// Handle one request that needs to answer on a later frame. Returns the request
    /// back if it is not deferred.
    fn try_defer(&mut self, request: RemoteRequest) -> Option<RemoteRequest> {
        match request.method.as_str() {
            "profile.capture" => {
                let frames = opt_u64(&request.params, "frames").unwrap_or(300);
                if !(2..=10000).contains(&frames) || self.profile.is_some() {
                    request.reply.send(Err(
                        "profile.capture needs 2..=10000 frames and no capture already running"
                            .into(),
                    ));
                } else {
                    self.profile = Some(ProfileCapture {
                        reply: request.reply,
                        frames: frames as usize,
                        samples: Vec::new(),
                        baseline: request.params.get("baseline").cloned(),
                    });
                }
                None
            }
            "screenshot" => {
                let params = &request.params;
                // Rendering through a camera is a preview of the game, which only makes
                // sense without the editor around it.
                let has_camera = params.get("camera").is_some_and(|value| !value.is_null());
                let target = opt_str(params, "target").unwrap_or(if has_camera {
                    "viewport"
                } else {
                    "window"
                });
                let viewport_only = match target {
                    "window" => false,
                    "viewport" => true,
                    other => {
                        request.reply.send(Err(format!(
                            "unknown screenshot target {other:?}; use \"window\" or \"viewport\""
                        )));
                        return None;
                    }
                };
                let aspect = match params.get("aspect").filter(|value| !value.is_null()) {
                    Some(value) => match parse_aspect(value) {
                        Ok(aspect) => Some(aspect),
                        Err(message) => {
                            request.reply.send(Err(message));
                            return None;
                        }
                    },
                    None => None,
                };
                let probe = match PixelProbe::parse(params) {
                    Ok(probe) => probe,
                    Err(message) => {
                        request.reply.send(Err(message));
                        return None;
                    }
                };
                let max_size = opt_u64(params, "max_size")
                    .map_or(DEFAULT_SCREENSHOT_MAX_SIZE, |size| {
                        size.clamp(16, 8192) as u32
                    });
                self.captures.push(CaptureRequest {
                    // An aspect ratio only describes the scene view, so crop to it.
                    viewport_only: viewport_only || aspect.is_some(),
                    max_size,
                    save_path: opt_str(params, "save_path").map(PathBuf::from),
                    inline: opt_bool(params, "inline").unwrap_or(true),
                    camera: params
                        .get("camera")
                        .filter(|value| !value.is_null())
                        .cloned(),
                    view: None,
                    aspect,
                    pixels: None,
                    settle_frames: opt_u64(params, "settle_frames").unwrap_or(0).min(600) as u32,
                    probe,
                    reply: request.reply,
                });
                None
            }
            "visual.capture" => {
                self.begin_visual_capture(&request.params, request.reply);
                None
            }
            "wait" => {
                let params = &request.params;
                let frames = opt_u64(params, "frames").unwrap_or(0).min(100_000);
                let seconds = opt_f64(params, "seconds").unwrap_or(0.0).clamp(0.0, 240.0);
                let start = Instant::now();
                self.waiters.push(Waiter {
                    reply: request.reply,
                    // A wait always spans at least one full frame.
                    until_frame: self.frame + frames.max(1),
                    deadline: (seconds > 0.0)
                        .then(|| start + std::time::Duration::from_secs_f64(seconds)),
                    start_frame: self.frame,
                    start,
                });
                None
            }
            _ => Some(request),
        }
    }
}

/// Handle every request that arrived since the last frame. Install as a GPU system.
pub fn process_requests(
    hub: &RemoteHubHandle,
    ctx: &mut GpuUpdateContext<'_>,
    app: &mut dyn RemoteApp,
) {
    let mut hub = hub.borrow_mut();
    let requests = hub.drain_requests();
    hub.step_visual_capture(ctx);
    if let Some(mut capture) = hub.profile.take() {
        let mut remote = RemoteContext::new(ctx, &mut hub, app.remote_methods());
        if let Ok(sample) = handlers::profile_sample(&mut remote) {
            capture.samples.push(sample);
        }
        if capture.samples.len() >= capture.frames {
            capture.reply.send(Ok(super::profile::summarize(
                &capture.samples,
                capture.baseline.as_ref(),
            )));
        } else {
            hub.profile = Some(capture);
        }
    }
    if requests.is_empty() {
        return;
    }

    let app_methods = app.remote_methods();
    let mut modified = false;
    {
        let mut remote = RemoteContext::new(ctx, &mut hub, app_methods);
        for request in requests {
            let Some(request) = remote.hub.try_defer(request) else {
                continue;
            };
            let RemoteRequest {
                method,
                params,
                reply,
            } = request;
            log::debug!(target: "remote", "Handling remote request {method}");
            let mut result = app
                .remote_command(&method, &params, &mut remote)
                .unwrap_or_else(|| handlers::handle(&method, &params, &mut remote));
            let changed_scene = remote.take_modified();
            modified |= changed_scene;
            // Stopping play restores the scene from before play, so say so in the
            // answer rather than letting the change vanish silently.
            if changed_scene && remote.runtime_mode() == Some(RuntimeMode::Playing) {
                if let Ok(Value::Object(object)) = &mut result {
                    object.insert("discarded_on_stop".into(), Value::Bool(true));
                }
            }
            reply.send(result);
        }
    }
    hub.resolve_capture_cameras(&ctx.scene);

    if modified {
        ctx.scene.mark_dirty();
        app.remote_scene_modified(ctx);
    }
}

/// The view through the camera a screenshot asked for: a camera entity, or `"game"`
/// for the scene's active camera (else its first camera).
/// An aspect ratio given as a number (`1.777`) or text (`"16:9"`, `"1.5"`).
pub(crate) fn parse_aspect(value: &Value) -> Result<f32, String> {
    let parsed = match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => match text.split_once(':') {
            Some((width, height)) => width
                .trim()
                .parse::<f64>()
                .ok()
                .zip(height.trim().parse::<f64>().ok())
                .filter(|(_, height)| *height > 0.0)
                .map(|(width, height)| width / height),
            None => text.trim().parse::<f64>().ok(),
        },
        _ => None,
    };
    parsed
        .filter(|aspect| (0.1..=10.0).contains(aspect))
        .map(|aspect| aspect as f32)
        .ok_or_else(|| {
            format!(
                "aspect must be a ratio between 0.1 and 10 such as 1.78 or \"16:9\", got {value}"
            )
        })
}

/// The largest region of shape `aspect` (width / height) that fits `base`, centered in it.
pub fn fit_region(base: RenderRegion, aspect: f32) -> Option<RenderRegion> {
    let (base_width, base_height) = (base.width() as f32, base.height() as f32);
    let (width, height) = if aspect >= base_width / base_height {
        (base_width, base_width / aspect)
    } else {
        (base_height * aspect, base_height)
    };
    let (width, height) = (
        (width.floor() as u32).max(1),
        (height.floor() as u32).max(1),
    );
    RenderRegion::new(
        base.x() + (base.width() - width) / 2,
        base.y() + (base.height() - height) / 2,
        width,
        height,
    )
}

pub(super) fn resolve_view(scene: &Scene, spec: &Value) -> Result<Camera, String> {
    let world = scene.main_world();
    if spec.is_object() {
        // A one-off pose: the view camera moved as `camera_set` would, without moving it.
        let mut camera = *scene.camera();
        super::handlers::adjust_camera(&mut camera, spec)?;
        return Ok(camera);
    }
    let entity = if spec.as_str() == Some("game") {
        scene
            .active_camera_entity()
            .or_else(|| {
                world
                    .query::<&CameraComponent>()
                    .iter()
                    .map(|(entity, _)| entity)
                    .min_by_key(|entity| entity.id())
            })
            .ok_or("the scene has no camera entity")?
    } else {
        entity_from_json(world, spec)?
    };
    scene
        .camera_for_entity(entity)
        .ok_or_else(|| format!("entity {} is not a camera", entity_id(entity)))
}

fn encode_capture(
    capture: &FrameCapture,
    region: Option<RenderRegion>,
    request: &CaptureRequest,
) -> Result<Value, String> {
    let mut image = image::RgbaImage::from_raw(capture.width, capture.height, capture.rgba.clone())
        .ok_or("capture buffer has the wrong size")?;

    if request.viewport_only {
        if let Some(region) = region {
            let x = region.x().min(capture.width.saturating_sub(1));
            let y = region.y().min(capture.height.saturating_sub(1));
            let width = region.width().min(capture.width - x).max(1);
            let height = region.height().min(capture.height - y).max(1);
            image = image::imageops::crop_imm(&image, x, y, width, height).to_image();
        }
    }

    let (source_width, source_height) = image.dimensions();
    let mut probed = json!({});
    if !request.probe.is_empty() {
        request.probe.read(&image, &mut probed)?;
    }
    let longest = source_width.max(source_height);
    if longest > request.max_size {
        let scale = request.max_size as f32 / longest as f32;
        let width = ((source_width as f32 * scale).round() as u32).max(1);
        let height = ((source_height as f32 * scale).round() as u32).max(1);
        image =
            image::imageops::resize(&image, width, height, image::imageops::FilterType::Triangle);
    }

    let mut result = json!({
        "width": image.width(),
        "height": image.height(),
        "source_width": source_width,
        "source_height": source_height,
    });
    if let (Value::Object(result), Value::Object(probed)) = (&mut result, probed) {
        result.extend(probed);
    }

    // Frames only wanted as pixels (a comparison's settle frames) skip the encoding.
    if request.save_path.is_some() || request.inline {
        let mut png = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(|err| format!("failed to encode PNG: {err}"))?;
        if let Some(path) = &request.save_path {
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                std::fs::create_dir_all(parent)
                    .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
            }
            std::fs::write(path, &png)
                .map_err(|err| format!("failed to write {}: {err}", path.display()))?;
            result["path"] = json!(path.display().to_string());
        }
        if request.inline {
            result["png_base64"] = json!(base64::encode(&png));
        }
    }
    if let Some(pixels) = &request.pixels {
        // The receiver may be gone if its capture was abandoned; nothing to do then.
        let _ = pixels.send(image);
    }
    Ok(result)
}

/// Record panics in the log tap so remote clients can see why the engine died.
#[cfg(not(target_arch = "wasm32"))]
fn install_panic_logger() {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            super::log_tap::push(log::Level::Error, "panic", info.to_string());
            previous(info);
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduled_input_applies_on_its_frame() {
        let mut hub = RemoteHub::detached();
        let mut input = InputState::new();
        hub.schedule_input(0, InputEvent::KeyDown("W".into()));
        hub.schedule_input(2, InputEvent::KeyUp("W".into()));

        // Scheduled during frame 0, so the press lands at the start of frame 1.
        hub.apply_input(&mut input);
        assert!(!input.is_key_pressed("W"));
        hub.end_frame(0.016);

        hub.apply_input(&mut input);
        assert!(input.is_key_just_pressed("W"));
        hub.end_frame(0.016);
        input.update();

        hub.apply_input(&mut input);
        assert!(input.is_key_pressed("W"));
        hub.end_frame(0.016);
        input.update();

        hub.apply_input(&mut input);
        assert!(!input.is_key_pressed("W"));
        assert_eq!(hub.pending_input(), 0);
    }

    #[test]
    fn waits_answer_after_their_frames() {
        let mut hub = RemoteHub::detached();
        let (sender, receiver) = std::sync::mpsc::channel();
        let request = RemoteRequest {
            method: "wait".into(),
            params: json!({ "frames": 2 }),
            reply: Reply::new(sender),
        };
        assert!(hub.try_defer(request).is_none());

        hub.end_frame(0.016);
        assert!(receiver.try_recv().is_err());
        hub.end_frame(0.016);
        let reply = receiver.try_recv().expect("wait answered").unwrap();
        assert_eq!(reply["frames_waited"], 2);
    }

    #[test]
    fn held_screenshots_settle_before_capturing_and_report_displayed_pixels() {
        let mut hub = RemoteHub::detached();
        let (sender, receiver) = std::sync::mpsc::channel();
        let request = RemoteRequest {
            method: "screenshot".into(),
            params: json!({
                "aspect": 2.0,
                "settle_frames": 2,
                "points": [[0, 0], [3, 1]],
                "rect": [1, 0, 2, 2],
                "inline": false,
            }),
            reply: Reply::new(sender),
        };
        assert!(hub.try_defer(request).is_none());
        // Two frames render through the held view without capturing anything.
        for _ in 0..2 {
            assert_eq!(hub.capture_frame_override().unwrap().aspect, Some(2.0));
            assert!(!hub.has_pending_captures());
            hub.end_frame(0.016);
        }
        assert_eq!(hub.capture_frame_override().unwrap().aspect, Some(2.0));
        assert!(hub.has_pending_captures());
        let rgba: Vec<u8> = (0..8u8).flat_map(|i| [i * 10, 100, 200 - i, 255]).collect();
        hub.complete_captures(
            Ok(FrameCapture {
                width: 4,
                height: 2,
                rgba,
            }),
            None,
        );
        let reply = receiver.try_recv().expect("screenshot answered").unwrap();
        assert_eq!(reply["points"][0]["rgba"], json!([0, 100, 200, 255]));
        assert_eq!(reply["points"][1]["rgba"], json!([70, 100, 193, 255]));
        // Pixels 1, 2, 5 and 6.
        assert_eq!(reply["rect"]["min"], json!([10, 100, 194, 255]));
        assert_eq!(reply["rect"]["max"], json!([60, 100, 199, 255]));
        assert_eq!(reply["rect"]["mean"][0], 35.0);
        assert_eq!(reply["rect"]["stddev"][1], 0.0);
        assert!(hub.capture_frame_override().is_none());

        let (sender, receiver) = std::sync::mpsc::channel();
        hub.try_defer(RemoteRequest {
            method: "screenshot".into(),
            params: json!({ "points": [[1]] }),
            reply: Reply::new(sender),
        });
        assert!(receiver.try_recv().unwrap().is_err());
    }
}
