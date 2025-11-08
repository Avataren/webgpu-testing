use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use hecs::{Entity, World};
use log::error;

use crate::scene;
use crate::scripting::component_registry::ComponentRegistry;

type WorldId = usize;

use super::commands::{entity_bits, PendingGltfImport, ScriptCommands};
use super::component::{FunctionCallOutcome, LuaScriptComponent, LuaScriptInstance};
use super::error::LuaScriptingError;
use super::runtime::LuaScriptingRuntime;
use super::types::{
    EventSubscription, EventSubscriptions, ScriptEvent, ScriptMode, ScriptStateMap,
};

/// State that owns the Lua runtime for a scene.
pub struct ScriptingState {
    runtime: LuaScriptingRuntime,
    instances: HashMap<WorldId, HashMap<Entity, LuaScriptInstance>>,
    pending_gltf_imports: Vec<PendingGltfImport>,
    component_registry: ComponentRegistry,
    event_subscriptions: EventSubscriptions,
    /// Pending state restoration per world after script reload
    pending_state_restoration: HashMap<WorldId, HashMap<Entity, ScriptStateMap>>,
    /// UI responses from the previous frame (per world)
    ui_responses: HashMap<WorldId, HashMap<Entity, HashMap<String, super::api::ui::UiResponse>>>,
}

impl ScriptingState {
    /// Construct a new scripting state.
    pub fn new() -> Result<Self, LuaScriptingError> {
        Ok(Self {
            runtime: LuaScriptingRuntime::new()?,
            instances: HashMap::new(),
            pending_gltf_imports: Vec::new(),
            component_registry: ComponentRegistry::new(),
            event_subscriptions: HashMap::new(),
            pending_state_restoration: HashMap::new(),
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

    /// Clear any cached script instances and pending work so that scripts
    /// re-run their creation logic on the next update cycle.
    pub fn reset_runtime(&mut self) {
        self.instances.clear();
        self.pending_gltf_imports.clear();
        self.event_subscriptions.clear();
        self.pending_state_restoration.clear();
        self.ui_responses.clear();
    }

    /// Extract script state for a specific world before reset.
    pub fn extract_state_for_world(&self, world: &World) -> HashMap<Entity, ScriptStateMap> {
        let world_id = scene::world_id(world);
        let mut all_state = HashMap::new();

        if let Some(instances) = self.instances.get(&world_id) {
            for (entity, instance) in instances {
                let state = instance.state_store.borrow().clone();
                if !state.is_empty() {
                    all_state.insert(*entity, state);
                }
            }
        }

        log::debug!(
            "Extracted state from {} script instances for world {:?}",
            all_state.len(),
            world_id
        );
        all_state
    }

    /// Restore state to script instances after they've been created.
    pub fn restore_state(&mut self, world: &World, state: HashMap<Entity, ScriptStateMap>) {
        if state.is_empty() {
            return;
        }

        let world_id = scene::world_id(world);
        self.pending_state_restoration.insert(world_id, state);
    }

    /// Process a single frame of script execution.
    ///
    /// This compiles any new scripts, calls lifecycle hooks, and applies commands.
    pub fn process_scripts(
        &mut self,
        world: &mut World,
        dt: f64,
        editor_mode: bool,
    ) -> Result<(), LuaScriptingError> {
        let world_id = scene::world_id(world);

        // Ensure all entities with LuaScriptComponent have instances
        self.ensure_instances(world_id, world)?;

        // Call on_created for new scripts
        self.call_on_created(world_id, world, editor_mode)?;

        // Restore any pending state AFTER on_created has run
        // This ensures default values from on_created() are overwritten by restored state
        if let Some(state_map) = self.pending_state_restoration.remove(&world_id) {
            self.apply_state_restoration(world_id, state_map);
        }

        // Call update for all scripts (if dt > 0 or editor_mode)
        if dt > 0.0 || editor_mode {
            self.call_update(world_id, world, dt, editor_mode)?;
        }

        // Remove instances for entities that no longer have scripts
        self.cleanup_removed_scripts(world_id, world);

        Ok(())
    }

    /// Ensure all entities with LuaScriptComponent have compiled instances.
    fn ensure_instances(
        &mut self,
        world_id: WorldId,
        world: &World,
    ) -> Result<(), LuaScriptingError> {
        let mut to_compile = Vec::new();
        let instances = self.instances.entry(world_id).or_insert_with(HashMap::new);

        for (entity, script_comp) in world.query::<&LuaScriptComponent>().iter() {
            if !instances.contains_key(&entity) {
                to_compile.push((entity, script_comp.source().clone()));
            }
        }

        for (entity, source) in to_compile {
            match self.runtime.compile(&source) {
                Ok((script, mode)) => {
                    match LuaScriptInstance::new(self.runtime.lua(), script.clone(), source.clone())
                    {
                        Ok(instance) => {
                            instances.insert(entity, instance);

                            if let Ok(mut comp) = world.get::<&mut LuaScriptComponent>(entity) {
                                comp.set_script_mode(mode);
                            }
                        }
                        Err(e) => {
                            error!(target: "script", "Failed to create instance for entity {:?}: {}", entity, e);
                        }
                    }
                }
                Err(e) => {
                    error!(target: "script", "Failed to compile script for entity {:?}: {}", entity, e);
                }
            }
        }

        Ok(())
    }

    /// Apply restored state to script instances.
    fn apply_state_restoration(
        &mut self,
        world_id: WorldId,
        state_map: HashMap<Entity, ScriptStateMap>,
    ) {
        if let Some(instances) = self.instances.get_mut(&world_id) {
            for (entity, state) in state_map {
                if let Some(instance) = instances.get_mut(&entity) {
                    *instance.state_store.borrow_mut() = state;
                    log::debug!(
                        "Restored state for entity {:?} (world {:?})",
                        entity,
                        world_id
                    );
                }
            }
        }
    }

    /// Call on_created for scripts that haven't had it called yet.
    fn call_on_created(
        &mut self,
        world_id: WorldId,
        world: &mut World,
        editor_mode: bool,
    ) -> Result<(), LuaScriptingError> {
        let mut to_call = Vec::new();

        // Find scripts that need on_created and should run in the current mode
        for (entity, script_comp) in world.query::<&mut LuaScriptComponent>().iter() {
            if !script_comp.created_called() {
                let script_mode = script_comp.script_mode();

                // Check if this script should run in the current mode
                let should_run = match script_mode {
                    ScriptMode::RuntimeOnly => !editor_mode,
                    ScriptMode::EditorOnly => editor_mode,
                    ScriptMode::Both => true,
                };

                if should_run {
                    to_call.push(entity);
                    script_comp.mark_created();
                }
            }
        }

        // Call on_created for each
        for entity in to_call {
            if let Some(instance) = self
                .instances
                .get_mut(&world_id)
                .and_then(|instances| instances.get_mut(&entity))
            {
                let commands = instance.command_buffer();
                let event_queue = Rc::new(RefCell::new(Vec::new()));

                let entity_bits_val = entity_bits(entity);

                match instance.call_on_created(
                    self.runtime.lua(),
                    entity_bits_val,
                    commands.clone(),
                    event_queue.clone(),
                ) {
                    Ok(FunctionCallOutcome::Executed) => {
                        // Apply commands
                        self.apply_commands(world, commands)?;

                        // Process any emitted events
                        let events = event_queue.borrow().clone();
                        self.queue_events(events);
                    }
                    Ok(FunctionCallOutcome::Missing) => {
                        // Script doesn't have on_created - that's okay
                    }
                    Err(e) => {
                        error!(target: "script", "Error calling on_created for entity {:?}: {}", entity, e);
                    }
                }
            }
        }

        Ok(())
    }

    /// Call update for all active scripts.
    fn call_update(
        &mut self,
        world_id: WorldId,
        world: &mut World,
        dt: f64,
        editor_mode: bool,
    ) -> Result<(), LuaScriptingError> {
        let entities: Vec<Entity> = self
            .instances
            .get(&world_id)
            .map(|map| map.keys().copied().collect())
            .unwrap_or_default();

        for entity in entities {
            // Check if entity still exists and has the component
            let script_mode = match world.get::<&LuaScriptComponent>(entity) {
                Ok(comp) => comp.script_mode(),
                Err(_) => continue,
            };

            // Check if this script should run in the current mode
            let should_run = match script_mode {
                ScriptMode::RuntimeOnly => !editor_mode,
                ScriptMode::EditorOnly => editor_mode,
                ScriptMode::Both => true,
            };

            if !should_run {
                continue;
            }

            if let Some(instance) = self
                .instances
                .get_mut(&world_id)
                .and_then(|map| map.get_mut(&entity))
            {
                let commands = instance.command_buffer();
                let event_queue = Rc::new(RefCell::new(Vec::new()));

                let entity_bits_val = entity_bits(entity);

                match instance.call_update(
                    self.runtime.lua(),
                    entity_bits_val,
                    dt,
                    commands.clone(),
                    event_queue.clone(),
                ) {
                    Ok(FunctionCallOutcome::Executed) => {
                        // Apply commands
                        self.apply_commands(world, commands)?;

                        // Process any emitted events
                        let events = event_queue.borrow().clone();
                        self.queue_events(events);
                    }
                    Ok(FunctionCallOutcome::Missing) => {
                        // Script doesn't have update - that's okay
                    }
                    Err(e) => {
                        error!(target: "script", "Error calling update for entity {:?}: {}", entity, e);
                    }
                }
            }
        }

        Ok(())
    }

    /// Apply script commands to the world.
    fn apply_commands(
        &mut self,
        world: &mut World,
        commands: Rc<RefCell<ScriptCommands>>,
    ) -> Result<(), LuaScriptingError> {
        let result = commands
            .borrow_mut()
            .apply(world, &self.component_registry)?;

        // Store pending glTF imports
        self.pending_gltf_imports.extend(result.gltf_imports);

        // Update event subscriptions
        for sub in result.event_subscriptions {
            self.event_subscriptions
                .entry(sub.event_name.clone())
                .or_insert_with(Vec::new)
                .push(EventSubscription {
                    entity_id: sub.entity,
                    callback_name: sub.callback_name,
                });
        }

        // Handle unsubscriptions
        for unsub in result.event_unsubscriptions {
            if let Some(subs) = self.event_subscriptions.get_mut(&unsub.event_name) {
                subs.retain(|s| s.entity_id != unsub.entity);
            }
        }

        Ok(())
    }

    /// Queue events for later dispatch.
    fn queue_events(&mut self, events: Vec<ScriptEvent>) {
        // TODO: Implement event dispatching
        // For now, just log them
        for event in events {
            log::debug!(target: "script", "Event emitted: {}", event.name);
        }
    }

    /// Remove instances for entities that no longer have scripts.
    fn cleanup_removed_scripts(&mut self, world_id: WorldId, world: &World) {
        let Some(instances) = self.instances.get_mut(&world_id) else {
            return;
        };

        let mut to_remove = Vec::new();

        for entity in instances.keys() {
            if world.get::<&LuaScriptComponent>(*entity).is_err() {
                to_remove.push(*entity);
            }
        }

        for entity in to_remove {
            instances.remove(&entity);
            log::debug!(
                target: "script",
                "Removed script instance for entity {:?} (world {:?})",
                entity,
                world_id
            );
        }

        if instances.is_empty() {
            self.instances.remove(&world_id);
        }
    }

    /// Get pending glTF imports and clear the list.
    pub fn take_pending_gltf_imports(&mut self) -> Vec<PendingGltfImport> {
        std::mem::take(&mut self.pending_gltf_imports)
    }

    /// Access the component registry.
    pub fn component_registry(&self) -> &ComponentRegistry {
        &self.component_registry
    }

    /// Mutably access the component registry.
    pub fn component_registry_mut(&mut self) -> &mut ComponentRegistry {
        &mut self.component_registry
    }
    /// Call on_ui() for all scripts and collect their UI commands.
    ///
    /// Returns a map of Entity -> Vec<UiCommand> for scripts that implemented on_ui().
    pub fn process_ui(
        &mut self,
        world: &World,
        editor_mode: bool,
    ) -> HashMap<Entity, Vec<super::api::ui::UiCommand>> {
        use super::api::ui::UiContext;
        use super::guards::{RegistryGuard, WorldGuard};

        log::debug!(target: "script_ui", "Lua process_ui called with editor_mode={}", editor_mode);

        let world_id = scene::world_id(world);

        let mut ui_commands = HashMap::new();
        let event_queue = Rc::new(RefCell::new(Vec::new()));

        // Set up guards for World and ComponentRegistry access
        let _world_guard = WorldGuard::enter(world);
        let _registry_guard = RegistryGuard::enter(&self.component_registry);

        let mut query = world.query::<&LuaScriptComponent>();
        for (entity, component) in query.iter() {
            if !component.created_called() {
                log::debug!(target: "script_ui", "Skipping entity {:?} - created not called", entity);
                continue;
            }

            // Filter UI scripts based on mode:
            // - EditorOnly (@editor): UI only in editor mode
            // - RuntimeOnly (no annotation): UI only in play mode
            // - Both (@tool): UI in both modes
            let script_mode = component.script_mode();
            log::debug!(target: "script_ui", "Entity {:?} script_mode={:?}, editor_mode={}",
                entity, script_mode, editor_mode);

            if editor_mode {
                // In editor mode - skip RuntimeOnly scripts
                if script_mode == ScriptMode::RuntimeOnly {
                    log::debug!(target: "script_ui", "Skipping RuntimeOnly script in editor mode");
                    continue;
                }
            } else {
                // In play mode - skip EditorOnly scripts
                if script_mode == ScriptMode::EditorOnly {
                    log::debug!(target: "script_ui", "Skipping EditorOnly script in play mode");
                    continue;
                }
            }

            // Get the script instance
            let instance = match self
                .instances
                .get_mut(&world_id)
                .and_then(|map| map.get_mut(&entity))
            {
                Some(inst) => inst,
                None => {
                    log::debug!(target: "script_ui", "No instance found for entity {:?}", entity);
                    continue;
                }
            };

            // Create a UI context for this script
            let ui_context = UiContext::new();

            // Set responses from the previous frame
            if let Some(responses) = self
                .ui_responses
                .get(&world_id)
                .and_then(|map| map.get(&entity))
            {
                ui_context.set_responses(responses.clone());
            }

            let commands = instance.command_buffer();

            // Call the on_ui function (if it exists)
            let lua_context = mlua::Value::UserData(
                self.runtime
                    .lua()
                    .create_userdata(ui_context.clone())
                    .expect("Failed to create UI context"),
            );

            match instance.call_on_ui(
                self.runtime.lua(),
                entity_bits(entity),
                lua_context,
                commands,
                event_queue.clone(),
            ) {
                Ok(FunctionCallOutcome::Executed) => {
                    // Collect the UI commands from the context
                    let cmds = ui_context.take_commands();
                    if !cmds.is_empty() {
                        ui_commands.insert(entity, cmds);
                    }
                }
                Ok(FunctionCallOutcome::Missing) => {
                    // Script doesn't have on_ui() - that's fine
                }
                Err(e) => {
                    error!(target: "script", "Error calling on_ui for entity {:?}: {}", entity, e);
                }
            }
        }

        ui_commands
    }

    /// Set UI responses for scripts. This should be called after rendering UI
    /// so that the next frame can access the responses.
    pub fn set_ui_responses(
        &mut self,
        world_id: WorldId,
        responses: HashMap<Entity, HashMap<String, super::api::ui::UiResponse>>,
    ) {
        if responses.is_empty() {
            self.ui_responses.remove(&world_id);
        } else {
            self.ui_responses.insert(world_id, responses);
        }
    }
}

impl Default for ScriptingState {
    fn default() -> Self {
        Self::new().expect("Failed to create scripting state")
    }
}
