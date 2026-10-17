use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use glam::{Quat, Vec3};
use hecs::{ComponentError, Entity, World};
use log::warn;
use serde_json::json;

use crate::scene::transform::{look_rotation, LookAtTarget};
use crate::scene::{
    CameraComponent, CanCastShadow, DirectionalLight, MaterialComponent, MeshComponent, Name,
    OrbitAnimation, Parent, ParticleEmitterComponent, PointLight, PrimitiveMeshComponent,
    RotateAnimation, SpotLight, Transform, TransformComponent, Visible,
};

use super::component::LuaScriptComponent;
use super::entity_registry::{
    EntityHandleRegistry, ExistingCommand, PendingAnimation, PendingEntity,
};
use super::error::LuaScriptingError;
use super::types::LuaScriptSource;

pub(crate) struct ScriptCommands {
    pub registry: Rc<RefCell<EntityHandleRegistry>>,
    pub pending: HashMap<i64, PendingEntity>,
    pub existing: Vec<ExistingCommand>,
    /// Mesh, material, light and camera changes by entity handle; they need the scene
    /// or the renderer, so they are handed to the scene rather than applied to the world.
    pub assets: Vec<(i64, AssetCommand)>,
    /// `emit_particles` requests, handed to the scene's particle runtime.
    pub particle_bursts: Vec<crate::gpu_particles::ParticleBurst>,
    /// `set_postprocess` patches: the volume's script handle (`None` for the scene's
    /// infinite volume) and the settings to change.
    pub post_process: Vec<(Option<i64>, serde_json::Value)>,
    /// `set_environment` patches, already checked.
    pub environment: Vec<serde_json::Value>,
    /// Velocity, impulse, character and spring camera changes for the scene's physics.
    pub physics: Vec<crate::physics::PhysicsCommand>,
    /// Destinations, stops and warps for the scene's navigation agents.
    #[cfg(feature = "navigation")]
    pub navigation: Vec<crate::navigation::NavCommand>,
    /// `animation_tree_generate` requests (entity handles), for the scene.
    pub generate_trees: Vec<i64>,
}

impl ScriptCommands {
    pub fn new(registry: Rc<RefCell<EntityHandleRegistry>>) -> Self {
        Self {
            registry,
            pending: HashMap::new(),
            existing: Vec::new(),
            assets: Vec::new(),
            particle_bursts: Vec::new(),
            post_process: Vec::new(),
            environment: Vec::new(),
            physics: Vec::new(),
            #[cfg(feature = "navigation")]
            navigation: Vec::new(),
            generate_trees: Vec::new(),
        }
    }

    /// Queue a change to the scene's environment (ambient light, background, sky).
    pub fn environment(&mut self, patch: serde_json::Value) {
        self.environment.push(patch);
    }

    /// Queue a post-processing change on the volume `handle` names, or on the scene's
    /// infinite volume when `handle` is `None`.
    pub fn post_process(
        &mut self,
        handle: Option<i64>,
        patch: serde_json::Value,
    ) -> Result<(), mlua::Error> {
        if let Some(handle) = handle {
            self.resolve_or_pending(handle)?;
        }
        self.post_process.push((handle, patch));
        Ok(())
    }

    /// Queue a mesh, material, light or camera change. Works for entities spawned this
    /// frame too.
    pub fn asset_command(&mut self, handle: i64, command: AssetCommand) -> Result<(), mlua::Error> {
        if !self.pending.contains_key(&handle) {
            self.resolve_entity_bits(handle)?;
        }
        self.assets.push((handle, command));
        Ok(())
    }

    /// Check that `handle` names a live entity or one spawned earlier in this chunk.
    pub fn resolve_or_pending(&self, handle: i64) -> Result<(), mlua::Error> {
        if !self.pending.contains_key(&handle) {
            self.resolve_entity_bits(handle)?;
        }
        Ok(())
    }

    pub fn spawn_entity(&mut self, name: Option<String>) -> i64 {
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

    pub fn resolve_entity_bits(&self, handle: i64) -> Result<u64, mlua::Error> {
        {
            let registry = self.registry.borrow();
            if let Some(bits) = registry.resolved_bits(handle) {
                return Ok(bits);
            }

            if registry.contains(handle) {
                return Err(mlua::Error::RuntimeError(
                    "entity handle is not yet available".into(),
                ));
            }
        }

        if handle == 0 {
            return Err(mlua::Error::RuntimeError("invalid entity handle".into()));
        }

        Ok(handle as u64)
    }

    pub fn set_name(&mut self, handle: i64, name: String) -> Result<(), mlua::Error> {
        if let Some(entry) = self.pending.get_mut(&handle) {
            entry.name = Some(name);
            return Ok(());
        }

        let entity_bits = self.resolve_entity_bits(handle)?;
        self.existing
            .push(ExistingCommand::SetName { entity_bits, name });
        Ok(())
    }

    pub fn set_translation(&mut self, handle: i64, position: Vec3) -> Result<(), mlua::Error> {
        if let Some(entry) = self.pending.get_mut(&handle) {
            entry.translation = Some(position);
            return Ok(());
        }

        let entity_bits = self.resolve_entity_bits(handle)?;
        self.existing.push(ExistingCommand::SetTranslation {
            entity_bits,
            position,
        });
        Ok(())
    }

    pub fn set_rotation(&mut self, handle: i64, rotation: Quat) -> Result<(), mlua::Error> {
        if let Some(entry) = self.pending.get_mut(&handle) {
            entry.rotation = Some(rotation);
            return Ok(());
        }

        let entity_bits = self.resolve_entity_bits(handle)?;
        self.existing.push(ExistingCommand::SetRotation {
            entity_bits,
            rotation,
        });
        Ok(())
    }

    pub fn attach_inline_script(
        &mut self,
        handle: i64,
        name: String,
        source: String,
    ) -> Result<(), mlua::Error> {
        let descriptor = LuaScriptSource::inline(name, source);
        if let Some(entry) = self.pending.get_mut(&handle) {
            entry.scripts.push(descriptor);
            return Ok(());
        }

        let entity_bits = self.resolve_entity_bits(handle)?;
        self.existing.push(ExistingCommand::AttachScript {
            entity_bits,
            source: descriptor,
        });
        Ok(())
    }

    pub fn attach_file_script(&mut self, handle: i64, path: String) -> Result<(), mlua::Error> {
        let descriptor = LuaScriptSource::file(path);
        if let Some(entry) = self.pending.get_mut(&handle) {
            entry.scripts.push(descriptor);
            return Ok(());
        }

        let entity_bits = self.resolve_entity_bits(handle)?;
        self.existing.push(ExistingCommand::AttachScript {
            entity_bits,
            source: descriptor,
        });
        Ok(())
    }

    pub fn import_gltf(
        &mut self,
        handle: i64,
        path: String,
        scale: f32,
    ) -> Result<(), mlua::Error> {
        let entity_bits = self.resolve_entity_bits(handle)?;

        self.existing.push(ExistingCommand::ImportGltf {
            entity_bits,
            path,
            scale,
        });
        Ok(())
    }

    pub fn set_component(
        &mut self,
        handle: i64,
        component_name: String,
        value: serde_json::Value,
    ) -> Result<(), mlua::Error> {
        // An entity spawned in this chunk has no world entity yet: keep the value with the
        // pending entity, exactly as `add_component` does (setting and adding are the same
        // operation).
        if let Some(entry) = self.pending.get_mut(&handle) {
            entry.components.insert(component_name, value);
            return Ok(());
        }

        let entity_bits = self.resolve_entity_bits(handle)?;

        self.existing.push(ExistingCommand::SetComponent {
            entity_bits,
            component_name,
            value,
        });
        Ok(())
    }

    pub fn add_component(
        &mut self,
        handle: i64,
        component_name: String,
        value: serde_json::Value,
    ) -> Result<(), mlua::Error> {
        // Check if this is a pending entity first
        if let Some(entry) = self.pending.get_mut(&handle) {
            entry.components.insert(component_name, value);
            return Ok(());
        }

        let entity_bits = self.resolve_entity_bits(handle)?;

        self.existing.push(ExistingCommand::AddComponent {
            entity_bits,
            component_name,
            value,
        });
        Ok(())
    }

    pub fn remove_component(
        &mut self,
        handle: i64,
        component_name: String,
    ) -> Result<(), mlua::Error> {
        // For an entity spawned in this chunk, drop any component queued for it so far.
        if let Some(entry) = self.pending.get_mut(&handle) {
            entry.components.remove(&component_name);
            return Ok(());
        }

        let entity_bits = self.resolve_entity_bits(handle)?;

        self.existing.push(ExistingCommand::RemoveComponent {
            entity_bits,
            component_name,
        });
        Ok(())
    }

    pub fn translate(&mut self, handle: i64, delta: Vec3) -> Result<(), mlua::Error> {
        if let Some(entry) = self.pending.get_mut(&handle) {
            entry.translation = Some(entry.translation.unwrap_or(Vec3::ZERO) + delta);
            return Ok(());
        }

        let entity_bits = self.resolve_entity_bits(handle)?;

        self.existing
            .push(ExistingCommand::Translate { entity_bits, delta });
        Ok(())
    }

    pub fn rotate(&mut self, handle: i64, axis: Vec3, angle: f32) -> Result<(), mlua::Error> {
        if let Some(entry) = self.pending.get_mut(&handle) {
            let rotation = Quat::from_axis_angle(axis.normalize(), angle);
            entry.rotation = Some(rotation * entry.rotation.unwrap_or(Quat::IDENTITY));
            return Ok(());
        }

        let entity_bits = self.resolve_entity_bits(handle)?;

        self.existing.push(ExistingCommand::Rotate {
            entity_bits,
            axis,
            angle,
        });
        Ok(())
    }

    pub fn set_scale(&mut self, handle: i64, scale: Vec3) -> Result<(), mlua::Error> {
        // Check if this is a pending entity first
        if let Some(entry) = self.pending.get_mut(&handle) {
            entry.scale = Some(scale);
            return Ok(());
        }

        let entity_bits = self.resolve_entity_bits(handle)?;

        self.existing
            .push(ExistingCommand::SetScale { entity_bits, scale });
        Ok(())
    }

    pub fn look_at(&mut self, handle: i64, target: Vec3) -> Result<(), mlua::Error> {
        if let Some(entry) = self.pending.get_mut(&handle) {
            if let Some(rotation) = look_rotation(entry.translation.unwrap_or(Vec3::ZERO), target) {
                entry.rotation = Some(rotation);
                entry.look_at_target = Some(target);
            }
            return Ok(());
        }

        let entity_bits = self.resolve_entity_bits(handle)?;

        self.existing.push(ExistingCommand::LookAt {
            entity_bits,
            target,
        });
        Ok(())
    }

    pub fn set_parent(
        &mut self,
        handle: i64,
        parent_handle: Option<i64>,
    ) -> Result<(), mlua::Error> {
        // If the child is a pending entity, store the parent handle to be resolved later
        if let Some(entry) = self.pending.get_mut(&handle) {
            entry.parent = parent_handle;
            entry.parent_set = true;
            return Ok(());
        }

        let entity_bits = self.resolve_entity_bits(handle)?;

        let parent_bits = if let Some(parent) = parent_handle {
            Some(self.resolve_entity_bits(parent)?)
        } else {
            None
        };

        self.existing.push(ExistingCommand::SetParent {
            entity_bits,
            parent_bits,
        });
        Ok(())
    }

    /// Queue a copy of `source` and everything below it (`clone_entity`). The copy is a
    /// sibling of the source until `set_parent` moves it; its name, transform, components
    /// and animations can be set in the same chunk, like a spawned entity's.
    pub fn clone_entity(&mut self, source: i64, name: Option<String>) -> Result<i64, mlua::Error> {
        let source_bits = self.resolve_entity_bits(source)?;
        let id = self.registry.borrow_mut().allocate();
        self.pending.insert(
            id,
            PendingEntity {
                name,
                clone_of: Some(source_bits),
                ..PendingEntity::default()
            },
        );
        Ok(id)
    }

    /// Queue an animator change (`animator_play` and friends).
    pub fn animator(
        &mut self,
        handle: i64,
        command: crate::scene::skeletal::AnimatorCommand,
    ) -> Result<(), mlua::Error> {
        if let Some(entry) = self.pending.get_mut(&handle) {
            entry.animation.push(PendingAnimation::Animator(command));
            return Ok(());
        }
        let entity_bits = self.resolve_entity_bits(handle)?;
        self.existing.push(ExistingCommand::Animator {
            entity_bits,
            command,
        });
        Ok(())
    }

    /// Queue an animation tree change (`animation_set` and friends).
    pub fn animation_tree(
        &mut self,
        handle: i64,
        command: crate::scene::skeletal::TreeCommand,
    ) -> Result<(), mlua::Error> {
        if let Some(entry) = self.pending.get_mut(&handle) {
            entry.animation.push(PendingAnimation::Tree(command));
            return Ok(());
        }
        let entity_bits = self.resolve_entity_bits(handle)?;
        self.existing.push(ExistingCommand::AnimationTree {
            entity_bits,
            command,
        });
        Ok(())
    }

    /// Queue the removal of an entity and its descendants. An entity spawned earlier in
    /// the same chunk is simply never created.
    pub fn despawn(&mut self, handle: i64) -> Result<(), mlua::Error> {
        if self.pending.remove(&handle).is_some() {
            self.assets.retain(|(target, _)| *target != handle);
            self.post_process
                .retain(|(target, _)| *target != Some(handle));
            return Ok(());
        }

        let entity_bits = self.resolve_entity_bits(handle)?;
        self.existing.push(ExistingCommand::Despawn { entity_bits });
        Ok(())
    }

    pub fn subscribe_event(
        &mut self,
        handle: i64,
        event_name: String,
        callback_name: String,
    ) -> Result<(), mlua::Error> {
        let entity_bits = self.resolve_entity_bits(handle)?;

        self.existing.push(ExistingCommand::SubscribeEvent {
            entity_bits,
            event_name,
            callback_name,
        });
        Ok(())
    }

    pub fn unsubscribe_event(
        &mut self,
        handle: i64,
        event_name: String,
    ) -> Result<(), mlua::Error> {
        let entity_bits = self.resolve_entity_bits(handle)?;

        self.existing.push(ExistingCommand::UnsubscribeEvent {
            entity_bits,
            event_name,
        });
        Ok(())
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
            && self.existing.is_empty()
            && self.assets.is_empty()
            && self.post_process.is_empty()
            && self.environment.is_empty()
            && self.physics.is_empty()
    }

    pub fn apply(&mut self, world: &mut World) -> Result<ScriptApplyResult, LuaScriptingError> {
        use log::error;

        let mut result = ScriptApplyResult {
            particle_bursts: std::mem::take(&mut self.particle_bursts),
            physics: std::mem::take(&mut self.physics),
            #[cfg(feature = "navigation")]
            navigation: std::mem::take(&mut self.navigation),
            ..ScriptApplyResult::default()
        };
        let mut deferred_parents: Vec<(Entity, i64)> = Vec::new();

        // First pass: spawn all entities and collect parent relationships
        for (handle, mut pending) in self.pending.drain() {
            let source = pending
                .clone_of
                .and_then(Entity::from_bits)
                .filter(|source| world.contains(*source));
            if pending.clone_of.is_some() && source.is_none() {
                warn!(target: "script", "clone_entity: the entity to copy no longer exists; spawned an empty one");
            }
            let entity = match source {
                Some(source) => {
                    let pairs = crate::scene::duplicate::duplicate_subtree(world, source, |m| m);
                    for &(_, copy) in &pairs {
                        if world.get::<&LuaScriptComponent>(copy).is_ok() {
                            result.scripts_added.push(copy);
                        }
                    }
                    pairs[0].1
                }
                None => world.spawn(()),
            };
            self.registry.borrow_mut().resolve(handle, entity);

            if let Some(name) = pending.name {
                world.insert_one(entity, Name(name))?;
            }

            // Every spawned entity gets a transform, even one the script never moves: an
            // entity without one is skipped by transform propagation, and so is everything
            // parented below it (a bare `spawn_entity` group node would strand its children).
            // A clone starts from its source's.
            let mut transform = world
                .get::<&TransformComponent>(entity)
                .map(|t| t.0)
                .unwrap_or_default();
            if let Some(translation) = pending.translation.take() {
                transform.translation = translation;
            }
            if let Some(rotation) = pending.rotation.take() {
                transform.rotation = rotation;
            }
            if let Some(scale) = pending.scale.take() {
                transform.scale = scale;
            }
            world.insert_one(entity, TransformComponent(transform))?;
            if let Some(target) = pending.look_at_target {
                world.insert_one(entity, LookAtTarget(target))?;
            }

            // Attach all scripts to this entity
            for script in pending.scripts {
                world.insert_one(entity, LuaScriptComponent::new(script))?;
                result.scripts_added.push(entity);
            }

            // Add pending components to the entity
            for (component_name, value) in pending.components {
                if let Err(e) =
                    Self::add_component_from_json(world, entity, &component_name, &value)
                {
                    warn!(target: "script", "Failed to add component '{}': {}", component_name, e);
                }
            }

            let bits = entity.to_bits().get();
            for command in pending.animation {
                self.existing.push(match command {
                    PendingAnimation::Animator(command) => ExistingCommand::Animator {
                        entity_bits: bits,
                        command,
                    },
                    PendingAnimation::Tree(command) => ExistingCommand::AnimationTree {
                        entity_bits: bits,
                        command,
                    },
                });
            }

            // A clone moved elsewhere leaves its source's parent first.
            if pending.parent_set {
                detach_from_parent(world, entity);
            }
            // Defer parent setting until all entities are spawned
            if let Some(parent_handle) = pending.parent {
                deferred_parents.push((entity, parent_handle));
            }
        }

        // Second pass: set all parent relationships now that all entities are spawned
        for (child_entity, parent_handle) in deferred_parents {
            let registry = self.registry.borrow();
            let parent_bits = if let Some(bits) = registry.resolved_bits(parent_handle) {
                bits
            } else if parent_handle > 0 {
                // It's a real entity bits value, not a registry handle
                parent_handle as u64
            } else {
                // Parent handle is not resolved - this shouldn't happen now that all entities are spawned
                error!(
                    "Failed to resolve parent handle {} for entity {:?}",
                    parent_handle, child_entity
                );
                continue;
            };
            drop(registry);

            if let Some(parent_entity) = Entity::from_bits(parent_bits) {
                world.insert_one(child_entity, Parent(parent_entity))?;
                // Keep the parent's child list in step with the Parent links, like
                // `SetParent` does for existing entities.
                let listed = world
                    .get::<&mut crate::scene::components::Children>(parent_entity)
                    .map(|mut children| {
                        if !children.0.contains(&child_entity) {
                            children.0.push(child_entity);
                        }
                    })
                    .is_ok();
                if !listed && world.contains(parent_entity) {
                    world.insert_one(
                        parent_entity,
                        crate::scene::components::Children(vec![child_entity]),
                    )?;
                }
            }
        }

        // Built by the first despawn and shared by the rest: a script clearing a crowd
        // despawns hundreds of characters in one hook.
        let mut children_index = None;
        for command in self.existing.drain(..) {
            if let Err(err) = Self::apply_existing(world, command, &mut result, &mut children_index)
            {
                warn!(target: "script", "Skipping script command: {err}");
            }
        }

        for (handle, mut command) in self.assets.drain(..) {
            let resolve = |handle: i64| {
                self.registry
                    .borrow()
                    .resolved_bits(handle)
                    .unwrap_or(handle as u64)
            };
            let bits = resolve(handle);
            if let AssetCommand::ShareMaterial(source) | AssetCommand::ShareMesh(source) =
                &mut command
            {
                *source = resolve(*source) as i64;
            }
            match Entity::from_bits(bits) {
                Some(entity) => result
                    .asset_commands
                    .push(PendingAssetCommand { entity, command }),
                None => {
                    warn!(target: "script", "Skipping mesh/material/light/camera change for invalid entity {handle}")
                }
            }
        }

        for (handle, patch) in self.post_process.drain(..) {
            let target = match handle {
                Some(handle) => {
                    let bits = self
                        .registry
                        .borrow()
                        .resolved_bits(handle)
                        .unwrap_or(handle as u64);
                    match Entity::from_bits(bits) {
                        Some(entity) => Some(entity),
                        None => {
                            warn!(target: "script", "Skipping set_postprocess for invalid entity {handle}");
                            continue;
                        }
                    }
                }
                None => None,
            };
            result
                .post_process
                .push(PendingPostProcess { target, patch });
        }
        result.environment.append(&mut self.environment);
        for handle in std::mem::take(&mut self.generate_trees) {
            match self
                .resolve_entity_bits(handle)
                .ok()
                .and_then(Entity::from_bits)
            {
                Some(entity) if world.contains(entity) => result.generate_trees.push(entity),
                _ => warn!(target: "script", "animation_tree_generate: no entity {handle}"),
            }
        }

        Ok(result)
    }

    /// Apply a single command that targets an existing entity.
    ///
    /// Errors are scoped to the command so one stale entity reference does not
    /// discard the rest of the script's commands for the frame.
    fn apply_existing(
        world: &mut World,
        command: ExistingCommand,
        result: &mut ScriptApplyResult,
        children_index: &mut Option<HashMap<Entity, Vec<Entity>>>,
    ) -> Result<(), LuaScriptingError> {
        match command {
            ExistingCommand::Animator {
                entity_bits,
                command,
            } => {
                use crate::scene::skeletal::{animator_entity, Animator, AnimatorCommand};
                let Some(entity) = Entity::from_bits(entity_bits) else {
                    return Ok(());
                };
                if !world.contains(entity) {
                    return Err(ComponentError::NoSuchEntity.into());
                }
                match animator_entity(world, entity) {
                    Some(owner) => {
                        if let Ok(mut animator) = world.get::<&mut Animator>(owner) {
                            animator.apply(command);
                        }
                    }
                    // Adding clips to an entity without an animator gives it one.
                    None if matches!(command, AnimatorCommand::AddLibrary(_)) => {
                        let mut animator = Animator::default();
                        animator.apply(command);
                        world.insert_one(entity, animator)?;
                    }
                    None => warn!(
                        target: "script",
                        "{command:?}: entity {entity:?} has no Animator (nor does any parent)"
                    ),
                }
            }
            ExistingCommand::AnimationTree {
                entity_bits,
                command,
            } => {
                use crate::scene::skeletal::{tree_entity, AnimationTree};
                let Some(entity) = Entity::from_bits(entity_bits) else {
                    return Ok(());
                };
                if !world.contains(entity) {
                    return Err(ComponentError::NoSuchEntity.into());
                }
                match tree_entity(world, entity) {
                    Some(owner) => {
                        if let Ok(mut tree) = world.get::<&mut AnimationTree>(owner) {
                            tree.apply(command);
                        }
                    }
                    None => warn!(
                        target: "script",
                        "{command:?}: entity {entity:?} has no AnimationTree (nor does any \
                         parent or child)"
                    ),
                }
            }
            ExistingCommand::SetName { entity_bits, name } => {
                let Some(entity) = Entity::from_bits(entity_bits) else {
                    return Ok(());
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
                position,
            } => {
                let Some(entity) = Entity::from_bits(entity_bits) else {
                    return Ok(());
                };

                Self::modify_transform(world, entity, |transform| {
                    transform.translation = position;
                })?;
            }
            ExistingCommand::SetRotation {
                entity_bits,
                rotation,
            } => {
                let Some(entity) = Entity::from_bits(entity_bits) else {
                    return Ok(());
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
                    return Ok(());
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
                    return Ok(());
                };
                world.insert_one(entity, LuaScriptComponent::new(source))?;
                result.scripts_added.push(entity);
            }
            ExistingCommand::SetComponent {
                entity_bits,
                component_name,
                value,
            } => {
                let Some(entity) = Entity::from_bits(entity_bits) else {
                    return Ok(());
                };

                if world.entity(entity).is_err() {
                    return Err(ComponentError::NoSuchEntity.into());
                }

                if let Err(e) =
                    Self::add_component_from_json(world, entity, &component_name, &value)
                {
                    warn!(target: "script", "Failed to set component '{}': {}", component_name, e);
                }
            }
            ExistingCommand::AddComponent {
                entity_bits,
                component_name,
                value,
            } => {
                let Some(entity) = Entity::from_bits(entity_bits) else {
                    return Ok(());
                };

                if world.entity(entity).is_err() {
                    return Err(ComponentError::NoSuchEntity.into());
                }

                if let Err(e) =
                    Self::add_component_from_json(world, entity, &component_name, &value)
                {
                    warn!(target: "script", "Failed to add component '{}': {}", component_name, e);
                }
            }
            ExistingCommand::RemoveComponent {
                entity_bits,
                component_name,
            } => {
                let Some(entity) = Entity::from_bits(entity_bits) else {
                    return Ok(());
                };

                if world.entity(entity).is_err() {
                    return Err(ComponentError::NoSuchEntity.into());
                }

                if let Err(e) = Self::remove_component_by_name(world, entity, &component_name) {
                    warn!(target: "script", "Failed to remove component '{}': {}", component_name, e);
                }
            }
            ExistingCommand::Translate { entity_bits, delta } => {
                let Some(entity) = Entity::from_bits(entity_bits) else {
                    return Ok(());
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
                    return Ok(());
                };

                Self::modify_transform(world, entity, |transform| {
                    let rotation = glam::Quat::from_axis_angle(axis.normalize(), angle);
                    transform.rotation = rotation * transform.rotation;
                })?;
            }
            ExistingCommand::SetScale { entity_bits, scale } => {
                let Some(entity) = Entity::from_bits(entity_bits) else {
                    return Ok(());
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
                    return Ok(());
                };

                let mut aimed = false;
                Self::modify_transform(world, entity, |transform| {
                    if let Some(rotation) = look_rotation(transform.translation, target) {
                        transform.rotation = rotation;
                        aimed = true;
                    }
                })?;
                if aimed {
                    world.insert_one(entity, LookAtTarget(target))?;
                }
            }
            ExistingCommand::SetParent {
                entity_bits,
                parent_bits,
            } => {
                use crate::scene::components::{Children, Parent};

                let Some(entity) = Entity::from_bits(entity_bits) else {
                    return Ok(());
                };

                if world.entity(entity).is_err() {
                    return Err(ComponentError::NoSuchEntity.into());
                }

                if let Some(parent) = parent_bits.and_then(Entity::from_bits) {
                    if Self::is_self_or_ancestor(world, entity, parent) {
                        warn!(
                            target: "script",
                            "Ignoring set_parent: {parent:?} is {entity:?} or one of its descendants"
                        );
                        return Ok(());
                    }
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
                        return Ok(());
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
            ExistingCommand::Despawn { entity_bits } => {
                let Some(entity) = Entity::from_bits(entity_bits) else {
                    return Ok(());
                };

                // Despawning twice (or an entity something else already removed) is not
                // an error: the entity is gone either way.
                if world.contains(entity) {
                    let index =
                        children_index.get_or_insert_with(|| Self::children_by_parent(world));
                    result
                        .despawned
                        .extend(Self::despawn_subtree(world, entity, index));
                }
            }
            ExistingCommand::SubscribeEvent {
                entity_bits,
                event_name,
                callback_name,
            } => {
                let Some(entity) = Entity::from_bits(entity_bits) else {
                    return Ok(());
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
                    return Ok(());
                };

                if world.entity(entity).is_err() {
                    return Err(ComponentError::NoSuchEntity.into());
                }

                result
                    .event_unsubscriptions
                    .push(PendingEventUnsubscription { entity, event_name });
            }
        }
        Ok(())
    }

    /// Despawn `root` and all its descendants (children first), detaching `root` from its
    /// parent. Returns the entities removed.
    /// Every entity's children by its `Parent` link (whatever the child lists say).
    fn children_by_parent(world: &World) -> HashMap<Entity, Vec<Entity>> {
        let mut children_of: HashMap<Entity, Vec<Entity>> = HashMap::new();
        for (child, parent) in world.query::<&Parent>().iter() {
            children_of.entry(parent.0).or_default().push(child);
        }
        children_of
    }

    /// Remove `root` and everything below it. `children_of` (from
    /// [`Self::children_by_parent`], possibly built before earlier despawns in the same
    /// batch) adds the children the child lists miss; entities already gone are skipped.
    fn despawn_subtree(
        world: &mut World,
        root: Entity,
        children_of: &HashMap<Entity, Vec<Entity>>,
    ) -> Vec<Entity> {
        use crate::scene::components::Children;

        if let Ok(parent) = world.get::<&Parent>(root).map(|parent| parent.0) {
            let mut now_empty = false;
            if let Ok(mut siblings) = world.get::<&mut Children>(parent) {
                siblings.0.retain(|&child| child != root);
                now_empty = siblings.0.is_empty();
            }
            if now_empty {
                let _ = world.remove_one::<Children>(parent);
            }
        }

        // Descendants come from the Parent links as well as the child lists, so a
        // hierarchy whose lists are out of step still goes away completely.
        let mut order = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut stack = vec![root];
        while let Some(entity) = stack.pop() {
            if !world.contains(entity) || !seen.insert(entity) {
                continue;
            }
            order.push(entity);
            if let Some(children) = children_of.get(&entity) {
                stack.extend(children.iter().copied());
            }
            if let Ok(children) = world.get::<&Children>(entity) {
                stack.extend(children.0.iter().copied());
            }
        }

        for entity in order.iter().rev() {
            let _ = world.despawn(*entity);
        }
        order
    }

    /// Returns true if `candidate` is `entity` itself or sits below it in the hierarchy,
    /// i.e. parenting `entity` to `candidate` would create a cycle.
    fn is_self_or_ancestor(world: &World, entity: Entity, candidate: Entity) -> bool {
        let mut current = Some(candidate);
        // Bound the walk so a pre-existing corrupt hierarchy cannot hang us.
        for _ in 0..4096 {
            match current {
                Some(node) if node == entity => return true,
                Some(node) => current = world.get::<&Parent>(node).ok().map(|parent| parent.0),
                None => return false,
            }
        }
        true
    }

    fn modify_transform(
        world: &mut World,
        entity: Entity,
        apply: impl FnOnce(&mut Transform),
    ) -> Result<(), LuaScriptingError> {
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

    pub(crate) fn component_exists(
        world: &World,
        entity: Entity,
        component_name: &str,
    ) -> Result<bool, String> {
        let kind = component_kind_from_name(component_name)
            .ok_or_else(|| format!("Unknown or unsupported component type: {}", component_name))?;

        let exists = component_matches(world, entity, kind);

        Ok(exists)
    }

    pub(crate) fn component_to_json(
        world: &World,
        entity: Entity,
        component_name: &str,
    ) -> Result<Option<serde_json::Value>, String> {
        let kind = component_kind_from_name(component_name)
            .ok_or_else(|| format!("Unknown or unsupported component type: {}", component_name))?;

        match kind {
            ComponentKind::Name => {
                Self::component_value(world, entity, |value: &Name| json!(value.0))
            }
            ComponentKind::Visible => {
                Self::component_value(world, entity, |value: &Visible| json!(value.0))
            }
            ComponentKind::CanCastShadow => {
                Self::component_value(world, entity, |value: &CanCastShadow| json!(value.0))
            }
            ComponentKind::RotateAnimation => {
                Self::component_value(world, entity, |value: &RotateAnimation| {
                    json!({
                        "axis": vec3_to_json(value.axis),
                        "speed": value.speed
                    })
                })
            }
            ComponentKind::OrbitAnimation => {
                Self::component_value(world, entity, |value: &OrbitAnimation| {
                    json!({
                        "center": vec3_to_json(value.center),
                        "radius": value.radius,
                        "speed": value.speed,
                        "offset": value.offset
                    })
                })
            }
            ComponentKind::Transform => {
                Self::component_value(world, entity, |value: &TransformComponent| {
                    json!({
                        "translation": vec3_to_json(value.0.translation),
                        "rotation": quat_to_json(value.0.rotation),
                        "scale": vec3_to_json(value.0.scale)
                    })
                })
            }
            ComponentKind::Camera => {
                Self::component_value_fallible(world, entity, |value: &CameraComponent| {
                    serde_json::to_value(value).map_err(|err| err.to_string())
                })
            }
            ComponentKind::PointLight => {
                Self::component_value(world, entity, |value: &PointLight| {
                    json!({
                        "color": vec3_to_json(value.color),
                        "intensity": value.intensity,
                        "range": value.range
                    })
                })
            }
            ComponentKind::DirectionalLight => {
                Self::component_value(world, entity, |value: &DirectionalLight| {
                    json!({
                        "color": vec3_to_json(value.color),
                        "intensity": value.intensity,
                        "shadow_size": value.shadow_size
                    })
                })
            }
            ComponentKind::SpotLight => {
                Self::component_value(world, entity, |value: &SpotLight| {
                    json!({
                        "color": vec3_to_json(value.color),
                        "intensity": value.intensity,
                        "inner_angle": value.inner_angle,
                        "outer_angle": value.outer_angle,
                        "range": value.range
                    })
                })
            }
            ComponentKind::Mesh => Self::component_value(
                world,
                entity,
                |value: &MeshComponent| json!({ "index": value.0.index() }),
            ),
            ComponentKind::Material => Self::component_value(
                world,
                entity,
                |value: &MaterialComponent| json!({ "index": value.0.index() }),
            ),
            ComponentKind::PrimitiveMesh => {
                Self::component_value_fallible(world, entity, |value: &PrimitiveMeshComponent| {
                    serde_json::to_value(value).map_err(|err| err.to_string())
                })
            }
            ComponentKind::ParticleEmitter => {
                Self::component_value_fallible(world, entity, |value: &ParticleEmitterComponent| {
                    serde_json::to_value(value).map_err(|err| err.to_string())
                })
            }
            ComponentKind::Physics(name) => {
                if !world.contains(entity) {
                    return Err("Entity does not exist".to_string());
                }
                Ok(crate::physics::edit::component_json(world, entity, name))
            }
            #[cfg(feature = "navigation")]
            ComponentKind::Navigation(name) => {
                if !world.contains(entity) {
                    return Err("Entity does not exist".to_string());
                }
                Ok(crate::navigation::edit::component_json(world, entity, name))
            }
        }
    }

    fn component_value<T>(
        world: &World,
        entity: Entity,
        f: impl FnOnce(&T) -> serde_json::Value,
    ) -> Result<Option<serde_json::Value>, String>
    where
        T: hecs::Component,
    {
        match world.get::<&T>(entity) {
            Ok(value) => Ok(Some(f(&value))),
            Err(ComponentError::MissingComponent(_)) => Ok(None),
            Err(ComponentError::NoSuchEntity) => Err("Entity does not exist".to_string()),
        }
    }

    fn component_value_fallible<T>(
        world: &World,
        entity: Entity,
        f: impl FnOnce(&T) -> Result<serde_json::Value, String>,
    ) -> Result<Option<serde_json::Value>, String>
    where
        T: hecs::Component,
    {
        match world.get::<&T>(entity) {
            Ok(value) => Ok(Some(f(&value)?)),
            Err(ComponentError::MissingComponent(_)) => Ok(None),
            Err(ComponentError::NoSuchEntity) => Err("Entity does not exist".to_string()),
        }
    }

    /// Add or set a component from a serde_json::Value
    fn add_component_from_json(
        world: &mut World,
        entity: Entity,
        component_name: &str,
        value: &serde_json::Value,
    ) -> Result<(), String> {
        use crate::scene::components::*;

        match component_kind_from_name(component_name) {
            Some(ComponentKind::Name) => {
                let name = value
                    .as_str()
                    .ok_or_else(|| "Name must be a string".to_string())?;
                world
                    .insert_one(entity, Name(name.to_string()))
                    .map_err(|e| e.to_string())?;
            }
            Some(ComponentKind::Visible) => {
                let visible = value
                    .as_bool()
                    .ok_or_else(|| "Visible must be a boolean".to_string())?;
                world
                    .insert_one(entity, Visible(visible))
                    .map_err(|e| e.to_string())?;
            }
            Some(ComponentKind::CanCastShadow) => {
                let can_cast = value
                    .as_bool()
                    .ok_or_else(|| "CanCastShadow must be a boolean".to_string())?;
                world
                    .insert_one(entity, CanCastShadow(can_cast))
                    .map_err(|e| e.to_string())?;
            }
            Some(ComponentKind::RotateAnimation) => {
                // Expect {axis: [x, y, z], speed: number}
                let obj = value
                    .as_object()
                    .ok_or_else(|| "RotateAnimation must be an object".to_string())?;
                let axis = obj
                    .get("axis")
                    .and_then(|v| v.as_array())
                    .ok_or_else(|| "RotateAnimation.axis must be an array".to_string())?;
                let speed = obj
                    .get("speed")
                    .and_then(|v| v.as_f64())
                    .ok_or_else(|| "RotateAnimation.speed must be a number".to_string())?;

                if axis.len() != 3 {
                    return Err("RotateAnimation.axis must have 3 elements".to_string());
                }

                let axis_vec = Vec3::new(
                    axis[0].as_f64().ok_or("Invalid axis value")? as f32,
                    axis[1].as_f64().ok_or("Invalid axis value")? as f32,
                    axis[2].as_f64().ok_or("Invalid axis value")? as f32,
                );

                world
                    .insert_one(
                        entity,
                        RotateAnimation {
                            axis: axis_vec,
                            speed: speed as f32,
                        },
                    )
                    .map_err(|e| e.to_string())?;
            }
            Some(ComponentKind::OrbitAnimation) => {
                // Expect {center: [x, y, z], radius: number, speed: number, offset: number}
                let obj = value
                    .as_object()
                    .ok_or_else(|| "OrbitAnimation must be an object".to_string())?;
                let center = obj
                    .get("center")
                    .and_then(|v| v.as_array())
                    .ok_or_else(|| "OrbitAnimation.center must be an array".to_string())?;
                let radius = obj
                    .get("radius")
                    .and_then(|v| v.as_f64())
                    .ok_or_else(|| "OrbitAnimation.radius must be a number".to_string())?;
                let speed = obj
                    .get("speed")
                    .and_then(|v| v.as_f64())
                    .ok_or_else(|| "OrbitAnimation.speed must be a number".to_string())?;
                let offset = obj.get("offset").and_then(|v| v.as_f64()).unwrap_or(0.0);

                if center.len() != 3 {
                    return Err("OrbitAnimation.center must have 3 elements".to_string());
                }

                let center_vec = Vec3::new(
                    center[0].as_f64().ok_or("Invalid center value")? as f32,
                    center[1].as_f64().ok_or("Invalid center value")? as f32,
                    center[2].as_f64().ok_or("Invalid center value")? as f32,
                );

                world
                    .insert_one(
                        entity,
                        OrbitAnimation {
                            center: center_vec,
                            radius: radius as f32,
                            speed: speed as f32,
                            offset: offset as f32,
                        },
                    )
                    .map_err(|e| e.to_string())?;
            }
            // Physics and navigation components merge the given fields into what is there.
            Some(ComponentKind::Physics(name)) => {
                crate::physics::edit::patch_component(world, entity, name, value)?;
            }
            #[cfg(feature = "navigation")]
            Some(ComponentKind::Navigation(name)) => {
                crate::navigation::edit::patch_component(world, entity, name, value)?;
            }
            Some(_) => {
                return Err(format!(
                    "Component type cannot be set from JSON: {}",
                    component_name
                ));
            }
            None => {
                return Err(format!(
                    "Unknown or unsupported component type: {}",
                    component_name
                ));
            }
        }

        Ok(())
    }

    /// Remove a component by name
    fn remove_component_by_name(
        world: &mut World,
        entity: Entity,
        component_name: &str,
    ) -> Result<(), String> {
        use crate::scene::components::*;

        match component_kind_from_name(component_name) {
            Some(ComponentKind::Name) => {
                world
                    .remove_one::<Name>(entity)
                    .map_err(|e| e.to_string())?;
            }
            Some(ComponentKind::Visible) => {
                world
                    .remove_one::<Visible>(entity)
                    .map_err(|e| e.to_string())?;
            }
            Some(ComponentKind::CanCastShadow) => {
                world
                    .remove_one::<CanCastShadow>(entity)
                    .map_err(|e| e.to_string())?;
            }
            Some(ComponentKind::RotateAnimation) => {
                world
                    .remove_one::<RotateAnimation>(entity)
                    .map_err(|e| e.to_string())?;
            }
            Some(ComponentKind::OrbitAnimation) => {
                world
                    .remove_one::<OrbitAnimation>(entity)
                    .map_err(|e| e.to_string())?;
            }
            Some(ComponentKind::Transform) => {
                world
                    .remove_one::<TransformComponent>(entity)
                    .map_err(|e| e.to_string())?;
            }
            Some(ComponentKind::Camera) => {
                world
                    .remove_one::<CameraComponent>(entity)
                    .map_err(|e| e.to_string())?;
            }
            Some(ComponentKind::PointLight) => {
                world
                    .remove_one::<PointLight>(entity)
                    .map_err(|e| e.to_string())?;
            }
            Some(ComponentKind::DirectionalLight) => {
                world
                    .remove_one::<DirectionalLight>(entity)
                    .map_err(|e| e.to_string())?;
            }
            Some(ComponentKind::SpotLight) => {
                world
                    .remove_one::<SpotLight>(entity)
                    .map_err(|e| e.to_string())?;
            }
            Some(ComponentKind::Mesh) => {
                world
                    .remove_one::<MeshComponent>(entity)
                    .map_err(|e| e.to_string())?;
            }
            Some(ComponentKind::Material) => {
                world
                    .remove_one::<MaterialComponent>(entity)
                    .map_err(|e| e.to_string())?;
            }
            Some(ComponentKind::PrimitiveMesh) => {
                world
                    .remove_one::<PrimitiveMeshComponent>(entity)
                    .map_err(|e| e.to_string())?;
            }
            Some(ComponentKind::ParticleEmitter) => {
                world
                    .remove_one::<ParticleEmitterComponent>(entity)
                    .map_err(|e| e.to_string())?;
            }
            #[cfg(feature = "navigation")]
            Some(ComponentKind::Navigation(name)) => {
                crate::navigation::edit::patch_component(
                    world,
                    entity,
                    name,
                    &serde_json::Value::Null,
                )?;
            }
            Some(ComponentKind::Physics(name)) => {
                crate::physics::edit::patch_component(
                    world,
                    entity,
                    name,
                    &serde_json::Value::Null,
                )?;
            }
            None => {
                return Err(format!(
                    "Unknown or unsupported component type: {}",
                    component_name
                ));
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ComponentKind {
    Name,
    Visible,
    CanCastShadow,
    RotateAnimation,
    OrbitAnimation,
    Transform,
    Camera,
    PointLight,
    DirectionalLight,
    SpotLight,
    Mesh,
    Material,
    PrimitiveMesh,
    ParticleEmitter,
    /// A physics component, by its snake_case name.
    Physics(&'static str),
    /// A navigation component, by its snake_case name.
    #[cfg(feature = "navigation")]
    Navigation(&'static str),
}

pub(crate) fn component_kind_from_name(component_name: &str) -> Option<ComponentKind> {
    match component_name {
        "Name" => Some(ComponentKind::Name),
        "Visible" => Some(ComponentKind::Visible),
        "CanCastShadow" => Some(ComponentKind::CanCastShadow),
        "RotateAnimation" => Some(ComponentKind::RotateAnimation),
        "OrbitAnimation" => Some(ComponentKind::OrbitAnimation),
        "TransformComponent" | "Transform" => Some(ComponentKind::Transform),
        "CameraComponent" | "Camera" => Some(ComponentKind::Camera),
        "PointLight" => Some(ComponentKind::PointLight),
        "DirectionalLight" => Some(ComponentKind::DirectionalLight),
        "SpotLight" => Some(ComponentKind::SpotLight),
        "MeshComponent" | "Mesh" => Some(ComponentKind::Mesh),
        "MaterialComponent" | "Material" => Some(ComponentKind::Material),
        "PrimitiveMeshComponent" => Some(ComponentKind::PrimitiveMesh),
        "ParticleEmitterComponent" => Some(ComponentKind::ParticleEmitter),
        #[cfg(feature = "navigation")]
        other if crate::navigation::edit::canonical_name(other).is_some() => {
            crate::navigation::edit::canonical_name(other).map(ComponentKind::Navigation)
        }
        other => crate::physics::edit::canonical_name(other).map(ComponentKind::Physics),
    }
}

pub(crate) fn component_matches(world: &World, entity: Entity, kind: ComponentKind) -> bool {
    match kind {
        ComponentKind::Name => world.satisfies::<&Name>(entity),
        ComponentKind::Visible => world.satisfies::<&Visible>(entity),
        ComponentKind::CanCastShadow => world.satisfies::<&CanCastShadow>(entity),
        ComponentKind::RotateAnimation => world.satisfies::<&RotateAnimation>(entity),
        ComponentKind::OrbitAnimation => world.satisfies::<&OrbitAnimation>(entity),
        ComponentKind::Transform => world.satisfies::<&TransformComponent>(entity),
        ComponentKind::Camera => world.satisfies::<&CameraComponent>(entity),
        ComponentKind::PointLight => world.satisfies::<&PointLight>(entity),
        ComponentKind::DirectionalLight => world.satisfies::<&DirectionalLight>(entity),
        ComponentKind::SpotLight => world.satisfies::<&SpotLight>(entity),
        ComponentKind::Mesh => world.satisfies::<&MeshComponent>(entity),
        ComponentKind::Material => world.satisfies::<&MaterialComponent>(entity),
        ComponentKind::PrimitiveMesh => world.satisfies::<&PrimitiveMeshComponent>(entity),
        ComponentKind::ParticleEmitter => world.satisfies::<&ParticleEmitterComponent>(entity),
        ComponentKind::Physics(name) => {
            Ok(crate::physics::edit::has_component(world, entity, name))
        }
        #[cfg(feature = "navigation")]
        ComponentKind::Navigation(name) => {
            Ok(crate::navigation::edit::has_component(world, entity, name))
        }
    }
    .unwrap_or(false)
}

fn vec3_to_json(value: Vec3) -> serde_json::Value {
    json!([value.x, value.y, value.z])
}

fn quat_to_json(value: Quat) -> serde_json::Value {
    json!([value.x, value.y, value.z, value.w])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Transform;
    use serde_json::json;

    #[test]
    fn look_at_aims_entities_spawned_in_the_same_frame() {
        let mut commands =
            ScriptCommands::new(Rc::new(RefCell::new(EntityHandleRegistry::default())));
        let handle = commands.spawn_entity(Some("Spot".into()));
        commands
            .set_translation(handle, Vec3::new(0.0, 6.0, 0.0))
            .unwrap();
        commands.look_at(handle, Vec3::ZERO).unwrap();
        commands.translate(handle, Vec3::X).unwrap();

        let mut world = World::new();
        commands.apply(&mut world).unwrap();
        let (_, transform) = world
            .query_mut::<&TransformComponent>()
            .into_iter()
            .next()
            .unwrap();
        assert_eq!(transform.0.translation, Vec3::new(1.0, 6.0, 0.0));
        let forward = transform.0.rotation * Vec3::NEG_Z;
        assert!(forward.abs_diff_eq(Vec3::NEG_Y, 1e-5), "{forward}");
    }

    #[test]
    fn look_at_keeps_the_horizon_level_through_a_full_orbit() {
        let mut commands =
            ScriptCommands::new(Rc::new(RefCell::new(EntityHandleRegistry::default())));
        let mut world = World::new();
        let target = Vec3::new(0.0, 3.7, 0.0);

        for step in 0..=72 {
            let angle = step as f32 * std::f32::consts::TAU / 72.0;
            let eye = Vec3::new(angle.sin() * 20.2, 8.4, angle.cos() * 20.2);
            let handle = commands.spawn_entity(None);
            commands.set_translation(handle, eye).unwrap();
            commands.look_at(handle, target).unwrap();
            commands.apply(&mut world).unwrap();
            let entity = Entity::from_bits(commands.resolve_entity_bits(handle).unwrap()).unwrap();

            // Check both freshly spawned entities and commands on existing entities.
            for existing in [false, true] {
                if existing {
                    commands.look_at(handle, target).unwrap();
                    commands.apply(&mut world).unwrap();
                }
                let transform = world.get::<&TransformComponent>(entity).unwrap();
                let rotation = transform.0.rotation;
                let forward = rotation * Vec3::NEG_Z;
                let up = rotation * Vec3::Y;
                let right = rotation * Vec3::X;
                assert!(
                    forward.abs_diff_eq((target - eye).normalize(), 1e-5),
                    "step {step}, existing {existing}: forward {forward}"
                );
                assert!(
                    up.y > 0.9 && right.y.abs() < 1e-5,
                    "step {step}, existing {existing}: up {up}, right {right}"
                );
            }
        }
    }

    #[test]
    fn look_at_coincident_target_preserves_rotation() {
        let mut commands =
            ScriptCommands::new(Rc::new(RefCell::new(EntityHandleRegistry::default())));
        let mut world = World::new();
        let eye = Vec3::new(2.0, 3.0, 4.0);
        let rotation = Quat::from_rotation_y(0.8);
        let handle = commands.spawn_entity(None);
        commands.set_translation(handle, eye).unwrap();
        commands.set_rotation(handle, rotation).unwrap();
        commands.look_at(handle, eye).unwrap();
        commands.apply(&mut world).unwrap();
        let entity = Entity::from_bits(commands.resolve_entity_bits(handle).unwrap()).unwrap();
        assert_eq!(
            world.get::<&TransformComponent>(entity).unwrap().0.rotation,
            rotation
        );

        commands.look_at(handle, eye).unwrap();
        commands.apply(&mut world).unwrap();
        assert_eq!(
            world.get::<&TransformComponent>(entity).unwrap().0.rotation,
            rotation
        );
    }

    #[test]
    fn set_and_remove_component_accept_entities_spawned_in_the_same_frame() {
        let mut commands =
            ScriptCommands::new(Rc::new(RefCell::new(EntityHandleRegistry::default())));
        let hidden = commands.spawn_entity(Some("Hidden".into()));
        commands
            .set_component(hidden, "Visible".into(), json!(false))
            .unwrap();
        let shadowless = commands.spawn_entity(Some("Shadowless".into()));
        commands
            .set_component(shadowless, "CanCastShadow".into(), json!(false))
            .unwrap();
        // A component that is set and then removed in the same chunk is never added.
        commands
            .remove_component(shadowless, "CanCastShadow".into())
            .unwrap();

        let mut world = World::new();
        commands.apply(&mut world).unwrap();

        let mut visible = None;
        let mut can_cast = None;
        for (_, (name, vis, cast)) in world
            .query::<(&Name, Option<&Visible>, Option<&CanCastShadow>)>()
            .iter()
        {
            match name.0.as_str() {
                "Hidden" => visible = vis.map(|v| v.0),
                "Shadowless" => can_cast = Some(cast.map(|c| c.0)),
                _ => {}
            }
        }
        assert_eq!(visible, Some(false));
        assert_eq!(can_cast, Some(None));
    }

    #[test]
    fn component_exists_recognizes_supported_components() {
        let mut world = World::new();
        let entity = world.spawn((Name::new("Test"),));

        let has_name = ScriptCommands::component_exists(&world, entity, "Name").unwrap();
        let has_visible = ScriptCommands::component_exists(&world, entity, "Visible").unwrap();

        assert!(has_name);
        assert!(!has_visible);
    }

    #[test]
    fn component_exists_rejects_unknown_components() {
        let mut world = World::new();
        let entity = world.spawn(());

        let result = ScriptCommands::component_exists(&world, entity, "MysteryComponent");

        assert!(result.is_err());
    }

    #[test]
    fn component_to_json_serializes_basic_components() {
        let mut world = World::new();
        let entity = world.spawn((
            Name::new("Player"),
            Visible(false),
            TransformComponent(Transform::from_translation(Vec3::new(1.0, 2.0, 3.0))),
        ));

        let name = ScriptCommands::component_to_json(&world, entity, "Name")
            .unwrap()
            .unwrap();
        let visible = ScriptCommands::component_to_json(&world, entity, "Visible")
            .unwrap()
            .unwrap();
        let transform = ScriptCommands::component_to_json(&world, entity, "Transform")
            .unwrap()
            .unwrap();

        assert_eq!(name, json!("Player"));
        assert_eq!(visible, json!(false));
        assert_eq!(
            transform,
            json!({
                "translation": [1.0, 2.0, 3.0],
                "rotation": [0.0, 0.0, 0.0, 1.0],
                "scale": [1.0, 1.0, 1.0]
            })
        );
    }

    #[test]
    fn component_to_json_handles_missing_components() {
        let mut world = World::new();
        let entity = world.spawn(());

        let result = ScriptCommands::component_to_json(&world, entity, "Visible").unwrap();

        assert!(result.is_none());
    }

    #[test]
    fn component_to_json_errors_on_missing_entity() {
        let mut world = World::new();
        let entity = world.spawn((Name::new("Gone"),));
        world.despawn(entity).unwrap();

        let result = ScriptCommands::component_to_json(&world, entity, "Name");

        assert!(result.is_err());
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

/// A mesh or material change a script asked for, in the same JSON form the remote
/// `entity.update` fields `mesh` and `material` take.
#[derive(Debug, Clone)]
pub enum AssetCommand {
    Material(serde_json::Value),
    Mesh(serde_json::Value),
    /// A mesh built by a script (one colour of a voxel model, ...).
    GeneratedMesh(std::sync::Arc<crate::procgen::GeneratedMesh>),
    Light(serde_json::Value),
    Camera(serde_json::Value),
    ActiveCamera,
    /// Use the material of this entity (a script handle while queued, entity bits once
    /// applied), so both draw with one material.
    ShareMaterial(i64),
    /// Draw the mesh of this entity (a script handle while queued, entity bits once
    /// applied): a pawn promoting to the queen's model, say.
    ShareMesh(i64),
}

/// An [`AssetCommand`] resolved to its entity, waiting for the renderer.
#[derive(Debug, Clone)]
pub struct PendingAssetCommand {
    pub entity: Entity,
    pub command: AssetCommand,
}

/// A `set_postprocess` change resolved to its volume entity (`None`: the scene's
/// infinite volume), waiting for the scene.
#[derive(Debug, Clone)]
pub struct PendingPostProcess {
    pub target: Option<Entity>,
    pub patch: serde_json::Value,
}

#[derive(Default)]
pub(crate) struct ScriptApplyResult {
    pub scripts_added: Vec<Entity>,
    /// Entities removed by `despawn_entity`, including descendants.
    pub despawned: Vec<Entity>,
    pub gltf_imports: Vec<PendingGltfImport>,
    pub asset_commands: Vec<PendingAssetCommand>,
    pub particle_bursts: Vec<crate::gpu_particles::ParticleBurst>,
    pub physics: Vec<crate::physics::PhysicsCommand>,
    #[cfg(feature = "navigation")]
    pub navigation: Vec<crate::navigation::NavCommand>,
    /// Entities to give a generated locomotion `AnimationTree`.
    pub generate_trees: Vec<Entity>,
    pub post_process: Vec<PendingPostProcess>,
    /// `set_environment` patches, in order.
    pub environment: Vec<serde_json::Value>,
    pub event_subscriptions: Vec<PendingEventSubscription>,
    pub event_unsubscriptions: Vec<PendingEventUnsubscription>,
}

/// Take `entity` out of its parent's child list and drop its `Parent`.
fn detach_from_parent(world: &mut World, entity: Entity) {
    use crate::scene::components::Children;

    let Ok(parent) = world.get::<&Parent>(entity).map(|p| p.0) else {
        return;
    };
    if let Ok(mut children) = world.get::<&mut Children>(parent) {
        children.0.retain(|&child| child != entity);
    }
    let _ = world.remove_one::<Parent>(entity);
}

pub(crate) fn entity_bits(entity: Entity) -> i64 {
    entity.to_bits().get() as i64
}
