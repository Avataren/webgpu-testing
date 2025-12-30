use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

use mlua::Lua;

use super::api::{set_current_coroutine_id, CoroutineGuard};
use super::commands::ScriptCommands;
use super::entity_registry::EntityHandleRegistry;
use super::error::LuaScriptingError;
use super::guards::{CommandGuard, EntityGuard, EventQueueGuard, StateGuard};
use super::types::{
    CoroutineMap, LuaScript, LuaScriptSource, ScriptEvent, ScriptMode, ScriptStateMap,
};

/// Component that attaches a Lua script to an entity.
#[derive(Clone)]
pub struct LuaScriptComponent {
    source: LuaScriptSource,
    created_called: bool,
    /// Script execution mode (@editor, @tool, or no annotation)
    script_mode: ScriptMode,
}

impl fmt::Debug for LuaScriptComponent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LuaScriptComponent")
            .field("created_called", &self.created_called)
            .field("script_mode", &self.script_mode)
            .finish()
    }
}

impl LuaScriptComponent {
    /// Create a component from an arbitrary source descriptor.
    pub fn new(source: LuaScriptSource) -> Self {
        Self {
            source,
            created_called: false,
            script_mode: ScriptMode::RuntimeOnly,
        }
    }

    /// Create a component with explicit script mode.
    pub fn with_script_mode(source: LuaScriptSource, script_mode: ScriptMode) -> Self {
        Self {
            source,
            created_called: false,
            script_mode,
        }
    }

    /// Convenience for building a component from inline source code.
    pub fn new_inline(name: impl Into<String>, source: impl Into<String>) -> Self {
        Self::new(LuaScriptSource::inline(name, source))
    }

    pub fn source(&self) -> &LuaScriptSource {
        &self.source
    }

    pub fn mark_created(&mut self) {
        self.created_called = true;
    }

    pub fn created_called(&self) -> bool {
        self.created_called
    }

    pub fn set_created_called(&mut self, called: bool) {
        self.created_called = called;
    }

    /// Returns the script execution mode.
    pub fn script_mode(&self) -> ScriptMode {
        self.script_mode
    }

    /// Set the script execution mode.
    pub fn set_script_mode(&mut self, script_mode: ScriptMode) {
        self.script_mode = script_mode;
    }
}

/// A per-entity script instance that holds the Lua execution state.
///
/// Each instance has its own environment table that inherits from _G,
/// preventing function name collisions between different scripts.
pub(crate) struct LuaScriptInstance {
    pub(crate) script: Arc<LuaScript>,
    pub(crate) source: LuaScriptSource,
    pub(crate) handles: Rc<RefCell<EntityHandleRegistry>>,
    pub(crate) state_store: Rc<RefCell<ScriptStateMap>>,
    /// Active coroutines for this script instance
    pub(crate) coroutines: Rc<RefCell<CoroutineMap>>,
    /// Registry key for this instance's environment table
    pub(crate) env_registry_key: mlua::RegistryKey,
}

impl fmt::Debug for LuaScriptInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LuaScriptInstance")
            .field("script", &self.script.name)
            .field("source", &self.source)
            .finish()
    }
}

impl LuaScriptInstance {
    pub(crate) fn new(
        lua: &Lua,
        script: Arc<LuaScript>,
        source: LuaScriptSource,
    ) -> Result<Self, LuaScriptingError> {
        // Create a new environment table that inherits from _G
        let env = lua.create_table()?;
        let globals = lua.globals();
        let metatable = lua.create_table()?;
        metatable.set("__index", globals)?;
        env.set_metatable(Some(metatable));

        // Load the script's bytecode into this environment
        lua.load(&**script.chunk)
            .set_environment(env.clone())
            .exec()?;

        // Store the environment in the registry to keep it alive
        let env_registry_key = lua.create_registry_value(env)?;

        Ok(Self {
            script,
            source,
            handles: Rc::new(RefCell::new(EntityHandleRegistry::default())),
            state_store: Rc::new(RefCell::new(HashMap::new())),
            coroutines: Rc::new(RefCell::new(HashMap::new())),
            env_registry_key,
        })
    }

    pub(crate) fn command_buffer(&self) -> Rc<RefCell<ScriptCommands>> {
        Rc::new(RefCell::new(ScriptCommands::new(self.handles.clone())))
    }

    pub(crate) fn call_on_created(
        &mut self,
        lua: &Lua,
        entity_bits: i64,
        commands: Rc<RefCell<ScriptCommands>>,
        event_queue: Rc<RefCell<Vec<ScriptEvent>>>,
    ) -> Result<FunctionCallOutcome, LuaScriptingError> {
        self.with_script_context(entity_bits, commands, event_queue, || {
            self.call_function(lua, "on_created", entity_bits)
        })
    }

    pub(crate) fn call_update(
        &mut self,
        lua: &Lua,
        entity_bits: i64,
        dt: f64,
        commands: Rc<RefCell<ScriptCommands>>,
        event_queue: Rc<RefCell<Vec<ScriptEvent>>>,
    ) -> Result<FunctionCallOutcome, LuaScriptingError> {
        self.with_script_context(entity_bits, commands, event_queue, || {
            self.call_function_with_args(lua, "update", (entity_bits, dt))
        })
    }

    pub(crate) fn call_on_destroyed(
        &mut self,
        lua: &Lua,
        entity_bits: i64,
        commands: Rc<RefCell<ScriptCommands>>,
        event_queue: Rc<RefCell<Vec<ScriptEvent>>>,
    ) -> Result<FunctionCallOutcome, LuaScriptingError> {
        self.with_script_context(entity_bits, commands, event_queue, || {
            self.call_function(lua, "on_destroyed", entity_bits)
        })
    }

    pub(crate) fn call_on_ui(
        &mut self,
        lua: &Lua,
        entity_bits: i64,
        ui_context: mlua::Value,
        commands: Rc<RefCell<ScriptCommands>>,
        event_queue: Rc<RefCell<Vec<ScriptEvent>>>,
    ) -> Result<FunctionCallOutcome, LuaScriptingError> {
        self.with_script_context(entity_bits, commands, event_queue, || {
            self.call_function_with_args(lua, "on_ui", (entity_bits, ui_context))
        })
    }

    // TODO: Implement in Phase 2 after we have proper event system with Lua value conversion
    // pub fn call_event(
    //     &mut self,
    //     lua: &Lua,
    //     entity_bits: i64,
    //     event_name: &str,
    //     data: serde_json::Value,
    //     commands: Rc<RefCell<ScriptCommands>>,
    //     event_queue: Rc<RefCell<Vec<ScriptEvent>>>,
    // ) -> Result<FunctionCallOutcome, LuaScriptingError> {
    //     // Pre-register self_entity in the registry to prevent collision with allocated handles
    //     if entity_bits != 0 {
    //         commands.borrow_mut().registry.borrow_mut().resolve_bits(entity_bits, entity_bits as u64);
    //     }
    //
    //     let _commands_guard = CommandGuard::enter(commands.clone());
    //     let state = Rc::clone(&self.state_store);
    //     let _state_guard = StateGuard::enter(&state);
    //     let _event_queue_guard = EventQueueGuard::enter(&event_queue);
    //     let _entity_guard = EntityGuard::enter(entity_bits);
    //
    //     // Event handlers are named "on_<event_name>"
    //     let handler_name = format!("on_{}", event_name);
    //     self.call_function_with_args(lua, &handler_name, data)
    // }

    fn call_function(
        &self,
        lua: &Lua,
        name: &str,
        arg: i64,
    ) -> Result<FunctionCallOutcome, LuaScriptingError> {
        let Some(func) = self.resolve_function(lua, name)? else {
            return Ok(FunctionCallOutcome::Missing);
        };

        match func.call::<()>(arg) {
            Ok(_) => Ok(FunctionCallOutcome::Executed),
            Err(e) => Err(LuaScriptingError::Lua(e)),
        }
    }

    fn call_function_with_args<A>(
        &self,
        lua: &Lua,
        name: &str,
        args: A,
    ) -> Result<FunctionCallOutcome, LuaScriptingError>
    where
        A: mlua::IntoLuaMulti,
    {
        let Some(func) = self.resolve_function(lua, name)? else {
            return Ok(FunctionCallOutcome::Missing);
        };

        // Call the function
        match func.call::<()>(args) {
            Ok(_) => Ok(FunctionCallOutcome::Executed),
            Err(e) => Err(LuaScriptingError::Lua(e)),
        }
    }

    fn resolve_function(
        &self,
        lua: &Lua,
        name: &str,
    ) -> Result<Option<mlua::Function>, LuaScriptingError> {
        let env: mlua::Table = lua.registry_value(&self.env_registry_key)?;

        if !env.contains_key(name)? {
            return Ok(None);
        }

        match env.get(name) {
            Ok(func) => Ok(Some(func)),
            Err(_) => Ok(None),
        }
    }

    fn with_script_context<F>(
        &self,
        entity_bits: i64,
        commands: Rc<RefCell<ScriptCommands>>,
        event_queue: Rc<RefCell<Vec<ScriptEvent>>>,
        call: F,
    ) -> Result<FunctionCallOutcome, LuaScriptingError>
    where
        F: FnOnce() -> Result<FunctionCallOutcome, LuaScriptingError>,
    {
        // Pre-register self_entity in the registry to prevent collision with allocated handles
        if entity_bits != 0 {
            commands
                .borrow_mut()
                .registry
                .borrow_mut()
                .resolve_bits(entity_bits, entity_bits as u64);
        }

        let _commands_guard = CommandGuard::enter(commands.clone());
        let state = Rc::clone(&self.state_store);
        let _state_guard = StateGuard::enter(&state);
        let _event_queue_guard = EventQueueGuard::enter(&event_queue);
        let _entity_guard = EntityGuard::enter(entity_bits);
        let _coroutine_guard = CoroutineGuard::enter(&self.coroutines);

        call()
    }

    /// Process and resume coroutines that are ready to run.
    ///
    /// This updates wait states and resumes coroutines that have completed their wait.
    pub(crate) fn process_coroutines(
        &mut self,
        _lua: &Lua,
        dt: f64,
        commands: Rc<RefCell<ScriptCommands>>,
        event_queue: Rc<RefCell<Vec<ScriptEvent>>>,
    ) -> Result<(), LuaScriptingError> {
        use super::types::CoroutineStatus;

        let _commands_guard = CommandGuard::enter(commands.clone());
        let state = Rc::clone(&self.state_store);
        let _state_guard = StateGuard::enter(&state);
        let _event_queue_guard = EventQueueGuard::enter(&event_queue);
        let _coroutine_guard = CoroutineGuard::enter(&self.coroutines);

        let to_resume = {
            let mut coroutines = self.coroutines.borrow_mut();
            let mut to_resume = Vec::with_capacity(coroutines.len());

            // Update all coroutines' wait states
            for (id, coro_state) in coroutines.iter_mut() {
                coro_state.update(dt);

                // Check if ready to resume
                if coro_state.status == CoroutineStatus::Suspended && coro_state.is_ready(dt) {
                    to_resume.push(*id);
                }
            }

            to_resume
        };

        // Resume ready coroutines
        for id in to_resume {
            // Set current coroutine ID before resuming
            set_current_coroutine_id(Some(id));

            {
                let mut coroutines = self.coroutines.borrow_mut();
                let Some(coro_state) = coroutines.get_mut(&id) else {
                    set_current_coroutine_id(None);
                    continue;
                };

                // Mark as running
                coro_state.status = CoroutineStatus::Running;

                // Reset the wait state
                coro_state.reset_wait();

                // Resume the coroutine and update status
                match coro_state.thread.resume::<()>(()) {
                    Ok(_) => match coro_state.thread.status() {
                        mlua::ThreadStatus::Error => {
                            coro_state.status = CoroutineStatus::Dead;
                        }
                        mlua::ThreadStatus::Resumable => {
                            coro_state.status = CoroutineStatus::Suspended;
                        }
                        _ => {
                            coro_state.status = CoroutineStatus::Suspended;
                        }
                    },
                    Err(e) => {
                        coro_state.status = CoroutineStatus::Dead;
                        log::error!(target: "script", "Coroutine {} error: {}", id, e);
                    }
                }
            }

            // Clear current coroutine ID after processing
            set_current_coroutine_id(None);
        }

        // Clean up dead coroutines
        self.coroutines
            .borrow_mut()
            .retain(|_, state| state.status != CoroutineStatus::Dead);

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionCallOutcome {
    Missing,
    Executed,
}
