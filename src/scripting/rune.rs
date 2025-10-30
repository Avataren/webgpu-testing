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
use crate::scene::{Name, Transform, TransformComponent};

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
    ) -> Result<FunctionCallOutcome, RuneScriptingError> {
        let _commands_guard = CommandGuard::enter(commands.clone());
        let state = Rc::clone(&self.state_store);
        let _state_guard = StateGuard::enter(&state);
        self.call_function(["on_created"], (entity_bits,))
    }

    fn call_update(
        &mut self,
        entity_bits: i64,
        dt: f64,
        commands: Rc<RefCell<ScriptCommands>>,
    ) -> Result<FunctionCallOutcome, RuneScriptingError> {
        let _commands_guard = CommandGuard::enter(commands.clone());
        let state = Rc::clone(&self.state_store);
        let _state_guard = StateGuard::enter(&state);
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

    fn is_empty(&self) -> bool {
        self.pending.is_empty() && self.existing.is_empty()
    }

    fn apply(&mut self, world: &mut World) -> Result<ScriptApplyResult, RuneScriptingError> {
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

#[derive(Default)]
struct ScriptApplyResult {
    scripts_added: Vec<Entity>,
    gltf_imports: Vec<PendingGltfImport>,
}

fn entity_bits(entity: Entity) -> i64 {
    entity.to_bits().get() as i64
}

/// State that owns the Rune runtime for a scene.
pub struct ScriptingState {
    runtime: RuneScriptingRuntime,
    instances: HashMap<Entity, RuneScriptInstance>,
    pending_gltf_imports: Vec<PendingGltfImport>,
}

impl ScriptingState {
    /// Construct a new scripting state.
    pub fn new() -> Result<Self, RuneScriptingError> {
        Ok(Self {
            runtime: RuneScriptingRuntime::new()?,
            instances: HashMap::new(),
            pending_gltf_imports: Vec::new(),
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

            {
                let mut query = world.query::<&mut RuneScriptComponent>();
                for (entity, component) in query.iter() {
                    if component.created_called() {
                        continue;
                    }

                    let instance = self.ensure_instance(entity, component)?;
                    let commands = instance.command_buffer();
                    instance.call_on_created(entity_bits(entity), commands.clone())?;
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
                let result = borrow.apply(world)?;
                if !result.scripts_added.is_empty() {
                    any_scripts_added = true;
                }
                if !result.gltf_imports.is_empty() {
                    self.pending_gltf_imports
                        .extend(result.gltf_imports.into_iter());
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

        {
            let mut query = world.query::<&mut RuneScriptComponent>();
            for (entity, component) in query.iter() {
                if !component.created_called() {
                    continue;
                }

                let instance = self.ensure_instance(entity, component)?;
                let commands = instance.command_buffer();
                instance.call_update(entity_bits(entity), dt, commands.clone())?;

                if !commands.borrow().is_empty() {
                    pending_commands.push(commands.clone());
                }
            }
        }

        if pending_commands.is_empty() {
            return Ok(());
        }

        for commands in pending_commands.iter_mut() {
            let mut borrow = commands.borrow_mut();
            let result = borrow.apply(world)?;
            if !result.gltf_imports.is_empty() {
                self.pending_gltf_imports
                    .extend(result.gltf_imports.into_iter());
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
