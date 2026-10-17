//! Built-in remote methods that work in any application.
//!
//! Every method takes a JSON object of parameters and returns a JSON value. See
//! [`CORE_METHODS`] for the list and [`super::json`] for value conventions.

mod ai;
mod animation;
mod diff;
mod entity;
mod geometry;
mod graphics;
mod info;
mod input;
mod light_probes;
#[cfg(feature = "navigation")]
mod navigation;
mod procedural;
mod rendering;
mod scripting;
mod simulation;
mod spatial;
mod transaction;
mod voxel;

use serde_json::{json, Value};

use crate::app::{GpuUpdateContext, RuntimeMode, RuntimeStateHandle};

use super::hub::RemoteHub;

pub(crate) use diff::SceneBaseline;
pub(crate) use entity::{
    patch_material, set_active_camera, set_camera, set_generated_mesh, set_light, set_mesh,
};
pub(crate) use rendering::{
    adjust_camera, environment_patch, patch_environment, patch_post_process, post_process_patch,
};

/// Outcome of a remote method: a JSON result or an error message.
pub type RemoteResult = Result<Value, String>;

/// Built-in methods with a one-line description each.
pub const CORE_METHODS: &[(&str, &str)] = &[
    ("profile.capture", "Capture frame percentiles, stage timings and asset growth"),
    ("project.validate", "Read-only saved-project asset, camera and Lua syntax validation"),
    (
        "project.content",
        "What an export of a saved project carries (the files its main scene, prefabs, materials and scripts use) and what it leaves out ({path, list_unused})",
    ),
    (
        "engine.info",
        "Frame, runtime mode, active scene, surface size, project root",
    ),
    (
        "engine.methods",
        "List the methods this application understands",
    ),
    (
        "stats",
        "Frame time and renderer statistics; {history: N} adds the last N frames' renderer figures, {compact: true} drops the notes",
    ),
    (
        "scene.entities",
        "Flat list of entities, filterable by name or component",
    ),
    ("scene.tree", "Entity hierarchy as a tree"),
    (
        "scene.mark",
        "Remember the scene's state for a later scene.diff",
    ),
    (
        "scene.diff",
        "Entities added, removed and changed since scene.mark",
    ),
    ("entity.get", "Every component of one entity"),
    (
        "entity.spawn",
        "Create an entity (empty, primitive, light, camera, particles)",
    ),
    (
        "entity.update",
        "Change name, transform, parent, material, lights, script, ...",
    ),
    (
        "entity.remove_component",
        "Remove a component from an entity",
    ),
    ("entity.delete", "Delete an entity and its children"),
    (
        "voxel.spawn",
        "Build a voxel model (text art, voxels or a fill expression) as optimized meshes",
    ),
    ("entity.duplicate", "Copy an entity and its children"),
    (
        "animation.inspect",
        "List the skeletal clips of a glTF/GLB file (names, durations, bones)",
    ),
    (
        "debug.skeleton",
        "Draw the bones of skinned meshes on top of the scene ({enabled})",
    ),
    (
        "character.make",
        "Make an imported model playable: a parent with a CharacterController, a generated AnimationTree, a spring camera and optionally a third-person script ({entity, name, camera, script})",
    ),
    (
        "debug.physics",
        "Draw every simulated collider ({enabled}); report bodies, steps and gravity",
    ),
    (
        "navigation.status",
        "Navmeshes (tiles, polygons, baked or built), agents, obstacles, a running bake and the last one",
    ),
    (
        "navigation.bake",
        "Bake a NavMeshSurface's navmesh in the background into content/navigation ({entity})",
    ),
    (
        "navigation.debug",
        "Draw the navmesh, agents, paths and obstacles ({mesh, agents, paths, obstacles, x_ray, all})",
    ),
    (
        "navigation.path",
        "A path between two points or entities, drawn in the view ({from, to, show, clear})",
    ),
    ("navigation.closest", "The navmesh point nearest a point ({point, radius})"),
    ("navigation.agents", "Every NavAgent's state, position, speed and destination"),
    (
        "ai.status",
        "Every AI agent's brain, state and action, counts by state, and the brains defined",
    ),
    (
        "ai.inspect",
        "One AI agent in depth: blackboards, its tree's nodes or machine's states, senses ({entity})",
    ),
    (
        "ai.define",
        "Define a brain from JSON, a tree's root node or a machine with states ({name, brain})",
    ),
    ("ai.validate", "Check a brain definition without defining it ({brain})"),
    (
        "ai.send",
        "Send an AI event to an agent, or to all / those near a point ({entity | radius, center, tag, group; event, data})",
    ),
    (
        "ai.set",
        "Set a blackboard value ({entity, key, value}; group.key / global.key reach shared boards)",
    ),
    ("ai.noise", "Make a noise agents can hear ({position, radius, tag, data})"),
    ("ai.restart", "Start an agent's brain again ({entity})"),
    ("ai.pause", "Pause or resume an agent's brain ({entity, paused})"),
    (
        "ai.debug",
        "Draw agents' senses, goals, what they perceive and their states ({senses, goals, percepts, states, all})",
    ),
    ("mesh.lathe", "Revolve a radius/height profile into a UV-mapped mesh"),
    ("mesh.extrude", "Extrude a concave XY polygon with bevel, twist and taper"),
    ("mesh.compose", "Combine transformed procedural shapes into one reusable mesh"),
    ("texture.generate", "Generate seeded tileable PNG textures and matching PBR maps"),
    ("texture.maps", "Derive normal, roughness, occlusion and packed maps from a height image"),
    ("audio.generate", "Synthesize a seeded sound effect as a portable PCM WAV file"),
    (
        "lua.eval",
        "Run Lua with the full scripting API and return the result",
    ),
    ("script.status", "Loaded scripts and their last errors"),
    (
        "script.reload",
        "Recompile scripts for an entity, a file, or everything",
    ),
    ("runtime.get", "Current runtime mode"),
    ("runtime.set", "Switch between editor and play mode"),
    ("runtime.step", "Advance explicit fixed simulation steps with step-indexed input"),
    ("scene.raycast", "Raycast visible world-space mesh bounding boxes"),
    ("scene.query", "Query mesh bounds by region, distance and name"),
    ("entity.place", "Place a hierarchy on a bounding-box surface"),
    ("scene.scatter", "Scatter copies using seeded spacing and surface constraints"),
    ("scene.transaction", "Validate, commit or roll back a group of scene edits with aliases"),
    ("mesh.sweep", "Sweep a circular or polygonal cross-section along a 3D path"),
    ("mesh.heightfield", "Build terrain from explicit heights, an image or seeded noise"),
    ("camera.get", "View camera position, target and projection"),
    (
        "camera.set",
        "Move the view camera or focus it on an entity",
    ),
    (
        "environment.get",
        "Clear color, ambient light and HDR sky",
    ),
    ("environment.set", "Change environment settings"),
    (
        "postprocess.get",
        "Post-processing at the view camera and the volumes it is blended from",
    ),
    (
        "graphics.get",
        "The player's graphics settings (display, AA, upscaling, ray tracing, FPS readout)",
    ),
    (
        "light_probes.bake",
        "Bake the scene's light probe volumes (or one) at high quality into content/lighting/",
    ),
    (
        "light_probes.status",
        "The light probe volumes, bake progress and the baked data each shows",
    ),
    (
        "light_probes.debug",
        "Show or hide the light probe debug view (a sphere per probe, lit by its data)",
    ),
    (
        "graphics.set",
        "Change the player's graphics settings or open/close the settings page",
    ),
    (
        "postprocess.set",
        "Tune exposure, tonemapping, colour grading, bloom, SSAO and lens effects on a volume",
    ),
    (
        "assets.list",
        "Meshes, materials and textures loaded in the scene",
    ),
    (
        "asset.reload",
        "Re-read textures and scripts from disk (all, or under a path)",
    ),
    (
        "view.preview",
        "Lay the game out at another view shape ({aspect} or {size: [w, h]}; {} restores)",
    ),
    ("audio.play", "Play a sound file (volume, pitch, loop)"),
    ("audio.stop", "Stop one sound, or all"),
    (
        "audio.inspect",
        "Measure a sound file: length, peak/RMS, clipping, silence, loop seam, waveform and spectrogram",
    ),
    (
        "audio.status",
        "Sounds playing now and recently played (with ids, paths, times)",
    ),
    (
        "input.send",
        "Inject key, mouse button, mouse move and scroll events",
    ),
    ("input.state", "Query keys and mouse buttons currently held"),
    ("input.clear", "Release all input and drop queued events"),
    (
        "logs",
        "Recent log lines, filterable by level, target and text",
    ),
    ("screenshot", "PNG of the window or the scene viewport"),
    (
        "texture.fetch",
        "Read a render target back (G-buffer, depth, HDR scene, shadow maps, path tracing buffers): per-channel statistics, values at points and an image; no texture lists them",
    ),
    (
        "shader.reload",
        "Read the WGSL shaders from src/shader again and rebuild the passes using them; a shader that fails to compile changes nothing and returns the error",
    ),
    (
        "visual.capture",
        "Capture consecutive frames of one frozen view under several render-only post-process variants",
    ),
    (
        "visual.compare",
        "Compare two saved captures (PNG files, or visual.capture directories) with the same noise-aware metrics, without rendering",
    ),
    (
        "render.freeze_noise",
        "Hold the per-frame noise still ({enabled}): TAA jitter phase, grain and dither, live light probe updates; for exact captures",
    ),
    (
        "renderdoc.capture",
        "Capture the next frame with RenderDoc when the engine runs under it; answers with the .rdc path",
    ),
    ("wait", "Answer after a number of frames and/or seconds, or until an absolute frame ({until_frame})"),
];

pub(crate) fn profile_sample(ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    info::stats(ctx)
}
pub(crate) use simulation::apply_event as apply_simulation_event;

/// Access to the engine while handling a remote request.
pub struct RemoteContext<'c, 'w> {
    /// The active scene and the renderer.
    pub gpu: &'c mut GpuUpdateContext<'w>,
    pub(crate) hub: &'c mut RemoteHub,
    app_methods: Vec<&'static str>,
    modified: bool,
}

impl<'c, 'w> RemoteContext<'c, 'w> {
    pub(crate) fn new(
        gpu: &'c mut GpuUpdateContext<'w>,
        hub: &'c mut RemoteHub,
        app_methods: Vec<&'static str>,
    ) -> Self {
        Self {
            gpu,
            hub,
            app_methods,
            modified: false,
        }
    }

    /// Record that the request changed the scene. The application is told once per
    /// frame (see [`super::RemoteApp::remote_scene_modified`]).
    pub fn mark_modified(&mut self) {
        self.modified = true;
    }

    pub(crate) fn take_modified(&mut self) -> bool {
        std::mem::take(&mut self.modified)
    }

    /// Frames completed since remote control started.
    pub fn frame(&self) -> u64 {
        self.hub.frame()
    }

    pub fn runtime_handle(&self) -> Option<RuntimeStateHandle> {
        self.hub.runtime_handle().cloned()
    }

    pub fn runtime_mode(&self) -> Option<RuntimeMode> {
        self.hub
            .runtime_handle()
            .map(RuntimeStateHandle::active_mode)
    }
}

/// Run a built-in method.
pub fn handle(method: &str, params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let empty = json!({});
    let params = if params.is_null() { &empty } else { params };
    if !params.is_object() {
        return Err("params must be a JSON object".into());
    }

    match method {
        "project.validate" => validate_project(params),
        "project.content" => project_content(params),
        "engine.info" => info::engine_info(ctx),
        "engine.methods" => Ok(methods(ctx)),
        "stats" => info::stats_with(params, ctx),
        "visual.compare" => super::visual_compare::compare_files(params),
        "render.freeze_noise" => {
            let enabled = super::json::opt_bool(params, "enabled").unwrap_or(true);
            ctx.gpu.renderer.set_frozen_noise(enabled);
            Ok(json!({ "frozen_noise": enabled }))
        }
        "scene.entities" => info::scene_entities(params, ctx),
        "scene.tree" => info::scene_tree(params, ctx),
        "scene.mark" => diff::mark(ctx),
        "scene.diff" => diff::diff(params, ctx),
        "assets.list" => info::assets_list(params, ctx),
        "logs" => info::logs(params),
        "entity.get" => entity::get(params, ctx),
        "entity.spawn" => entity::spawn(params, ctx),
        "entity.update" => entity::update(params, ctx),
        "entity.remove_component" => entity::remove_component(params, ctx),
        "entity.delete" => entity::delete(params, ctx),
        "entity.duplicate" => entity::duplicate(params, ctx),
        "animation.inspect" => animation::inspect(params, ctx),
        "debug.skeleton" => animation::debug_skeleton(params, ctx),
        "character.make" => animation::make_character(params, ctx),
        "debug.physics" => animation::debug_physics(params, ctx),
        #[cfg(feature = "navigation")]
        "navigation.status" => navigation::status(ctx),
        #[cfg(feature = "navigation")]
        "navigation.bake" => navigation::bake(params, ctx),
        #[cfg(feature = "navigation")]
        "navigation.debug" => navigation::debug(params, ctx),
        #[cfg(feature = "navigation")]
        "navigation.path" => navigation::path(params, ctx),
        #[cfg(feature = "navigation")]
        "navigation.closest" => navigation::closest(params, ctx),
        #[cfg(feature = "navigation")]
        "navigation.agents" => navigation::agents(ctx),
        "ai.status" => ai::status(ctx),
        "ai.inspect" => ai::inspect(params, ctx),
        "ai.define" => ai::define(params, ctx),
        "ai.validate" => ai::validate(params),
        "ai.send" => ai::send(params, ctx),
        "ai.set" => ai::set(params, ctx),
        "ai.noise" => ai::noise(params, ctx),
        "ai.restart" => ai::restart(params, ctx),
        "ai.pause" => ai::pause(params, ctx),
        "ai.debug" => ai::debug(params, ctx),
        "voxel.spawn" => voxel::spawn_voxels(params, ctx),
        "mesh.lathe" | "mesh.extrude" | "mesh.compose" | "mesh.sweep" | "mesh.heightfield" => {
            geometry::generate(method, params, ctx)
        }
        "texture.generate" => procedural::texture_generate(params, ctx, false),
        "texture.maps" => procedural::texture_generate(params, ctx, true),
        "audio.generate" => procedural::sound_generate(params),
        "lua.eval" => scripting::eval(params, ctx),
        "script.status" => scripting::status(ctx),
        "script.reload" => scripting::reload(params, ctx),
        "runtime.get" => scripting::runtime_get(ctx),
        "runtime.set" => scripting::runtime_set(params, ctx),
        "runtime.step" => simulation::step(params, ctx),
        "scene.raycast" => spatial::raycast(params, ctx),
        "scene.query" => spatial::query(params, ctx),
        "entity.place" => spatial::place(params, ctx),
        "scene.scatter" => spatial::scatter(params, ctx),
        "scene.transaction" => transaction::run(params, ctx),
        "camera.get" => rendering::camera_get(ctx),
        "camera.set" => rendering::camera_set(params, ctx),
        "environment.get" => rendering::environment_get(ctx),
        "environment.set" => rendering::environment_set(params, ctx),
        "postprocess.get" => rendering::postprocess_get(ctx),
        "texture.fetch" => crate::remote::texture_fetch::fetch(params, ctx.gpu.renderer),
        "shader.reload" => shader_reload(ctx),
        "graphics.get" => graphics::get(),
        "light_probes.bake" => light_probes::bake(params, ctx),
        "light_probes.status" => light_probes::status(ctx),
        "light_probes.debug" => light_probes::debug(params, ctx),
        "graphics.set" => graphics::set(params),
        "postprocess.set" => rendering::postprocess_set(params, ctx),
        "asset.reload" => rendering::asset_reload(params, ctx),
        "audio.play" => audio_play(params),
        "audio.stop" => match super::json::opt_u64(params, "id") {
            Some(id) => Ok(json!({ "stopped": crate::audio::stop(id) })),
            None => {
                crate::audio::stop_all();
                Ok(json!({ "stopped": "all" }))
            }
        },
        "audio.inspect" => audio_inspect(params),
        "view.preview" => super::preview::set_from_params(params),
        "audio.status" => {
            let recent = crate::audio::history(super::json::opt_u64(params, "since").unwrap_or(0));
            let mut status = json!({
                "playing": crate::audio::playing(),
                "next_id": crate::audio::next_id(),
            });
            if super::json::opt_bool(params, "summary").unwrap_or(false) {
                status["summary"] = summarize_sounds(&recent);
            } else {
                status["recent"] = recent;
            }
            Ok(status)
        }
        "input.send" => input::send(params, ctx),
        "input.state" => input::state(params, ctx),
        "input.clear" => input::clear(ctx),
        _ => Err(format!(
            "unknown method {method:?}; call engine.methods for the list"
        )),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn project_content(params: &Value) -> RemoteResult {
    let path = super::json::opt_str(params, "path")
        .map(std::path::PathBuf::from)
        .or_else(crate::project::active_project_root)
        .ok_or("give path or open a project")?;
    let used = crate::export::used_content(&path)?;
    let list_unused = super::json::opt_bool(params, "list_unused").unwrap_or(true);
    let paths = |files: &mut dyn Iterator<Item = &std::path::PathBuf>| -> Vec<String> {
        files
            .map(|file| file.to_string_lossy().into_owned())
            .collect()
    };
    let mut out = json!({
        "project": path.to_string_lossy(),
        "scenes": used.scenes,
        "files": paths(&mut used.files.iter()),
        "file_count": used.files.len(),
        "bytes": used.bytes,
        "unused_count": used.unused.len(),
        "unused_bytes": used.unused_bytes,
        "problems": used.problems,
        "note": "Exports (the editor's Build menu, web_build, build_web.sh) copy only `files`, with a project.json listing only `scenes`. A script building a path at run time is covered when it names the folder as a literal (\"content/audio/\").",
    });
    if list_unused {
        out["unused"] = json!(paths(&mut used.unused.iter()));
    }
    Ok(out)
}
#[cfg(target_arch = "wasm32")]
fn project_content(_: &Value) -> RemoteResult {
    Err("project content analysis requires native authoring".into())
}

#[cfg(not(target_arch = "wasm32"))]
fn validate_project(params: &Value) -> RemoteResult {
    let path = super::json::opt_str(params, "path")
        .map(std::path::PathBuf::from)
        .or_else(crate::project::active_project_root)
        .ok_or("give path or open a project")?;
    super::validation::validate(&path)
}
#[cfg(target_arch = "wasm32")]
fn validate_project(_: &Value) -> RemoteResult {
    Err("project validation requires native authoring".into())
}

#[cfg(not(target_arch = "wasm32"))]
fn audio_inspect(params: &Value) -> RemoteResult {
    let path = crate::audio::resolve_path(super::json::req_str(params, "path")?)?;
    let picture = super::json::opt_bool(params, "image").unwrap_or(true);
    let (mut report, png) = crate::audio_inspect::inspect(&path, picture)?;
    if let Some(png) = png {
        report["png_base64"] = json!(base64::encode(&png));
        report["image"] = json!(
            "top: waveform (red where it clips); bottom: spectrogram, 30 Hz (bottom) to Nyquist (top) on a log scale, -90..0 dB"
        );
    }
    Ok(report)
}

#[cfg(target_arch = "wasm32")]
fn audio_inspect(_params: &Value) -> RemoteResult {
    Err("audio.inspect needs a native build".into())
}

/// Group a sound history by file: how often each played, when first and last, and the
/// volumes and pitches used. Files are listed in the order they first played.
pub(crate) fn summarize_sounds(history: &Value) -> Value {
    let mut groups: Vec<Value> = Vec::new();
    for played in history.as_array().into_iter().flatten() {
        let path = played["path"].as_str().unwrap_or("");
        let index = match groups.iter().position(|group| group["path"] == path) {
            Some(index) => index,
            None => {
                groups.push(json!({
                    "path": path,
                    "count": 0,
                    "first_time": played["time"],
                    "last_time": played["time"],
                    "pitch": [played["pitch"], played["pitch"]],
                    "volume": [played["volume"], played["volume"]],
                }));
                groups.len() - 1
            }
        };
        let group = &mut groups[index];
        group["count"] = json!(group["count"].as_u64().unwrap_or(0) + 1);
        group["last_time"] = played["time"].clone();
        for key in ["pitch", "volume"] {
            let value = played[key].as_f64().unwrap_or(0.0);
            let low = group[key][0].as_f64().unwrap_or(value).min(value);
            let high = group[key][1].as_f64().unwrap_or(value).max(value);
            group[key] = json!([low, high]);
        }
        if played["audible"] == json!(false) {
            group["silent"] = json!(group["silent"].as_u64().unwrap_or(0) + 1);
        }
    }
    json!(groups)
}

/// Reload shaders from disk now, rather than waiting for the file watcher.
fn shader_reload(ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let result = ctx.gpu.renderer.reload_shaders();
    crate::renderer::shader_source::log_reload(&result);
    let reload = result?;
    Ok(json!({
        "changed": reload.changed,
        "not_reloaded": reload.not_reloaded,
        "note": if reload.changed.is_empty() {
            "no shader text differed from what was already in use"
        } else {
            "rebuilt; temporal effects (TAA, the path tracer's history) start over"
        },
    }))
}

fn audio_play(params: &Value) -> RemoteResult {
    use super::json::{opt_bool, opt_f64, req_str};
    let path = req_str(params, "path")?;
    let options = crate::audio::PlayOptions {
        volume: opt_f64(params, "volume").unwrap_or(1.0).max(0.0) as f32,
        pitch: opt_f64(params, "pitch").unwrap_or(1.0) as f32,
        looped: opt_bool(params, "loop").unwrap_or(false),
    };
    let since = crate::audio::next_id();
    let id = crate::audio::play(path, options)?;
    Ok(json!({ "id": id, "played": crate::audio::history(since)[0].clone() }))
}

fn methods(ctx: &RemoteContext<'_, '_>) -> Value {
    let core: Vec<Value> = CORE_METHODS
        .iter()
        .map(|(name, description)| json!({ "name": name, "description": description }))
        .collect();
    json!({ "core": core, "application": ctx.app_methods })
}
