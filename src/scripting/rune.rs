use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::{anyhow, Context as AnyhowContext, Result};
use glam::Vec3;
use hecs::{Entity, World};
use rune::runtime::RuntimeContext;
use rune::support;
use rune::termcolor::Buffer;
use rune::Unit;
use rune::{self, Diagnostics, Hash, Module, Sources, Vm};
use rune_macros::Any;

use crate::app::{AppBuilder, Plugin, StartupContext, UpdateContext};
use crate::scene::components::{Name, ScriptComponent, TransformComponent, Visible};
use crate::scene::{Scene, Transform};

#[derive(Clone)]
pub struct RuneScriptingPlugin {
    runtime: Option<Rc<RefCell<RuneScriptRuntimeInner>>>,
}

#[derive(Clone)]
pub struct RuneScriptingHandle {
    inner: Rc<RefCell<RuneScriptRuntimeInner>>,
}

impl RuneScriptingPlugin {
    pub fn new() -> Result<Self> {
        let runtime =
            RuneScriptRuntimeInner::new().context("failed to initialize Rune scripting runtime")?;
        Ok(Self {
            runtime: Some(Rc::new(RefCell::new(runtime))),
        })
    }

    pub fn disabled() -> Self {
        Self { runtime: None }
    }

    pub fn handle(&self) -> Option<RuneScriptingHandle> {
        self.runtime.as_ref().map(|inner| RuneScriptingHandle {
            inner: inner.clone(),
        })
    }
}

impl Plugin for RuneScriptingPlugin {
    fn build(&self, builder: &mut AppBuilder) {
        let Some(runtime) = &self.runtime else {
            return;
        };

        let startup_runtime = runtime.clone();
        builder.add_startup_system(move |ctx: &mut StartupContext<'_>| {
            if let Err(err) = startup_runtime.borrow_mut().initialize_scene(ctx.scene) {
                log::error!("Failed to initialize Rune scripts: {err:?}");
            }
        });

        let update_runtime = runtime.clone();
        builder.add_system(move |ctx: &mut UpdateContext<'_>| {
            if let Err(err) = update_runtime.borrow_mut().update_scene(ctx.scene, ctx.dt) {
                log::error!("Rune script update failed: {err:?}");
            }
        });
    }
}

impl RuneScriptingHandle {
    pub fn load_script_from_source(
        &self,
        name: impl Into<String>,
        source: impl Into<String>,
    ) -> Result<()> {
        self.inner
            .borrow_mut()
            .load_script_from_source(name.into(), source.into())
    }

    pub fn load_script_from_file(
        &self,
        name: impl Into<String>,
        path: impl AsRef<Path>,
    ) -> Result<()> {
        let contents = fs::read_to_string(&path).with_context(|| {
            format!(
                "failed to read Rune script from {}",
                path.as_ref().display()
            )
        })?;
        self.load_script_from_source(name.into(), contents)
    }
}

#[derive(Any, Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ScriptEntity {
    id: u64,
}

impl ScriptEntity {
    fn from_entity(entity: Entity) -> Self {
        Self {
            id: entity.to_bits().get(),
        }
    }

    fn to_entity(self) -> support::Result<Entity> {
        let id = std::num::NonZeroU64::new(self.id)
            .ok_or_else(|| support::Error::msg("invalid entity id"))?;
        Entity::from_bits(id.get()).ok_or_else(|| support::Error::msg("invalid entity id"))
    }
}

#[derive(Clone, Copy)]
struct ActiveContext {
    runtime: *mut RuneScriptRuntimeInner,
    scene: *mut Scene,
    world: *mut World,
    entity: Entity,
    dt: f64,
}

thread_local! {
    static ACTIVE_CONTEXT: RefCell<Option<ActiveContext>> = const { RefCell::new(None) };
}

struct RuneScriptRuntimeInner {
    context: rune::Context,
    runtime: Arc<RuntimeContext>,
    scripts: HashMap<String, Arc<ScriptDefinition>>,
    instances: HashMap<Entity, EntityScript>,
}

struct ScriptDefinition {
    unit: Arc<Unit>,
    on_created: Option<Hash>,
    update: Option<Hash>,
}

struct EntityScript {
    script_name: String,
    definition: Arc<ScriptDefinition>,
    instance: ScriptInstance,
    ran_on_created: bool,
}

struct ScriptInstance {
    vm: Vm,
}

impl RuneScriptRuntimeInner {
    fn new() -> Result<Self> {
        let mut context =
            rune_modules::default_context().context("failed to build default Rune context")?;
        context
            .install(create_engine_module().context("failed to install engine module")?)
            .context("failed to install engine module into Rune context")?;
        let runtime = Arc::new(context.runtime().context("failed to build Rune runtime")?);

        Ok(Self {
            context,
            runtime,
            scripts: HashMap::new(),
            instances: HashMap::new(),
        })
    }

    fn initialize_scene(&mut self, scene: &mut Scene) -> Result<()> {
        self.update_scene(scene, 0.0)
    }

    fn update_scene(&mut self, scene: &mut Scene, dt: f64) -> Result<()> {
        {
            let world = scene.main_world_mut();
            self.sync_instances(world)?;
        }
        self.run_scripts(scene, dt)
    }

    fn load_script_from_source(&mut self, name: String, source: String) -> Result<()> {
        let mut sources = Sources::new();
        sources.insert(
            rune::Source::new(name.clone(), source).context("failed to create Rune source")?,
        )?;

        let mut diagnostics = Diagnostics::new();
        let result = rune::prepare(&mut sources)
            .with_context(&self.context)
            .with_diagnostics(&mut diagnostics)
            .build();

        if !diagnostics.is_empty() {
            let mut buffer = Buffer::no_color();
            diagnostics
                .emit(&mut buffer, &sources)
                .context("failed to format Rune diagnostics")?;
            let message = String::from_utf8_lossy(buffer.as_slice());
            log::warn!("Rune diagnostics for '{}':\n{}", name, message);
        }

        let unit = result.context("failed to compile Rune script")?;
        let unit = Arc::new(unit);

        let vm = Vm::new(self.runtime.clone(), unit.clone());
        let on_created = vm
            .lookup_function(["on_created"])
            .ok()
            .map(|f| f.type_hash());
        let update = vm.lookup_function(["update"]).ok().map(|f| f.type_hash());

        let definition = Arc::new(ScriptDefinition {
            unit,
            on_created,
            update,
        });

        self.scripts.insert(name.clone(), definition.clone());

        for entry in self.instances.values_mut() {
            if entry.script_name == name {
                entry.definition = definition.clone();
                entry.instance = ScriptInstance::new(self.runtime.clone(), definition.clone())?;
                entry.ran_on_created = false;
            }
        }

        Ok(())
    }

    fn sync_instances(&mut self, world: &mut World) -> Result<()> {
        let mut seen = HashSet::new();
        let mut to_add = Vec::new();

        {
            let mut query = world.query::<&ScriptComponent>();
            for (entity, script) in query.iter() {
                seen.insert(entity);
                match self.instances.get(&entity) {
                    Some(existing) if existing.script_name == script.script => {}
                    _ => {
                        to_add.push((entity, script.script.clone()));
                    }
                }
            }
        }

        self.instances.retain(|entity, _| seen.contains(entity));

        for (entity, script_name) in to_add {
            if let Some(definition) = self.scripts.get(&script_name).cloned() {
                match ScriptInstance::new(self.runtime.clone(), definition.clone()) {
                    Ok(instance) => {
                        self.instances.insert(
                            entity,
                            EntityScript {
                                script_name,
                                definition,
                                instance,
                                ran_on_created: false,
                            },
                        );
                    }
                    Err(err) => {
                        log::error!(
                            "Failed to instantiate Rune script '{}' for entity {:?}: {err:?}",
                            script_name,
                            entity
                        );
                    }
                }
            } else {
                log::warn!(
                    "Entity {:?} references unknown Rune script '{}'",
                    entity,
                    script_name
                );
            }
        }

        Ok(())
    }

    fn run_scripts(&mut self, scene: &mut Scene, dt: f64) -> Result<()> {
        let runtime_ptr = self as *mut _;
        let scene_ptr = scene as *mut _;
        let entities: Vec<Entity> = self.instances.keys().copied().collect();

        let world = scene.main_world_mut();
        let world_ptr = world as *mut _;

        for entity in &entities {
            if let Some(entry) = self.instances.get_mut(entity) {
                if !entry.ran_on_created {
                    if let Err(err) =
                        entry.ensure_on_created(runtime_ptr, scene_ptr, world_ptr, *entity)
                    {
                        log::error!("Rune on_created failed for entity {:?}: {err:?}", entity);
                    }
                }
            }
        }

        for entity in entities {
            if let Some(entry) = self.instances.get_mut(&entity) {
                if let Err(err) = entry.update(runtime_ptr, scene_ptr, world_ptr, entity, dt) {
                    log::error!("Rune update failed for entity {:?}: {err:?}", entity);
                }
            }
        }

        Ok(())
    }

    fn attach_script_to_entity(
        &mut self,
        world: &mut World,
        entity: Entity,
        script_name: &str,
    ) -> Result<()> {
        let definition = self
            .scripts
            .get(script_name)
            .cloned()
            .ok_or_else(|| anyhow!("script '{}' has not been loaded", script_name))?;

        world
            .insert_one(entity, ScriptComponent::new(script_name))
            .with_context(|| format!("failed to attach script component to entity {:?}", entity))?;

        let instance = ScriptInstance::new(self.runtime.clone(), definition.clone())?;
        self.instances.insert(
            entity,
            EntityScript {
                script_name: script_name.to_string(),
                definition,
                instance,
                ran_on_created: false,
            },
        );
        Ok(())
    }
}

impl EntityScript {
    fn ensure_on_created(
        &mut self,
        runtime: *mut RuneScriptRuntimeInner,
        scene: *mut Scene,
        world: *mut World,
        entity: Entity,
    ) -> Result<()> {
        if self.ran_on_created {
            return Ok(());
        }

        self.instance
            .call_on_created(runtime, scene, world, entity, self.definition.on_created)?;
        self.ran_on_created = true;
        Ok(())
    }

    fn update(
        &mut self,
        runtime: *mut RuneScriptRuntimeInner,
        scene: *mut Scene,
        world: *mut World,
        entity: Entity,
        dt: f64,
    ) -> Result<()> {
        self.instance
            .call_update(runtime, scene, world, entity, dt, self.definition.update)
    }
}

impl ScriptInstance {
    fn new(runtime: Arc<RuntimeContext>, definition: Arc<ScriptDefinition>) -> Result<Self> {
        let vm = Vm::new(runtime, definition.unit.clone());
        Ok(Self { vm })
    }

    fn call_on_created(
        &mut self,
        runtime: *mut RuneScriptRuntimeInner,
        scene: *mut Scene,
        world: *mut World,
        entity: Entity,
        on_created: Option<Hash>,
    ) -> Result<()> {
        if let Some(hash) = on_created {
            with_script_context(runtime, scene, world, entity, 0.0, || {
                self.vm
                    .call(hash, (ScriptEntity::from_entity(entity),))
                    .context("Rune on_created call failed")?;
                Ok(())
            })?;
        }
        Ok(())
    }

    fn call_update(
        &mut self,
        runtime: *mut RuneScriptRuntimeInner,
        scene: *mut Scene,
        world: *mut World,
        entity: Entity,
        dt: f64,
        update: Option<Hash>,
    ) -> Result<()> {
        if let Some(hash) = update {
            with_script_context(runtime, scene, world, entity, dt, || {
                self.vm
                    .call(hash, (ScriptEntity::from_entity(entity), dt))
                    .context("Rune update call failed")?;
                Ok(())
            })?;
        }
        Ok(())
    }
}

fn with_script_context<F, R>(
    runtime: *mut RuneScriptRuntimeInner,
    scene: *mut Scene,
    world: *mut World,
    entity: Entity,
    dt: f64,
    f: F,
) -> Result<R>
where
    F: FnOnce() -> Result<R>,
{
    ACTIVE_CONTEXT.with(|cell| {
        {
            let mut slot = cell.borrow_mut();
            if slot.is_some() {
                return Err(anyhow!("another script context is already active"));
            }
            *slot = Some(ActiveContext {
                runtime,
                scene,
                world,
                entity,
                dt,
            });
        }

        let result = f();

        let mut slot = cell.borrow_mut();
        *slot = None;

        result
    })
}

fn with_active_context<F, R>(f: F) -> support::Result<R>
where
    F: FnOnce(
        &mut RuneScriptRuntimeInner,
        &mut Scene,
        &mut World,
        Entity,
        f64,
    ) -> support::Result<R>,
{
    ACTIVE_CONTEXT.with(|cell| {
        let ctx = cell
            .borrow()
            .as_ref()
            .copied()
            .ok_or_else(|| support::Error::msg("no active Rune script context"))?;

        let runtime = unsafe { &mut *ctx.runtime };
        let scene = unsafe { &mut *ctx.scene };
        let world = unsafe { &mut *ctx.world };

        f(runtime, scene, world, ctx.entity, ctx.dt)
    })
}

fn create_engine_module() -> Result<Module, rune::ContextError> {
    let mut module = Module::with_crate("engine")?;
    module.ty::<ScriptEntity>()?;
    module.function(["spawn_entity"], spawn_entity).build()?;
    module
        .function(["current_entity"], current_entity)
        .build()?;
    module.function(["set_name"], set_name).build()?;
    module
        .function(["set_translation"], set_translation)
        .build()?;
    module.function(["set_visible"], set_visible).build()?;
    module.function(["attach_script"], attach_script).build()?;
    module.function(["scene_time"], scene_time).build()?;
    module.function(["delta_time"], delta_time).build()?;
    Ok(module)
}

fn spawn_entity() -> support::Result<ScriptEntity> {
    with_active_context(|_runtime, _scene, world, _entity, _dt| {
        let entity = world.spawn((TransformComponent(Transform::default()), Visible::default()));
        Ok(ScriptEntity::from_entity(entity))
    })
}

fn current_entity() -> support::Result<ScriptEntity> {
    with_active_context(|_runtime, _scene, _world, entity, _dt| {
        Ok(ScriptEntity::from_entity(entity))
    })
}

fn set_name(entity: ScriptEntity, name: String) -> support::Result<()> {
    with_active_context(|_runtime, _scene, world, _ctx_entity, _dt| {
        let target = entity
            .to_entity()
            .map_err(|err| support::Error::msg(err.to_string()))?;
        if let Ok(mut existing) = world.get::<&mut Name>(target) {
            existing.0 = name;
            return Ok(());
        }
        world
            .insert_one(target, Name(name))
            .map_err(|err| support::Error::msg(err.to_string()))?;
        Ok(())
    })
}

fn set_translation(entity: ScriptEntity, x: f32, y: f32, z: f32) -> support::Result<()> {
    with_active_context(|_runtime, _scene, world, _ctx_entity, _dt| {
        let target = entity
            .to_entity()
            .map_err(|err| support::Error::msg(err.to_string()))?;
        let translation = Vec3::new(x, y, z);
        if let Ok(mut transform) = world.get::<&mut TransformComponent>(target) {
            transform.0.translation = translation;
            return Ok(());
        }
        world
            .insert_one(
                target,
                TransformComponent(Transform::from_translation(translation)),
            )
            .map_err(|err| support::Error::msg(err.to_string()))?;
        Ok(())
    })
}

fn set_visible(entity: ScriptEntity, visible: bool) -> support::Result<()> {
    with_active_context(|_runtime, _scene, world, _ctx_entity, _dt| {
        let target = entity
            .to_entity()
            .map_err(|err| support::Error::msg(err.to_string()))?;
        if let Ok(mut visibility) = world.get::<&mut Visible>(target) {
            visibility.0 = visible;
            return Ok(());
        }
        world
            .insert_one(target, Visible(visible))
            .map_err(|err| support::Error::msg(err.to_string()))?;
        Ok(())
    })
}

fn attach_script(entity: ScriptEntity, script: String) -> support::Result<()> {
    with_active_context(|runtime, _scene, world, _ctx_entity, _dt| {
        let target = entity
            .to_entity()
            .map_err(|err| support::Error::msg(err.to_string()))?;
        runtime.attach_script_to_entity(world, target, &script)?;
        Ok(())
    })
}

fn scene_time() -> support::Result<f64> {
    with_active_context(|_runtime, scene, _world, _entity, _dt| Ok(scene.time()))
}

fn delta_time() -> support::Result<f64> {
    with_active_context(|_runtime, _scene, _world, _entity, dt| Ok(dt))
}
