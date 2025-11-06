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
use crate::scene::components::{
    Visible, CameraComponent, PointLight, DirectionalLight, SpotLight,
    RotateAnimation, OrbitAnimation, Parent, Children,
    MeshComponent, MaterialComponent, PrimitiveMeshComponent,
    ParticleEmitterComponent, CanCastShadow,
};

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

        // Register core components
        registry.register::<Name>("Name");
        registry.register::<TransformComponent>("TransformComponent");
        registry.register::<TransformComponent>("Transform"); // Alias
        registry.register::<Visible>("Visible");

        // Register camera
        registry.register::<CameraComponent>("CameraComponent");
        registry.register::<CameraComponent>("Camera"); // Alias

        // Register lights
        registry.register::<PointLight>("PointLight");
        registry.register::<DirectionalLight>("DirectionalLight");
        registry.register::<SpotLight>("SpotLight");
        registry.register::<CanCastShadow>("CanCastShadow");

        // Register rendering components
        registry.register::<MeshComponent>("MeshComponent");
        registry.register::<MeshComponent>("Mesh"); // Alias
        registry.register::<MaterialComponent>("MaterialComponent");
        registry.register::<MaterialComponent>("Material"); // Alias
        registry.register::<PrimitiveMeshComponent>("PrimitiveMeshComponent");

        // Register animation components
        registry.register::<RotateAnimation>("RotateAnimation");
        registry.register::<OrbitAnimation>("OrbitAnimation");

        // Register hierarchy components
        registry.register::<Parent>("Parent");
        registry.register::<Children>("Children");

        // Register particle components
        registry.register::<ParticleEmitterComponent>("ParticleEmitterComponent");

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

    /// Check if a component type is registered.
    pub fn is_registered(&self, component_name: &str) -> bool {
        self.handlers.contains_key(component_name)
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

// ============================================================================
// Visible Component
// ============================================================================

impl ToRuneValue for Visible {
    fn to_rune_value(&self) -> Result<Value, ComponentRegistryError> {
        Ok(Value::from(self.0))
    }
}

impl FromRuneValue for Visible {
    fn from_rune_value(value: &Value) -> Result<Self, ComponentRegistryError> {
        let visible = rune::from_value::<bool>(value.clone())
            .map_err(|e| ComponentRegistryError::FromRuneValue(format!("expected bool: {}", e)))?;
        Ok(Visible(visible))
    }
}

// ============================================================================
// Camera Component
// ============================================================================

impl ToRuneValue for CameraComponent {
    fn to_rune_value(&self) -> Result<Value, ComponentRegistryError> {
        use rune::runtime::Object;

        let mut obj = Object::new();

        // Expose projection data
        obj.insert(RuneString::try_from("near")?, Value::from(self.near() as f64))?;
        obj.insert(RuneString::try_from("far")?, Value::from(self.far() as f64))?;

        Ok(rune::to_value(obj)?)
    }
}

impl FromRuneValue for CameraComponent {
    fn from_rune_value(_value: &Value) -> Result<Self, ComponentRegistryError> {
        // Return default camera for now
        // TODO: Parse projection parameters
        Ok(CameraComponent::default())
    }
}

// ============================================================================
// Light Components
// ============================================================================

impl ToRuneValue for PointLight {
    fn to_rune_value(&self) -> Result<Value, ComponentRegistryError> {
        use rune::runtime::Object;
        use rune::alloc::Vec as RuneVec;

        let mut obj = Object::new();

        // Color as array
        let mut color_vec = RuneVec::new();
        color_vec.try_push(Value::from(self.color.x as f64))?;
        color_vec.try_push(Value::from(self.color.y as f64))?;
        color_vec.try_push(Value::from(self.color.z as f64))?;
        obj.insert(RuneString::try_from("color")?, rune::to_value(color_vec)?)?;

        obj.insert(RuneString::try_from("intensity")?, Value::from(self.intensity as f64))?;
        obj.insert(RuneString::try_from("range")?, Value::from(self.range as f64))?;

        Ok(rune::to_value(obj)?)
    }
}

impl FromRuneValue for PointLight {
    fn from_rune_value(_value: &Value) -> Result<Self, ComponentRegistryError> {
        // TODO: Parse light parameters
        Ok(PointLight {
            color: glam::Vec3::ONE,
            intensity: 1.0,
            range: 10.0,
        })
    }
}

impl ToRuneValue for DirectionalLight {
    fn to_rune_value(&self) -> Result<Value, ComponentRegistryError> {
        use rune::runtime::Object;
        use rune::alloc::Vec as RuneVec;

        let mut obj = Object::new();

        // Color as array
        let mut color_vec = RuneVec::new();
        color_vec.try_push(Value::from(self.color.x as f64))?;
        color_vec.try_push(Value::from(self.color.y as f64))?;
        color_vec.try_push(Value::from(self.color.z as f64))?;
        obj.insert(RuneString::try_from("color")?, rune::to_value(color_vec)?)?;

        obj.insert(RuneString::try_from("intensity")?, Value::from(self.intensity as f64))?;
        obj.insert(RuneString::try_from("shadow_size")?, Value::from(self.shadow_size as f64))?;

        Ok(rune::to_value(obj)?)
    }
}

impl FromRuneValue for DirectionalLight {
    fn from_rune_value(_value: &Value) -> Result<Self, ComponentRegistryError> {
        // TODO: Parse light parameters
        Ok(DirectionalLight {
            color: glam::Vec3::ONE,
            intensity: 1.0,
            shadow_size: DirectionalLight::DEFAULT_SHADOW_SIZE,
        })
    }
}

impl ToRuneValue for SpotLight {
    fn to_rune_value(&self) -> Result<Value, ComponentRegistryError> {
        use rune::runtime::Object;
        use rune::alloc::Vec as RuneVec;

        let mut obj = Object::new();

        // Color as array
        let mut color_vec = RuneVec::new();
        color_vec.try_push(Value::from(self.color.x as f64))?;
        color_vec.try_push(Value::from(self.color.y as f64))?;
        color_vec.try_push(Value::from(self.color.z as f64))?;
        obj.insert(RuneString::try_from("color")?, rune::to_value(color_vec)?)?;

        obj.insert(RuneString::try_from("intensity")?, Value::from(self.intensity as f64))?;
        obj.insert(RuneString::try_from("inner_angle")?, Value::from(self.inner_angle as f64))?;
        obj.insert(RuneString::try_from("outer_angle")?, Value::from(self.outer_angle as f64))?;
        obj.insert(RuneString::try_from("range")?, Value::from(self.range as f64))?;

        Ok(rune::to_value(obj)?)
    }
}

impl FromRuneValue for SpotLight {
    fn from_rune_value(_value: &Value) -> Result<Self, ComponentRegistryError> {
        // TODO: Parse light parameters
        Ok(SpotLight {
            color: glam::Vec3::ONE,
            intensity: 1.0,
            inner_angle: std::f32::consts::PI / 8.0,
            outer_angle: std::f32::consts::PI / 4.0,
            range: 10.0,
        })
    }
}

impl ToRuneValue for CanCastShadow {
    fn to_rune_value(&self) -> Result<Value, ComponentRegistryError> {
        Ok(Value::from(self.0))
    }
}

impl FromRuneValue for CanCastShadow {
    fn from_rune_value(value: &Value) -> Result<Self, ComponentRegistryError> {
        let can_cast = rune::from_value::<bool>(value.clone())
            .map_err(|e| ComponentRegistryError::FromRuneValue(format!("expected bool: {}", e)))?;
        Ok(CanCastShadow(can_cast))
    }
}

// ============================================================================
// Mesh and Material Components
// ============================================================================

impl ToRuneValue for MeshComponent {
    fn to_rune_value(&self) -> Result<Value, ComponentRegistryError> {
        // Return handle index as i64
        Ok(Value::from(self.0.index() as i64))
    }
}

impl FromRuneValue for MeshComponent {
    fn from_rune_value(value: &Value) -> Result<Self, ComponentRegistryError> {
        let index = rune::from_value::<i64>(value.clone())
            .map_err(|e| ComponentRegistryError::FromRuneValue(format!("expected handle index: {}", e)))?;
        Ok(MeshComponent(crate::asset::Handle::new(index as usize)))
    }
}

impl ToRuneValue for MaterialComponent {
    fn to_rune_value(&self) -> Result<Value, ComponentRegistryError> {
        // Return handle index as i64
        Ok(Value::from(self.0.index() as i64))
    }
}

impl FromRuneValue for MaterialComponent {
    fn from_rune_value(value: &Value) -> Result<Self, ComponentRegistryError> {
        let index = rune::from_value::<i64>(value.clone())
            .map_err(|e| ComponentRegistryError::FromRuneValue(format!("expected handle index: {}", e)))?;
        Ok(MaterialComponent(crate::asset::Handle::new(index as usize)))
    }
}

impl ToRuneValue for PrimitiveMeshComponent {
    fn to_rune_value(&self) -> Result<Value, ComponentRegistryError> {
        use crate::renderer::primitives::PrimitiveMeshDescriptor;

        let type_name = match self.descriptor {
            PrimitiveMeshDescriptor::Cube => "cube",
            PrimitiveMeshDescriptor::Sphere => "sphere",
            PrimitiveMeshDescriptor::Plane => "plane",
            PrimitiveMeshDescriptor::Cylinder => "cylinder",
            PrimitiveMeshDescriptor::Cone => "cone",
            PrimitiveMeshDescriptor::Torus => "torus",
        };

        Ok(rune::to_value(type_name)?)
    }
}

impl FromRuneValue for PrimitiveMeshComponent {
    fn from_rune_value(_value: &Value) -> Result<Self, ComponentRegistryError> {
        // Default to cube
        use crate::renderer::primitives::PrimitiveMeshDescriptor;
        Ok(PrimitiveMeshComponent {
            descriptor: PrimitiveMeshDescriptor::Cube,
        })
    }
}

// ============================================================================
// Animation Components
// ============================================================================

impl ToRuneValue for RotateAnimation {
    fn to_rune_value(&self) -> Result<Value, ComponentRegistryError> {
        use rune::runtime::Object;
        use rune::alloc::Vec as RuneVec;

        let mut obj = Object::new();

        // Axis as array
        let mut axis_vec = RuneVec::new();
        axis_vec.try_push(Value::from(self.axis.x as f64))?;
        axis_vec.try_push(Value::from(self.axis.y as f64))?;
        axis_vec.try_push(Value::from(self.axis.z as f64))?;
        obj.insert(RuneString::try_from("axis")?, rune::to_value(axis_vec)?)?;

        obj.insert(RuneString::try_from("speed")?, Value::from(self.speed as f64))?;

        Ok(rune::to_value(obj)?)
    }
}

impl FromRuneValue for RotateAnimation {
    fn from_rune_value(_value: &Value) -> Result<Self, ComponentRegistryError> {
        // Default rotation around Y axis
        Ok(RotateAnimation {
            axis: glam::Vec3::Y,
            speed: 1.0,
        })
    }
}

impl ToRuneValue for OrbitAnimation {
    fn to_rune_value(&self) -> Result<Value, ComponentRegistryError> {
        use rune::runtime::Object;
        use rune::alloc::Vec as RuneVec;

        let mut obj = Object::new();

        // Center as array
        let mut center_vec = RuneVec::new();
        center_vec.try_push(Value::from(self.center.x as f64))?;
        center_vec.try_push(Value::from(self.center.y as f64))?;
        center_vec.try_push(Value::from(self.center.z as f64))?;
        obj.insert(RuneString::try_from("center")?, rune::to_value(center_vec)?)?;

        obj.insert(RuneString::try_from("radius")?, Value::from(self.radius as f64))?;
        obj.insert(RuneString::try_from("speed")?, Value::from(self.speed as f64))?;
        obj.insert(RuneString::try_from("offset")?, Value::from(self.offset as f64))?;

        Ok(rune::to_value(obj)?)
    }
}

impl FromRuneValue for OrbitAnimation {
    fn from_rune_value(_value: &Value) -> Result<Self, ComponentRegistryError> {
        // Default orbit
        Ok(OrbitAnimation {
            center: glam::Vec3::ZERO,
            radius: 5.0,
            speed: 1.0,
            offset: 0.0,
        })
    }
}

// ============================================================================
// Hierarchy Components
// ============================================================================

impl ToRuneValue for Parent {
    fn to_rune_value(&self) -> Result<Value, ComponentRegistryError> {
        // Return entity bits as i64
        Ok(Value::from(self.0.to_bits().get() as i64))
    }
}

impl FromRuneValue for Parent {
    fn from_rune_value(value: &Value) -> Result<Self, ComponentRegistryError> {
        let bits = rune::from_value::<i64>(value.clone())
            .map_err(|e| ComponentRegistryError::FromRuneValue(format!("expected entity bits: {}", e)))?;
        let entity = hecs::Entity::from_bits(bits as u64)
            .ok_or_else(|| ComponentRegistryError::FromRuneValue("invalid entity bits".to_string()))?;
        Ok(Parent(entity))
    }
}

impl ToRuneValue for Children {
    fn to_rune_value(&self) -> Result<Value, ComponentRegistryError> {
        use rune::alloc::Vec as RuneVec;

        let mut children_vec = RuneVec::new();
        for child in &self.0 {
            children_vec.try_push(Value::from(child.to_bits().get() as i64))?;
        }

        Ok(rune::to_value(children_vec)?)
    }
}

impl FromRuneValue for Children {
    fn from_rune_value(_value: &Value) -> Result<Self, ComponentRegistryError> {
        // Return empty children for now
        // TODO: Parse array of entity bits
        Ok(Children(Vec::new()))
    }
}

// ============================================================================
// Particle System Components
// ============================================================================

impl ToRuneValue for ParticleEmitterComponent {
    fn to_rune_value(&self) -> Result<Value, ComponentRegistryError> {
        use rune::runtime::Object;

        let mut obj = Object::new();

        obj.insert(RuneString::try_from("spawn_rate")?, Value::from(self.spawn_rate as f64))?;
        obj.insert(RuneString::try_from("auto_respawn")?, Value::from(self.auto_respawn))?;

        if let Some(burst) = self.burst_count {
            obj.insert(RuneString::try_from("burst_count")?, Value::from(burst as i64))?;
        }

        Ok(rune::to_value(obj)?)
    }
}

impl FromRuneValue for ParticleEmitterComponent {
    fn from_rune_value(_value: &Value) -> Result<Self, ComponentRegistryError> {
        // Return default emitter
        Ok(ParticleEmitterComponent::default())
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
