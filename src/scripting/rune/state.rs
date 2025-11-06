use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use hecs::{Entity, World};
use log::{error, warn};

use crate::scripting::component_registry::ComponentRegistry;

use super::commands::{entity_bits, PendingGltfImport, ScriptCommands};
use super::component::{FunctionCallOutcome, RuneScriptComponent, RuneScriptInstance};
use super::error::RuneScriptingError;
use super::guards::{CommandGuard, EntityGuard, EventQueueGuard, RegistryGuard, StateGuard, WorldGuard};
use super::runtime::RuneScriptingRuntime;
use super::types::{EventSubscription, EventSubscriptions, ScriptEvent};

/// State that owns the Rune runtime for a scene.
pub struct ScriptingState {
    runtime: RuneScriptingRuntime,
    instances: HashMap<Entity, RuneScriptInstance>,
    pending_gltf_imports: Vec<PendingGltfImport>,
    component_registry: ComponentRegistry,
    event_subscriptions: EventSubscriptions,
    /// UI responses from the previous frame, keyed by entity
    ui_responses: HashMap<Entity, HashMap<String, super::api::ui::UiResponse>>,
}

impl ScriptingState {
    /// Construct a new scripting state.
    pub fn new() -> Result<Self, RuneScriptingError> {
        Ok(Self {
            runtime: RuneScriptingRuntime::new()?,
            instances: HashMap::new(),
            pending_gltf_imports: Vec::new(),
            component_registry: ComponentRegistry::new(),
            event_subscriptions: HashMap::new(),
            ui_responses: HashMap::new(),
        })
    }

    /// Access the underlying runtime.
    pub fn runtime(&self) -> &RuneScriptingRuntime {
        &self.runtime
    }

    /// Mutably access the underlying runtime.
    pub fn runtime_mut(&mut self) -> &mut RuneScriptingRuntime {
        &mut self.runtime
    }

    /// Clear any cached script instances and pending work so that scripts
    /// re-run their creation logic on the next update cycle.
    pub fn reset_runtime(&mut self) {
        self.instances.clear();
        self.pending_gltf_imports.clear();
        self.event_subscriptions.clear();
        self.ui_responses.clear();
    }

    /// Run pending scripts for the current frame.
    ///
    /// * `world` - The entity world
    /// * `dt` - Delta time in seconds. If 0, only editor tool scripts will run.
    /// * `editor_mode` - If true, editor tool scripts will run even when dt is 0
    pub fn update_scripts(&mut self, world: &mut World, dt: f64, editor_mode: bool) -> Result<(), RuneScriptingError> {
        log::debug!(target: "script_update", "update_scripts called with dt={}, editor_mode={}", dt, editor_mode);
        self.retain_instances(world);
        self.process_on_created(world)?;

        // Run updates for:
        // - All scripts in play mode (dt > 0)
        // - Editor tool scripts in editor mode (dt == 0 && editor_mode)
        if dt != 0.0 || editor_mode {
            log::debug!(target: "script_update", "Calling process_updates");
            self.process_updates(world, dt, editor_mode)?;
        } else {
            log::debug!(target: "script_update", "Skipping process_updates (dt=0 and not editor_mode)");
        }
        self.process_on_created(world)?;
        Ok(())
    }

    fn process_on_created(&mut self, world: &mut World) -> Result<(), RuneScriptingError> {
        loop {
            let mut pending_commands: Vec<Rc<RefCell<ScriptCommands>>> = Vec::new();
            let event_queue = Rc::new(RefCell::new(Vec::new()));

            {
                // Set up guards for World and ComponentRegistry access
                let _world_guard = WorldGuard::enter(world as &World);
                let _registry_guard = RegistryGuard::enter(&self.component_registry);

                let mut query = world.query::<&mut RuneScriptComponent>();
                for (entity, component) in query.iter() {
                    if component.created_called() {
                        continue;
                    }

                    let instance = self.ensure_instance(entity, component)?;
                    let commands = instance.command_buffer();
                    instance.call_on_created(entity_bits(entity), commands.clone(), event_queue.clone())?;
                    component.mark_created();

                    if !commands.borrow().is_empty() {
                        pending_commands.push(commands.clone());
                    }
                }
            }

            if pending_commands.is_empty() {
                break;
            }

            let mut any_scripts_added = false;
            for commands in pending_commands.iter_mut() {
                let mut borrow = commands.borrow_mut();
                let result = borrow.apply(world, &self.component_registry)?;
                if !result.scripts_added.is_empty() {
                    any_scripts_added = true;
                }
                if !result.gltf_imports.is_empty() {
                    self.pending_gltf_imports
                        .extend(result.gltf_imports.into_iter());
                }
                // Apply event subscriptions
                for sub in result.event_subscriptions {
                    self.event_subscriptions
                        .entry(sub.event_name)
                        .or_default()
                        .push(EventSubscription {
                            entity_id: sub.entity,
                            callback_name: sub.callback_name,
                        });
                }
                // Apply event unsubscriptions
                for unsub in result.event_unsubscriptions {
                    if let Some(subs) = self.event_subscriptions.get_mut(&unsub.event_name) {
                        subs.retain(|s| s.entity_id != unsub.entity);
                    }
                }
            }

            // Dispatch any events emitted during on_created callbacks
            self.dispatch_events(world, event_queue.clone())?;

            if !any_scripts_added {
                break;
            }
        }

        Ok(())
    }

    fn process_updates(&mut self, world: &mut World, dt: f64, editor_mode: bool) -> Result<(), RuneScriptingError> {
        let mut pending_commands: Vec<Rc<RefCell<ScriptCommands>>> = Vec::new();
        let event_queue = Rc::new(RefCell::new(Vec::new()));

        {
            // Set up guards for World and ComponentRegistry access
            let _world_guard = WorldGuard::enter(world as &World);
            let _registry_guard = RegistryGuard::enter(&self.component_registry);

            let mut query = world.query::<&mut RuneScriptComponent>();
            for (entity, component) in query.iter() {
                if !component.created_called() {
                    continue;
                }

                // Filter scripts based on mode:
                // - EditorOnly: skip in play mode (dt > 0)
                // - RuntimeOnly: skip in editor mode (dt == 0 && editor_mode)
                // - Both: run in both modes
                use crate::scripting::rune::runtime::ScriptMode;
                let script_mode = component.script_mode();
                if dt == 0.0 && editor_mode {
                    // In editor mode - skip RuntimeOnly scripts
                    if script_mode == ScriptMode::RuntimeOnly {
                        continue;
                    }
                } else if dt > 0.0 {
                    // In play mode - skip EditorOnly scripts
                    if script_mode == ScriptMode::EditorOnly {
                        continue;
                    }
                }

                let instance = self.ensure_instance(entity, component)?;
                let commands = instance.command_buffer();
                instance.call_update(entity_bits(entity), dt, commands.clone(), event_queue.clone())?;

                if !commands.borrow().is_empty() {
                    pending_commands.push(commands.clone());
                }
            }
        }

        // Apply pending commands
        for commands in pending_commands.iter_mut() {
            let mut borrow = commands.borrow_mut();
            let result = borrow.apply(world, &self.component_registry)?;
            if !result.gltf_imports.is_empty() {
                self.pending_gltf_imports
                    .extend(result.gltf_imports.into_iter());
            }
            // Apply event subscriptions
            for sub in result.event_subscriptions {
                self.event_subscriptions
                    .entry(sub.event_name)
                    .or_default()
                    .push(EventSubscription {
                        entity_id: sub.entity,
                        callback_name: sub.callback_name,
                    });
            }
            // Apply event unsubscriptions
            for unsub in result.event_unsubscriptions {
                if let Some(subs) = self.event_subscriptions.get_mut(&unsub.event_name) {
                    subs.retain(|s| s.entity_id != unsub.entity);
                }
            }
        }

        // Dispatch events to subscribers
        self.dispatch_events(world, event_queue)?;

        Ok(())
    }

    fn dispatch_events(
        &mut self,
        world: &mut World,
        event_queue: Rc<RefCell<Vec<ScriptEvent>>>,
    ) -> Result<(), RuneScriptingError> {
        let events = event_queue.borrow_mut().drain(..).collect::<Vec<_>>();

        if events.is_empty() {
            return Ok(());
        }

        for event in events {
            // Get subscribers for this event
            let subscribers = match self.event_subscriptions.get(&event.name) {
                Some(subs) => subs.clone(),
                None => continue,
            };

            // Call each subscriber's callback
            for subscription in subscribers {
                // Check if the entity still exists
                if world.entity(subscription.entity_id).is_err() {
                    continue;
                }

                // Prepare the command buffer and call the event handler
                let commands = {
                    // Get the script component in a scope to drop the borrow
                    let mut component = match world.get::<&mut RuneScriptComponent>(subscription.entity_id) {
                        Ok(comp) => comp,
                        Err(_) => continue,
                    };

                    if !component.created_called() {
                        continue;
                    }

                    // Get the script instance
                    let instance = match self.ensure_instance(subscription.entity_id, &mut *component) {
                        Ok(inst) => inst,
                        Err(e) => {
                            error!(target: "script", "Failed to get script instance for event dispatch: {}", e);
                            continue;
                        }
                    };

                    // Set up execution context
                    let commands = instance.command_buffer();
                    let event_data = event.data.clone();

                    // Pre-register self_entity in the registry to prevent collision with allocated handles
                    {
                        use hecs::Entity;
                        use std::num::NonZeroU64;
                        let entity_bits_val = entity_bits(subscription.entity_id);
                        if let Some(nz) = NonZeroU64::new(entity_bits_val as u64) {
                            if let Some(entity) = Entity::from_bits(nz.into()) {
                                commands.borrow_mut().registry.borrow_mut().resolve(entity_bits_val, entity);
                            }
                        }
                    }

                    {
                        let _commands_guard = CommandGuard::enter(commands.clone());
                        let state = Rc::clone(&instance.state_store);
                        let _state_guard = StateGuard::enter(&state);
                        let _event_queue_guard = EventQueueGuard::enter(&event_queue);
                        let _entity_guard = EntityGuard::enter(entity_bits(subscription.entity_id));

                        // Call the event handler function
                        let result = instance.call_function([subscription.callback_name.as_str()], (event_data,));

                        match result {
                            Ok(FunctionCallOutcome::Executed) => {},
                            Ok(FunctionCallOutcome::Missing) => {
                                warn!(target: "script",
                                    "Event handler '{}' not found on entity {:?}",
                                    subscription.callback_name, subscription.entity_id);
                            }
                            Err(e) => {
                                error!(target: "script",
                                    "Error calling event handler '{}': {}",
                                    subscription.callback_name, e);
                            }
                        }
                    }

                    commands
                };  // component borrow is dropped here

                // Apply any commands generated by the event handler
                if !commands.borrow().is_empty() {
                    let mut borrow = commands.borrow_mut();
                    match borrow.apply(world, &self.component_registry) {
                        Ok(result) => {
                            // Process GLTF imports
                            if !result.gltf_imports.is_empty() {
                                self.pending_gltf_imports
                                    .extend(result.gltf_imports.into_iter());
                            }

                            // Process event subscriptions
                            for sub in result.event_subscriptions {
                                self.event_subscriptions
                                    .entry(sub.event_name)
                                    .or_default()
                                    .push(EventSubscription {
                                        entity_id: sub.entity,
                                        callback_name: sub.callback_name,
                                    });
                            }

                            // Process event unsubscriptions
                            for unsub in result.event_unsubscriptions {
                                if let Some(subs) = self.event_subscriptions.get_mut(&unsub.event_name) {
                                    subs.retain(|s| s.entity_id != unsub.entity);
                                }
                            }

                            // Note: scripts_added are not processed here to avoid recursion during event dispatch
                            // Any scripts spawned by event handlers will be initialized on the next frame
                            if !result.scripts_added.is_empty() {
                                warn!(target: "script",
                                    "Event handler spawned {} script(s) - they will be initialized on the next frame",
                                    result.scripts_added.len());
                            }
                        }
                        Err(e) => {
                            error!(target: "script", "Failed to apply commands from event handler: {}", e);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    pub fn take_pending_gltf_imports(&mut self) -> Vec<PendingGltfImport> {
        std::mem::take(&mut self.pending_gltf_imports)
    }

    /// Call on_ui() for all scripts and collect their UI commands.
    ///
    /// Returns a map of Entity -> Vec<UiCommand> for scripts that implemented on_ui().
    pub fn process_ui(&mut self, world: &World, editor_mode: bool) -> HashMap<Entity, Vec<super::api::ui::UiCommand>> {
        use super::api::ui::UiContext;
        use super::commands::entity_bits;
        use super::runtime::ScriptMode;

        log::debug!(target: "script_ui", "process_ui called with editor_mode={}", editor_mode);

        let mut ui_commands = HashMap::new();
        let event_queue = Rc::new(RefCell::new(Vec::new()));

        // Set up guards for World and ComponentRegistry access
        let _world_guard = WorldGuard::enter(world);
        let _registry_guard = RegistryGuard::enter(&self.component_registry);

        let mut query = world.query::<&mut RuneScriptComponent>();
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
            let instance = match self.instances.get_mut(&entity) {
                Some(inst) => inst,
                None => {
                    log::debug!(target: "script_ui", "No instance found for entity {:?}", entity);
                    continue;
                }
            };

            // Create a UI context for this script
            let ui_context = UiContext::new();

            // Set responses from the previous frame
            if let Some(responses) = self.ui_responses.get(&entity) {
                ui_context.set_responses(responses.clone());
            }

            let commands = instance.command_buffer();

            // Call the on_ui function (if it exists)
            match instance.call_on_ui(entity_bits(entity), ui_context.clone(), commands, event_queue.clone()) {
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
    pub fn set_ui_responses(&mut self, responses: HashMap<Entity, HashMap<String, super::api::ui::UiResponse>>) {
        self.ui_responses = responses;
    }

    fn retain_instances(&mut self, world: &World) {
        self.instances.retain(|entity, _| {
            world
                .entity(*entity)
                .ok()
                .map(|entry| entry.has::<RuneScriptComponent>())
                .unwrap_or(false)
        });
    }

    fn ensure_instance(
        &mut self,
        entity: Entity,
        component: &mut RuneScriptComponent,
    ) -> Result<&mut RuneScriptInstance, RuneScriptingError> {
        let needs_rebuild = !matches!(
            self.instances.get(&entity),
            Some(existing) if existing.source == *component.source()
        );

        if needs_rebuild {
            let (script, script_mode) = self.runtime.compile(component.source())?;
            component.set_script_mode(script_mode);
            let source = component.source().clone();
            let instance = self.runtime.instantiate(script, source);
            self.instances.insert(entity, instance);
        }

        Ok(self
            .instances
            .get_mut(&entity)
            .expect("script instance exists"))
    }
}

impl Default for ScriptingState {
    fn default() -> Self {
        Self::new().expect("failed to initialize Rune scripting state")
    }
}
