//! Script lifecycle management for a single ECS world.
//!
//! [`ScriptingState`] owns a Lua VM and one script instance per entity that carries a
//! [`LuaScriptComponent`]. Each frame it:
//!
//! 1. applies commands that `on_ui` recorded during the previous UI pass,
//! 2. compiles scripts for new or replaced components and tears down removed ones,
//! 3. runs `on_created`, `update` and coroutines for scripts allowed in the current mode,
//! 4. dispatches queued events with a per-frame budget.
//!
//! Script instances are tied to their component through the component's binding id, so
//! replacing or cloning a component always produces a fresh instance. One state drives
//! one world; use a separate state per world.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use hecs::{Entity, World};
use log::{error, warn};
use mlua::LuaSerdeExt;

use super::api::ui::{UiCommand, UiContext, UiResponse};
use super::api::CoroutineGuard;
use super::commands::{
    entity_bits, PendingAssetCommand, PendingGltfImport, PendingPostProcess, ScriptCommands,
};
use super::component::{FunctionCallOutcome, LuaScriptComponent, LuaScriptInstance};
use super::entity_registry::EntityHandleRegistry;
use super::error::LuaScriptingError;
use super::guards::{CommandGuard, EntityGuard, EventQueueGuard, StateGuard, WorldGuard};
use super::runtime::LuaScriptingRuntime;
use super::types::{
    CoroutineMap, EventSubscription, EventSubscriptions, LuaScriptSource, ScriptEvent, ScriptMode,
    ScriptStateMap,
};

/// Maximum number of events dispatched in one frame. Anything beyond this is dropped,
/// which stops event feedback loops (a handler re-emitting its own event) from hanging
/// the engine.
const MAX_EVENTS_PER_FRAME: usize = 4096;

/// Distinct error messages remembered per script, used to avoid logging the same
/// failure every frame.
const MAX_REPORTED_ERRORS: usize = 32;

/// Viewport information handed to `on_ui` as `(width, height, pixels_per_point)` in
/// logical points.
pub type UiViewportInfo = (f32, f32, f32);

/// Returns whether a script with `mode` runs when the engine is in editor (or play) mode.
fn runs_in_mode(mode: ScriptMode, editor_mode: bool) -> bool {
    match mode {
        ScriptMode::RuntimeOnly => !editor_mode,
        ScriptMode::EditorOnly => editor_mode,
        ScriptMode::Both => true,
    }
}

/// A lifecycle hook invoked on a script instance.
enum Hook<'a> {
    Chunk,
    Created,
    Update(f64),
    /// `reload` is true when the script is being rebuilt rather than removed.
    Destroyed {
        reload: bool,
    },
    Event {
        callback: &'a str,
        data: &'a serde_json::Value,
    },
    /// `callback(self_entity, data)`, skipped when the script does not define it.
    Callback {
        callback: &'a str,
        data: &'a serde_json::Value,
    },
    Coroutines(f64),
}

impl Hook<'_> {
    fn name(&self) -> &str {
        match self {
            Hook::Chunk => "load",
            Hook::Created => "on_created",
            Hook::Update(_) => "update",
            Hook::Destroyed { .. } => "on_destroyed",
            Hook::Event { callback, .. } | Hook::Callback { callback, .. } => callback,
            Hook::Coroutines(_) => "coroutine",
        }
    }
}

/// The runtime side of one entity's script.
struct ScriptSlot {
    /// Binding id of the component this slot was built for.
    binding: u64,
    source: LuaScriptSource,
    /// `None` when the script failed to compile; it is retried after a reload.
    instance: Option<LuaScriptInstance>,
    created: bool,
    /// Most recent error as `(context, message)`. Cleared once the failing hook
    /// succeeds again; compile and load errors stay until the script is reloaded.
    last_error: Option<(String, String)>,
    reported_errors: HashSet<String>,
    cpu_ms: f64,
    invocations: u64,
}

impl ScriptSlot {
    fn report(&mut self, entity: Entity, context: &str, err: impl std::fmt::Display) {
        let message = format!("{context}: {err}");
        if self.reported_errors.len() < MAX_REPORTED_ERRORS
            && self.reported_errors.insert(message.clone())
        {
            error!(target: "script", "[{}] entity {entity:?}: {message}", self.source.display_name());
        }
        self.last_error = Some((context.to_owned(), message));
    }

    fn clear_error(&mut self, context: &str) {
        if self
            .last_error
            .as_ref()
            .is_some_and(|(failed, _)| failed == context)
        {
            self.last_error = None;
        }
    }
}

/// Side effects produced by scripts that outlive a single hook invocation.
#[derive(Default)]
struct ScriptOutputs {
    pending_gltf_imports: Vec<PendingGltfImport>,
    pending_asset_commands: Vec<PendingAssetCommand>,
    event_subscriptions: EventSubscriptions,
    pending_events: VecDeque<ScriptEvent>,
    /// Set when applied commands despawned entities.
    entities_despawned: bool,
    /// `emit_particles` requests waiting for the particle runtime.
    particle_bursts: Vec<crate::gpu_particles::ParticleBurst>,
    /// `set_postprocess` changes waiting for the scene.
    pending_post_process: Vec<PendingPostProcess>,
    /// `set_environment` changes waiting for the scene.
    pending_environment: Vec<serde_json::Value>,
    /// Physics changes waiting for the scene's simulation.
    physics_commands: Vec<crate::physics::PhysicsCommand>,
    /// Navigation changes waiting for the scene's agents.
    #[cfg(feature = "navigation")]
    navigation_commands: Vec<crate::navigation::NavCommand>,
    /// Entities waiting for a generated locomotion tree.
    pending_trees: Vec<Entity>,
}

impl ScriptOutputs {
    /// Apply a script's command buffer to the world. Failures are reported per command
    /// and never prevent other scripts from running.
    fn apply_commands(&mut self, world: &mut World, commands: &RefCell<ScriptCommands>) {
        let result = match commands.borrow_mut().apply(world) {
            Ok(result) => result,
            Err(err) => {
                warn!(target: "script", "Failed to apply script commands: {err}");
                return;
            }
        };

        self.pending_gltf_imports.extend(result.gltf_imports);
        self.pending_asset_commands.extend(result.asset_commands);
        self.particle_bursts.extend(result.particle_bursts);
        self.physics_commands.extend(result.physics);
        #[cfg(feature = "navigation")]
        self.navigation_commands.extend(result.navigation);
        self.pending_trees.extend(result.generate_trees);
        self.pending_post_process.extend(result.post_process);
        self.pending_environment.extend(result.environment);

        self.entities_despawned |= !result.despawned.is_empty();
        for entity in result.despawned {
            self.remove_subscriptions(entity);
        }

        for sub in result.event_subscriptions {
            let subscribers = self.event_subscriptions.entry(sub.event_name).or_default();
            let already_subscribed = subscribers
                .iter()
                .any(|s| s.entity_id == sub.entity && s.callback_name == sub.callback_name);
            if !already_subscribed {
                subscribers.push(EventSubscription {
                    entity_id: sub.entity,
                    callback_name: sub.callback_name,
                });
            }
        }

        for unsub in result.event_unsubscriptions {
            if let Some(subs) = self.event_subscriptions.get_mut(&unsub.event_name) {
                subs.retain(|s| s.entity_id != unsub.entity);
            }
        }
    }

    fn remove_subscriptions(&mut self, entity: Entity) {
        for subscriptions in self.event_subscriptions.values_mut() {
            subscriptions.retain(|subscription| subscription.entity_id != entity);
        }
        self.event_subscriptions
            .retain(|_, subscriptions| !subscriptions.is_empty());
    }
}

/// Invoke `hook` on the script in `slot`, then apply its commands and queue its events.
///
/// Returns `None` if the script is not loaded or the hook raised an error (which is
/// reported on the slot).
///
/// Scripts get read access to `world` while they run; their mutations are recorded as
/// commands and applied once the call returns.
fn invoke(
    runtime: &LuaScriptingRuntime,
    outputs: &mut ScriptOutputs,
    world: &mut World,
    entity: Entity,
    slot: &mut ScriptSlot,
    hook: Hook<'_>,
) -> Option<FunctionCallOutcome> {
    let instance = slot.instance.as_mut()?;
    let started = crate::time::Instant::now();

    let lua = runtime.lua();
    let bits = entity_bits(entity);
    let commands = instance.command_buffer();
    let events = Rc::new(RefCell::new(Vec::new()));

    let result = {
        let _world_guard = WorldGuard::enter(world);
        let (cmds, evts) = (commands.clone(), events.clone());
        match &hook {
            Hook::Chunk => instance.run_chunk(lua, bits, cmds, evts),
            Hook::Created => instance.call_on_created(lua, bits, cmds, evts),
            Hook::Update(dt) => instance.call_update(lua, bits, *dt, cmds, evts),
            Hook::Destroyed { reload } => {
                instance.call_on_destroyed(lua, bits, *reload, cmds, evts)
            }
            Hook::Event { callback, data } => {
                instance.call_event(lua, bits, callback, (*data).clone(), cmds, evts)
            }
            Hook::Callback { callback, data } => {
                instance.call_callback(lua, bits, callback, (*data).clone(), cmds, evts)
            }
            Hook::Coroutines(dt) => instance
                .process_coroutines(*dt, cmds, evts)
                .map(|()| FunctionCallOutcome::Executed),
        }
    };

    // Apply whatever the script recorded, even if it failed part way through.
    outputs.apply_commands(world, &commands);
    outputs.pending_events.extend(events.take());
    slot.cpu_ms += started.elapsed().as_secs_f64() * 1000.0;
    slot.invocations += 1;

    match result {
        Ok(outcome) => {
            slot.clear_error(hook.name());
            Some(outcome)
        }
        Err(err) => {
            slot.report(entity, hook.name(), err);
            None
        }
    }
}

/// Maximum number of `print` lines kept from one [`ScriptingState::eval`] call.
const MAX_EVAL_PRINT_LINES: usize = 1000;

/// Result of running a chunk with [`ScriptingState::eval`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LuaEvalOutput {
    /// Values the chunk returned, converted to JSON. Values that have no JSON form
    /// (functions, userdata) are returned as their `tostring` text.
    pub values: Vec<serde_json::Value>,
    /// Lines the chunk passed to `print`.
    pub printed: Vec<String>,
}

/// Environment and per-instance state for console code that is not bound to a script.
///
/// It persists between [`ScriptingState::eval`] calls, so globals defined by one call
/// are visible to the next, like a REPL.
struct ConsoleScope {
    env_registry_key: mlua::RegistryKey,
    handles: Rc<RefCell<EntityHandleRegistry>>,
    state_store: Rc<RefCell<ScriptStateMap>>,
    coroutines: Rc<RefCell<CoroutineMap>>,
}

/// Owns the Lua runtime and the script instances of one ECS world.
pub struct ScriptingState {
    runtime: LuaScriptingRuntime,
    slots: HashMap<Entity, ScriptSlot>,
    outputs: ScriptOutputs,
    /// State to restore into scripts once they are re-created after a reload.
    pending_state_restoration: HashMap<Entity, ScriptStateMap>,
    /// What `on_destroyed(self, "reload")` returned, for the reloaded `on_created`.
    pending_carried: HashMap<Entity, mlua::RegistryKey>,
    /// Spawn handles retained by the state handed to the replacement script.
    pending_handles: HashMap<Entity, Rc<RefCell<EntityHandleRegistry>>>,
    /// Scripts to rebuild: `on_destroyed` runs for them at the start of the next frame
    /// (it needs the world), then they are recompiled and `on_created` runs again.
    pending_reloads: Vec<Entity>,
    /// Seconds of frame steps scripts have run for since the last reset.
    elapsed: f64,
    /// Where the game view is in the window (x, y, width, height in pixels), as of the
    /// last rendered frame.
    view_rect: Option<[f32; 4]>,
    /// Commands recorded by `on_ui`, applied at the start of the next update.
    deferred_ui_commands: Vec<(Entity, Rc<RefCell<ScriptCommands>>)>,
    /// UI responses from the previous frame.
    ui_responses: HashMap<Entity, HashMap<String, UiResponse>>,
    /// Environment used by [`Self::eval`] when no entity is given; created on first use.
    console: Option<ConsoleScope>,
    /// Engine callbacks (`on_landed`, `on_trigger_enter`, ...) for the next frame's scripts.
    pending_callbacks: Vec<(Entity, &'static str, serde_json::Value)>,
}

impl ScriptingState {
    /// Cumulative CPU hook cost per script, reset when the script is rebuilt.
    pub fn script_timings(&self) -> Vec<serde_json::Value> {
        let mut rows = self
            .slots
            .iter()
            .map(|(entity, slot)| {
                serde_json::json!({
                    "entity":entity.to_bits().get(),"source":slot.source.display_name(),
                    "cpu_ms":slot.cpu_ms,"invocations":slot.invocations
                })
            })
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| {
            b["cpu_ms"]
                .as_f64()
                .unwrap()
                .total_cmp(&a["cpu_ms"].as_f64().unwrap())
        });
        rows
    }
    /// Seed the VM's shared Lua PRNG before compiling scripts or running on_created.
    ///
    /// ```
    /// let mut scripts = wgpu_cube::scripting::ScriptingState::new().unwrap();
    /// scripts.seed_random(123).unwrap();
    /// assert!(scripts.script_timings().is_empty());
    /// ```
    pub fn seed_random(&mut self, seed: u64) -> Result<(), LuaScriptingError> {
        let math: mlua::Table = self.runtime.lua().globals().get("math")?;
        let seed_random: mlua::Function = math.get("randomseed")?;
        seed_random.call::<()>((seed as i64, 0i64))?;
        Ok(())
    }

    /// Construct a new scripting state.
    pub fn new() -> Result<Self, LuaScriptingError> {
        Ok(Self {
            runtime: LuaScriptingRuntime::new()?,
            slots: HashMap::new(),
            outputs: ScriptOutputs::default(),
            pending_state_restoration: HashMap::new(),
            pending_carried: HashMap::new(),
            pending_handles: HashMap::new(),
            pending_reloads: Vec::new(),
            elapsed: 0.0,
            view_rect: None,
            deferred_ui_commands: Vec::new(),
            ui_responses: HashMap::new(),
            console: None,
            pending_callbacks: Vec::new(),
        })
    }

    /// Access the underlying runtime.
    pub fn runtime(&self) -> &LuaScriptingRuntime {
        &self.runtime
    }

    /// Mutably access the underlying runtime.
    pub fn runtime_mut(&mut self) -> &mut LuaScriptingRuntime {
        &mut self.runtime
    }

    /// Tell scripts where the game view is in the window, in pixels (x, y, width, height).
    pub fn set_view_rect(&mut self, rect: Option<[f32; 4]>) {
        self.view_rect = rect.filter(|rect| rect[2] > 0.0 && rect[3] > 0.0);
    }

    /// The game view's area, if the renderer has reported one.
    pub fn view_rect(&self) -> Option<[f32; 4]> {
        self.view_rect
    }

    /// Give scripts the camera to convert between the screen and the world with
    /// (`view_proj` is built for the aspect ratio of [`Self::view_rect`]).
    pub fn set_view_projection(&mut self, view_proj: Option<glam::Mat4>) {
        let lua = self.runtime.lua();
        match (view_proj, self.view_rect) {
            (Some(view_proj), Some(rect)) => {
                lua.set_app_data(super::api::ScriptView { view_proj, rect });
            }
            _ => {
                lua.remove_app_data::<super::api::ScriptView>();
            }
        }
    }

    /// Recompile every script and run `on_created` again on the next update.
    ///
    /// Values scripts stored with the state API (`set_f64`, `set_state`, ...) survive
    /// the reset and are restored after `on_created` runs.
    pub fn reset_runtime(&mut self) {
        let entities: Vec<Entity> = self.slots.keys().copied().collect();
        for entity in entities {
            self.unload(entity);
        }
        // A reset (entering or leaving play) starts games fresh.
        self.pending_carried.clear();
        self.pending_handles.clear();
        self.outputs = ScriptOutputs::default();
        self.pending_reloads.clear();
        self.elapsed = 0.0;
        self.deferred_ui_commands.clear();
        self.ui_responses.clear();
        self.pending_callbacks.clear();
    }

    /// Hot-reload every script loaded from `path`, preserving script state.
    ///
    /// Returns the number of scripts scheduled for reload.
    pub fn reload_path(&mut self, path: &Path) -> usize {
        let target = canonical(path);
        let script_root = self.runtime.script_root().map(Path::to_path_buf);
        let entities: Vec<Entity> = self
            .slots
            .iter()
            .filter(|(_, slot)| {
                let own = match &slot.source {
                    LuaScriptSource::File { path } => {
                        let resolved = match &script_root {
                            Some(root) if path.is_relative() => root.join(path),
                            _ => path.clone(),
                        };
                        canonical(&resolved) == target
                    }
                    LuaScriptSource::Inline { .. } => false,
                };
                // Scripts that `require` or `include` the file reload with it.
                own || slot.instance.as_ref().is_some_and(|instance| {
                    instance
                        .dependencies
                        .lock()
                        .is_ok_and(|deps| deps.iter().any(|dep| canonical(dep) == target))
                })
            })
            .map(|(entity, _)| *entity)
            .collect();

        for entity in &entities {
            self.schedule_reload(*entity);
        }
        entities.len()
    }

    /// Recompile the script on `entity` and run `on_created` again, preserving its state.
    ///
    /// Returns false if the entity has no script loaded.
    pub fn reload_entity(&mut self, entity: Entity) -> bool {
        if !self.slots.contains_key(&entity) {
            return false;
        }
        self.schedule_reload(entity);
        true
    }

    fn schedule_reload(&mut self, entity: Entity) {
        if !self.pending_reloads.contains(&entity) {
            self.pending_reloads.push(entity);
        }
    }

    /// Run `on_destroyed` for scripts scheduled for a reload, then drop their instances
    /// (keeping their state) so the next sync recompiles them.
    fn run_pending_reloads(&mut self, world: &mut World) {
        for entity in std::mem::take(&mut self.pending_reloads) {
            if let Some(slot) = self.slots.get_mut(&entity) {
                if slot.created {
                    invoke(
                        &self.runtime,
                        &mut self.outputs,
                        world,
                        entity,
                        slot,
                        Hook::Destroyed { reload: true },
                    );
                }
            }
            self.unload(entity);
            self.outputs.remove_subscriptions(entity);
        }
    }

    /// Drop the instance for `entity` without running `on_destroyed`, keeping its state
    /// so it can be restored when the script is re-created.
    fn unload(&mut self, entity: Entity) {
        if let Some(slot) = self.slots.remove(&entity) {
            if let Some(mut instance) = slot.instance {
                self.pending_handles
                    .insert(entity, instance.handles.clone());
                let state = instance.state_store.borrow().clone();
                if !state.is_empty() {
                    self.pending_state_restoration.insert(entity, state);
                }
                if let Some(carried) = instance.carried.take() {
                    self.pending_carried.insert(entity, carried);
                }
            }
        }
    }

    /// Most recent error raised by the script on `entity`, if any.
    pub fn last_error(&self, entity: Entity) -> Option<&str> {
        self.slots
            .get(&entity)?
            .last_error
            .as_ref()
            .map(|(_, message)| message.as_str())
    }

    /// Entities that currently have a script instance, with their source and whether
    /// it compiled. Useful for diagnostics.
    pub fn script_entities(&self) -> Vec<(Entity, String, bool)> {
        let mut entries: Vec<_> = self
            .slots
            .iter()
            .map(|(entity, slot)| (*entity, slot.source.display_name(), slot.instance.is_some()))
            .collect();
        entries.sort_unstable_by_key(|(entity, ..)| entity.to_bits());
        entries
    }

    /// Entities whose script environment defines the global `name`.
    pub fn entities_with_global(&self, name: &str) -> Vec<Entity> {
        let lua = self.runtime.lua();
        let mut entities: Vec<Entity> = self
            .slots
            .iter()
            .filter(|(_, slot)| {
                slot.instance.as_ref().is_some_and(|instance| {
                    lua.registry_value::<mlua::Table>(&instance.env_registry_key)
                        .and_then(|env| env.raw_get::<mlua::Value>(name))
                        .is_ok_and(|value| !value.is_nil())
                })
            })
            .map(|(entity, _)| *entity)
            .collect();
        entities.sort_unstable_by_key(|entity| entity.to_bits());
        entities
    }

    /// Run `code` against `world` with the full scripting API, like a console.
    ///
    /// With `entity`, the code runs inside that entity's script environment: it reads
    /// and assigns the script's globals, the state API uses the script's state and
    /// `get_self()` returns the entity. Without it, the code runs in a console
    /// environment that persists between calls.
    ///
    /// A single expression is evaluated and returned (`1 + 1` yields `2`). World changes
    /// are recorded as commands and applied when the chunk returns, as for any hook;
    /// emitted events are delivered on the next update.
    pub fn eval(
        &mut self,
        world: &mut World,
        code: &str,
        entity: Option<Entity>,
    ) -> Result<LuaEvalOutput, LuaScriptingError> {
        let lua = self.runtime.lua();

        let (base_env, handles, state_store, coroutines) = match entity {
            Some(entity) => {
                let instance = self
                    .slots
                    .get(&entity)
                    .and_then(|slot| slot.instance.as_ref())
                    .ok_or_else(|| {
                        LuaScriptingError::Lua(mlua::Error::RuntimeError(format!(
                            "entity {entity:?} has no loaded script instance"
                        )))
                    })?;
                (
                    lua.registry_value::<mlua::Table>(&instance.env_registry_key)?,
                    instance.handles.clone(),
                    instance.state_store.clone(),
                    instance.coroutines.clone(),
                )
            }
            None => {
                if self.console.is_none() {
                    let env = lua.create_table()?;
                    let metatable = lua.create_table()?;
                    metatable.set("__index", lua.globals())?;
                    env.set_metatable(Some(metatable));
                    self.console = Some(ConsoleScope {
                        env_registry_key: lua.create_registry_value(env)?,
                        handles: Rc::new(RefCell::new(EntityHandleRegistry::default())),
                        state_store: Rc::new(RefCell::new(HashMap::new())),
                        coroutines: Rc::new(RefCell::new(HashMap::new())),
                    });
                }
                let console = self.console.as_ref().expect("console scope created above");
                (
                    lua.registry_value::<mlua::Table>(&console.env_registry_key)?,
                    console.handles.clone(),
                    console.state_store.clone(),
                    console.coroutines.clone(),
                )
            }
        };

        // A scope over the base environment: reads fall through to it and assignments
        // land in it, while `print` is captured for this call only.
        // mlua is built with `send`, so callbacks must be Send.
        let printed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let scope = lua.create_table()?;
        let metatable = lua.create_table()?;
        metatable.set("__index", base_env.clone())?;
        metatable.set("__newindex", base_env)?;
        scope.set_metatable(Some(metatable));
        let sink = printed.clone();
        scope.raw_set(
            "print",
            lua.create_function(move |lua, args: mlua::MultiValue| {
                let mut parts = Vec::with_capacity(args.len());
                for value in args {
                    parts.push(lua_display(lua, &value));
                }
                if let Ok(mut sink) = sink.lock() {
                    if sink.len() < MAX_EVAL_PRINT_LINES {
                        sink.push(parts.join("\t"));
                    }
                }
                Ok(())
            })?,
        )?;

        // Try the code as an expression first so `1 + 1` returns its value.
        let function = match lua
            .load(format!("return {code}"))
            .set_name("=console")
            .set_environment(scope.clone())
            .into_function()
        {
            Ok(function) => function,
            Err(_) => lua
                .load(code)
                .set_name("=console")
                .set_environment(scope)
                .into_function()?,
        };

        let bits = entity.map_or(0, entity_bits);
        let commands = Rc::new(RefCell::new(ScriptCommands::new(handles)));
        let events = Rc::new(RefCell::new(Vec::new()));
        let result = {
            if bits != 0 {
                commands
                    .borrow()
                    .registry
                    .borrow_mut()
                    .resolve_bits(bits, bits as u64);
            }
            let _world_guard = WorldGuard::enter(world);
            let _commands_guard = CommandGuard::enter(commands.clone());
            let _state_guard = StateGuard::enter(&state_store);
            let _event_guard = EventQueueGuard::enter(&events);
            let _entity_guard = EntityGuard::enter(bits);
            let _coroutine_guard = CoroutineGuard::enter(&coroutines);
            function.call::<mlua::MultiValue>(())
        };

        self.outputs.apply_commands(world, &commands);
        self.outputs.pending_events.extend(events.take());

        let values = result?
            .into_iter()
            .map(|value| {
                lua.from_value::<serde_json::Value>(value.clone())
                    .unwrap_or_else(|_| serde_json::Value::String(lua_display(lua, &value)))
            })
            .collect();

        Ok(LuaEvalOutput {
            values,
            printed: printed
                .lock()
                .map(|mut lines| std::mem::take(&mut *lines))
                .unwrap_or_default(),
        })
    }

    /// Process a single frame of script execution.
    ///
    /// Script errors are logged and recorded per script (see [`Self::last_error`]);
    /// they never abort the frame for other scripts.
    pub fn process_scripts(
        &mut self,
        world: &mut World,
        dt: f64,
        editor_mode: bool,
    ) -> Result<(), LuaScriptingError> {
        for (_, commands) in std::mem::take(&mut self.deferred_ui_commands) {
            self.outputs.apply_commands(world, &commands);
        }

        self.elapsed += dt.max(0.0);
        self.runtime
            .lua()
            .set_app_data(super::api::PlayMode(!editor_mode));
        self.runtime
            .lua()
            .set_app_data(super::api::ScriptClock(self.elapsed));
        self.run_pending_reloads(world);
        self.sync_instances(world);

        let active = self.active_entities(world, editor_mode);

        for &entity in &active {
            let Some(slot) = self.slots.get_mut(&entity) else {
                continue;
            };
            if slot.created || slot.instance.is_none() {
                continue;
            }
            slot.created = true;
            invoke(
                &self.runtime,
                &mut self.outputs,
                world,
                entity,
                slot,
                Hook::Created,
            );
        }

        for (entity, callback, data) in std::mem::take(&mut self.pending_callbacks) {
            if !active.contains(&entity) {
                continue;
            }
            if let Some(slot) = self.slots.get_mut(&entity) {
                invoke(
                    &self.runtime,
                    &mut self.outputs,
                    world,
                    entity,
                    slot,
                    Hook::Callback {
                        callback,
                        data: &data,
                    },
                );
            }
        }

        // Restore state after on_created so restored values win over its defaults.
        self.pending_state_restoration.retain(|entity, state| {
            match self
                .slots
                .get(entity)
                .and_then(|slot| slot.instance.as_ref())
            {
                Some(instance) => {
                    *instance.state_store.borrow_mut() = std::mem::take(state);
                    false
                }
                None => true,
            }
        });

        if dt > 0.0 || editor_mode {
            for &entity in &active {
                if let Some(slot) = self.slots.get_mut(&entity) {
                    invoke(
                        &self.runtime,
                        &mut self.outputs,
                        world,
                        entity,
                        slot,
                        Hook::Update(dt),
                    );
                }
            }
        }

        for &entity in &active {
            if let Some(slot) = self.slots.get_mut(&entity) {
                invoke(
                    &self.runtime,
                    &mut self.outputs,
                    world,
                    entity,
                    slot,
                    Hook::Coroutines(dt),
                );
            }
        }

        self.dispatch_events(world);

        Ok(())
    }

    /// Entities whose scripts run in the current mode, in a stable order.
    fn active_entities(&self, world: &World, editor_mode: bool) -> Vec<Entity> {
        let mut active: Vec<Entity> = world
            .query::<&LuaScriptComponent>()
            .iter()
            .filter(|(entity, component)| {
                runs_in_mode(component.script_mode(), editor_mode)
                    && self
                        .slots
                        .get(entity)
                        .is_some_and(|slot| slot.binding == component.binding())
            })
            .map(|(entity, _)| entity)
            .collect();
        active.sort_unstable_by_key(|entity| entity.to_bits());
        active
    }

    /// Create instances for new or replaced components and tear down stale ones.
    fn sync_instances(&mut self, world: &mut World) {
        let mut wanted: Vec<(Entity, u64, LuaScriptSource)> = Vec::new();
        let mut live: HashMap<Entity, u64> = HashMap::new();
        for (entity, component) in world.query::<&LuaScriptComponent>().iter() {
            live.insert(entity, component.binding());
            let current = self.slots.get(&entity).map(|slot| slot.binding);
            if current != Some(component.binding()) {
                wanted.push((entity, component.binding(), component.source().clone()));
            }
        }

        let mut stale: Vec<Entity> = self
            .slots
            .iter()
            .filter(|(entity, slot)| live.get(entity) != Some(&slot.binding))
            .map(|(entity, _)| *entity)
            .collect();
        stale.sort_unstable_by_key(|entity| entity.to_bits());

        for entity in stale {
            let Some(mut slot) = self.slots.remove(&entity) else {
                continue;
            };
            if slot.created {
                invoke(
                    &self.runtime,
                    &mut self.outputs,
                    world,
                    entity,
                    &mut slot,
                    Hook::Destroyed { reload: false },
                );
            }
            self.outputs.remove_subscriptions(entity);
            self.ui_responses.remove(&entity);
            log::debug!(target: "script", "Removed script instance for entity {entity:?}");
        }

        wanted.sort_unstable_by_key(|(entity, ..)| entity.to_bits());
        for (entity, binding, source) in wanted {
            let mut slot = ScriptSlot {
                binding,
                source: source.clone(),
                instance: None,
                created: false,
                last_error: None,
                reported_errors: HashSet::new(),
                cpu_ms: 0.0,
                invocations: 0,
            };

            match self.runtime.compile(&source) {
                Ok((script, mode)) => {
                    if let Ok(mut component) = world.get::<&mut LuaScriptComponent>(entity) {
                        component.set_script_mode(mode);
                    }
                    let module_dirs = self.runtime.module_search_dirs(&script);
                    match LuaScriptInstance::new(self.runtime.lua(), script, source, module_dirs) {
                        Ok(mut instance) => {
                            instance.carried = self.pending_carried.remove(&entity);
                            if let Some(handles) = self.pending_handles.remove(&entity) {
                                instance.handles = handles;
                            }
                            slot.instance = Some(instance);
                        }
                        Err(err) => slot.report(entity, "instantiate", err),
                    }
                }
                Err(err) => slot.report(entity, "compile", err),
            }

            if slot.instance.is_some() {
                let loaded = invoke(
                    &self.runtime,
                    &mut self.outputs,
                    world,
                    entity,
                    &mut slot,
                    Hook::Chunk,
                );
                // A script whose top-level code fails is unusable until it is reloaded.
                if loaded.is_none() {
                    slot.instance = None;
                }
            }

            self.slots.insert(entity, slot);
        }
    }

    /// Deliver queued events to their subscribers, including events emitted by handlers.
    fn dispatch_events(&mut self, world: &mut World) {
        let mut dispatched = 0usize;
        while let Some(event) = self.outputs.pending_events.pop_front() {
            if dispatched == MAX_EVENTS_PER_FRAME {
                let dropped = self.outputs.pending_events.len() + 1;
                self.outputs.pending_events.clear();
                warn!(
                    target: "script",
                    "Dropped {dropped} script event(s) after dispatching {MAX_EVENTS_PER_FRAME} \
                     this frame; check for handlers that re-emit their own event"
                );
                break;
            }
            dispatched += 1;

            let Some(subscribers) = self.outputs.event_subscriptions.get(&event.name).cloned()
            else {
                log::debug!(target: "script", "Event '{}' has no subscribers", event.name);
                continue;
            };

            for subscriber in subscribers {
                let entity = subscriber.entity_id;
                let Some(slot) = self.slots.get_mut(&entity) else {
                    continue;
                };
                let outcome = invoke(
                    &self.runtime,
                    &mut self.outputs,
                    world,
                    entity,
                    slot,
                    Hook::Event {
                        callback: &subscriber.callback_name,
                        data: &event.data,
                    },
                );
                if outcome == Some(FunctionCallOutcome::Missing) {
                    slot.report(
                        entity,
                        "event",
                        format!(
                            "handler '{}' for event '{}' is not defined",
                            subscriber.callback_name, event.name
                        ),
                    );
                }
            }
        }
    }

    /// Get pending glTF imports and clear the list.
    pub fn take_pending_gltf_imports(&mut self) -> Vec<PendingGltfImport> {
        std::mem::take(&mut self.outputs.pending_gltf_imports)
    }

    /// Particle bursts scripts requested since the last call.
    pub fn take_particle_bursts(&mut self) -> Vec<crate::gpu_particles::ParticleBurst> {
        std::mem::take(&mut self.outputs.particle_bursts)
    }

    /// Physics changes scripts asked for since the last call.
    pub fn take_physics_commands(&mut self) -> Vec<crate::physics::PhysicsCommand> {
        std::mem::take(&mut self.outputs.physics_commands)
    }

    /// Entities scripts asked a locomotion tree for (`animation_tree_generate`).
    pub fn take_pending_trees(&mut self) -> Vec<Entity> {
        std::mem::take(&mut self.outputs.pending_trees)
    }

    /// Navigation changes scripts asked for since the last call.
    #[cfg(feature = "navigation")]
    pub fn take_navigation_commands(&mut self) -> Vec<crate::navigation::NavCommand> {
        std::mem::take(&mut self.outputs.navigation_commands)
    }

    /// Call `callback(self_entity, data)` on each entity's script at the start of the
    /// next frame's scripts (after `on_created`, before `update`). Scripts that do not
    /// define the callback are skipped.
    pub fn queue_callbacks(
        &mut self,
        callbacks: impl IntoIterator<Item = (Entity, &'static str, serde_json::Value)>,
    ) {
        self.pending_callbacks.extend(callbacks);
    }

    /// Whether scripts despawned entities since the last call (so their materials may
    /// be unused now).
    pub fn take_entities_despawned(&mut self) -> bool {
        std::mem::take(&mut self.outputs.entities_despawned)
    }

    /// Post-processing changes scripts asked for since the last call.
    pub fn take_pending_post_process(&mut self) -> Vec<PendingPostProcess> {
        std::mem::take(&mut self.outputs.pending_post_process)
    }

    /// Environment changes scripts asked for since the last call.
    pub fn take_pending_environment(&mut self) -> Vec<serde_json::Value> {
        std::mem::take(&mut self.outputs.pending_environment)
    }

    /// Mesh and material changes scripts asked for since the last call.
    pub fn take_pending_asset_commands(&mut self) -> Vec<PendingAssetCommand> {
        std::mem::take(&mut self.outputs.pending_asset_commands)
    }

    /// Call `on_ui()` for all scripts and collect their UI commands.
    ///
    /// Returns a map of entity to recorded UI commands for scripts that implement
    /// `on_ui()`. Commands and events that `on_ui` produces are applied on the next
    /// [`Self::process_scripts`].
    ///
    /// `default_viewport` is used unless `per_entity_viewports` has an entry for the
    /// entity.
    pub fn process_ui(
        &mut self,
        world: &World,
        editor_mode: bool,
        default_viewport: Option<UiViewportInfo>,
        per_entity_viewports: Option<&HashMap<Entity, UiViewportInfo>>,
    ) -> HashMap<Entity, Vec<UiCommand>> {
        let mut ui_commands = HashMap::new();
        let lua = self.runtime.lua();
        lua.set_app_data(super::api::PlayMode(!editor_mode));
        let _world_guard = WorldGuard::enter(world);

        let mut entities: Vec<Entity> = world
            .query::<&LuaScriptComponent>()
            .iter()
            .filter(|(entity, component)| {
                runs_in_mode(component.script_mode(), editor_mode)
                    && self
                        .slots
                        .get(entity)
                        .is_some_and(|slot| slot.created && slot.binding == component.binding())
            })
            .map(|(entity, _)| entity)
            .collect();
        entities.sort_unstable_by_key(|entity| entity.to_bits());

        for entity in entities {
            let Some(slot) = self.slots.get_mut(&entity) else {
                continue;
            };
            let Some(instance) = slot.instance.as_mut() else {
                continue;
            };

            let viewport = per_entity_viewports
                .and_then(|map| map.get(&entity).copied())
                .or(default_viewport);
            let ui_context = match viewport {
                Some((width, height, ppp)) => UiContext::new_with_viewport_info(width, height, ppp),
                None => UiContext::new(),
            };
            if let Some(responses) = self.ui_responses.get(&entity) {
                ui_context.set_responses(responses.clone());
            }

            let userdata = match lua.create_userdata(ui_context.clone()) {
                Ok(userdata) => mlua::Value::UserData(userdata),
                Err(err) => {
                    slot.report(entity, "on_ui", err);
                    continue;
                }
            };

            let commands = instance.command_buffer();
            let events = Rc::new(RefCell::new(Vec::new()));
            let result = instance.call_on_ui(
                lua,
                entity_bits(entity),
                userdata,
                commands.clone(),
                events.clone(),
            );

            if !commands.borrow().is_empty() {
                self.deferred_ui_commands.push((entity, commands));
            }
            self.outputs.pending_events.extend(events.take());

            match result {
                Ok(FunctionCallOutcome::Executed) => {
                    slot.clear_error("on_ui");
                    let cmds = ui_context.take_commands();
                    if !cmds.is_empty() {
                        ui_commands.insert(entity, cmds);
                    }
                }
                Ok(FunctionCallOutcome::Missing) => {}
                Err(err) => slot.report(entity, "on_ui", err),
            }
        }

        ui_commands
    }

    /// Set the UI responses scripts will see during the next [`Self::process_ui`].
    ///
    /// Call this every frame after rendering script UI, even when `responses` is
    /// empty, so stale clicks are not replayed.
    pub fn set_ui_responses(&mut self, responses: HashMap<Entity, HashMap<String, UiResponse>>) {
        self.ui_responses = responses;
    }
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Text Lua's `tostring` would produce for `value`.
fn lua_display(lua: &mlua::Lua, value: &mlua::Value) -> String {
    lua.globals()
        .get::<mlua::Function>("tostring")
        .and_then(|tostring| tostring.call::<String>(value.clone()))
        .unwrap_or_else(|_| format!("{value:?}"))
}

impl Default for ScriptingState {
    fn default() -> Self {
        Self::new().expect("Failed to create scripting state")
    }
}

#[cfg(test)]
impl ScriptingState {
    /// Read a global from the script environment attached to `entity`.
    pub(crate) fn script_global(
        &self,
        _world: &World,
        entity: Entity,
        name: &str,
    ) -> Option<mlua::Value> {
        let instance = self.slots.get(&entity)?.instance.as_ref()?;
        let env: mlua::Table = self
            .runtime
            .lua()
            .registry_value(&instance.env_registry_key)
            .ok()?;
        env.get(name).ok()
    }

    pub(crate) fn has_subscriptions(&self) -> bool {
        !self.outputs.event_subscriptions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::ScriptingState;
    use crate::scripting::lua::component::LuaScriptComponent;
    use crate::scripting::lua::types::LuaScriptSource;
    use hecs::World;
    use mlua::Value as LuaValue;

    fn payload_value(state: &ScriptingState, world: &World, entity: hecs::Entity) -> i64 {
        let payload = state
            .script_global(world, entity, "last_payload")
            .expect("Missing payload");
        let LuaValue::Table(table) = payload else {
            panic!("Expected payload to be a table");
        };
        table.get("value").expect("Missing payload value")
    }

    #[test]
    fn eval_returns_values_and_captures_print() {
        let mut state = ScriptingState::new().expect("Failed to create scripting state");
        let mut world = World::new();

        let output = state.eval(&mut world, "1 + 2", None).unwrap();
        assert_eq!(output.values, vec![serde_json::json!(3)]);

        let output = state
            .eval(
                &mut world,
                "print('a', 1)\nprint('b')\nreturn 'x', true",
                None,
            )
            .unwrap();
        assert_eq!(output.printed, vec!["a\t1".to_string(), "b".to_string()]);
        assert_eq!(
            output.values,
            vec![serde_json::json!("x"), serde_json::json!(true)]
        );

        assert!(state.eval(&mut world, "error('boom')", None).is_err());
    }

    #[test]
    fn eval_console_persists_globals_and_applies_commands() {
        let mut state = ScriptingState::new().expect("Failed to create scripting state");
        let mut world = World::new();

        state
            .eval(
                &mut world,
                "counter = 41\nspawn_entity('FromConsole')",
                None,
            )
            .unwrap();
        let output = state.eval(&mut world, "counter + 1", None).unwrap();
        assert_eq!(output.values, vec![serde_json::json!(42)]);

        let spawned = world
            .query::<&crate::scene::Name>()
            .iter()
            .any(|(_, name)| name.0 == "FromConsole");
        assert!(
            spawned,
            "spawn_entity from the console should reach the world"
        );
    }

    #[test]
    fn eval_with_entity_uses_the_script_environment() {
        let mut state = ScriptingState::new().expect("Failed to create scripting state");
        let mut world = World::new();
        let entity = world.spawn((LuaScriptComponent::new(LuaScriptSource::inline(
            "scoped",
            "speed = 3\nfunction update(self_entity, dt) end",
        )),));
        state.process_scripts(&mut world, 0.016, false).unwrap();

        let output = state.eval(&mut world, "speed", Some(entity)).unwrap();
        assert_eq!(output.values, vec![serde_json::json!(3)]);

        state
            .eval(&mut world, "speed = 10", Some(entity))
            .expect("assignment");
        let speed = state.script_global(&world, entity, "speed").unwrap();
        assert_eq!(speed.as_integer(), Some(10));

        // Console globals stay separate from the script's.
        let output = state.eval(&mut world, "speed", None).unwrap();
        assert_eq!(output.values, vec![serde_json::Value::Null]);

        let missing = world.spawn(());
        assert!(state.eval(&mut world, "1", Some(missing)).is_err());
    }

    #[test]
    fn event_callback_receives_payload_only() {
        let mut state = ScriptingState::new().expect("Failed to create scripting state");
        let mut world = World::new();

        let script = r#"
            function on_created(self_entity)
                subscribe_event("test_event", "on_test_event")
            end

            function update(self_entity, dt)
                emit_event("test_event", { value = 42 })
            end

            function on_test_event(data)
                last_payload = data
            end
        "#;

        let entity = world.spawn((LuaScriptComponent::new(LuaScriptSource::inline(
            "event_payload",
            script,
        )),));

        state
            .process_scripts(&mut world, 0.016, false)
            .expect("Script processing failed");

        assert_eq!(payload_value(&state, &world, entity), 42);
    }

    #[test]
    fn event_callback_receives_payload_with_implicit_prefix() {
        let mut state = ScriptingState::new().expect("Failed to create scripting state");
        let mut world = World::new();

        let script = r#"
            function on_created(self_entity)
                subscribe_event("payload_event", "payload_handler")
            end

            function update(self_entity, dt)
                emit_event("payload_event", { value = 7 })
            end

            function on_payload_handler(data)
                last_payload = data
            end
        "#;

        let entity = world.spawn((LuaScriptComponent::new(LuaScriptSource::inline(
            "event_payload_prefix",
            script,
        )),));

        state
            .process_scripts(&mut world, 0.016, false)
            .expect("Script processing failed");

        assert_eq!(payload_value(&state, &world, entity), 7);
    }

    #[test]
    fn removed_script_clears_event_subscriptions() {
        let mut state = ScriptingState::new().expect("Failed to create scripting state");
        let mut world = World::new();

        let script = r#"
            function on_created(self_entity)
                subscribe_event("cleanup_event", "on_cleanup_event")
            end

            function on_cleanup_event(data)
            end
        "#;

        let entity = world.spawn((LuaScriptComponent::new(LuaScriptSource::inline(
            "event_cleanup",
            script,
        )),));

        state
            .process_scripts(&mut world, 0.016, false)
            .expect("Script processing failed");

        assert!(state.has_subscriptions());

        world
            .remove_one::<LuaScriptComponent>(entity)
            .expect("Failed to remove script component");

        state
            .process_scripts(&mut world, 0.016, false)
            .expect("Script processing failed");

        assert!(!state.has_subscriptions());
    }
}
