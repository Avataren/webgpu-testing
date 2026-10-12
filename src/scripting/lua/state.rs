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

use super::api::ui::{UiCommand, UiContext, UiResponse};
use super::commands::{entity_bits, PendingGltfImport, ScriptCommands};
use super::component::{FunctionCallOutcome, LuaScriptComponent, LuaScriptInstance};
use super::error::LuaScriptingError;
use super::guards::WorldGuard;
use super::runtime::LuaScriptingRuntime;
use super::types::{
    EventSubscription, EventSubscriptions, LuaScriptSource, ScriptEvent, ScriptMode, ScriptStateMap,
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
    Destroyed,
    Event {
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
            Hook::Destroyed => "on_destroyed",
            Hook::Event { callback, .. } => callback,
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
    event_subscriptions: EventSubscriptions,
    pending_events: VecDeque<ScriptEvent>,
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
            Hook::Destroyed => instance.call_on_destroyed(lua, bits, cmds, evts),
            Hook::Event { callback, data } => {
                instance.call_event(lua, bits, callback, (*data).clone(), cmds, evts)
            }
            Hook::Coroutines(dt) => instance
                .process_coroutines(*dt, cmds, evts)
                .map(|()| FunctionCallOutcome::Executed),
        }
    };

    // Apply whatever the script recorded, even if it failed part way through.
    outputs.apply_commands(world, &commands);
    outputs.pending_events.extend(events.take());

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

/// Owns the Lua runtime and the script instances of one ECS world.
pub struct ScriptingState {
    runtime: LuaScriptingRuntime,
    slots: HashMap<Entity, ScriptSlot>,
    outputs: ScriptOutputs,
    /// State to restore into scripts once they are re-created after a reload.
    pending_state_restoration: HashMap<Entity, ScriptStateMap>,
    /// Commands recorded by `on_ui`, applied at the start of the next update.
    deferred_ui_commands: Vec<(Entity, Rc<RefCell<ScriptCommands>>)>,
    /// UI responses from the previous frame.
    ui_responses: HashMap<Entity, HashMap<String, UiResponse>>,
}

impl ScriptingState {
    /// Construct a new scripting state.
    pub fn new() -> Result<Self, LuaScriptingError> {
        Ok(Self {
            runtime: LuaScriptingRuntime::new()?,
            slots: HashMap::new(),
            outputs: ScriptOutputs::default(),
            pending_state_restoration: HashMap::new(),
            deferred_ui_commands: Vec::new(),
            ui_responses: HashMap::new(),
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

    /// Recompile every script and run `on_created` again on the next update.
    ///
    /// Values scripts stored with the state API (`set_f64`, `set_state`, ...) survive
    /// the reset and are restored after `on_created` runs.
    pub fn reset_runtime(&mut self) {
        let entities: Vec<Entity> = self.slots.keys().copied().collect();
        for entity in entities {
            self.unload(entity);
        }
        self.outputs = ScriptOutputs::default();
        self.deferred_ui_commands.clear();
        self.ui_responses.clear();
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
            .filter(|(_, slot)| match &slot.source {
                LuaScriptSource::File { path } => {
                    let resolved = match &script_root {
                        Some(root) if path.is_relative() => root.join(path),
                        _ => path.clone(),
                    };
                    canonical(&resolved) == target
                }
                LuaScriptSource::Inline { .. } => false,
            })
            .map(|(entity, _)| *entity)
            .collect();

        for entity in &entities {
            self.unload(*entity);
            self.outputs.remove_subscriptions(*entity);
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
        self.unload(entity);
        self.outputs.remove_subscriptions(entity);
        true
    }

    /// Drop the instance for `entity` without running `on_destroyed`, keeping its state
    /// so it can be restored when the script is re-created.
    fn unload(&mut self, entity: Entity) {
        if let Some(slot) = self.slots.remove(&entity) {
            if let Some(instance) = slot.instance {
                let state = instance.state_store.borrow().clone();
                if !state.is_empty() {
                    self.pending_state_restoration.insert(entity, state);
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
                    Hook::Destroyed,
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
            };

            match self.runtime.compile(&source) {
                Ok((script, mode)) => {
                    if let Ok(mut component) = world.get::<&mut LuaScriptComponent>(entity) {
                        component.set_script_mode(mode);
                    }
                    match LuaScriptInstance::new(self.runtime.lua(), script, source) {
                        Ok(instance) => slot.instance = Some(instance),
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
