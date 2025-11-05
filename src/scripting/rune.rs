use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use glam::{EulerRot, Quat, Vec3};
use hecs::{ComponentError, Entity, NoSuchEntity, World};
use rune::alloc::Error as RuneAllocError;
use rune::runtime::{try_result, RuntimeContext, Vm, VmResult};
use rune::{Context, ContextError, Diagnostics, FromValue, Module, Source, Sources, Value};
use thiserror::Error;

use log::{debug, error, info, warn};

use crate::app::{AppBuilder, Plugin, StartupContext};
use crate::scene::{InputState, Name, Transform, TransformComponent};
use crate::scripting::component_registry::{ComponentRegistry, ComponentRegistryError};

/// Error type produced by the Rune scripting integration.
#[derive(Debug, Error)]
pub enum RuneScriptingError {
    /// Loading a script from disk failed.
    #[error("failed to load script `{path}`: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Compiling a script failed.
    #[error("failed to compile script `{name}`: {message}")]
    Compile { name: Arc<str>, message: String },
    /// A virtual machine execution error occurred.
    #[error("{0}")]
    Vm(#[from] rune::runtime::VmError),
    /// Allocation within the Rune runtime failed.
    #[error("failed to allocate Rune resources: {0}")]
    Allocation(#[from] RuneAllocError),
    /// An error occurred while mutating the ECS world.
    #[error("failed to mutate ECS world: {0}")]
    Hecs(#[from] ComponentError),
    /// Tried to access an entity that no longer exists.
    #[error("failed to access entity: {0}")]
    MissingEntity(#[from] NoSuchEntity),
    /// Failed to initialize the Rune runtime context.
    #[error("failed to initialize Rune context: {0}")]
    Context(#[from] ContextError),
    /// Component registry error.
    #[error("component registry error: {0}")]
    ComponentRegistry(#[from] ComponentRegistryError),
}

/// Host-facing representation of a script source.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RuneScriptSource {
    /// Inline source code bundled with an entity.
    Inline { name: Arc<str>, source: Arc<str> },
    /// External file that should be loaded at runtime.
    File { path: PathBuf },
}

impl RuneScriptSource {
    /// Construct an inline script source.
    pub fn inline(name: impl Into<String>, source: impl Into<String>) -> Self {
        Self::Inline {
            name: Arc::from(name.into().into_boxed_str()),
            source: Arc::from(source.into().into_boxed_str()),
        }
    }

    /// Construct a file-backed script source.
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self::File { path: path.into() }
    }

    fn load(&self, script_root: Option<&Path>) -> Result<LoadedScript, RuneScriptingError> {
        match self {
            Self::Inline { name, source } => Ok(LoadedScript {
                name: name.clone(),
                contents: source.clone(),
                path: None,
            }),
            Self::File { path } => {
                let absolute = if path.is_absolute() {
                    path.clone()
                } else if let Some(root) = script_root {
                    root.join(path)
                } else {
                    path.clone()
                };

                let contents = std::fs::read_to_string(&absolute).map_err(|source| {
                    RuneScriptingError::Io {
                        path: absolute.clone(),
                        source,
                    }
                })?;

                Ok(LoadedScript {
                    name: Arc::from(absolute.to_string_lossy().into_owned().into_boxed_str()),
                    contents: Arc::from(contents.into_boxed_str()),
                    path: Some(absolute),
                })
            }
        }
    }
}

struct LoadedScript {
    name: Arc<str>,
    contents: Arc<str>,
    path: Option<PathBuf>,
}

/// Compiled Rune script.
#[derive(Debug)]
struct RuneScript {
    _name: Arc<str>,
    unit: Arc<rune::Unit>,
}

impl RuneScript {
    fn new(name: Arc<str>, unit: rune::Unit) -> Arc<Self> {
        Arc::new(Self {
            _name: name,
            unit: Arc::new(unit),
        })
    }
}

/// Runtime wrapper responsible for compiling and executing scripts.
pub struct RuneScriptingRuntime {
    context: Context,
    runtime: Arc<RuntimeContext>,
    script_root: Option<PathBuf>,
}

impl RuneScriptingRuntime {
    /// Construct a new runtime with the default Rune modules installed.
    pub fn new() -> Result<Self, RuneScriptingError> {
        let mut context = Context::with_default_modules()?;
        context.install(&script_module()?)?;
        let runtime = Arc::new(context.runtime()?);
        Ok(Self {
            context,
            runtime,
            script_root: None,
        })
    }

    /// Configure a base directory for resolving relative script paths.
    pub fn set_script_root(&mut self, root: impl Into<PathBuf>) {
        self.script_root = Some(root.into());
    }

    fn compile(
        &mut self,
        source: &RuneScriptSource,
    ) -> Result<Arc<RuneScript>, RuneScriptingError> {
        let loaded = source.load(self.script_root.as_deref())?;

        let mut sources = Sources::new();
        let source = if let Some(path) = &loaded.path {
            Source::with_path(loaded.name.as_ref(), loaded.contents.as_ref(), path)?
        } else {
            Source::new(loaded.name.as_ref(), loaded.contents.as_ref())?
        };
        sources.insert(source)?;

        let mut diagnostics = Diagnostics::new();
        let result = rune::prepare(&mut sources)
            .with_context(&self.context)
            .with_diagnostics(&mut diagnostics)
            .build();

        if diagnostics.has_error() {
            return Err(RuneScriptingError::Compile {
                name: loaded.name.clone(),
                message: format!("{diagnostics:?}"),
            });
        }

        let unit = result.map_err(|error| RuneScriptingError::Compile {
            name: loaded.name.clone(),
            message: error.to_string(),
        })?;

        Ok(RuneScript::new(loaded.name, unit))
    }

    fn instantiate(&self, script: Arc<RuneScript>, source: RuneScriptSource) -> RuneScriptInstance {
        RuneScriptInstance::new(self.runtime.clone(), script, source)
    }
}

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

type ScriptStateMap = HashMap<(i64, String), Value>;

// ============================================================================
// EVENT SYSTEM DATA STRUCTURES
// ============================================================================

/// An event that can be emitted by scripts.
#[derive(Debug, Clone)]
pub struct ScriptEvent {
    pub name: String,
    pub data: Value,
}

/// A subscription to an event by a specific entity.
#[derive(Debug, Clone)]
struct EventSubscription {
    entity_id: Entity,
    callback_name: String,
}

/// Map of event names to their subscribers.
type EventSubscriptions = HashMap<String, Vec<EventSubscription>>;

#[derive(Debug)]
struct RuneScriptInstance {
    _script: Arc<RuneScript>,
    vm: Vm,
    source: RuneScriptSource,
    handles: Rc<RefCell<EntityHandleRegistry>>,
    state_store: Rc<RefCell<ScriptStateMap>>,
}

impl RuneScriptInstance {
    fn new(
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

    fn command_buffer(&self) -> Rc<RefCell<ScriptCommands>> {
        Rc::new(RefCell::new(ScriptCommands::new(self.handles.clone())))
    }

    fn call_on_created(
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

    fn call_update(
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

    fn call_function(
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
enum FunctionCallOutcome {
    Missing,
    Executed,
}

fn is_missing_entry(err: &rune::runtime::VmError) -> bool {
    let message = err.to_string();
    message.contains("Missing entry") || message.contains("Missing function")
}

/// Tracks the active command queue while executing a script.
#[derive(Default)]
struct ActiveCommands(Option<Rc<RefCell<ScriptCommands>>>);

impl ActiveCommands {
    fn set(&mut self, commands: Rc<RefCell<ScriptCommands>>) {
        self.0 = Some(commands);
    }

    fn clear(&mut self) {
        self.0 = None;
    }

    fn with<R>(&mut self, f: impl FnOnce(&mut ScriptCommands) -> VmResult<R>) -> VmResult<R> {
        let rc = match &self.0 {
            Some(rc) => rc.clone(),
            None => return VmResult::panic("script command context missing"),
        };
        let mut guard = rc.borrow_mut();
        f(&mut guard)
    }
}

thread_local! {
    static ACTIVE_COMMANDS: RefCell<ActiveCommands> = RefCell::new(ActiveCommands::default());
    static ACTIVE_STATE: RefCell<Option<Rc<RefCell<ScriptStateMap>>>> = const { RefCell::new(None) };
    static ACTIVE_WORLD: RefCell<Option<*const World>> = const { RefCell::new(None) };
    static ACTIVE_REGISTRY: RefCell<Option<*const ComponentRegistry>> = const { RefCell::new(None) };
    static ACTIVE_INPUT_STATE: RefCell<Option<*const InputState>> = const { RefCell::new(None) };
    static ACTIVE_EVENT_QUEUE: RefCell<Option<Rc<RefCell<Vec<ScriptEvent>>>>> = const { RefCell::new(None) };
    static ACTIVE_ENTITY: RefCell<Option<i64>> = const { RefCell::new(None) };
}

struct CommandGuard;

impl CommandGuard {
    fn enter(commands: Rc<RefCell<ScriptCommands>>) -> Self {
        ACTIVE_COMMANDS.with(|cell| cell.borrow_mut().set(commands));
        Self
    }
}

impl Drop for CommandGuard {
    fn drop(&mut self) {
        ACTIVE_COMMANDS.with(|cell| cell.borrow_mut().clear());
    }
}

struct StateGuard {
    // Keep an Rc clone around so the state remains available while the guard
    // exists. We don't hold a RefMut here to avoid double-borrow issues when
    // `with_active_state` borrows the map later.
    _state: Rc<RefCell<ScriptStateMap>>,
}

impl StateGuard {
    fn enter(state: &Rc<RefCell<ScriptStateMap>>) -> Self {
        let state_clone = Rc::clone(state);
        ACTIVE_STATE.with(|cell| *cell.borrow_mut() = Some(Rc::clone(&state_clone)));
        Self {
            _state: state_clone,
        }
    }
}

impl Drop for StateGuard {
    fn drop(&mut self) {
        ACTIVE_STATE.with(|cell| *cell.borrow_mut() = None);
    }
}

fn with_active_state<R>(f: impl FnOnce(&mut ScriptStateMap) -> VmResult<R>) -> VmResult<R> {
    ACTIVE_STATE.with(|cell| {
        let opt = cell.borrow();
        let Some(rc) = opt.as_ref() else {
            return VmResult::panic("state store missing");
        };
        let mut borrow = rc.borrow_mut();
        f(&mut borrow)
    })
}

struct WorldGuard;

impl WorldGuard {
    fn enter(world: &World) -> Self {
        let ptr = world as *const World;
        ACTIVE_WORLD.with(|cell| *cell.borrow_mut() = Some(ptr));
        Self
    }
}

impl Drop for WorldGuard {
    fn drop(&mut self) {
        ACTIVE_WORLD.with(|cell| *cell.borrow_mut() = None);
    }
}

struct RegistryGuard;

impl RegistryGuard {
    fn enter(registry: &ComponentRegistry) -> Self {
        let ptr = registry as *const ComponentRegistry;
        ACTIVE_REGISTRY.with(|cell| *cell.borrow_mut() = Some(ptr));
        Self
    }
}

impl Drop for RegistryGuard {
    fn drop(&mut self) {
        ACTIVE_REGISTRY.with(|cell| *cell.borrow_mut() = None);
    }
}

struct InputStateGuard;

impl InputStateGuard {
    fn enter(input_state: &InputState) -> Self {
        let ptr = input_state as *const InputState;
        ACTIVE_INPUT_STATE.with(|cell| *cell.borrow_mut() = Some(ptr));
        Self
    }
}

impl Drop for InputStateGuard {
    fn drop(&mut self) {
        ACTIVE_INPUT_STATE.with(|cell| *cell.borrow_mut() = None);
    }
}

struct EventQueueGuard {
    _queue: Rc<RefCell<Vec<ScriptEvent>>>,
}

impl EventQueueGuard {
    fn enter(queue: &Rc<RefCell<Vec<ScriptEvent>>>) -> Self {
        let queue_clone = Rc::clone(queue);
        ACTIVE_EVENT_QUEUE.with(|cell| *cell.borrow_mut() = Some(Rc::clone(&queue_clone)));
        Self {
            _queue: queue_clone,
        }
    }
}

impl Drop for EventQueueGuard {
    fn drop(&mut self) {
        ACTIVE_EVENT_QUEUE.with(|cell| *cell.borrow_mut() = None);
    }
}

struct EntityGuard;

impl EntityGuard {
    fn enter(entity_bits: i64) -> Self {
        ACTIVE_ENTITY.with(|cell| *cell.borrow_mut() = Some(entity_bits));
        Self
    }
}

impl Drop for EntityGuard {
    fn drop(&mut self) {
        ACTIVE_ENTITY.with(|cell| *cell.borrow_mut() = None);
    }
}

fn with_active_world<R>(f: impl FnOnce(&World) -> VmResult<R>) -> VmResult<R> {
    ACTIVE_WORLD.with(|cell| {
        let opt = cell.borrow();
        let Some(ptr) = *opt else {
            return VmResult::panic("world not available");
        };
        // SAFETY: The World pointer is only set during script execution and cleared after.
        // We control script execution to be single-threaded and non-reentrant.
        let world = unsafe { &*ptr };
        f(world)
    })
}

fn with_active_registry<R>(f: impl FnOnce(&ComponentRegistry) -> VmResult<R>) -> VmResult<R> {
    ACTIVE_REGISTRY.with(|cell| {
        let opt = cell.borrow();
        let Some(ptr) = *opt else {
            return VmResult::panic("component registry not available");
        };
        // SAFETY: The ComponentRegistry pointer is only set during script execution and cleared after.
        // We control script execution to be single-threaded and non-reentrant.
        let registry = unsafe { &*ptr };
        f(registry)
    })
}

fn with_active_input_state<R>(f: impl FnOnce(&InputState) -> VmResult<R>) -> VmResult<R> {
    ACTIVE_INPUT_STATE.with(|cell| {
        let opt = cell.borrow();
        let Some(ptr) = *opt else {
            return VmResult::panic("input state not available");
        };
        // SAFETY: The InputState pointer is only set during script execution and cleared after.
        // We control script execution to be single-threaded and non-reentrant.
        let input_state = unsafe { &*ptr };
        f(input_state)
    })
}

fn with_active_event_queue<R>(f: impl FnOnce(&mut Vec<ScriptEvent>) -> VmResult<R>) -> VmResult<R> {
    ACTIVE_EVENT_QUEUE.with(|cell| {
        let opt = cell.borrow();
        let Some(rc) = opt.as_ref() else {
            return VmResult::panic("event queue not available");
        };
        let mut borrow = rc.borrow_mut();
        f(&mut borrow)
    })
}

fn get_active_entity() -> VmResult<i64> {
    ACTIVE_ENTITY.with(|cell| {
        let opt = cell.borrow();
        match *opt {
            Some(entity_bits) => VmResult::Ok(entity_bits),
            None => VmResult::panic("active entity not available"),
        }
    })
}

#[derive(Default, Debug)]
struct EntityHandleRegistry {
    next_handle: i64,
    handles: HashMap<i64, Option<u64>>,
}

impl EntityHandleRegistry {
    fn allocate(&mut self) -> i64 {
        let handle = self.next_handle;
        self.next_handle -= 1;
        self.handles.insert(handle, None);
        handle
    }

    fn resolve(&mut self, handle: i64, entity: Entity) {
        self.handles.insert(handle, Some(entity.to_bits().get()));
    }

    fn resolved_bits(&self, handle: i64) -> Option<u64> {
        self.handles.get(&handle).and_then(|bits| *bits)
    }

    fn contains(&self, handle: i64) -> bool {
        self.handles.contains_key(&handle)
    }
}

#[derive(Default)]
struct PendingEntity {
    name: Option<String>,
    translation: Option<Vec3>,
    rotation: Option<Quat>,
    script: Option<RuneScriptSource>,
}

enum ExistingCommand {
    SetName {
        entity_bits: u64,
        name: String,
    },
    SetTranslation {
        entity_bits: u64,
        translation: Vec3,
    },
    SetRotation {
        entity_bits: u64,
        rotation: Quat,
    },
    AttachScript {
        entity_bits: u64,
        source: RuneScriptSource,
    },
    ImportGltf {
        entity_bits: u64,
        path: String,
        scale: f32,
    },
    SetComponent {
        entity_bits: u64,
        component_name: String,
        value: Value,
    },
    AddComponent {
        entity_bits: u64,
        component_name: String,
        value: Value,
    },
    RemoveComponent {
        entity_bits: u64,
        component_name: String,
    },
    Translate {
        entity_bits: u64,
        delta: Vec3,
    },
    Rotate {
        entity_bits: u64,
        axis: Vec3,
        angle: f32,
    },
    SetScale {
        entity_bits: u64,
        scale: Vec3,
    },
    LookAt {
        entity_bits: u64,
        target: Vec3,
    },
    SetParent {
        entity_bits: u64,
        parent_bits: Option<u64>,
    },
    SubscribeEvent {
        entity_bits: u64,
        event_name: String,
        callback_name: String,
    },
    UnsubscribeEvent {
        entity_bits: u64,
        event_name: String,
    },
}

struct ScriptCommands {
    registry: Rc<RefCell<EntityHandleRegistry>>,
    pending: HashMap<i64, PendingEntity>,
    existing: Vec<ExistingCommand>,
}

impl ScriptCommands {
    fn new(registry: Rc<RefCell<EntityHandleRegistry>>) -> Self {
        Self {
            registry,
            pending: HashMap::new(),
            existing: Vec::new(),
        }
    }

    fn spawn_entity(&mut self, name: Option<String>) -> i64 {
        let id = {
            let mut registry = self.registry.borrow_mut();
            registry.allocate()
        };
        self.pending.insert(
            id,
            PendingEntity {
                name,
                ..PendingEntity::default()
            },
        );
        id
    }

    fn resolve_entity_bits(&self, handle: i64) -> VmResult<u64> {
        {
            let registry = self.registry.borrow();
            if let Some(bits) = registry.resolved_bits(handle) {
                return VmResult::Ok(bits);
            }

            if registry.contains(handle) {
                return VmResult::panic("entity handle is not yet available");
            }
        }

        if handle == 0 {
            return VmResult::panic("invalid entity handle");
        }

        VmResult::Ok(handle as u64)
    }

    fn set_name(&mut self, handle: i64, name: String) -> VmResult<()> {
        if let Some(entry) = self.pending.get_mut(&handle) {
            entry.name = Some(name);
            return VmResult::Ok(());
        }

        let entity_bits = match self.resolve_entity_bits(handle) {
            VmResult::Ok(bits) => bits,
            VmResult::Err(err) => return VmResult::Err(err),
        };
        self.existing
            .push(ExistingCommand::SetName { entity_bits, name });
        VmResult::Ok(())
    }

    fn set_translation(&mut self, handle: i64, translation: Vec3) -> VmResult<()> {
        if let Some(entry) = self.pending.get_mut(&handle) {
            entry.translation = Some(translation);
            return VmResult::Ok(());
        }

        let entity_bits = match self.resolve_entity_bits(handle) {
            VmResult::Ok(bits) => bits,
            VmResult::Err(err) => return VmResult::Err(err),
        };
        self.existing.push(ExistingCommand::SetTranslation {
            entity_bits,
            translation,
        });
        VmResult::Ok(())
    }

    fn set_rotation(&mut self, handle: i64, rotation: Quat) -> VmResult<()> {
        if let Some(entry) = self.pending.get_mut(&handle) {
            entry.rotation = Some(rotation);
            return VmResult::Ok(());
        }

        let entity_bits = match self.resolve_entity_bits(handle) {
            VmResult::Ok(bits) => bits,
            VmResult::Err(err) => return VmResult::Err(err),
        };
        self.existing.push(ExistingCommand::SetRotation {
            entity_bits,
            rotation,
        });
        VmResult::Ok(())
    }

    fn attach_inline_script(&mut self, handle: i64, name: String, source: String) -> VmResult<()> {
        let descriptor = RuneScriptSource::inline(name, source);
        if let Some(entry) = self.pending.get_mut(&handle) {
            entry.script = Some(descriptor);
            return VmResult::Ok(());
        }

        let entity_bits = match self.resolve_entity_bits(handle) {
            VmResult::Ok(bits) => bits,
            VmResult::Err(err) => return VmResult::Err(err),
        };
        self.existing.push(ExistingCommand::AttachScript {
            entity_bits,
            source: descriptor,
        });
        VmResult::Ok(())
    }

    fn attach_file_script(&mut self, handle: i64, path: String) -> VmResult<()> {
        let descriptor = RuneScriptSource::file(path);
        if let Some(entry) = self.pending.get_mut(&handle) {
            entry.script = Some(descriptor);
            return VmResult::Ok(());
        }

        let entity_bits = match self.resolve_entity_bits(handle) {
            VmResult::Ok(bits) => bits,
            VmResult::Err(err) => return VmResult::Err(err),
        };
        self.existing.push(ExistingCommand::AttachScript {
            entity_bits,
            source: descriptor,
        });
        VmResult::Ok(())
    }

    fn import_gltf(&mut self, handle: i64, path: String, scale: f32) -> VmResult<()> {
        let entity_bits = match self.resolve_entity_bits(handle) {
            VmResult::Ok(bits) => bits,
            VmResult::Err(err) => return VmResult::Err(err),
        };

        self.existing.push(ExistingCommand::ImportGltf {
            entity_bits,
            path,
            scale,
        });
        VmResult::Ok(())
    }

    fn set_component(&mut self, handle: i64, component_name: String, value: Value) -> VmResult<()> {
        let entity_bits = match self.resolve_entity_bits(handle) {
            VmResult::Ok(bits) => bits,
            VmResult::Err(err) => return VmResult::Err(err),
        };

        self.existing.push(ExistingCommand::SetComponent {
            entity_bits,
            component_name,
            value,
        });
        VmResult::Ok(())
    }

    fn add_component(&mut self, handle: i64, component_name: String, value: Value) -> VmResult<()> {
        let entity_bits = match self.resolve_entity_bits(handle) {
            VmResult::Ok(bits) => bits,
            VmResult::Err(err) => return VmResult::Err(err),
        };

        self.existing.push(ExistingCommand::AddComponent {
            entity_bits,
            component_name,
            value,
        });
        VmResult::Ok(())
    }

    fn remove_component(&mut self, handle: i64, component_name: String) -> VmResult<()> {
        let entity_bits = match self.resolve_entity_bits(handle) {
            VmResult::Ok(bits) => bits,
            VmResult::Err(err) => return VmResult::Err(err),
        };

        self.existing.push(ExistingCommand::RemoveComponent {
            entity_bits,
            component_name,
        });
        VmResult::Ok(())
    }

    fn translate(&mut self, handle: i64, delta: Vec3) -> VmResult<()> {
        let entity_bits = match self.resolve_entity_bits(handle) {
            VmResult::Ok(bits) => bits,
            VmResult::Err(err) => return VmResult::Err(err),
        };

        self.existing.push(ExistingCommand::Translate {
            entity_bits,
            delta,
        });
        VmResult::Ok(())
    }

    fn rotate(&mut self, handle: i64, axis: Vec3, angle: f32) -> VmResult<()> {
        let entity_bits = match self.resolve_entity_bits(handle) {
            VmResult::Ok(bits) => bits,
            VmResult::Err(err) => return VmResult::Err(err),
        };

        self.existing.push(ExistingCommand::Rotate {
            entity_bits,
            axis,
            angle,
        });
        VmResult::Ok(())
    }

    fn set_scale(&mut self, handle: i64, scale: Vec3) -> VmResult<()> {
        let entity_bits = match self.resolve_entity_bits(handle) {
            VmResult::Ok(bits) => bits,
            VmResult::Err(err) => return VmResult::Err(err),
        };

        self.existing.push(ExistingCommand::SetScale {
            entity_bits,
            scale,
        });
        VmResult::Ok(())
    }

    fn look_at(&mut self, handle: i64, target: Vec3) -> VmResult<()> {
        let entity_bits = match self.resolve_entity_bits(handle) {
            VmResult::Ok(bits) => bits,
            VmResult::Err(err) => return VmResult::Err(err),
        };

        self.existing.push(ExistingCommand::LookAt {
            entity_bits,
            target,
        });
        VmResult::Ok(())
    }

    fn set_parent(&mut self, handle: i64, parent_handle: Option<i64>) -> VmResult<()> {
        let entity_bits = match self.resolve_entity_bits(handle) {
            VmResult::Ok(bits) => bits,
            VmResult::Err(err) => return VmResult::Err(err),
        };

        let parent_bits = if let Some(parent) = parent_handle {
            Some(match self.resolve_entity_bits(parent) {
                VmResult::Ok(bits) => bits,
                VmResult::Err(err) => return VmResult::Err(err),
            })
        } else {
            None
        };

        self.existing.push(ExistingCommand::SetParent {
            entity_bits,
            parent_bits,
        });
        VmResult::Ok(())
    }

    fn subscribe_event(
        &mut self,
        handle: i64,
        event_name: String,
        callback_name: String,
    ) -> VmResult<()> {
        let entity_bits = match self.resolve_entity_bits(handle) {
            VmResult::Ok(bits) => bits,
            VmResult::Err(err) => return VmResult::Err(err),
        };

        self.existing.push(ExistingCommand::SubscribeEvent {
            entity_bits,
            event_name,
            callback_name,
        });
        VmResult::Ok(())
    }

    fn unsubscribe_event(&mut self, handle: i64, event_name: String) -> VmResult<()> {
        let entity_bits = match self.resolve_entity_bits(handle) {
            VmResult::Ok(bits) => bits,
            VmResult::Err(err) => return VmResult::Err(err),
        };

        self.existing.push(ExistingCommand::UnsubscribeEvent {
            entity_bits,
            event_name,
        });
        VmResult::Ok(())
    }

    fn is_empty(&self) -> bool {
        self.pending.is_empty() && self.existing.is_empty()
    }

    fn apply(&mut self, world: &mut World, registry: &ComponentRegistry) -> Result<ScriptApplyResult, RuneScriptingError> {
        let mut result = ScriptApplyResult::default();

        for (handle, mut pending) in self.pending.drain() {
            let entity = world.spawn(());
            self.registry.borrow_mut().resolve(handle, entity);

            if let Some(name) = pending.name {
                world.insert_one(entity, Name(name))?;
            }

            if pending.translation.is_some() || pending.rotation.is_some() {
                let mut transform = Transform::default();
                if let Some(translation) = pending.translation.take() {
                    transform.translation = translation;
                }
                if let Some(rotation) = pending.rotation.take() {
                    transform.rotation = rotation;
                }
                world.insert_one(entity, TransformComponent(transform))?;
            }

            if let Some(script) = pending.script {
                world.insert_one(entity, RuneScriptComponent::new(script))?;
                result.scripts_added.push(entity);
            }
        }

        for command in self.existing.drain(..) {
            match command {
                ExistingCommand::SetName { entity_bits, name } => {
                    let Some(entity) = Entity::from_bits(entity_bits) else {
                        continue;
                    };

                    let mut pending_name = Some(name);
                    match world.get::<&mut Name>(entity) {
                        Ok(mut current) => {
                            current.0 = pending_name
                                .take()
                                .expect("name should remain available to update");
                        }
                        Err(ComponentError::MissingComponent(_)) => {}
                        Err(ComponentError::NoSuchEntity) => {
                            return Err(ComponentError::NoSuchEntity.into());
                        }
                    }

                    if let Some(name) = pending_name {
                        world.insert_one(entity, Name(name))?;
                    }
                }
                ExistingCommand::SetTranslation {
                    entity_bits,
                    translation,
                } => {
                    let Some(entity) = Entity::from_bits(entity_bits) else {
                        continue;
                    };

                    Self::modify_transform(world, entity, |transform| {
                        transform.translation = translation;
                    })?;
                }
                ExistingCommand::SetRotation {
                    entity_bits,
                    rotation,
                } => {
                    let Some(entity) = Entity::from_bits(entity_bits) else {
                        continue;
                    };

                    Self::modify_transform(world, entity, |transform| {
                        transform.rotation = rotation;
                    })?;
                }
                ExistingCommand::ImportGltf {
                    entity_bits,
                    path,
                    scale,
                } => {
                    let Some(entity) = Entity::from_bits(entity_bits) else {
                        continue;
                    };

                    if world.entity(entity).is_err() {
                        return Err(ComponentError::NoSuchEntity.into());
                    }

                    result.gltf_imports.push(PendingGltfImport {
                        parent: entity,
                        path: PathBuf::from(path),
                        scale,
                    });
                }
                ExistingCommand::AttachScript {
                    entity_bits,
                    source,
                } => {
                    let Some(entity) = Entity::from_bits(entity_bits) else {
                        continue;
                    };
                    world.insert_one(entity, RuneScriptComponent::new(source))?;
                    result.scripts_added.push(entity);
                }
                ExistingCommand::SetComponent {
                    entity_bits,
                    component_name,
                    value,
                } => {
                    let Some(entity) = Entity::from_bits(entity_bits) else {
                        continue;
                    };

                    if world.entity(entity).is_err() {
                        return Err(ComponentError::NoSuchEntity.into());
                    }

                    // Use the registry to set the component
                    registry.set_component(world, entity, &component_name, &value)?;
                }
                ExistingCommand::AddComponent {
                    entity_bits,
                    component_name,
                    value,
                } => {
                    let Some(entity) = Entity::from_bits(entity_bits) else {
                        continue;
                    };

                    if world.entity(entity).is_err() {
                        return Err(ComponentError::NoSuchEntity.into());
                    }

                    // Use the registry to add the component
                    registry.set_component(world, entity, &component_name, &value)?;
                }
                ExistingCommand::RemoveComponent {
                    entity_bits,
                    component_name,
                } => {
                    let Some(entity) = Entity::from_bits(entity_bits) else {
                        continue;
                    };

                    if world.entity(entity).is_err() {
                        return Err(ComponentError::NoSuchEntity.into());
                    }

                    // We need a remove_component method in the registry
                    // For now, log a warning
                    warn!(target: "script", "remove_component not yet fully implemented for {}", component_name);
                }
                ExistingCommand::Translate {
                    entity_bits,
                    delta,
                } => {
                    let Some(entity) = Entity::from_bits(entity_bits) else {
                        continue;
                    };

                    Self::modify_transform(world, entity, |transform| {
                        transform.translation += delta;
                    })?;
                }
                ExistingCommand::Rotate {
                    entity_bits,
                    axis,
                    angle,
                } => {
                    let Some(entity) = Entity::from_bits(entity_bits) else {
                        continue;
                    };

                    Self::modify_transform(world, entity, |transform| {
                        let rotation = glam::Quat::from_axis_angle(axis.normalize(), angle);
                        transform.rotation = rotation * transform.rotation;
                    })?;
                }
                ExistingCommand::SetScale {
                    entity_bits,
                    scale,
                } => {
                    let Some(entity) = Entity::from_bits(entity_bits) else {
                        continue;
                    };

                    Self::modify_transform(world, entity, |transform| {
                        transform.scale = scale;
                    })?;
                }
                ExistingCommand::LookAt {
                    entity_bits,
                    target,
                } => {
                    let Some(entity) = Entity::from_bits(entity_bits) else {
                        continue;
                    };

                    Self::modify_transform(world, entity, |transform| {
                        let direction = (target - transform.translation).normalize();
                        if direction.length_squared() > 0.0 {
                            transform.rotation = glam::Quat::from_rotation_arc(glam::Vec3::NEG_Z, direction);
                        }
                    })?;
                }
                ExistingCommand::SetParent {
                    entity_bits,
                    parent_bits,
                } => {
                    use crate::scene::components::{Parent, Children};

                    let Some(entity) = Entity::from_bits(entity_bits) else {
                        continue;
                    };

                    if world.entity(entity).is_err() {
                        return Err(ComponentError::NoSuchEntity.into());
                    }

                    // Get old parent entity (if any) before any mutable borrows
                    let old_parent_entity = world.get::<&Parent>(entity).ok().map(|p| p.0);

                    // Remove from old parent's children list
                    if let Some(old_parent) = old_parent_entity {
                        if let Ok(mut children) = world.get::<&mut Children>(old_parent) {
                            children.0.retain(|&child| child != entity);
                        }
                    }

                    // Set new parent
                    if let Some(parent_bits_val) = parent_bits {
                        let Some(parent_entity) = Entity::from_bits(parent_bits_val) else {
                            continue;
                        };

                        if world.entity(parent_entity).is_err() {
                            return Err(ComponentError::NoSuchEntity.into());
                        }

                        // Set parent component
                        world.insert_one(entity, Parent(parent_entity))?;

                        // Add to parent's children
                        // Check if parent has Children component first
                        let has_children = world.satisfies::<&Children>(parent_entity).unwrap_or(false);

                        if has_children {
                            // Parent has Children, add this entity
                            if let Ok(mut children) = world.get::<&mut Children>(parent_entity) {
                                if !children.0.contains(&entity) {
                                    children.0.push(entity);
                                }
                            }
                        } else {
                            // Parent doesn't have Children component yet
                            world.insert_one(parent_entity, Children(vec![entity]))?;
                        }
                    } else {
                        // Remove parent (unparent)
                        let _ = world.remove_one::<Parent>(entity);
                    }
                }
                ExistingCommand::SubscribeEvent {
                    entity_bits,
                    event_name,
                    callback_name,
                } => {
                    let Some(entity) = Entity::from_bits(entity_bits) else {
                        continue;
                    };

                    if world.entity(entity).is_err() {
                        return Err(ComponentError::NoSuchEntity.into());
                    }

                    result.event_subscriptions.push(PendingEventSubscription {
                        entity,
                        event_name,
                        callback_name,
                    });
                }
                ExistingCommand::UnsubscribeEvent {
                    entity_bits,
                    event_name,
                } => {
                    let Some(entity) = Entity::from_bits(entity_bits) else {
                        continue;
                    };

                    if world.entity(entity).is_err() {
                        return Err(ComponentError::NoSuchEntity.into());
                    }

                    result.event_unsubscriptions.push(PendingEventUnsubscription {
                        entity,
                        event_name,
                    });
                }
            }
        }

        Ok(result)
    }

    fn modify_transform(
        world: &mut World,
        entity: Entity,
        apply: impl FnOnce(&mut Transform),
    ) -> Result<(), RuneScriptingError> {
        if let Ok(mut transform) = world.get::<&mut TransformComponent>(entity) {
            apply(&mut transform.0);
            return Ok(());
        }

        if world.entity(entity).is_err() {
            return Err(ComponentError::NoSuchEntity.into());
        }

        let mut transform = Transform::default();
        apply(&mut transform);
        world.insert_one(entity, TransformComponent(transform))?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct PendingGltfImport {
    pub parent: Entity,
    pub path: PathBuf,
    pub scale: f32,
}

#[derive(Debug, Clone)]
pub struct PendingEventSubscription {
    pub entity: Entity,
    pub event_name: String,
    pub callback_name: String,
}

#[derive(Debug, Clone)]
pub struct PendingEventUnsubscription {
    pub entity: Entity,
    pub event_name: String,
}

#[derive(Default)]
struct ScriptApplyResult {
    scripts_added: Vec<Entity>,
    gltf_imports: Vec<PendingGltfImport>,
    event_subscriptions: Vec<PendingEventSubscription>,
    event_unsubscriptions: Vec<PendingEventUnsubscription>,
}

fn entity_bits(entity: Entity) -> i64 {
    entity.to_bits().get() as i64
}

/// State that owns the Rune runtime for a scene.
pub struct ScriptingState {
    runtime: RuneScriptingRuntime,
    instances: HashMap<Entity, RuneScriptInstance>,
    pending_gltf_imports: Vec<PendingGltfImport>,
    component_registry: ComponentRegistry,
    event_subscriptions: EventSubscriptions,
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
    }

    /// Run pending scripts for the current frame.
    pub fn update_scripts(&mut self, world: &mut World, dt: f64) -> Result<(), RuneScriptingError> {
        self.retain_instances(world);
        self.process_on_created(world)?;
        if dt != 0.0 {
            self.process_updates(world, dt)?;
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
                        .or_insert_with(Vec::new)
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

            if !any_scripts_added {
                break;
            }
        }

        Ok(())
    }

    fn process_updates(&mut self, world: &mut World, dt: f64) -> Result<(), RuneScriptingError> {
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
                    .or_insert_with(Vec::new)
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
                    let component = match world.get::<&mut RuneScriptComponent>(subscription.entity_id) {
                        Ok(comp) => comp,
                        Err(_) => continue,
                    };

                    if !component.created_called() {
                        continue;
                    }

                    // Get the script instance
                    let instance = match self.ensure_instance(subscription.entity_id, &component) {
                        Ok(inst) => inst,
                        Err(e) => {
                            error!(target: "script", "Failed to get script instance for event dispatch: {}", e);
                            continue;
                        }
                    };

                    // Set up execution context
                    let commands = instance.command_buffer();
                    let event_data = event.data.clone();

                    {
                        let _commands_guard = CommandGuard::enter(commands.clone());
                        let state = Rc::clone(&instance.state_store);
                        let _state_guard = StateGuard::enter(&state);
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
                                    .or_insert_with(Vec::new)
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
        component: &RuneScriptComponent,
    ) -> Result<&mut RuneScriptInstance, RuneScriptingError> {
        let needs_rebuild = !matches!(
            self.instances.get(&entity),
            Some(existing) if existing.source == *component.source()
        );

        if needs_rebuild {
            let script = self.runtime.compile(component.source())?;
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

/// Plugin that configures Rune scripting for the editor.
#[derive(Default)]
pub struct RuneScriptingPlugin {
    script_root: Option<PathBuf>,
}

impl RuneScriptingPlugin {
    /// Create a new plugin.
    pub fn new() -> Self {
        Self { script_root: None }
    }

    /// Configure the base directory for script loading.
    pub fn with_script_root(mut self, path: impl Into<PathBuf>) -> Self {
        self.script_root = Some(path.into());
        self
    }
}

impl Plugin for RuneScriptingPlugin {
    fn build(&self, app: &mut AppBuilder) {
        let script_root = self.script_root.clone();
        if let Some(root) = script_root {
            app.add_startup_system(move |ctx: &mut StartupContext<'_>| {
                ctx.scene
                    .scripting_mut()
                    .runtime_mut()
                    .set_script_root(root.clone());
            });
        }
    }
}

fn script_module() -> Result<Module, RuneScriptingError> {
    let mut module = Module::new();
    module.function_meta(spawn_entity)?;
    module.function_meta(set_name)?;
    module.function_meta(set_translation)?;
    module.function_meta(set_rotation)?;
    module.function_meta(import_gltf)?;
    module.function_meta(set_state)?;
    module.function_meta(get_state)?;
    module.function_meta(try_get_state)?;
    module.function_meta(get_f64)?;
    module.function_meta(set_f64)?;
    module.function_meta(attach_inline_script)?;
    module.function_meta(attach_script_file)?;
    module.function_meta(log_debug)?;
    module.function_meta(log_info)?;
    module.function_meta(log_warn)?;
    module.function_meta(log_error)?;
    // Component access functions
    module.function_meta(get_component)?;
    module.function_meta(set_component)?;
    module.function_meta(add_component)?;
    module.function_meta(remove_component)?;
    module.function_meta(has_component)?;
    module.function_meta(find_entity_by_name)?;
    // Transform manipulation functions
    module.function_meta(translate)?;
    module.function_meta(rotate)?;
    module.function_meta(set_scale)?;
    module.function_meta(look_at)?;
    module.function_meta(get_world_translation)?;
    module.function_meta(get_world_rotation)?;
    // Hierarchy functions
    module.function_meta(set_parent)?;
    module.function_meta(get_parent)?;
    module.function_meta(get_children)?;
    // Input functions
    module.function_meta(is_key_pressed)?;
    module.function_meta(is_key_just_pressed)?;
    module.function_meta(is_key_just_released)?;
    module.function_meta(is_mouse_button_pressed)?;
    module.function_meta(is_mouse_button_just_pressed)?;
    module.function_meta(is_mouse_button_just_released)?;
    module.function_meta(get_mouse_position)?;
    module.function_meta(get_mouse_delta)?;
    module.function_meta(get_mouse_scroll_delta)?;
    // Entity query functions
    module.function_meta(query_entities_with_component)?;
    // Spatial query functions
    module.function_meta(get_entities_in_radius)?;
    module.function_meta(get_nearest_entity)?;
    module.function_meta(get_nearest_entity_with_component)?;
    module.function_meta(get_entities_in_box)?;
    // Event system functions
    module.function_meta(emit_event)?;
    module.function_meta(subscribe_event)?;
    module.function_meta(unsubscribe_event)?;
    Ok(module)
}

#[rune::function]
fn spawn_entity(name: Option<String>) -> VmResult<i64> {
    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut()
            .with(|commands| VmResult::Ok(commands.spawn_entity(name)))
    })
}

#[rune::function]
fn set_name(handle: i64, name: String) -> VmResult<()> {
    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut()
            .with(|commands| commands.set_name(handle, name))
    })
}

#[rune::function]
fn set_translation(handle: i64, x: f64, y: f64, z: f64) -> VmResult<()> {
    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut().with(|commands| {
            commands.set_translation(handle, Vec3::new(x as f32, y as f32, z as f32))
        })
    })
}

#[rune::function]
fn set_rotation(handle: i64, yaw: f64, pitch: f64, roll: f64) -> VmResult<()> {
    let rotation = Quat::from_euler(EulerRot::YXZ, yaw as f32, pitch as f32, roll as f32);
    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut()
            .with(|commands| commands.set_rotation(handle, rotation))
    })
}

#[rune::function]
fn import_gltf(handle: i64, path: String, scale: f64) -> VmResult<()> {
    let scale = scale as f32;
    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut()
            .with(|commands| commands.import_gltf(handle, path, scale))
    })
}

#[rune::function]
fn set_state(handle: i64, key: String, value: Value) -> VmResult<()> {
    with_active_state(move |map| {
        map.insert((handle, key), value);
        VmResult::Ok(())
    })
}

#[rune::function]
fn get_state(handle: i64, key: String, default: Value) -> VmResult<Value> {
    with_active_state(move |map| {
        let entry_key = (handle, key);
        match map.get(&entry_key) {
            Some(value) => VmResult::Ok(value.clone()),
            None => VmResult::Ok(default),
        }
    })
}

#[rune::function]
fn try_get_state(handle: i64, key: String) -> VmResult<Option<Value>> {
    with_active_state(move |map| {
        let entry_key = (handle, key);
        let value = map.get(&entry_key).cloned();
        VmResult::Ok(value)
    })
}

#[rune::function]
fn get_f64(handle: i64, key: String, default: f64) -> VmResult<f64> {
    with_active_state(move |map| {
        let entry_key = (handle, key);
        match map.get(&entry_key) {
            Some(value) => try_result(f64::from_value(value.clone())),
            None => VmResult::Ok(default),
        }
    })
}

#[rune::function]
fn set_f64(handle: i64, key: String, value: f64) -> VmResult<()> {
    with_active_state(move |map| {
        map.insert((handle, key), Value::from(value));
        VmResult::Ok(())
    })
}

#[rune::function(path = attach_inline_script)]
fn attach_inline_script(handle: i64, name: String, source: String) -> VmResult<()> {
    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut()
            .with(|commands| commands.attach_inline_script(handle, name, source))
    })
}

#[rune::function(path = attach_script)]
fn attach_script_file(handle: i64, path: String) -> VmResult<()> {
    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut()
            .with(|commands| commands.attach_file_script(handle, path))
    })
}

#[rune::function]
fn log_debug(message: String) -> VmResult<()> {
    debug!(target: "script", "{message}");
    VmResult::Ok(())
}

#[rune::function]
fn log_info(message: String) -> VmResult<()> {
    info!(target: "script", "{message}");
    VmResult::Ok(())
}

#[rune::function]
fn log_warn(message: String) -> VmResult<()> {
    warn!(target: "script", "{message}");
    VmResult::Ok(())
}

#[rune::function]
fn log_error(message: String) -> VmResult<()> {
    error!(target: "script", "{message}");
    VmResult::Ok(())
}

// ============================================================================
// Component Access Functions
// ============================================================================

/// Get a component from an entity.
///
/// Returns the component value as a Rune object, or None if the entity
/// doesn't have the component or the component type is unknown.
///
/// # Example
/// ```rune
/// let transform = get_component(entity, "TransformComponent");
/// if transform != None {
///     log_info(`Position: ${transform.translation.x}, ${transform.translation.y}, ${transform.translation.z}`);
/// }
/// ```
#[rune::function]
fn get_component(entity_bits: i64, component_name: String) -> VmResult<Option<Value>> {
    with_active_world(|world| {
        with_active_registry(|registry| {
            let entity = match Entity::from_bits(entity_bits as u64) {
                Some(e) => e,
                None => return VmResult::Ok(None),
            };

            match registry.get_component(world, entity, &component_name) {
                Ok(value) => VmResult::Ok(Some(value)),
                Err(ComponentRegistryError::MissingComponent(_)) => VmResult::Ok(None),
                Err(ComponentRegistryError::UnknownComponent(name)) => {
                    warn!(target: "script", "Unknown component type: {}", name);
                    VmResult::Ok(None)
                }
                Err(e) => {
                    error!(target: "script", "Failed to get component: {}", e);
                    VmResult::Ok(None)
                }
            }
        })
    })
}

/// Set a component on an entity.
///
/// If the entity already has the component, it will be updated.
/// If the entity doesn't have the component, it will be added.
///
/// # Example
/// ```rune
/// set_component(entity, "Visible", true);
/// ```
#[rune::function]
fn set_component(entity_bits: i64, component_name: String, value: Value) -> VmResult<()> {
    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut()
            .with(|commands| commands.set_component(entity_bits, component_name, value))
    })
}

/// Add a component to an entity.
///
/// This is an alias for set_component - both functions work the same way.
///
/// # Example
/// ```rune
/// add_component(entity, "PointLight", #{
///     color: [1.0, 0.8, 0.6],
///     intensity: 5.0,
///     range: 10.0
/// });
/// ```
#[rune::function]
fn add_component(entity_bits: i64, component_name: String, value: Value) -> VmResult<()> {
    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut()
            .with(|commands| commands.add_component(entity_bits, component_name, value))
    })
}

/// Remove a component from an entity.
///
/// # Example
/// ```rune
/// remove_component(entity, "RotateAnimation");
/// ```
#[rune::function]
fn remove_component(entity_bits: i64, component_name: String) -> VmResult<()> {
    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut()
            .with(|commands| commands.remove_component(entity_bits, component_name))
    })
}

/// Check if an entity has a component.
///
/// # Example
/// ```rune
/// if has_component(entity, "MeshComponent") {
///     log_info("Entity has a mesh!");
/// }
/// ```
#[rune::function]
fn has_component(entity_bits: i64, component_name: String) -> VmResult<bool> {
    with_active_world(|world| {
        with_active_registry(|registry| {
            let entity = match Entity::from_bits(entity_bits as u64) {
                Some(e) => e,
                None => return VmResult::Ok(false),
            };

            match registry.has_component(world, entity, &component_name) {
                Ok(has) => VmResult::Ok(has),
                Err(ComponentRegistryError::UnknownComponent(name)) => {
                    warn!(target: "script", "Unknown component type: {}", name);
                    VmResult::Ok(false)
                }
                Err(e) => {
                    error!(target: "script", "Failed to check component: {}", e);
                    VmResult::Ok(false)
                }
            }
        })
    })
}

/// Find an entity by name.
///
/// Returns the entity handle if found, or None if no entity with that name exists.
///
/// # Example
/// ```rune
/// let player = find_entity_by_name("Player");
/// if player != None {
///     log_info("Found the player!");
/// }
/// ```
#[rune::function]
fn find_entity_by_name(name: String) -> VmResult<Option<i64>> {
    with_active_world(|world| {
        for (entity, entity_name) in world.query::<&Name>().iter() {
            if entity_name.0 == name {
                return VmResult::Ok(Some(entity_bits(entity)));
            }
        }
        VmResult::Ok(None)
    })
}

// ============================================================================
// Transform Manipulation Functions
// ============================================================================

/// Translate an entity by a delta vector.
///
/// Adds the delta to the entity's current position.
///
/// # Example
/// ```rune
/// translate(entity, 1.0, 0.0, 0.0);  // Move right by 1 unit
/// ```
#[rune::function]
fn translate(entity_bits: i64, x: f64, y: f64, z: f64) -> VmResult<()> {
    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut().with(|commands| {
            commands.translate(entity_bits, Vec3::new(x as f32, y as f32, z as f32))
        })
    })
}

/// Rotate an entity around an axis by an angle in radians.
///
/// # Example
/// ```rune
/// // Rotate 45 degrees around Y axis
/// rotate(entity, 0.0, 1.0, 0.0, 0.785);
/// ```
#[rune::function]
fn rotate(entity_bits: i64, axis_x: f64, axis_y: f64, axis_z: f64, angle: f64) -> VmResult<()> {
    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut().with(|commands| {
            commands.rotate(
                entity_bits,
                Vec3::new(axis_x as f32, axis_y as f32, axis_z as f32),
                angle as f32,
            )
        })
    })
}

/// Set the scale of an entity.
///
/// # Example
/// ```rune
/// set_scale(entity, 2.0, 2.0, 2.0);  // Double the size
/// ```
#[rune::function]
fn set_scale(entity_bits: i64, x: f64, y: f64, z: f64) -> VmResult<()> {
    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut().with(|commands| {
            commands.set_scale(entity_bits, Vec3::new(x as f32, y as f32, z as f32))
        })
    })
}

/// Make an entity look at a target position.
///
/// # Example
/// ```rune
/// look_at(entity, 0.0, 0.0, 10.0);  // Look at point in front
/// ```
#[rune::function]
fn look_at(entity_bits: i64, target_x: f64, target_y: f64, target_z: f64) -> VmResult<()> {
    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut().with(|commands| {
            commands.look_at(
                entity_bits,
                Vec3::new(target_x as f32, target_y as f32, target_z as f32),
            )
        })
    })
}

/// Get the world-space translation of an entity.
///
/// Returns an array [x, y, z] of the entity's world position.
/// Note: Currently returns local translation. World transform will be added later.
///
/// # Example
/// ```rune
/// let pos = get_world_translation(entity);
/// if pos != None {
///     log_info("Got position");
/// }
/// ```
#[rune::function]
fn get_world_translation(entity_bits: i64) -> VmResult<Option<rune::alloc::Vec<f64>>> {
    with_active_world(|world| {
        let entity = match Entity::from_bits(entity_bits as u64) {
            Some(e) => e,
            None => return VmResult::Ok(None),
        };

        // Get local transform for now
        if let Ok(transform) = world.get::<&TransformComponent>(entity) {
            let translation = transform.0.translation;
            let mut vec = rune::alloc::Vec::new();
            if let Err(e) = vec.try_push(translation.x as f64) {
                return VmResult::Err(e.into());
            }
            if let Err(e) = vec.try_push(translation.y as f64) {
                return VmResult::Err(e.into());
            }
            if let Err(e) = vec.try_push(translation.z as f64) {
                return VmResult::Err(e.into());
            }
            return VmResult::Ok(Some(vec));
        }

        VmResult::Ok(None)
    })
}

/// Get the world-space rotation of an entity as euler angles.
///
/// Returns an array [yaw, pitch, roll] in radians.
/// Note: Currently returns local rotation. World transform will be added later.
///
/// # Example
/// ```rune
/// let rot = get_world_rotation(entity);
/// if rot != None {
///     log_info("Got rotation");
/// }
/// ```
#[rune::function]
fn get_world_rotation(entity_bits: i64) -> VmResult<Option<rune::alloc::Vec<f64>>> {
    use glam::EulerRot;

    with_active_world(|world| {
        let entity = match Entity::from_bits(entity_bits as u64) {
            Some(e) => e,
            None => return VmResult::Ok(None),
        };

        // Get local transform for now
        if let Ok(transform) = world.get::<&TransformComponent>(entity) {
            let (yaw, pitch, roll) = transform.0.rotation.to_euler(EulerRot::YXZ);
            let mut vec = rune::alloc::Vec::new();
            if let Err(e) = vec.try_push(yaw as f64) {
                return VmResult::Err(e.into());
            }
            if let Err(e) = vec.try_push(pitch as f64) {
                return VmResult::Err(e.into());
            }
            if let Err(e) = vec.try_push(roll as f64) {
                return VmResult::Err(e.into());
            }
            return VmResult::Ok(Some(vec));
        }

        VmResult::Ok(None)
    })
}

// ============================================================================
// Hierarchy Functions
// ============================================================================

/// Set the parent of an entity.
///
/// Pass None to unparent the entity.
///
/// # Example
/// ```rune
/// let parent = find_entity_by_name("ParentObject");
/// // Note: Can't easily unwrap Option in Rune yet
/// set_parent(child_entity, parent);
/// ```
#[rune::function]
fn set_parent(entity_bits: i64, parent_bits: Option<i64>) -> VmResult<()> {
    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut()
            .with(|commands| commands.set_parent(entity_bits, parent_bits))
    })
}

/// Get the parent of an entity.
///
/// Returns the entity handle of the parent, or None if no parent.
///
/// # Example
/// ```rune
/// let parent = get_parent(entity);
/// if parent != None {
///     log_info("Has parent");
/// }
/// ```
#[rune::function]
fn get_parent(entity_handle: i64) -> VmResult<Option<i64>> {
    use crate::scene::components::Parent;

    with_active_world(|world| {
        let entity = match Entity::from_bits(entity_handle as u64) {
            Some(e) => e,
            None => return VmResult::Ok(None),
        };

        if let Ok(parent) = world.get::<&Parent>(entity) {
            return VmResult::Ok(Some(entity_bits(parent.0)));
        }

        VmResult::Ok(None)
    })
}

/// Get the children of an entity.
///
/// Returns an array of entity handles.
///
/// # Example
/// ```rune
/// let children = get_children(entity);
/// if children != None {
///     log_info("Has children");
/// }
/// ```
#[rune::function]
fn get_children(entity_handle: i64) -> VmResult<Option<rune::alloc::Vec<i64>>> {
    use crate::scene::components::Children;

    with_active_world(|world| {
        let entity = match Entity::from_bits(entity_handle as u64) {
            Some(e) => e,
            None => return VmResult::Ok(None),
        };

        if let Ok(children) = world.get::<&Children>(entity) {
            let mut vec = rune::alloc::Vec::new();
            for &child in &children.0 {
                if let Err(e) = vec.try_push(entity_bits(child)) {
                    return VmResult::Err(e.into());
                }
            }
            return VmResult::Ok(Some(vec));
        }

        VmResult::Ok(None)
    })
}

// ============================================================================
// INPUT FUNCTIONS
// ============================================================================

/// Check if a keyboard key is currently pressed (down).
///
/// # Arguments
/// * `key` - The key name as a string (e.g., "W", "Space", "Escape", "A", "D", "S")
///
/// Returns `true` if the key is currently held down, `false` otherwise.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     if is_key_pressed("W") {
///         translate(self_entity, 0.0, 0.0, -5.0 * dt);
///     }
/// }
/// ```
#[rune::function]
fn is_key_pressed(key: String) -> VmResult<bool> {
    with_active_input_state(|input_state| {
        VmResult::Ok(input_state.is_key_pressed(&key))
    })
}

/// Check if a keyboard key was just pressed this frame.
///
/// # Arguments
/// * `key` - The key name as a string
///
/// Returns `true` only on the frame the key transitions from up to down.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     if is_key_just_pressed("Space") {
///         log_info("Jump!");
///     }
/// }
/// ```
#[rune::function]
fn is_key_just_pressed(key: String) -> VmResult<bool> {
    with_active_input_state(|input_state| {
        VmResult::Ok(input_state.is_key_just_pressed(&key))
    })
}

/// Check if a keyboard key was just released this frame.
///
/// # Arguments
/// * `key` - The key name as a string
///
/// Returns `true` only on the frame the key transitions from down to up.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     if is_key_just_released("W") {
///         log_info("Stopped moving forward");
///     }
/// }
/// ```
#[rune::function]
fn is_key_just_released(key: String) -> VmResult<bool> {
    with_active_input_state(|input_state| {
        VmResult::Ok(input_state.is_key_just_released(&key))
    })
}

/// Check if a mouse button is currently pressed (down).
///
/// # Arguments
/// * `button` - The mouse button index (0 = left, 1 = right, 2 = middle)
///
/// Returns `true` if the button is currently held down, `false` otherwise.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     if is_mouse_button_pressed(0) {
///         log_info("Left mouse button is down");
///     }
/// }
/// ```
#[rune::function]
fn is_mouse_button_pressed(button: i64) -> VmResult<bool> {
    with_active_input_state(|input_state| {
        VmResult::Ok(input_state.is_mouse_button_pressed(button as u32))
    })
}

/// Check if a mouse button was just pressed this frame.
///
/// # Arguments
/// * `button` - The mouse button index (0 = left, 1 = right, 2 = middle)
///
/// Returns `true` only on the frame the button transitions from up to down.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     if is_mouse_button_just_pressed(0) {
///         log_info("Click!");
///     }
/// }
/// ```
#[rune::function]
fn is_mouse_button_just_pressed(button: i64) -> VmResult<bool> {
    with_active_input_state(|input_state| {
        VmResult::Ok(input_state.is_mouse_button_just_pressed(button as u32))
    })
}

/// Check if a mouse button was just released this frame.
///
/// # Arguments
/// * `button` - The mouse button index (0 = left, 1 = right, 2 = middle)
///
/// Returns `true` only on the frame the button transitions from down to up.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     if is_mouse_button_just_released(0) {
///         log_info("Released left click");
///     }
/// }
/// ```
#[rune::function]
fn is_mouse_button_just_released(button: i64) -> VmResult<bool> {
    with_active_input_state(|input_state| {
        VmResult::Ok(input_state.is_mouse_button_just_released(button as u32))
    })
}

/// Get the current mouse position.
///
/// Returns an array `[x, y]` with the mouse position in screen coordinates.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     let pos = get_mouse_position();
///     log_info(`Mouse at ${pos[0]}, ${pos[1]}`);
/// }
/// ```
#[rune::function]
fn get_mouse_position() -> VmResult<rune::alloc::Vec<f64>> {
    with_active_input_state(|input_state| {
        let pos = input_state.mouse_position();
        let mut vec = rune::alloc::Vec::new();
        if let Err(e) = vec.try_push(pos.x as f64) {
            return VmResult::Err(e.into());
        }
        if let Err(e) = vec.try_push(pos.y as f64) {
            return VmResult::Err(e.into());
        }
        VmResult::Ok(vec)
    })
}

/// Get the mouse movement delta for this frame.
///
/// Returns an array `[dx, dy]` with the mouse movement since last frame.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     let delta = get_mouse_delta();
///     if delta[0].abs() > 0.0 || delta[1].abs() > 0.0 {
///         log_info(`Mouse moved by ${delta[0]}, ${delta[1]}`);
///     }
/// }
/// ```
#[rune::function]
fn get_mouse_delta() -> VmResult<rune::alloc::Vec<f64>> {
    with_active_input_state(|input_state| {
        let delta = input_state.mouse_delta();
        let mut vec = rune::alloc::Vec::new();
        if let Err(e) = vec.try_push(delta.x as f64) {
            return VmResult::Err(e.into());
        }
        if let Err(e) = vec.try_push(delta.y as f64) {
            return VmResult::Err(e.into());
        }
        VmResult::Ok(vec)
    })
}

/// Get the mouse scroll delta for this frame.
///
/// Returns an array `[dx, dy]` with the scroll amount. Typically `dy` is used for vertical scrolling.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     let scroll = get_mouse_scroll_delta();
///     if scroll[1] > 0.0 {
///         log_info("Scrolled up");
///     } else if scroll[1] < 0.0 {
///         log_info("Scrolled down");
///     }
/// }
/// ```
#[rune::function]
fn get_mouse_scroll_delta() -> VmResult<rune::alloc::Vec<f64>> {
    with_active_input_state(|input_state| {
        let scroll = input_state.scroll_delta();
        let mut vec = rune::alloc::Vec::new();
        if let Err(e) = vec.try_push(scroll.x as f64) {
            return VmResult::Err(e.into());
        }
        if let Err(e) = vec.try_push(scroll.y as f64) {
            return VmResult::Err(e.into());
        }
        VmResult::Ok(vec)
    })
}

// ============================================================================
// ENTITY QUERY FUNCTIONS
// ============================================================================

/// Query all entities that have a specific component.
///
/// # Arguments
/// * `component_name` - The name of the component to search for (e.g., "MeshComponent", "CameraComponent")
///
/// Returns an array of entity handles that have the specified component.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     // Find all entities with cameras
///     let cameras = query_entities_with_component("CameraComponent");
///     log_info(`Found ${cameras.len()} cameras`);
///
///     // Find all entities with meshes
///     let meshes = query_entities_with_component("MeshComponent");
///     for entity in meshes {
///         // Do something with each mesh entity
///     }
/// }
/// ```
#[rune::function]
fn query_entities_with_component(component_name: String) -> VmResult<rune::alloc::Vec<i64>> {
    with_active_world(|world| {
        with_active_registry(|registry| {
            let mut result = rune::alloc::Vec::new();

            // Check if component exists in registry
            if !registry.is_registered(&component_name) {
                return VmResult::Ok(result);
            }

            // Iterate all entities and check if they have the component
            for entity_ref in world.iter() {
                let entity = entity_ref.entity();
                match registry.has_component(world, entity, &component_name) {
                    Ok(true) => {
                        let handle = entity_bits(entity);
                        if let Err(e) = result.try_push(handle) {
                            return VmResult::Err(e.into());
                        }
                    }
                    Ok(false) => {}
                    Err(_) => {}
                }
            }

            VmResult::Ok(result)
        })
    })
}

// ============================================================================
// SPATIAL QUERY FUNCTIONS
// ============================================================================

/// Find all entities within a radius of a point.
///
/// # Arguments
/// * `x`, `y`, `z` - The center position
/// * `radius` - The search radius
///
/// Returns an array of entity handles within the radius.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     // Find all entities within 10 units
///     let nearby = get_entities_in_radius(0.0, 0.0, 0.0, 10.0);
///     log_info(`Found ${nearby.len()} nearby entities`);
///
///     for entity in nearby {
///         // Do something with nearby entities
///     }
/// }
/// ```
#[rune::function]
fn get_entities_in_radius(x: f64, y: f64, z: f64, radius: f64) -> VmResult<rune::alloc::Vec<i64>> {
    with_active_world(|world| {
        let center = Vec3::new(x as f32, y as f32, z as f32);
        let radius_sq = (radius * radius) as f32;
        let mut result = rune::alloc::Vec::new();

        // Query all entities with TransformComponent
        for (entity, transform) in world.query::<&TransformComponent>().iter() {
            let pos = transform.0.translation;
            let dist_sq = center.distance_squared(pos);

            if dist_sq <= radius_sq {
                let handle = entity_bits(entity);
                if let Err(e) = result.try_push(handle) {
                    return VmResult::Err(e.into());
                }
            }
        }

        VmResult::Ok(result)
    })
}

/// Find the nearest entity to a point.
///
/// # Arguments
/// * `x`, `y`, `z` - The position to search from
///
/// Returns the entity handle of the nearest entity, or `None` if no entities found.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     let pos = get_world_translation(self_entity);
///     if pos != None {
///         let nearest = get_nearest_entity(pos[0], pos[1], pos[2]);
///         if nearest != None {
///             log_info(`Nearest entity: ${nearest}`);
///         }
///     }
/// }
/// ```
#[rune::function]
fn get_nearest_entity(x: f64, y: f64, z: f64) -> VmResult<Option<i64>> {
    with_active_world(|world| {
        let pos = Vec3::new(x as f32, y as f32, z as f32);
        let mut nearest: Option<(Entity, f32)> = None;

        // Query all entities with TransformComponent
        for (entity, transform) in world.query::<&TransformComponent>().iter() {
            let entity_pos = transform.0.translation;
            let dist_sq = pos.distance_squared(entity_pos);

            match nearest {
                None => nearest = Some((entity, dist_sq)),
                Some((_, best_dist_sq)) => {
                    if dist_sq < best_dist_sq {
                        nearest = Some((entity, dist_sq));
                    }
                }
            }
        }

        match nearest {
            Some((entity, _)) => VmResult::Ok(Some(entity_bits(entity))),
            None => VmResult::Ok(None),
        }
    })
}

/// Find the nearest entity with a specific component.
///
/// # Arguments
/// * `x`, `y`, `z` - The position to search from
/// * `component_name` - The component to filter by
///
/// Returns the entity handle of the nearest entity with the component, or `None` if not found.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     // Find nearest enemy
///     let nearest_enemy = get_nearest_entity_with_component(0.0, 0.0, 0.0, "EnemyTag");
///     if nearest_enemy != None {
///         log_info(`Nearest enemy found!`);
///     }
/// }
/// ```
#[rune::function]
fn get_nearest_entity_with_component(
    x: f64,
    y: f64,
    z: f64,
    component_name: String,
) -> VmResult<Option<i64>> {
    with_active_world(|world| {
        with_active_registry(|registry| {
            let pos = Vec3::new(x as f32, y as f32, z as f32);
            let mut nearest: Option<(Entity, f32)> = None;

            // Check if component exists in registry
            if !registry.is_registered(&component_name) {
                return VmResult::Ok(None);
            }

            // Query all entities with TransformComponent
            for (entity, transform) in world.query::<&TransformComponent>().iter() {
                // Check if entity has the required component
                match registry.has_component(world, entity, &component_name) {
                    Ok(true) => {
                        let entity_pos = transform.0.translation;
                        let dist_sq = pos.distance_squared(entity_pos);

                        match nearest {
                            None => nearest = Some((entity, dist_sq)),
                            Some((_, best_dist_sq)) => {
                                if dist_sq < best_dist_sq {
                                    nearest = Some((entity, dist_sq));
                                }
                            }
                        }
                    }
                    _ => continue,
                }
            }

            match nearest {
                Some((entity, _)) => VmResult::Ok(Some(entity_bits(entity))),
                None => VmResult::Ok(None),
            }
        })
    })
}

/// Find all entities within an axis-aligned bounding box.
///
/// # Arguments
/// * `min` - Minimum corner of the box as `[x, y, z]`
/// * `max` - Maximum corner of the box as `[x, y, z]`
///
/// Returns an array of entity handles within the box.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     // Find entities in a region
///     let min = [-10.0, 0.0, -10.0];
///     let max = [10.0, 5.0, 10.0];
///     let entities = get_entities_in_box(min, max);
///     log_info(`Found ${entities.len()} entities in box`);
/// }
/// ```
#[rune::function]
fn get_entities_in_box(
    min: rune::alloc::Vec<f64>,
    max: rune::alloc::Vec<f64>,
) -> VmResult<rune::alloc::Vec<i64>> {
    with_active_world(|world| {
        // Validate array sizes
        if min.len() != 3 || max.len() != 3 {
            return VmResult::panic("min and max must be arrays of length 3");
        }

        let min_vec = Vec3::new(min[0] as f32, min[1] as f32, min[2] as f32);
        let max_vec = Vec3::new(max[0] as f32, max[1] as f32, max[2] as f32);
        let mut result = rune::alloc::Vec::new();

        // Query all entities with TransformComponent
        for (entity, transform) in world.query::<&TransformComponent>().iter() {
            let pos = transform.0.translation;

            // Check if position is within bounds
            if pos.x >= min_vec.x && pos.x <= max_vec.x
                && pos.y >= min_vec.y && pos.y <= max_vec.y
                && pos.z >= min_vec.z && pos.z <= max_vec.z
            {
                let handle = entity_bits(entity);
                if let Err(e) = result.try_push(handle) {
                    return VmResult::Err(e.into());
                }
            }
        }

        VmResult::Ok(result)
    })
}

// ============================================================================
// EVENT SYSTEM FUNCTIONS
// ============================================================================

/// Emit an event that can be received by subscribed scripts.
///
/// # Arguments
/// * `event_name` - The name of the event to emit
/// * `data` - The event data (can be any Rune value: string, number, object, etc.)
///
/// Events are queued during script execution and dispatched after all scripts
/// have finished updating. Subscribed scripts will have their registered callback
/// function called with the event data.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     if is_key_just_pressed("Space") {
///         // Emit an event when space is pressed
///         emit_event("player_jumped", #{
///             entity: self_entity,
///             height: 5.0,
///             timestamp: 12345.0
///         });
///     }
/// }
/// ```
#[rune::function]
fn emit_event(event_name: String, data: Value) -> VmResult<()> {
    with_active_event_queue(|queue| {
        queue.push(ScriptEvent {
            name: event_name,
            data,
        });
        VmResult::Ok(())
    })
}

/// Subscribe to an event with a callback function.
///
/// # Arguments
/// * `event_name` - The name of the event to subscribe to
/// * `callback_name` - The name of the function to call when the event is received
///
/// The callback function must accept one parameter: the event data.
/// When an event with the matching name is emitted, the callback will be called
/// with the event data.
///
/// # Example
/// ```rune
/// pub fn on_created(self_entity) {
///     // Subscribe to player_jumped event
///     subscribe_event("player_jumped", "on_player_jumped");
/// }
///
/// pub fn on_player_jumped(event_data) {
///     log_info(`Player jumped! Height: ${event_data.height}`);
/// }
/// ```
#[rune::function]
fn subscribe_event(event_name: String, callback_name: String) -> VmResult<()> {
    // Get the current entity from the active context
    let entity_bits = match get_active_entity() {
        VmResult::Ok(bits) => bits,
        VmResult::Err(err) => return VmResult::Err(err),
    };

    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut().with(|commands| {
            commands.subscribe_event(entity_bits, event_name, callback_name)
        })
    })
}

/// Unsubscribe from an event.
///
/// # Arguments
/// * `event_name` - The name of the event to unsubscribe from
///
/// Removes the subscription for the current entity from the specified event.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     if is_key_just_pressed("U") {
///         unsubscribe_event("player_jumped");
///         log_info("Unsubscribed from player_jumped event");
///     }
/// }
/// ```
#[rune::function]
fn unsubscribe_event(event_name: String) -> VmResult<()> {
    // Get the current entity from the active context
    let entity_bits = match get_active_entity() {
        VmResult::Ok(bits) => bits,
        VmResult::Err(err) => return VmResult::Err(err),
    };

    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut()
            .with(|commands| commands.unsubscribe_event(entity_bits, event_name))
    })
}
