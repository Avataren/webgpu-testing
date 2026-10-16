//! Reading and changing physics components by name, as JSON: shared by the remote API
//! (`entity_get` / `entity_update`) and Lua (`get_component` / `set_component`).

use hecs::{Entity, World};
use serde_json::Value;

use super::components::{CharacterController, Collider, RigidBody, SpringCamera};
use crate::remote::json::patch_serde;

/// The physics components' field names in saved scenes and the remote API.
pub const COMPONENT_NAMES: [&str; 4] = [
    "rigid_body",
    "collider",
    "character_controller",
    "spring_camera",
];

/// The snake_case name of a physics component, from that or its type name
/// (`"CharacterController"`).
pub fn canonical_name(name: &str) -> Option<&'static str> {
    Some(match name {
        "rigid_body" | "RigidBody" => "rigid_body",
        "collider" | "Collider" => "collider",
        "character_controller" | "CharacterController" => "character_controller",
        "spring_camera" | "SpringCamera" => "spring_camera",
        _ => return None,
    })
}

fn get<T: hecs::Component + serde::Serialize>(world: &World, entity: Entity) -> Option<Value> {
    let component = world.get::<&T>(entity).ok()?;
    serde_json::to_value(&*component).ok()
}

/// The named component as JSON, or `None` when the entity has none.
pub fn component_json(world: &World, entity: Entity, name: &str) -> Option<Value> {
    match canonical_name(name)? {
        "rigid_body" => get::<RigidBody>(world, entity),
        "collider" => get::<Collider>(world, entity),
        "character_controller" => get::<CharacterController>(world, entity),
        "spring_camera" => get::<SpringCamera>(world, entity),
        _ => None,
    }
}

pub fn has_component(world: &World, entity: Entity, name: &str) -> bool {
    match canonical_name(name) {
        Some("rigid_body") => world.satisfies::<&RigidBody>(entity),
        Some("collider") => world.satisfies::<&Collider>(entity),
        Some("character_controller") => world.satisfies::<&CharacterController>(entity),
        Some("spring_camera") => world.satisfies::<&SpringCamera>(entity),
        _ => Ok(false),
    }
    .unwrap_or(false)
}

fn patch<T>(world: &mut World, entity: Entity, value: &Value) -> Result<(), String>
where
    T: hecs::Component + Clone + Default + serde::Serialize + serde::de::DeserializeOwned,
{
    if value.is_null() {
        return world
            .remove_one::<T>(entity)
            .map(|_| ())
            .map_err(|err| err.to_string());
    }
    if !value.is_object() {
        return Err(format!("expected an object or null, got {value}"));
    }
    let current = world
        .get::<&T>(entity)
        .map(|component| (*component).clone())
        .unwrap_or_default();
    let patched = patch_serde(&current, value)?;
    world
        .insert_one(entity, patched)
        .map_err(|err| err.to_string())
}

/// Add the named component or change the fields `value` gives (the rest keep their
/// values, or the defaults for a new component); `null` removes it.
pub fn patch_component(
    world: &mut World,
    entity: Entity,
    name: &str,
    value: &Value,
) -> Result<(), String> {
    match canonical_name(name) {
        Some("rigid_body") => patch::<RigidBody>(world, entity, value),
        Some("collider") => patch::<Collider>(world, entity, value),
        Some("character_controller") => patch::<CharacterController>(world, entity, value),
        Some("spring_camera") => patch::<SpringCamera>(world, entity, value),
        _ => Err(format!("{name:?} is not a physics component")),
    }
}

/// Copy every physics component of `source` into `builder` (entity duplication).
pub fn clone_components(world: &World, source: Entity, builder: &mut hecs::EntityBuilder) {
    if let Ok(component) = world.get::<&RigidBody>(source) {
        builder.add((*component).clone());
    }
    if let Ok(component) = world.get::<&Collider>(source) {
        builder.add((*component).clone());
    }
    if let Ok(component) = world.get::<&CharacterController>(source) {
        builder.add((*component).clone());
    }
    if let Ok(component) = world.get::<&SpringCamera>(source) {
        builder.add((*component).clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::ColliderShape;
    use serde_json::json;

    #[test]
    fn patches_merge_and_null_removes() {
        let mut world = World::new();
        let entity = world.spawn(());
        patch_component(
            &mut world,
            entity,
            "Collider",
            &json!({"shape": {"kind": "box", "size": [2, 1, 2]}}),
        )
        .unwrap();
        patch_component(&mut world, entity, "collider", &json!({"friction": 0.1})).unwrap();
        {
            let collider = world.get::<&Collider>(entity).unwrap();
            assert_eq!(collider.friction, 0.1);
            assert_eq!(
                collider.shape,
                ColliderShape::Box {
                    size: [2.0, 1.0, 2.0]
                }
            );
        }
        // Switching the shape kind keeps nothing of the old one's dimensions.
        patch_component(
            &mut world,
            entity,
            "collider",
            &json!({"shape": {"kind": "sphere"}}),
        )
        .unwrap();
        assert_eq!(
            world.get::<&Collider>(entity).unwrap().shape,
            ColliderShape::Sphere { radius: 0.5 }
        );
        assert!(has_component(&world, entity, "collider"));
        patch_component(&mut world, entity, "collider", &Value::Null).unwrap();
        assert!(!has_component(&world, entity, "collider"));
        assert!(component_json(&world, entity, "collider").is_none());
    }
}
