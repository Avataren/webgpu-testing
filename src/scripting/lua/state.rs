use std::collections::HashMap;
use std::rc::Rc;
use std::cell::RefCell;

use hecs::{Entity, World};
use log::{error, warn};

use crate::scripting::component_registry::ComponentRegistry;

use super::commands::{entity_bits, PendingGltfImport, ScriptCommands};
use super::component::{FunctionCallOutcome, LuaScriptComponent, LuaScriptInstance};
use super::error::LuaScriptingError;
use super::runtime::LuaScriptingRuntime;
use super::types::{EventSubscription, EventSubscriptions, ScriptEvent, ScriptStateMap};

/// State that owns the Lua runtime for a scene.
pub struct ScriptingState {
    runtime: LuaScriptingRuntime,
    instances: HashMap<Entity, LuaScriptInstance>,
    pending_gltf_imports: Vec<PendingGltfImport>,
    component_registry: ComponentRegistry,
    event_subscriptions: EventSubscriptions,
    /// Pending state restoration after script reload
    pending_state_restoration: Option<HashMap<Entity, ScriptStateMap>>,
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
            pending_state_restoration: None,
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
    }

    /// Extract all script state before reset.
    /// Returns a map of entity -> state map for later restoration.
    pub fn extract_all_state(&self) -> HashMap<Entity, ScriptStateMap> {
        let mut all_state = HashMap::new();

        for (entity, instance) in &self.instances {
            // Clone the state map
            let state = instance.state_store.borrow().clone();
            if !state.is_empty() {
                all_state.insert(*entity, state);
            }
        }

        log::debug!("Extracted state from {} script instances", all_state.len());
        all_state
    }

    /// Restore state to script instances after they've been created.
    pub fn restore_state(&mut self, state: HashMap<Entity, ScriptStateMap>) {
        self.pending_state_restoration = Some(state);
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
        // Ensure all entities with LuaScriptComponent have instances
        self.ensure_instances(world)?;

        // Restore any pending state
        if let Some(state_map) = self.pending_state_restoration.take() {
            self.apply_state_restoration(state_map);
        }

        // Call on_created for new scripts
        self.call_on_created(world)?;

        // Call update for all scripts (if dt > 0 or editor_mode)
        if dt > 0.0 || editor_mode {
            self.call_update(world, dt)?;
        }

        // Remove instances for entities that no longer have scripts
        self.cleanup_removed_scripts(world);

        Ok(())
    }

    /// Ensure all entities with LuaScriptComponent have compiled instances.
    fn ensure_instances(&mut self, world: &World) -> Result<(), LuaScriptingError> {
        let mut to_compile = Vec::new();

        // Find entities that need compilation
        for (entity, script_comp) in world.query::<&LuaScriptComponent>().iter() {
            if !self.instances.contains_key(&entity) {
                to_compile.push((entity, script_comp.source().clone()));
            }
        }

        // Compile and create instances
        for (entity, source) in to_compile {
            match self.runtime.compile(&source) {
                Ok((script, mode)) => {
                    let instance = LuaScriptInstance::new(script.clone(), source.clone());

                    // Load the script into the Lua VM
                    if let Err(e) = self.runtime.load_script(&script) {
                        error!(target: "script", "Failed to load script for entity {:?}: {}", entity, e);
                        continue;
                    }

                    self.instances.insert(entity, instance);

                    // Update the component's script mode
                    if let Ok(mut comp) = world.get::<&mut LuaScriptComponent>(entity) {
                        comp.set_script_mode(mode);
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
    fn apply_state_restoration(&mut self, state_map: HashMap<Entity, ScriptStateMap>) {
        for (entity, state) in state_map {
            if let Some(instance) = self.instances.get_mut(&entity) {
                *instance.state_store.borrow_mut() = state;
                log::debug!("Restored state for entity {:?}", entity);
            }
        }
    }

    /// Call on_created for scripts that haven't had it called yet.
    fn call_on_created(&mut self, world: &mut World) -> Result<(), LuaScriptingError> {
        let mut to_call = Vec::new();

        // Find scripts that need on_created
        for (entity, script_comp) in world.query::<&mut LuaScriptComponent>().iter() {
            if !script_comp.created_called() {
                to_call.push(entity);
                script_comp.mark_created();
            }
        }

        // Call on_created for each
        for entity in to_call {
            if let Some(instance) = self.instances.get_mut(&entity) {
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
    fn call_update(&mut self, world: &mut World, dt: f64) -> Result<(), LuaScriptingError> {
        let entities: Vec<Entity> = self.instances.keys().copied().collect();

        for entity in entities {
            // Check if entity still exists and has the component
            let _script_mode = match world.get::<&LuaScriptComponent>(entity) {
                Ok(comp) => comp.script_mode(),
                Err(_) => continue,
            };

            if let Some(instance) = self.instances.get_mut(&entity) {
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
        let result = commands.borrow_mut().apply(world, &self.component_registry)?;

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
    fn cleanup_removed_scripts(&mut self, world: &World) {
        let mut to_remove = Vec::new();

        for entity in self.instances.keys() {
            if world.get::<&LuaScriptComponent>(*entity).is_err() {
                to_remove.push(*entity);
            }
        }

        for entity in to_remove {
            self.instances.remove(&entity);
            log::debug!(target: "script", "Removed script instance for entity {:?}", entity);
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
}

impl Default for ScriptingState {
    fn default() -> Self {
        Self::new().expect("Failed to create scripting state")
    }
}
