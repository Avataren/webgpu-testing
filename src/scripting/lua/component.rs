use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use mlua::{Lua, LuaSerdeExt};

use super::api::{resume_coroutine, CoroutineGuard};
use super::commands::ScriptCommands;
use super::entity_registry::EntityHandleRegistry;
use super::error::LuaScriptingError;
use super::guards::{CommandGuard, EntityGuard, EventQueueGuard, StateGuard};
use super::types::{
    CoroutineMap, LuaScript, LuaScriptSource, ScriptEvent, ScriptMode, ScriptStateMap,
};

/// Source of unique [`LuaScriptComponent`] binding ids. Zero is never handed out.
static NEXT_BINDING: AtomicU64 = AtomicU64::new(1);

fn next_binding() -> u64 {
    NEXT_BINDING.fetch_add(1, Ordering::Relaxed)
}

/// Component that attaches a Lua script to an entity.
///
/// Each component carries a unique binding id that ties it to a runtime script
/// instance. Cloning a component (duplicating an entity, restoring a snapshot,
/// instantiating a prefab) yields a new binding, so the copy always starts its
/// own fresh instance and runs `on_created`. Replacing an entity's component
/// likewise causes the script to be recompiled.
pub struct LuaScriptComponent {
    source: LuaScriptSource,
    /// Script execution mode (@editor, @tool, or no annotation)
    script_mode: ScriptMode,
    binding: u64,
}

impl Clone for LuaScriptComponent {
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
            script_mode: self.script_mode,
            binding: next_binding(),
        }
    }
}

impl fmt::Debug for LuaScriptComponent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LuaScriptComponent")
            .field("source", &self.source)
            .field("script_mode", &self.script_mode)
            .field("binding", &self.binding)
            .finish()
    }
}

impl LuaScriptComponent {
    /// Create a component from an arbitrary source descriptor.
    pub fn new(source: LuaScriptSource) -> Self {
        Self::with_script_mode(source, ScriptMode::RuntimeOnly)
    }

    /// Create a component with explicit script mode.
    ///
    /// The mode is re-derived from the script's annotation when it is compiled.
    pub fn with_script_mode(source: LuaScriptSource, script_mode: ScriptMode) -> Self {
        Self {
            source,
            script_mode,
            binding: next_binding(),
        }
    }

    /// Convenience for building a component from inline source code.
    pub fn new_inline(name: impl Into<String>, source: impl Into<String>) -> Self {
        Self::new(LuaScriptSource::inline(name, source))
    }

    pub fn source(&self) -> &LuaScriptSource {
        &self.source
    }

    /// Returns the script execution mode.
    pub fn script_mode(&self) -> ScriptMode {
        self.script_mode
    }

    /// Set the script execution mode.
    pub fn set_script_mode(&mut self, script_mode: ScriptMode) {
        self.script_mode = script_mode;
    }

    /// Identifier of the runtime instance this component is bound to.
    pub(crate) fn binding(&self) -> u64 {
        self.binding
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
    /// Files loaded with `require`/`include`; editing one reloads this script.
    pub(crate) dependencies: crate::scripting::lua::api::modules::Dependencies,
    /// What `on_destroyed(self, "reload")` returned: handed out once the reloaded
    /// script is rebuilt, then passed to its `on_created(self, previous)`.
    pub(crate) carried: Option<mlua::RegistryKey>,
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
        module_dirs: Vec<std::path::PathBuf>,
    ) -> Result<Self, LuaScriptingError> {
        // Create a new environment table that inherits from _G
        let env = lua.create_table()?;
        let globals = lua.globals();
        let metatable = lua.create_table()?;
        metatable.set("__index", globals)?;
        env.set_metatable(Some(metatable));
        let dependencies = crate::scripting::lua::api::modules::Dependencies::default();
        crate::scripting::lua::api::modules::install(lua, &env, module_dirs, dependencies.clone())?;

        // Store the environment in the registry to keep it alive
        let env_registry_key = lua.create_registry_value(env)?;

        Ok(Self {
            script,
            source,
            handles: Rc::new(RefCell::new(EntityHandleRegistry::default())),
            state_store: Rc::new(RefCell::new(HashMap::new())),
            coroutines: Rc::new(RefCell::new(HashMap::new())),
            env_registry_key,
            dependencies,
            carried: None,
        })
    }

    /// Execute the script's top-level chunk inside its environment.
    ///
    /// This defines the script's functions and runs any top-level statements with the
    /// full scripting API available, exactly like a lifecycle hook.
    pub(crate) fn run_chunk(
        &mut self,
        lua: &Lua,
        entity_bits: i64,
        commands: Rc<RefCell<ScriptCommands>>,
        event_queue: Rc<RefCell<Vec<ScriptEvent>>>,
    ) -> Result<FunctionCallOutcome, LuaScriptingError> {
        let env: mlua::Table = lua.registry_value(&self.env_registry_key)?;
        let chunk = lua
            .load(&**self.script.chunk)
            .set_name(&*self.script.name)
            .set_environment(env)
            .into_function()?;
        self.with_script_context(entity_bits, commands, event_queue, || {
            chunk.call::<()>(())?;
            Ok(FunctionCallOutcome::Executed)
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
        // `previous` is what on_destroyed returned before a hot reload, or nil.
        let previous = match self.carried.take() {
            Some(key) => {
                let value = lua.registry_value::<mlua::Value>(&key)?;
                lua.remove_registry_value(key)?;
                value
            }
            None => mlua::Value::Nil,
        };
        self.with_script_context(entity_bits, commands, event_queue, || {
            self.call_function_with_args(lua, "on_created", (entity_bits, previous))
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

    /// Call `on_destroyed(self_entity, reason)`; `reason` is `"reload"` when the script is
    /// about to be rebuilt after an edit and `"removed"` when the entity or its script
    /// component is going away.
    pub(crate) fn call_on_destroyed(
        &mut self,
        lua: &Lua,
        entity_bits: i64,
        reload: bool,
        commands: Rc<RefCell<ScriptCommands>>,
        event_queue: Rc<RefCell<Vec<ScriptEvent>>>,
    ) -> Result<FunctionCallOutcome, LuaScriptingError> {
        let reason = if reload { "reload" } else { "removed" };
        let mut kept = None;
        let outcome = self.with_script_context(entity_bits, commands, event_queue, || {
            let Some(func) = self.resolve_function(lua, "on_destroyed")? else {
                return Ok(FunctionCallOutcome::Missing);
            };
            let value = func.call::<mlua::Value>((entity_bits, reason))?;
            // Before a reload, the returned value is kept for the new code.
            if reload && !value.is_nil() {
                kept = Some(lua.create_registry_value(value)?);
            }
            Ok(FunctionCallOutcome::Executed)
        })?;
        self.carried = kept;
        Ok(outcome)
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

    pub(crate) fn call_event(
        &mut self,
        lua: &Lua,
        entity_bits: i64,
        event_name: &str,
        data: serde_json::Value,
        commands: Rc<RefCell<ScriptCommands>>,
        event_queue: Rc<RefCell<Vec<ScriptEvent>>>,
    ) -> Result<FunctionCallOutcome, LuaScriptingError> {
        self.with_script_context(entity_bits, commands, event_queue, || {
            // Event handlers are named "on_<event_name>"
            let handler_name = if event_name.starts_with("on_") {
                event_name.to_string()
            } else {
                format!("on_{}", event_name)
            };

            let lua_data = lua.to_value(&data).map_err(LuaScriptingError::Lua)?;
            self.call_function_with_args(lua, &handler_name, lua_data)
        })
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
    /// This advances wait states and resumes coroutines whose wait has completed.
    pub(crate) fn process_coroutines(
        &mut self,
        dt: f64,
        commands: Rc<RefCell<ScriptCommands>>,
        event_queue: Rc<RefCell<Vec<ScriptEvent>>>,
    ) -> Result<(), LuaScriptingError> {
        use super::types::CoroutineStatus;

        if self.coroutines.borrow().is_empty() {
            return Ok(());
        }

        let _commands_guard = CommandGuard::enter(commands);
        let state = Rc::clone(&self.state_store);
        let _state_guard = StateGuard::enter(&state);
        let _event_queue_guard = EventQueueGuard::enter(&event_queue);
        let _coroutine_guard = CoroutineGuard::enter(&self.coroutines);

        let mut ready: Vec<_> = self
            .coroutines
            .borrow_mut()
            .iter_mut()
            .filter_map(|(id, coroutine)| {
                coroutine.update(dt);
                (coroutine.status == CoroutineStatus::Suspended && coroutine.is_ready())
                    .then_some(*id)
            })
            .collect();
        // Resume in creation order so behaviour is deterministic.
        ready.sort_unstable();

        let mut first_error = None;
        for id in ready {
            if let Err(err) = resume_coroutine(&self.coroutines, id, mlua::MultiValue::new()) {
                first_error.get_or_insert(LuaScriptingError::Lua(err));
            }
        }

        self.coroutines
            .borrow_mut()
            .retain(|_, coroutine| coroutine.status != CoroutineStatus::Dead);

        first_error.map_or(Ok(()), Err)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionCallOutcome {
    Missing,
    Executed,
}
