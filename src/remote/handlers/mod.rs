//! Built-in remote methods that work in any application.
//!
//! Every method takes a JSON object of parameters and returns a JSON value. See
//! [`CORE_METHODS`] for the list and [`super::json`] for value conventions.

mod entity;
mod info;
mod input;
mod rendering;
mod scripting;

use serde_json::{json, Value};

use crate::app::{GpuUpdateContext, RuntimeMode, RuntimeStateHandle};

use super::hub::RemoteHub;

/// Outcome of a remote method: a JSON result or an error message.
pub type RemoteResult = Result<Value, String>;

/// Built-in methods with a one-line description each.
pub const CORE_METHODS: &[(&str, &str)] = &[
    (
        "engine.info",
        "Frame, runtime mode, active scene, surface size, project root",
    ),
    (
        "engine.methods",
        "List the methods this application understands",
    ),
    ("stats", "Frame time and renderer statistics"),
    (
        "scene.entities",
        "Flat list of entities, filterable by name or component",
    ),
    ("scene.tree", "Entity hierarchy as a tree"),
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
    ("entity.duplicate", "Copy an entity and its children"),
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
    ("camera.get", "View camera position, target and projection"),
    (
        "camera.set",
        "Move the view camera or focus it on an entity",
    ),
    (
        "environment.get",
        "Clear color, ambient light, HDR sky and color grading",
    ),
    ("environment.set", "Change environment settings"),
    ("postprocess.get", "Post-processing settings"),
    ("postprocess.set", "Toggle and tune SSAO, bloom and FXAA"),
    (
        "assets.list",
        "Meshes, materials and textures loaded in the scene",
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
    ("wait", "Answer after a number of frames and/or seconds"),
];

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
        "engine.info" => info::engine_info(ctx),
        "engine.methods" => Ok(methods(ctx)),
        "stats" => info::stats(ctx),
        "scene.entities" => info::scene_entities(params, ctx),
        "scene.tree" => info::scene_tree(params, ctx),
        "assets.list" => info::assets_list(params, ctx),
        "logs" => info::logs(params),
        "entity.get" => entity::get(params, ctx),
        "entity.spawn" => entity::spawn(params, ctx),
        "entity.update" => entity::update(params, ctx),
        "entity.remove_component" => entity::remove_component(params, ctx),
        "entity.delete" => entity::delete(params, ctx),
        "entity.duplicate" => entity::duplicate(params, ctx),
        "lua.eval" => scripting::eval(params, ctx),
        "script.status" => scripting::status(ctx),
        "script.reload" => scripting::reload(params, ctx),
        "runtime.get" => scripting::runtime_get(ctx),
        "runtime.set" => scripting::runtime_set(params, ctx),
        "camera.get" => rendering::camera_get(ctx),
        "camera.set" => rendering::camera_set(params, ctx),
        "environment.get" => rendering::environment_get(ctx),
        "environment.set" => rendering::environment_set(params, ctx),
        "postprocess.get" => Ok(rendering::postprocess_json(
            &ctx.gpu.renderer.postprocess_effects(),
        )),
        "postprocess.set" => rendering::postprocess_set(params, ctx),
        "input.send" => input::send(params, ctx),
        "input.state" => input::state(params, ctx),
        "input.clear" => input::clear(ctx),
        _ => Err(format!(
            "unknown method {method:?}; call engine.methods for the list"
        )),
    }
}

fn methods(ctx: &RemoteContext<'_, '_>) -> Value {
    let core: Vec<Value> = CORE_METHODS
        .iter()
        .map(|(name, description)| json!({ "name": name, "description": description }))
        .collect();
    json!({ "core": core, "application": ctx.app_methods })
}
