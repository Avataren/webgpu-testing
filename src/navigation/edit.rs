//! Reading and changing navigation components by name, as JSON: shared by the remote
//! API (`entity_get` / `entity_update`), Lua (`get_component` / `set_component`) and the
//! inspector.

use hecs::{Entity, World};
use serde_json::Value;

use super::components::{NavAgent, NavLink, NavMeshSurface, NavModifier, NavObstacle};
use crate::remote::json::patch_serde;

/// The navigation components' field names in saved scenes and the remote API.
pub const COMPONENT_NAMES: [&str; 5] = [
    "nav_mesh_surface",
    "nav_agent",
    "nav_obstacle",
    "nav_modifier",
    "nav_link",
];

/// The snake_case name of a navigation component, from that or its type name
/// (`"NavAgent"`).
pub fn canonical_name(name: &str) -> Option<&'static str> {
    Some(match name {
        "nav_mesh_surface" | "NavMeshSurface" => "nav_mesh_surface",
        "nav_agent" | "NavAgent" => "nav_agent",
        "nav_obstacle" | "NavObstacle" => "nav_obstacle",
        "nav_modifier" | "NavModifier" => "nav_modifier",
        "nav_link" | "NavLink" => "nav_link",
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
        "nav_mesh_surface" => get::<NavMeshSurface>(world, entity),
        "nav_agent" => get::<NavAgent>(world, entity),
        "nav_obstacle" => get::<NavObstacle>(world, entity),
        "nav_modifier" => get::<NavModifier>(world, entity),
        "nav_link" => get::<NavLink>(world, entity),
        _ => None,
    }
}

pub fn has_component(world: &World, entity: Entity, name: &str) -> bool {
    match canonical_name(name) {
        Some("nav_mesh_surface") => world.satisfies::<&NavMeshSurface>(entity),
        Some("nav_agent") => world.satisfies::<&NavAgent>(entity),
        Some("nav_obstacle") => world.satisfies::<&NavObstacle>(entity),
        Some("nav_modifier") => world.satisfies::<&NavModifier>(entity),
        Some("nav_link") => world.satisfies::<&NavLink>(entity),
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
        Some("nav_mesh_surface") => patch::<NavMeshSurface>(world, entity, value),
        Some("nav_agent") => patch::<NavAgent>(world, entity, value),
        Some("nav_obstacle") => patch::<NavObstacle>(world, entity, value),
        Some("nav_modifier") => patch::<NavModifier>(world, entity, value),
        Some("nav_link") => patch::<NavLink>(world, entity, value),
        _ => Err(format!("{name:?} is not a navigation component")),
    }
}

/// Copy every navigation component of `source` into `builder` (entity duplication).
pub fn clone_components(world: &World, source: Entity, builder: &mut hecs::EntityBuilder) {
    if let Ok(component) = world.get::<&NavMeshSurface>(source) {
        builder.add((*component).clone());
    }
    if let Ok(component) = world.get::<&NavAgent>(source) {
        builder.add((*component).clone());
    }
    if let Ok(component) = world.get::<&NavObstacle>(source) {
        builder.add((*component).clone());
    }
    if let Ok(component) = world.get::<&NavModifier>(source) {
        builder.add((*component).clone());
    }
    if let Ok(component) = world.get::<&NavLink>(source) {
        builder.add((*component).clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn patches_merge_and_null_removes() {
        let mut world = World::new();
        let entity = world.spawn(());
        patch_component(&mut world, entity, "NavAgent", &json!({"speed": 3.0})).unwrap();
        patch_component(&mut world, entity, "nav_agent", &json!({"radius": 0.5})).unwrap();
        {
            let agent = world.get::<&NavAgent>(entity).unwrap();
            assert_eq!(agent.speed, 3.0);
            assert_eq!(agent.radius, 0.5);
        }
        assert!(has_component(&world, entity, "nav_agent"));
        patch_component(&mut world, entity, "nav_agent", &Value::Null).unwrap();
        assert!(!has_component(&world, entity, "nav_agent"));
        assert!(patch_component(&mut world, entity, "collider", &json!({})).is_err());
    }
}
