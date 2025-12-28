use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use glam::{Quat, Vec3};
use hecs::{ComponentError, Entity, World};
use log::warn;
use serde_json::json;

use crate::scene::{
    CameraComponent, CanCastShadow, DirectionalLight, MaterialComponent, MeshComponent, Name,
    OrbitAnimation, Parent, ParticleEmitterComponent, PointLight, PrimitiveMeshComponent,
    RotateAnimation, SpotLight, Transform, TransformComponent, Visible,
};

use super::component::LuaScriptComponent;
use super::entity_registry::{EntityHandleRegistry, ExistingCommand, PendingEntity};
use super::error::LuaScriptingError;
use super::types::LuaScriptSource;

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
        let entity_bits = self.resolve_entity_bits(handle)?;

        self.existing.push(ExistingCommand::RemoveComponent {
            entity_bits,
            component_name,
        });
        Ok(())
    }

    pub fn translate(&mut self, handle: i64, delta: Vec3) -> Result<(), mlua::Error> {
        let entity_bits = self.resolve_entity_bits(handle)?;

        self.existing
            .push(ExistingCommand::Translate { entity_bits, delta });
        Ok(())
    }

    pub fn rotate(&mut self, handle: i64, axis: Vec3, angle: f32) -> Result<(), mlua::Error> {
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
        self.pending.is_empty() && self.existing.is_empty()
    }

    pub fn apply(&mut self, world: &mut World) -> Result<ScriptApplyResult, LuaScriptingError> {
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

            if pending.translation.is_some()
                || pending.rotation.is_some()
                || pending.scale.is_some()
            {
                let mut transform = Transform::default();
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
                    position,
                } => {
                    let Some(entity) = Entity::from_bits(entity_bits) else {
                        continue;
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
                    world.insert_one(entity, LuaScriptComponent::new(source))?;
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
                        continue;
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
                        continue;
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
                ExistingCommand::SetScale { entity_bits, scale } => {
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
                            transform.rotation =
                                glam::Quat::from_rotation_arc(glam::Vec3::NEG_Z, direction);
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
                        let has_children =
                            world.satisfies::<&Children>(parent_entity).unwrap_or(false);

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

                    result
                        .event_unsubscriptions
                        .push(PendingEventUnsubscription { entity, event_name });
                }
            }
        }

        Ok(result)
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

        let exists = match kind {
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
        }
        .unwrap_or(false);

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
enum ComponentKind {
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
}

fn component_kind_from_name(component_name: &str) -> Option<ComponentKind> {
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
        _ => None,
    }
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
