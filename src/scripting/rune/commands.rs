use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use glam::{Quat, Vec3};
use hecs::{ComponentError, Entity, World};
use log::warn;
use rune::runtime::VmResult;
use rune::Value;

use crate::scene::{Name, Parent, Transform, TransformComponent};
use crate::scripting::component_registry::ComponentRegistry;

use super::component::RuneScriptComponent;
use super::entity_registry::{EntityHandleRegistry, ExistingCommand, PendingEntity};
use super::error::RuneScriptingError;
use super::types::RuneScriptSource;

pub(crate) struct ScriptCommands {
    pub registry: Rc<RefCell<EntityHandleRegistry>>,
    pub pending: HashMap<i64, PendingEntity>,
    pub existing: Vec<ExistingCommand>,
}

impl ScriptCommands {
    pub fn new(registry: Rc<RefCell<EntityHandleRegistry>>) -> Self {
        Self {
            registry,
            pending: HashMap::new(),
            existing: Vec::new(),
        }
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

    pub fn resolve_entity_bits(&self, handle: i64) -> VmResult<u64> {
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

    pub fn set_name(&mut self, handle: i64, name: String) -> VmResult<()> {
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

    pub fn set_translation(&mut self, handle: i64, translation: Vec3) -> VmResult<()> {
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

    pub fn set_rotation(&mut self, handle: i64, rotation: Quat) -> VmResult<()> {
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

    pub fn attach_inline_script(&mut self, handle: i64, name: String, source: String) -> VmResult<()> {
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

    pub fn attach_file_script(&mut self, handle: i64, path: String) -> VmResult<()> {
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

    pub fn import_gltf(&mut self, handle: i64, path: String, scale: f32) -> VmResult<()> {
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

    pub fn set_component(&mut self, handle: i64, component_name: String, value: Value) -> VmResult<()> {
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

    pub fn add_component(&mut self, handle: i64, component_name: String, value: Value) -> VmResult<()> {
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

    pub fn remove_component(&mut self, handle: i64, component_name: String) -> VmResult<()> {
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

    pub fn translate(&mut self, handle: i64, delta: Vec3) -> VmResult<()> {
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

    pub fn rotate(&mut self, handle: i64, axis: Vec3, angle: f32) -> VmResult<()> {
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

    pub fn set_scale(&mut self, handle: i64, scale: Vec3) -> VmResult<()> {
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

    pub fn look_at(&mut self, handle: i64, target: Vec3) -> VmResult<()> {
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

    pub fn set_parent(&mut self, handle: i64, parent_handle: Option<i64>) -> VmResult<()> {
        // If the child is a pending entity, store the parent handle to be resolved later
        if let Some(entry) = self.pending.get_mut(&handle) {
            entry.parent = parent_handle;
            return VmResult::Ok(());
        }

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

    pub fn subscribe_event(
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

    pub fn unsubscribe_event(&mut self, handle: i64, event_name: String) -> VmResult<()> {
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

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty() && self.existing.is_empty()
    }

    pub fn apply(&mut self, world: &mut World, registry: &ComponentRegistry) -> Result<ScriptApplyResult, RuneScriptingError> {
        use log::error;

        let mut result = ScriptApplyResult::default();
        let mut deferred_parents: Vec<(Entity, i64)> = Vec::new();

        // First pass: spawn all entities and collect parent relationships
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
                error!("Failed to resolve parent handle {} for entity {:?}", parent_handle, child_entity);
                continue;
            };
            drop(registry);

            if let Some(parent_entity) = Entity::from_bits(parent_bits) {
                world.insert_one(child_entity, Parent(parent_entity))?;
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
                    use crate::scene::components::{Children, Parent};

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
pub(crate) struct ScriptApplyResult {
    pub scripts_added: Vec<Entity>,
    pub gltf_imports: Vec<PendingGltfImport>,
    pub event_subscriptions: Vec<PendingEventSubscription>,
    pub event_unsubscriptions: Vec<PendingEventUnsubscription>,
}

pub(crate) fn entity_bits(entity: Entity) -> i64 {
    entity.to_bits().get() as i64
}
