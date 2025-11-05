use hecs::Entity;
use log::{error, warn};
use rune::runtime::VmResult;
use rune::Value;

use crate::scripting::component_registry::ComponentRegistryError;
use crate::scripting::rune::guards::{with_active_registry, with_active_world, ACTIVE_COMMANDS};

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
pub(crate) fn get_component(entity_bits: i64, component_name: String) -> VmResult<Option<Value>> {
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
pub(crate) fn set_component(entity_bits: i64, component_name: String, value: Value) -> VmResult<()> {
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
pub(crate) fn add_component(entity_bits: i64, component_name: String, value: Value) -> VmResult<()> {
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
pub(crate) fn remove_component(entity_bits: i64, component_name: String) -> VmResult<()> {
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
pub(crate) fn has_component(entity_bits: i64, component_name: String) -> VmResult<bool> {
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
