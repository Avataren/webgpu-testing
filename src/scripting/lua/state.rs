use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use hecs::{Entity, World};
use log::error;
use mlua::LuaSerdeExt;

use crate::scene;

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

        // Process coroutines for all script instances
        self.process_coroutines(world_id, world, dt)?;

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
        let instances = self.instances.entry(world_id).or_default();

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

    /// Process coroutines for all active script instances.
    fn process_coroutines(
        &mut self,
        world_id: WorldId,
        world: &mut World,
        dt: f64,
    ) -> Result<(), LuaScriptingError> {
        let entities: Vec<Entity> = self
            .instances
            .get(&world_id)
            .map(|map| map.keys().copied().collect())
            .unwrap_or_default();

        for entity in entities {
            if let Some(instance) = self
                .instances
                .get_mut(&world_id)
                .and_then(|map| map.get_mut(&entity))
            {
                let commands = instance.command_buffer();
                let event_queue = Rc::new(RefCell::new(Vec::new()));

                // Process coroutines for this instance
                if let Err(e) = instance.process_coroutines(
                    self.runtime.lua(),
                    dt,
                    commands.clone(),
                    event_queue.clone(),
                ) {
                    error!(target: "script", "Error processing coroutines for entity {:?}: {}", entity, e);
                    continue;
                }

                // Apply any commands generated by coroutines
                if let Err(e) = self.apply_commands(world, commands) {
                    error!(target: "script", "Error applying coroutine commands for entity {:?}: {}", entity, e);
                }

                // Process any events emitted by coroutines
                let events = event_queue.borrow().clone();
                self.queue_events(events);
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
        let result = commands.borrow_mut().apply(world)?;

        // Store pending glTF imports
        self.pending_gltf_imports.extend(result.gltf_imports);

        // Update event subscriptions
        for sub in result.event_subscriptions {
            self.event_subscriptions
                .entry(sub.event_name.clone())
                .or_default()
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
        for event in events {
            self.dispatch_event(event);
        }
    }

    /// Dispatch a single event to all subscribers.
    fn dispatch_event(&mut self, event: ScriptEvent) {
        let subscribers = match self.event_subscriptions.get(&event.name) {
            Some(subs) if !subs.is_empty() => subs.clone(),
            _ => {
                log::debug!(target: "script", "Event '{}' emitted but has no subscribers", event.name);
                return;
            }
        };

        log::debug!(
            target: "script",
            "Dispatching event '{}' to {} subscriber(s)",
            event.name,
            subscribers.len()
        );

        // Dispatch to each subscriber
        for subscriber in subscribers {
            if let Err(e) = self.call_event_callback(
                subscriber.entity_id,
                &subscriber.callback_name,
                &event.name,
                &event.data,
            ) {
                log::error!(
                    target: "script",
                    "Error calling event callback '{}' on entity {:?}: {}",
                    subscriber.callback_name,
                    subscriber.entity_id,
                    e
                );
            }
        }
    }

    /// Call an event callback on a specific entity's script instance.
    fn call_event_callback(
        &mut self,
        entity: Entity,
        callback_name: &str,
        event_name: &str,
        event_data: &serde_json::Value,
    ) -> Result<(), String> {
        // Find the instance for this entity across all worlds
        let mut instance = None;
        let mut world_id = 0;

        for (wid, instances) in &mut self.instances {
            if let Some(inst) = instances.get_mut(&entity) {
                instance = Some(inst);
                world_id = *wid;
                break;
            }
        }

        let instance = instance.ok_or_else(|| {
            format!(
                "No script instance found for entity {:?} when dispatching event '{}'",
                entity, event_name
            )
        })?;

        // Convert event data to Lua value using mlua's serde integration
        let lua = self.runtime.lua();
        let event_data_lua: mlua::Value = lua
            .to_value(event_data)
            .map_err(|e| format!("Failed to convert event data to Lua: {}", e))?;

        // Get the environment table from the registry
        let env: mlua::Table = lua
            .registry_value(&instance.env_registry_key)
            .map_err(|e| format!("Failed to get environment table: {}", e))?;

        // Check if callback exists in this instance's environment
        if !env.contains_key(callback_name).unwrap_or(false) {
            return Err(format!(
                "Callback function '{}' not found in script for entity {:?}",
                callback_name, entity
            ));
        }

        // Get the callback function from the environment
        let callback: mlua::Function = env
            .get(callback_name)
            .map_err(|e| format!("Failed to get callback '{}': {}", callback_name, e))?;

        // Call the callback with event data
        callback
            .call::<()>(event_data_lua)
            .map_err(|e| format!("Callback execution failed: {}", e))?;

        log::debug!(
            target: "script",
            "Successfully called callback '{}' on entity {:?} (world {:?}) for event '{}'",
            callback_name,
            entity,
            world_id,
            event_name
        );

        Ok(())
    }

    /// Remove instances for entities that no longer have scripts.
    fn cleanup_removed_scripts(&mut self, world_id: WorldId, world: &mut World) {
        let (removed_instances, should_remove_world) = {
            let Some(instances) = self.instances.get_mut(&world_id) else {
                return;
            };

            let mut to_remove = Vec::new();

            for entity in instances.keys() {
                if world.get::<&LuaScriptComponent>(*entity).is_err() {
                    to_remove.push(*entity);
                }
            }

            let mut removed_instances = Vec::with_capacity(to_remove.len());

            for entity in to_remove {
                if let Some(instance) = instances.remove(&entity) {
                    removed_instances.push((entity, instance));
                }
            }

            (removed_instances, instances.is_empty())
        };

        for (entity, mut instance) in removed_instances {
            let commands = instance.command_buffer();
            let event_queue = Rc::new(RefCell::new(Vec::new()));
            let entity_bits_val = entity_bits(entity);

            match instance.call_on_destroyed(
                self.runtime.lua(),
                entity_bits_val,
                commands.clone(),
                event_queue.clone(),
            ) {
                Ok(FunctionCallOutcome::Executed) => {
                    if let Err(e) = self.apply_commands(world, commands) {
                        error!(target: "script", "Error applying on_destroyed commands for entity {:?}: {}", entity, e);
                    }

                    let events = event_queue.borrow().clone();
                    self.queue_events(events);
                }
                Ok(FunctionCallOutcome::Missing) => {}
                Err(e) => {
                    error!(target: "script", "Error calling on_destroyed for entity {:?}: {}", entity, e);
                }
            }

            self.remove_event_subscriptions(entity);

            log::debug!(
                target: "script",
                "Removed script instance for entity {:?} (world {:?})",
                entity,
                world_id
            );
        }

        if should_remove_world {
            self.instances.remove(&world_id);
        }
    }

    fn remove_event_subscriptions(&mut self, entity: Entity) {
        for subscriptions in self.event_subscriptions.values_mut() {
            subscriptions.retain(|subscription| subscription.entity_id != entity);
        }

        self.event_subscriptions
            .retain(|_, subscriptions| !subscriptions.is_empty());
    }

    /// Get pending glTF imports and clear the list.
    pub fn take_pending_gltf_imports(&mut self) -> Vec<PendingGltfImport> {
        std::mem::take(&mut self.pending_gltf_imports)
    }

    /// Call on_ui() for all scripts and collect their UI commands.
    ///
    /// Returns a map of Entity -> Vec<UiCommand> for scripts that implemented on_ui().
    ///
    /// # Parameters
    /// - `viewport_width`: Optional viewport width in logical points
    /// - `viewport_height`: Optional viewport height in logical points
    /// - `pixels_per_point`: Optional DPI scaling factor
    pub fn process_ui(
        &mut self,
        world: &World,
        editor_mode: bool,
        viewport_width: Option<f32>,
        viewport_height: Option<f32>,
        pixels_per_point: Option<f32>,
        per_entity_viewports: Option<&HashMap<Entity, (f32, f32, f32)>>,
    ) -> HashMap<Entity, Vec<super::api::ui::UiCommand>> {
        use super::api::ui::UiContext;
        use super::guards::WorldGuard;

        log::debug!(target: "script_ui", "Lua process_ui called with editor_mode={}", editor_mode);

        let world_id = scene::world_id(world);

        let mut ui_commands = HashMap::new();
        let event_queue = Rc::new(RefCell::new(Vec::new()));

        // Set up guard for World access
        let _world_guard = WorldGuard::enter(world);

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
            let ui_context = if let Some(map) = per_entity_viewports {
                if let Some((width, height, ppp)) = map.get(&entity) {
                    UiContext::new_with_viewport_info(*width, *height, *ppp)
                } else if let (Some(width), Some(height), Some(ppp)) =
                    (viewport_width, viewport_height, pixels_per_point)
                {
                    UiContext::new_with_viewport_info(width, height, ppp)
                } else {
                    UiContext::new()
                }
            } else if let (Some(width), Some(height), Some(ppp)) =
                (viewport_width, viewport_height, pixels_per_point)
            {
                UiContext::new_with_viewport_info(width, height, ppp)
            } else {
                UiContext::new()
            };

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

#[cfg(test)]
mod tests {
    use super::ScriptingState;
    use crate::scene;
    use crate::scripting::lua::component::LuaScriptComponent;
    use crate::scripting::lua::types::LuaScriptSource;
    use hecs::World;
    use mlua::Value as LuaValue;

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

        let world_id = scene::world_id(&world);
        let instance = state
            .instances
            .get(&world_id)
            .and_then(|instances| instances.get(&entity))
            .expect("Missing script instance");

        let lua = state.runtime().lua();
        let env: mlua::Table = lua
            .registry_value(&instance.env_registry_key)
            .expect("Missing environment table");

        let payload: LuaValue = env.get("last_payload").expect("Missing payload");
        let payload_table = payload.as_table().expect("Expected payload to be a table");
        let value: i64 = payload_table.get("value").expect("Missing payload value");

        assert_eq!(value, 42);
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

        assert!(state
            .event_subscriptions
            .values()
            .any(|subs| subs.iter().any(|sub| sub.entity_id == entity)));

        world
            .remove_one::<LuaScriptComponent>(entity)
            .expect("Failed to remove script component");

        state
            .process_scripts(&mut world, 0.016, false)
            .expect("Script processing failed");

        assert!(state.event_subscriptions.is_empty());
    }
}
