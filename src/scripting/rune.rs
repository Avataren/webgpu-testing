use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use glam::Vec3;
use hecs::Entity;
use rune::runtime::{RuntimeContext, VmError, VmResult};
use rune::{Context, Module, Source, Sources, Vm};

use crate::app::{AppBuilder, Plugin, StartupContext, UpdateContext};
use crate::scene::{Name, Scene, Transform, TransformComponent, Visible};

/// Component that attaches a Rune script to an entity.
#[derive(Debug, Clone)]
pub struct RuneScriptComponent {
    pub script: String,
}

impl RuneScriptComponent {
    pub fn new(script: impl Into<String>) -> Self {
        Self {
            script: script.into(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct RuneScriptLibrary {
    scripts: HashMap<String, String>,
}

impl RuneScriptLibrary {
    pub fn new() -> Self {
        Self {
            scripts: HashMap::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.scripts.is_empty()
    }

    pub fn add_script(&mut self, name: impl Into<String>, source: impl Into<String>) {
        self.scripts.insert(name.into(), source.into());
    }

    pub fn from_directory(path: impl AsRef<Path>) -> Result<Self, RuneScriptingError> {
        let mut library = Self::new();
        let dir = path.as_ref();
        if !dir.exists() {
            return Ok(library);
        }

        for entry in fs::read_dir(dir).map_err(|error| RuneScriptingError::io(dir, error))? {
            let entry = entry.map_err(|error| RuneScriptingError::io(dir, error))?;
            let file_type = entry
                .file_type()
                .map_err(|error| RuneScriptingError::io(entry.path(), error))?;
            if !file_type.is_file() {
                continue;
            }

            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("rn") {
                continue;
            }

            let name = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| path.to_string_lossy().into_owned());
            let source =
                fs::read_to_string(&path).map_err(|error| RuneScriptingError::io(&path, error))?;
            library.add_script(name, source);
        }

        Ok(library)
    }

    fn compile_all(
        &self,
        context: &Context,
    ) -> Result<HashMap<String, Arc<rune::Unit>>, RuneScriptingError> {
        let mut compiled = HashMap::new();
        for (name, source) in &self.scripts {
            let unit = compile_script(name, source, context)?;
            compiled.insert(name.clone(), Arc::new(unit));
        }
        Ok(compiled)
    }
}

#[derive(Debug)]
pub enum RuneScriptingError {
    Io {
        path: PathBuf,
        error: std::io::Error,
    },
    Context(rune::ContextError),
    Compile {
        script: String,
        message: String,
    },
    Allocation(rune::alloc::Error),
}

impl RuneScriptingError {
    fn io(path: impl Into<PathBuf>, error: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            error,
        }
    }
}

impl std::fmt::Display for RuneScriptingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, error } => write!(f, "{}: {}", path.display(), error),
            Self::Context(err) => write!(f, "{err}"),
            Self::Compile { script, message } => write!(f, "{script}: {message}"),
            Self::Allocation(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for RuneScriptingError {}

impl From<rune::ContextError> for RuneScriptingError {
    fn from(value: rune::ContextError) -> Self {
        Self::Context(value)
    }
}

impl From<rune::alloc::Error> for RuneScriptingError {
    fn from(value: rune::alloc::Error) -> Self {
        Self::Allocation(value)
    }
}

#[derive(Clone)]
pub struct RuneScriptingPlugin {
    source: RuneLibrarySource,
}

#[derive(Clone)]
enum RuneLibrarySource {
    Directory(PathBuf),
    Library(RuneScriptLibrary),
}

impl RuneScriptingPlugin {
    pub fn from_directory(path: impl Into<PathBuf>) -> Self {
        Self {
            source: RuneLibrarySource::Directory(path.into()),
        }
    }

    pub fn with_library(library: RuneScriptLibrary) -> Self {
        Self {
            source: RuneLibrarySource::Library(library),
        }
    }
}

impl Plugin for RuneScriptingPlugin {
    fn build(&self, app: &mut AppBuilder) {
        let library = match &self.source {
            RuneLibrarySource::Directory(path) => match RuneScriptLibrary::from_directory(path) {
                Ok(library) => library,
                Err(error) => {
                    log::error!("Failed to load Rune scripts: {error}");
                    return;
                }
            },
            RuneLibrarySource::Library(library) => library.clone(),
        };

        if library.is_empty() {
            log::info!("Rune scripting plugin initialized without scripts");
            return;
        }

        let runtime = match RuneRuntime::new(library) {
            Ok(runtime) => Rc::new(RefCell::new(runtime)),
            Err(error) => {
                log::error!("Failed to initialize Rune runtime: {error}");
                return;
            }
        };

        let startup_runtime = runtime.clone();
        app.add_startup_system(move |ctx| {
            if let Err(error) = startup_runtime.borrow_mut().startup(ctx) {
                log::error!("Rune startup failed: {error}");
            }
        });

        let update_runtime = runtime.clone();
        app.add_system(move |ctx| {
            if let Err(error) = update_runtime.borrow_mut().update(ctx) {
                log::error!("Rune update failed: {error}");
            }
        });
    }
}

struct RuneRuntime {
    runtime: Arc<RuntimeContext>,
    host: Arc<Mutex<ScriptHostState>>,
    scripts: HashMap<String, Arc<rune::Unit>>,
    instances: HashMap<Entity, ScriptInstance>,
}

impl RuneRuntime {
    fn new(library: RuneScriptLibrary) -> Result<Self, RuneScriptingError> {
        let host = Arc::new(Mutex::new(ScriptHostState::default()));

        let mut context = Context::new();
        context.install(build_host_module(host.clone())?)?;

        let compiled = library.compile_all(&context)?;
        let runtime = Arc::new(context.runtime()?);

        Ok(Self {
            runtime,
            host,
            scripts: compiled,
            instances: HashMap::new(),
        })
    }

    fn startup(&mut self, ctx: &mut StartupContext) -> Result<(), RuneScriptingError> {
        self.process_commands(ctx.scene);
        Ok(())
    }

    fn update(&mut self, ctx: &mut UpdateContext) -> Result<(), RuneScriptingError> {
        self.process_commands(ctx.scene);
        self.sync_instances(ctx.scene);
        let absolute_time = ctx.scene.time();
        self.run_instances(ctx.scene, ctx.dt, absolute_time);
        Ok(())
    }

    fn process_commands(&mut self, scene: &mut Scene) {
        let mut host = self.host.lock().expect("Rune host mutex poisoned");
        let commands = host.take_commands();
        drop(host);

        if commands.is_empty() {
            return;
        }

        let mut spawns: HashMap<u64, SpawnBuild> = HashMap::new();
        let mut entity_ops: Vec<EntityCommand> = Vec::new();

        for command in commands {
            match command {
                ScriptCommand::SpawnEntity { handle, name } => {
                    let entry = spawns.entry(handle).or_default();
                    entry.spawn_requested = true;
                    entry.name = name;
                }
                ScriptCommand::SetSpawnTransform { handle, transform } => {
                    spawns.entry(handle).or_default().transform = Some(transform);
                }
                ScriptCommand::SetSpawnVisible { handle, visible } => {
                    spawns.entry(handle).or_default().visible = Some(visible);
                }
                ScriptCommand::AttachSpawnScript { handle, script } => {
                    spawns.entry(handle).or_default().script = Some(script);
                }
                ScriptCommand::SetEntityTransform {
                    entity_bits,
                    transform,
                } => {
                    entity_ops.push(EntityCommand::SetTransform {
                        entity_bits,
                        transform,
                    });
                }
            }
        }

        let world = scene.main_world_mut();
        for (_handle, build) in spawns {
            if !build.spawn_requested {
                log::warn!("Rune script issued spawn modifiers without spawn_entity call");
                continue;
            }
            let mut builder = hecs::EntityBuilder::new();
            if let Some(name) = build.name {
                builder.add(Name::new(name));
            }
            if let Some(transform) = build.transform {
                builder.add(TransformComponent(transform));
            }
            if let Some(visible) = build.visible {
                builder.add(Visible(visible));
            }
            if let Some(script) = build.script {
                builder.add(RuneScriptComponent::new(script));
            }

            let _ = world.spawn(builder.build());
        }

        for command in entity_ops {
            match command {
                EntityCommand::SetTransform {
                    entity_bits,
                    transform,
                } => match Entity::from_bits(entity_bits) {
                    Some(entity) => {
                        if world.contains(entity) {
                            match world.query_one_mut::<&mut TransformComponent>(entity) {
                                Ok(existing) => {
                                    existing.0 = transform;
                                }
                                Err(_) => {
                                    let _ = world.insert(entity, (TransformComponent(transform),));
                                }
                            }
                        } else {
                            log::warn!("Rune command targeted unknown entity {entity_bits}");
                        }
                    }
                    None => {
                        log::warn!("Rune command referenced invalid entity bits {entity_bits}");
                    }
                },
            }
        }
    }

    fn sync_instances(&mut self, scene: &mut Scene) {
        let world = scene.main_world_mut();

        self.instances.retain(|entity, instance| {
            if !world.contains(*entity) {
                return false;
            }

            match world.get::<&RuneScriptComponent>(*entity) {
                Ok(component) => component.script == instance.script_name,
                Err(_) => false,
            }
        });

        let mut query = world.query::<&RuneScriptComponent>();
        for (entity, component) in query.iter() {
            if let Some(instance) = self.instances.get_mut(&entity) {
                instance.entity_bits = entity.to_bits().get();
                continue;
            }

            let Some(unit) = self.scripts.get(&component.script) else {
                log::error!("Unknown Rune script '{}'", component.script);
                continue;
            };

            let vm = Vm::new(self.runtime.clone(), unit.clone());
            self.instances.insert(
                entity,
                ScriptInstance {
                    script_name: component.script.clone(),
                    entity_bits: entity.to_bits().get(),
                    vm,
                    initialized: false,
                },
            );
        }
    }

    fn run_instances(&mut self, scene: &Scene, dt: f64, time: f64) {
        let mut finished = Vec::new();

        for (entity, instance) in &mut self.instances {
            if !scene.world().contains(*entity) {
                finished.push(*entity);
                continue;
            }

            if !instance.initialized {
                if let Err(error) = instance
                    .vm
                    .call(["on_created"], (instance.entity_bits as i64,))
                {
                    if !is_missing_function(&error) {
                        log::error!(
                            "Rune on_created failed for {}: {error}",
                            instance.script_name
                        );
                    }
                }
                instance.initialized = true;
            }

            if let Err(error) = instance
                .vm
                .call(["update"], (instance.entity_bits as i64, dt as f64, time))
            {
                if !is_missing_function(&error) {
                    log::error!("Rune update failed for {}: {error}", instance.script_name);
                }
            }
        }

        for entity in finished {
            self.instances.remove(&entity);
        }
    }
}

struct ScriptInstance {
    script_name: String,
    entity_bits: u64,
    vm: Vm,
    initialized: bool,
}

#[derive(Default)]
struct ScriptHostState {
    next_handle: u64,
    commands: Vec<ScriptCommand>,
}

impl ScriptHostState {
    fn spawn_entity(&mut self, name: Option<String>) -> i64 {
        self.next_handle += 1;
        let handle = self.next_handle;
        self.commands
            .push(ScriptCommand::SpawnEntity { handle, name });
        handle as i64
    }

    fn set_spawn_transform(&mut self, handle: i64, transform: Transform) {
        if handle >= 0 {
            self.commands.push(ScriptCommand::SetSpawnTransform {
                handle: handle as u64,
                transform,
            });
        }
    }

    fn set_spawn_visible(&mut self, handle: i64, visible: bool) {
        if handle >= 0 {
            self.commands.push(ScriptCommand::SetSpawnVisible {
                handle: handle as u64,
                visible,
            });
        }
    }

    fn attach_spawn_script(&mut self, handle: i64, script: String) {
        if handle >= 0 {
            self.commands.push(ScriptCommand::AttachSpawnScript {
                handle: handle as u64,
                script,
            });
        }
    }

    fn set_entity_transform(&mut self, entity_bits: i64, transform: Transform) {
        if entity_bits >= 0 {
            self.commands.push(ScriptCommand::SetEntityTransform {
                entity_bits: entity_bits as u64,
                transform,
            });
        }
    }

    fn take_commands(&mut self) -> Vec<ScriptCommand> {
        std::mem::take(&mut self.commands)
    }
}

struct SpawnBuild {
    spawn_requested: bool,
    name: Option<String>,
    transform: Option<Transform>,
    visible: Option<bool>,
    script: Option<String>,
}

impl Default for SpawnBuild {
    fn default() -> Self {
        Self {
            spawn_requested: false,
            name: None,
            transform: None,
            visible: None,
            script: None,
        }
    }
}

enum ScriptCommand {
    SpawnEntity {
        handle: u64,
        name: Option<String>,
    },
    SetSpawnTransform {
        handle: u64,
        transform: Transform,
    },
    SetSpawnVisible {
        handle: u64,
        visible: bool,
    },
    AttachSpawnScript {
        handle: u64,
        script: String,
    },
    SetEntityTransform {
        entity_bits: u64,
        transform: Transform,
    },
}

enum EntityCommand {
    SetTransform {
        entity_bits: u64,
        transform: Transform,
    },
}

fn compile_script(
    name: &str,
    source: &str,
    context: &Context,
) -> Result<rune::Unit, RuneScriptingError> {
    let mut sources = Sources::new();
    let source = Source::new(name, source)?;
    let _ = sources.insert(source)?;

    rune::prepare(&mut sources)
        .with_context(context)
        .build()
        .map_err(|error| RuneScriptingError::Compile {
            script: name.to_string(),
            message: error.to_string(),
        })
}

fn is_missing_function(error: &VmError) -> bool {
    error.to_string().contains("Missing function")
}

fn build_host_module(host: Arc<Mutex<ScriptHostState>>) -> Result<Module, RuneScriptingError> {
    let mut module = Module::with_item(["engine"])?;

    let spawn_host = host.clone();
    module
        .function(
            "spawn_entity",
            move |name: Option<String>| -> VmResult<i64> {
                let mut host = spawn_host.lock().expect("Rune host mutex poisoned");
                VmResult::Ok(host.spawn_entity(name))
            },
        )
        .build()?;

    let transform_host = host.clone();
    module
        .function(
            "set_spawn_translation",
            move |handle: i64, x: f32, y: f32, z: f32| -> VmResult<()> {
                let mut host = transform_host.lock().expect("Rune host mutex poisoned");
                host.set_spawn_transform(handle, Transform::from_translation(Vec3::new(x, y, z)));
                VmResult::Ok(())
            },
        )
        .build()?;

    let visible_host = host.clone();
    module
        .function(
            "set_spawn_visible",
            move |handle: i64, visible: bool| -> VmResult<()> {
                let mut host = visible_host.lock().expect("Rune host mutex poisoned");
                host.set_spawn_visible(handle, visible);
                VmResult::Ok(())
            },
        )
        .build()?;

    let script_host = host.clone();
    module
        .function(
            "attach_spawn_script",
            move |handle: i64, script: String| -> VmResult<()> {
                let mut host = script_host.lock().expect("Rune host mutex poisoned");
                host.attach_spawn_script(handle, script);
                VmResult::Ok(())
            },
        )
        .build()?;

    let entity_transform_host = host.clone();
    module
        .function(
            "set_entity_translation",
            move |entity_bits: i64, x: f32, y: f32, z: f32| -> VmResult<()> {
                let mut host = entity_transform_host
                    .lock()
                    .expect("Rune host mutex poisoned");
                host.set_entity_transform(
                    entity_bits,
                    Transform::from_translation(Vec3::new(x, y, z)),
                );
                VmResult::Ok(())
            },
        )
        .build()?;

    Ok(module)
}
