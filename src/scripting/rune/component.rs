use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

use rune::runtime::{RuntimeContext, Vm};

use super::commands::ScriptCommands;
use super::entity_registry::EntityHandleRegistry;
use super::error::RuneScriptingError;
use super::guards::{CommandGuard, EntityGuard, EventQueueGuard, StateGuard};
use super::types::{RuneScript, RuneScriptSource, ScriptEvent, ScriptStateMap};

/// Component that attaches a Rune script to an entity.
#[derive(Clone)]
pub struct RuneScriptComponent {
    source: RuneScriptSource,
    created_called: bool,
}

impl fmt::Debug for RuneScriptComponent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuneScriptComponent")
            .field("created_called", &self.created_called)
            .finish()
    }
}

impl RuneScriptComponent {
    /// Create a component from an arbitrary source descriptor.
    pub fn new(source: RuneScriptSource) -> Self {
        Self {
            source,
            created_called: false,
        }
    }

    /// Convenience for building a component from inline source code.
    pub fn new_inline(name: impl Into<String>, source: impl Into<String>) -> Self {
        Self::new(RuneScriptSource::inline(name, source))
    }

    pub fn source(&self) -> &RuneScriptSource {
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
}

#[derive(Debug)]
pub(crate) struct RuneScriptInstance {
    pub _script: Arc<RuneScript>,
    pub vm: Vm,
    pub source: RuneScriptSource,
    pub handles: Rc<RefCell<EntityHandleRegistry>>,
    pub state_store: Rc<RefCell<ScriptStateMap>>,
}

impl RuneScriptInstance {
    pub fn new(
        runtime: Arc<RuntimeContext>,
        script: Arc<RuneScript>,
        source: RuneScriptSource,
    ) -> Self {
        Self {
            vm: Vm::new(runtime, script.unit.clone()),
            _script: script,
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
        entity_bits: i64,
        commands: Rc<RefCell<ScriptCommands>>,
        event_queue: Rc<RefCell<Vec<ScriptEvent>>>,
    ) -> Result<FunctionCallOutcome, RuneScriptingError> {
        let _commands_guard = CommandGuard::enter(commands.clone());
        let state = Rc::clone(&self.state_store);
        let _state_guard = StateGuard::enter(&state);
        let _event_queue_guard = EventQueueGuard::enter(&event_queue);
        let _entity_guard = EntityGuard::enter(entity_bits);
        self.call_function(["on_created"], (entity_bits,))
    }

    pub fn call_update(
        &mut self,
        entity_bits: i64,
        dt: f64,
        commands: Rc<RefCell<ScriptCommands>>,
        event_queue: Rc<RefCell<Vec<ScriptEvent>>>,
    ) -> Result<FunctionCallOutcome, RuneScriptingError> {
        let _commands_guard = CommandGuard::enter(commands.clone());
        let state = Rc::clone(&self.state_store);
        let _state_guard = StateGuard::enter(&state);
        let _event_queue_guard = EventQueueGuard::enter(&event_queue);
        let _entity_guard = EntityGuard::enter(entity_bits);
        self.call_function(["update"], (entity_bits, dt))
    }

    pub fn call_function(
        &mut self,
        path: impl rune::ToTypeHash,
        args: impl rune::runtime::GuardedArgs,
    ) -> Result<FunctionCallOutcome, RuneScriptingError> {
        match self.vm.call(path, args) {
            Ok(_) => Ok(FunctionCallOutcome::Executed),
            Err(err) if is_missing_entry(&err) => Ok(FunctionCallOutcome::Missing),
            Err(err) => Err(RuneScriptingError::Vm(err)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FunctionCallOutcome {
    Missing,
    Executed,
}

fn is_missing_entry(err: &rune::runtime::VmError) -> bool {
    let message = err.to_string();
    message.contains("Missing entry") || message.contains("Missing function")
}
