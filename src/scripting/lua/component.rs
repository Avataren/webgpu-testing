use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

use mlua::Lua;

use super::commands::ScriptCommands;
use super::entity_registry::EntityHandleRegistry;
use super::error::LuaScriptingError;
use super::guards::{CommandGuard, EntityGuard, EventQueueGuard, StateGuard};
use super::types::{LuaScript, LuaScriptSource, ScriptEvent, ScriptMode, ScriptStateMap};

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
/// Unlike Rune which uses per-instance VMs, Lua scripts share a single Lua VM
/// but maintain per-entity state through the state_store.
pub struct LuaScriptInstance {
    pub script: Arc<LuaScript>,
    pub source: LuaScriptSource,
    pub handles: Rc<RefCell<EntityHandleRegistry>>,
    pub state_store: Rc<RefCell<ScriptStateMap>>,
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
    pub fn new(
        script: Arc<LuaScript>,
        source: LuaScriptSource,
    ) -> Self {
        Self {
            script,
            source,
            handles: Rc::new(RefCell::new(EntityHandleRegistry::default())),
            state_store: Rc::new(RefCell::new(HashMap::new())),
        }
    }

    pub fn command_buffer(&self) -> Rc<RefCell<ScriptCommands>> {
        Rc::new(RefCell::new(ScriptCommands::new(self.handles.clone())))
    }

    pub fn call_on_created(
        &mut self,
        lua: &Lua,
        entity_bits: i64,
        commands: Rc<RefCell<ScriptCommands>>,
        event_queue: Rc<RefCell<Vec<ScriptEvent>>>,
    ) -> Result<FunctionCallOutcome, LuaScriptingError> {
        // Pre-register self_entity in the registry to prevent collision with allocated handles
        if entity_bits != 0 {
            commands.borrow_mut().registry.borrow_mut().resolve_bits(entity_bits, entity_bits as u64);
        }

        let _commands_guard = CommandGuard::enter(commands.clone());
        let state = Rc::clone(&self.state_store);
        let _state_guard = StateGuard::enter(&state);
        let _event_queue_guard = EventQueueGuard::enter(&event_queue);
        let _entity_guard = EntityGuard::enter(entity_bits);

        self.call_function(lua, "on_created", entity_bits)
    }

    pub fn call_update(
        &mut self,
        lua: &Lua,
        entity_bits: i64,
        dt: f64,
        commands: Rc<RefCell<ScriptCommands>>,
        event_queue: Rc<RefCell<Vec<ScriptEvent>>>,
    ) -> Result<FunctionCallOutcome, LuaScriptingError> {
        // Pre-register self_entity in the registry to prevent collision with allocated handles
        if entity_bits != 0 {
            commands.borrow_mut().registry.borrow_mut().resolve_bits(entity_bits, entity_bits as u64);
        }

        let _commands_guard = CommandGuard::enter(commands.clone());
        let state = Rc::clone(&self.state_store);
        let _state_guard = StateGuard::enter(&state);
        let _event_queue_guard = EventQueueGuard::enter(&event_queue);
        let _entity_guard = EntityGuard::enter(entity_bits);

        self.call_function_with_args(lua, "update", (entity_bits, dt))
    }

    pub fn call_on_ui(
        &mut self,
        lua: &Lua,
        entity_bits: i64,
        ui_context: mlua::Value,
        commands: Rc<RefCell<ScriptCommands>>,
        event_queue: Rc<RefCell<Vec<ScriptEvent>>>,
    ) -> Result<FunctionCallOutcome, LuaScriptingError> {
        // Pre-register self_entity in the registry to prevent collision with allocated handles
        if entity_bits != 0 {
            commands.borrow_mut().registry.borrow_mut().resolve_bits(entity_bits, entity_bits as u64);
        }

        let _commands_guard = CommandGuard::enter(commands.clone());
        let state = Rc::clone(&self.state_store);
        let _state_guard = StateGuard::enter(&state);
        let _event_queue_guard = EventQueueGuard::enter(&event_queue);
        let _entity_guard = EntityGuard::enter(entity_bits);

        self.call_function_with_args(lua, "on_ui", (entity_bits, ui_context))
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
        let globals = lua.globals();

        // Check if function exists
        if !globals.contains_key(name)? {
            return Ok(FunctionCallOutcome::Missing);
        }

        // Try to get the function
        let func: mlua::Function = match globals.get(name) {
            Ok(f) => f,
            Err(_) => return Ok(FunctionCallOutcome::Missing),
        };

        // Call the function
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
        let globals = lua.globals();

        // Check if function exists
        if !globals.contains_key(name)? {
            return Ok(FunctionCallOutcome::Missing);
        }

        // Try to get the function
        let func: mlua::Function = match globals.get(name) {
            Ok(f) => f,
            Err(_) => return Ok(FunctionCallOutcome::Missing),
        };

        // Call the function
        match func.call::<()>(args) {
            Ok(_) => Ok(FunctionCallOutcome::Executed),
            Err(e) => Err(LuaScriptingError::Lua(e)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionCallOutcome {
    Missing,
    Executed,
}
