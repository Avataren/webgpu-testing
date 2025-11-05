//! Component registry for script access to ECS components.
//!
//! This module provides a type-erased registry that allows scripts to read and write
//! ECS components by name. Components must implement conversion traits to be accessible.

use std::collections::HashMap;
use std::sync::Arc;

use hecs::{Entity, World};
use rune::runtime::Value;
use rune::alloc::String as RuneString;
use thiserror::Error;

use crate::scene::{Name, TransformComponent, Transform};

/// Error type for component registry operations.
#[derive(Debug, Error)]
pub enum ComponentRegistryError {
    /// Component type not found in registry.
    #[error("unknown component type: {0}")]
    UnknownComponent(String),

    /// Failed to convert component to Rune value.
    #[error("failed to convert component to Rune value: {0}")]
    ToRuneValue(String),

    /// Failed to convert Rune value to component.
    #[error("failed to convert Rune value to component: {0}")]
    FromRuneValue(String),

    /// Entity does not have the requested component.
    #[error("entity does not have component: {0}")]
    MissingComponent(String),

    /// Rune runtime error during conversion.
    #[error("Rune runtime error: {0}")]
    RuneError(#[from] rune::alloc::Error),

    /// Rune runtime error during to_value conversion.
    #[error("Rune to_value error: {0}")]
    RuneRuntimeError(#[from] rune::runtime::RuntimeError),
}

/// Trait for converting a Rust component to a Rune value.
pub trait ToRuneValue {
    fn to_rune_value(&self) -> Result<Value, ComponentRegistryError>;
}

/// Trait for converting a Rune value to a Rust component.
pub trait FromRuneValue: Sized {
    fn from_rune_value(value: &Value) -> Result<Self, ComponentRegistryError>;
}

/// Type-erased handler for a specific component type.
///
/// This allows the registry to store handlers for different component types
/// in a homogeneous collection.
trait ComponentHandler: Send + Sync {
    /// Get component from entity and convert to Rune value.
    fn get(&self, world: &World, entity: Entity) -> Result<Value, ComponentRegistryError>;

    /// Set component on entity from Rune value.
    fn set(&self, world: &mut World, entity: Entity, value: &Value) -> Result<(), ComponentRegistryError>;

    /// Check if entity has this component.
    fn has(&self, world: &World, entity: Entity) -> bool;

    /// Get the component type name.
    fn type_name(&self) -> &str;
}

/// Concrete handler implementation for a specific component type.
struct TypedComponentHandler<T> {
    type_name: String,
    _phantom: std::marker::PhantomData<T>,
}

impl<T> TypedComponentHandler<T> {
    fn new(type_name: impl Into<String>) -> Self {
        Self {
            type_name: type_name.into(),
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<T> ComponentHandler for TypedComponentHandler<T>
where
    T: ToRuneValue + FromRuneValue + hecs::Component,
{
    fn get(&self, world: &World, entity: Entity) -> Result<Value, ComponentRegistryError> {
        world
            .get::<&T>(entity)
            .map_err(|_| ComponentRegistryError::MissingComponent(self.type_name.clone()))?
            .to_rune_value()
    }

    fn set(&self, world: &mut World, entity: Entity, value: &Value) -> Result<(), ComponentRegistryError> {
        let component = T::from_rune_value(value)?;
        world
            .insert_one(entity, component)
            .map_err(|e| ComponentRegistryError::FromRuneValue(format!("failed to insert component: {}", e)))?;
        Ok(())
    }

    fn has(&self, world: &World, entity: Entity) -> bool {
        world.get::<&T>(entity).is_ok()
    }

    fn type_name(&self) -> &str {
        &self.type_name
    }
}

/// Registry mapping component type names to handlers.
pub struct ComponentRegistry {
    handlers: HashMap<String, Arc<dyn ComponentHandler>>,
}

impl ComponentRegistry {
    /// Create a new registry with default components registered.
    pub fn new() -> Self {
        let mut registry = Self {
            handlers: HashMap::new(),
        };

        // Register built-in components
        registry.register::<Name>("Name");
        registry.register::<TransformComponent>("TransformComponent");
        registry.register::<TransformComponent>("Transform"); // Alias

        registry
    }

    /// Register a component type with the registry.
    pub fn register<T>(&mut self, name: &str)
    where
        T: ToRuneValue + FromRuneValue + hecs::Component,
    {
        let handler = Arc::new(TypedComponentHandler::<T>::new(name));
        self.handlers.insert(name.to_string(), handler);
    }

    /// Get a component from an entity and convert to Rune value.
    pub fn get_component(
        &self,
        world: &World,
        entity: Entity,
        component_name: &str,
    ) -> Result<Value, ComponentRegistryError> {
        let handler = self
            .handlers
            .get(component_name)
            .ok_or_else(|| ComponentRegistryError::UnknownComponent(component_name.to_string()))?;

        handler.get(world, entity)
    }

    /// Set a component on an entity from a Rune value.
    pub fn set_component(
        &self,
        world: &mut World,
        entity: Entity,
        component_name: &str,
        value: &Value,
    ) -> Result<(), ComponentRegistryError> {
        let handler = self
            .handlers
            .get(component_name)
            .ok_or_else(|| ComponentRegistryError::UnknownComponent(component_name.to_string()))?;

        handler.set(world, entity, value)
    }

    /// Check if an entity has a component.
    pub fn has_component(
        &self,
        world: &World,
        entity: Entity,
        component_name: &str,
    ) -> Result<bool, ComponentRegistryError> {
        let handler = self
            .handlers
            .get(component_name)
            .ok_or_else(|| ComponentRegistryError::UnknownComponent(component_name.to_string()))?;

        Ok(handler.has(world, entity))
    }

    /// Get list of all registered component types.
    pub fn registered_types(&self) -> Vec<String> {
        self.handlers.keys().cloned().collect()
    }
}

impl Default for ComponentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Component Implementations
// ============================================================================

impl ToRuneValue for Name {
    fn to_rune_value(&self) -> Result<Value, ComponentRegistryError> {
        Ok(Value::try_from(RuneString::try_from(self.0.as_str())?)?)
    }
}

impl FromRuneValue for Name {
    fn from_rune_value(value: &Value) -> Result<Self, ComponentRegistryError> {
        let string = value
            .borrow_string_ref()
            .map_err(|e| ComponentRegistryError::FromRuneValue(format!("expected string: {}", e)))?;

        Ok(Name((*string).to_string()))
    }
}

impl ToRuneValue for TransformComponent {
    fn to_rune_value(&self) -> Result<Value, ComponentRegistryError> {
        use rune::runtime::Object;
        use rune::alloc::Vec as RuneVec;

        let mut obj = Object::new();

        // Translation as array [x, y, z]
        let translation = self.0.translation;
        let mut trans_vec = RuneVec::new();
        trans_vec.try_push(Value::from(translation.x as f64))?;
        trans_vec.try_push(Value::from(translation.y as f64))?;
        trans_vec.try_push(Value::from(translation.z as f64))?;
        obj.insert(RuneString::try_from("translation")?, rune::to_value(trans_vec)?)?;

        // Rotation (as euler angles YXZ) as array [yaw, pitch, roll]
        let (yaw, pitch, roll) = self.0.rotation.to_euler(glam::EulerRot::YXZ);
        let mut rot_vec = RuneVec::new();
        rot_vec.try_push(Value::from(yaw as f64))?;
        rot_vec.try_push(Value::from(pitch as f64))?;
        rot_vec.try_push(Value::from(roll as f64))?;
        obj.insert(RuneString::try_from("rotation")?, rune::to_value(rot_vec)?)?;

        // Scale as array [x, y, z]
        let scale = self.0.scale;
        let mut scale_vec = RuneVec::new();
        scale_vec.try_push(Value::from(scale.x as f64))?;
        scale_vec.try_push(Value::from(scale.y as f64))?;
        scale_vec.try_push(Value::from(scale.z as f64))?;
        obj.insert(RuneString::try_from("scale")?, rune::to_value(scale_vec)?)?;

        Ok(rune::to_value(obj)?)
    }
}

impl FromRuneValue for TransformComponent {
    fn from_rune_value(_value: &Value) -> Result<Self, ComponentRegistryError> {
        // For now, just support simple formats
        // We can expand this later to handle the full object format
        let transform = Transform::default();

        // Try to deserialize as a simple format for testing
        // In a full implementation, we'd parse the object structure properly
        // For now, return default transform
        // TODO: Implement proper deserialization when we need set_component

        Ok(TransformComponent(transform))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Vec3, Quat};

    #[test]
    fn test_name_component_roundtrip() {
        let name = Name("TestEntity".to_string());
        let value = name.to_rune_value().unwrap();
        let restored = Name::from_rune_value(&value).unwrap();
        assert_eq!(name.0, restored.0);
    }

    #[test]
    fn test_transform_component_roundtrip() {
        let mut transform = Transform::default();
        transform.translation = Vec3::new(1.0, 2.0, 3.0);
        transform.rotation = Quat::from_euler(glam::EulerRot::YXZ, 0.5, 0.3, 0.1);
        transform.scale = Vec3::new(2.0, 2.0, 2.0);

        let component = TransformComponent(transform);
        let value = component.to_rune_value().unwrap();
        let restored = TransformComponent::from_rune_value(&value).unwrap();

        assert!((restored.0.translation - transform.translation).length() < 1e-6);
        assert!(restored.0.rotation.abs_diff_eq(transform.rotation, 1e-6));
        assert!((restored.0.scale - transform.scale).length() < 1e-6);
    }

    #[test]
    fn test_registry_get_set() {
        let registry = ComponentRegistry::new();
        let mut world = World::new();

        let entity = world.spawn((Name("Test".to_string()),));

        // Get component
        let value = registry.get_component(&world, entity, "Name").unwrap();
        let name = Name::from_rune_value(&value).unwrap();
        assert_eq!(name.0, "Test");

        // Set component
        let new_name = Name("Updated".to_string());
        let new_value = new_name.to_rune_value().unwrap();
        registry.set_component(&mut world, entity, "Name", &new_value).unwrap();

        let updated = world.get::<&Name>(entity).unwrap();
        assert_eq!(updated.0, "Updated");
    }

    #[test]
    fn test_registry_has_component() {
        let registry = ComponentRegistry::new();
        let mut world = World::new();

        let entity = world.spawn((Name("Test".to_string()),));

        assert!(registry.has_component(&world, entity, "Name").unwrap());
        assert!(!registry.has_component(&world, entity, "TransformComponent").unwrap());
    }
}
