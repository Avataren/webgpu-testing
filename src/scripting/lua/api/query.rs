//! # Query API
//!
//! This module provides functions for querying entities in the scene.
//!
//! ## Features
//!
//! - **Component queries** - Find all entities with a specific component (not yet implemented)
//! - **Spatial queries** - Find entities by position (radius, box, nearest)
//! - **Combined queries** - Find nearest entity with specific component (not yet implemented)
//!
//! ## Array Indexing
//!
//! All query results return **1-indexed tables** (Lua convention).
//! Use `#table` to get the count and `table[i]` to access elements (starting at 1).
//!
//! ## Performance Notes
//!
//! - Component queries iterate all entities in the world
//! - Spatial queries only check entities with `TransformComponent`
//! - For repeated queries, consider caching results when appropriate

use glam::Vec3;
use hecs::Entity;
use mlua::{Lua, Result as LuaResult, Table as LuaTable};

use crate::scene::TransformComponent;
use crate::scripting::lua::commands::{component_kind_from_name, component_matches, entity_bits};
use crate::scripting::lua::guards::with_active_world;

/// Registers query API functions with the Lua runtime.
///
/// This function exposes entity query functions to Lua scripts.
///
/// ## Available Functions
///
/// ### Component Queries
/// - `query_entities_with_component(component_name)` - Returns 1-indexed table (not yet implemented)
///
/// ### Spatial Queries
/// - `get_entities_in_radius(x, y, z, radius)` - Returns entities within radius
/// - `get_entities_in_box(min_table, max_table)` - Returns entities in AABB
/// - `get_nearest_entity(x, y, z)` - Returns nearest entity or nil
/// - `get_nearest_entity_with_component(x, y, z, component_name)` - Combined query (not yet implemented)
///
/// # Example Lua usage
///
/// ```lua
/// -- Spatial queries work normally
/// local nearby = get_entities_in_radius(0, 0, 0, 10.0)
/// log_info("Found " .. #nearby .. " nearby entities")
/// ```
///
/// # Arguments
///
/// * `lua` - The Lua runtime to register functions with
///
/// # Returns
///
/// `Ok(())` on success, or a Lua error if registration fails.
pub(crate) fn register_query_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    // query_entities_with_component(component_name: string) -> table
    globals.set(
        "query_entities_with_component",
        lua.create_function(|lua, component_name: String| {
            with_active_world(|world| {
                let kind = component_kind_from_name(&component_name).ok_or_else(|| {
                    mlua::Error::RuntimeError(format!(
                        "Unknown or unsupported component type: {}",
                        component_name
                    ))
                })?;
                let table = lua.create_table()?;
                let mut index = 1;

                for entity_ref in world.iter() {
                    let entity = entity_ref.entity();
                    if component_matches(world, entity, kind) {
                        let handle = entity_bits(entity);
                        table.raw_set(index, handle)?;
                        index += 1;
                    }
                }

                Ok(table)
            })
        })?,
    )?;

    // get_entities_in_radius(x: number, y: number, z: number, radius: number) -> table
    globals.set(
        "get_entities_in_radius",
        lua.create_function(|lua, (x, y, z, radius): (f64, f64, f64, f64)| {
            with_active_world(|world| {
                let table = lua.create_table()?;
                let pos = Vec3::new(x as f32, y as f32, z as f32);
                let radius_sq = (radius as f32).powi(2);

                let mut index = 1;
                for (entity, transform) in world.query::<&TransformComponent>().iter() {
                    let entity_pos = transform.0.translation;
                    let dist_sq = pos.distance_squared(entity_pos);

                    if dist_sq <= radius_sq {
                        let handle = entity_bits(entity);
                        table.raw_set(index, handle)?;
                        index += 1;
                    }
                }

                Ok(table)
            })
        })?,
    )?;

    // get_entities_in_box(min: table, max: table) -> table
    globals.set(
        "get_entities_in_box",
        lua.create_function(|lua, (min_table, max_table): (LuaTable, LuaTable)| {
            with_active_world(|world| {
                let table = lua.create_table()?;

                // Extract min/max vectors from Lua tables
                let min = Vec3::new(
                    min_table.get::<f64>(1)? as f32,
                    min_table.get::<f64>(2)? as f32,
                    min_table.get::<f64>(3)? as f32,
                );
                let max = Vec3::new(
                    max_table.get::<f64>(1)? as f32,
                    max_table.get::<f64>(2)? as f32,
                    max_table.get::<f64>(3)? as f32,
                );

                let mut index = 1;
                for (entity, transform) in world.query::<&TransformComponent>().iter() {
                    let pos = transform.0.translation;

                    // Check if position is within AABB
                    if pos.x >= min.x
                        && pos.x <= max.x
                        && pos.y >= min.y
                        && pos.y <= max.y
                        && pos.z >= min.z
                        && pos.z <= max.z
                    {
                        let handle = entity_bits(entity);
                        table.raw_set(index, handle)?;
                        index += 1;
                    }
                }

                Ok(table)
            })
        })?,
    )?;

    // get_nearest_entity(x: number, y: number, z: number) -> number | nil
    globals.set(
        "get_nearest_entity",
        lua.create_function(|_, (x, y, z): (f64, f64, f64)| {
            with_active_world(|world| {
                let pos = Vec3::new(x as f32, y as f32, z as f32);
                let mut nearest: Option<(Entity, f32)> = None;

                for (entity, transform) in world.query::<&TransformComponent>().iter() {
                    let entity_pos = transform.0.translation;
                    let dist_sq = pos.distance_squared(entity_pos);

                    match nearest {
                        None => nearest = Some((entity, dist_sq)),
                        Some((_, best_dist_sq)) => {
                            if dist_sq < best_dist_sq {
                                nearest = Some((entity, dist_sq));
                            }
                        }
                    }
                }

                match nearest {
                    Some((entity, _)) => Ok(Some(entity_bits(entity))),
                    None => Ok(None),
                }
            })
        })?,
    )?;

    // get_nearest_entity_with_component(x: number, y: number, z: number, component_name: string) -> number | nil
    globals.set(
        "get_nearest_entity_with_component",
        lua.create_function(|_, (x, y, z, component_name): (f64, f64, f64, String)| {
            with_active_world(|world| {
                let kind = component_kind_from_name(&component_name).ok_or_else(|| {
                    mlua::Error::RuntimeError(format!(
                        "Unknown or unsupported component type: {}",
                        component_name
                    ))
                })?;
                let pos = Vec3::new(x as f32, y as f32, z as f32);
                let mut nearest: Option<(Entity, f32)> = None;

                for entity_ref in world.iter() {
                    let entity = entity_ref.entity();
                    if !component_matches(world, entity, kind) {
                        continue;
                    }

                    let Ok(transform) = world.get::<&TransformComponent>(entity) else {
                        continue;
                    };
                    let entity_pos = transform.0.translation;
                    let dist_sq = pos.distance_squared(entity_pos);

                    match nearest {
                        None => nearest = Some((entity, dist_sq)),
                        Some((_, best_dist_sq)) => {
                            if dist_sq < best_dist_sq {
                                nearest = Some((entity, dist_sq));
                            }
                        }
                    }
                }

                match nearest {
                    Some((entity, _)) => Ok(Some(entity_bits(entity))),
                    None => Ok(None),
                }
            })
        })?,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Name, Transform, TransformComponent, Visible};
    use crate::scripting::lua::guards::WorldGuard;
    use hecs::World;
    use mlua::Lua;

    #[test]
    fn query_entities_with_component_returns_matching_entities() {
        let lua = Lua::new();
        register_query_api(&lua).unwrap();

        let mut world = World::new();
        let name_entity = world.spawn((Name::new("Alpha"),));
        let _visible_entity = world.spawn((Visible(true),));

        let _guard = WorldGuard::enter(&world);
        let table: LuaTable = lua
            .load("return query_entities_with_component('Name')")
            .eval()
            .unwrap();

        let handles: Vec<i64> = table
            .sequence_values()
            .collect::<LuaResult<Vec<_>>>()
            .unwrap();
        assert_eq!(handles.len(), 1);
        assert_eq!(handles[0], entity_bits(name_entity));
    }

    #[test]
    fn get_nearest_entity_with_component_uses_transform_distance() {
        let lua = Lua::new();
        register_query_api(&lua).unwrap();

        let mut world = World::new();
        let nearest = world.spawn((
            Name::new("Near"),
            TransformComponent(Transform::from_translation(Vec3::new(0.0, 0.0, 0.0))),
        ));
        let _far = world.spawn((
            Name::new("Far"),
            TransformComponent(Transform::from_translation(Vec3::new(10.0, 0.0, 0.0))),
        ));
        let _no_transform = world.spawn((Name::new("NoTransform"),));

        let _guard = WorldGuard::enter(&world);
        let result: Option<i64> = lua
            .load("return get_nearest_entity_with_component(1.0, 0.0, 0.0, 'Name')")
            .eval()
            .unwrap();

        assert_eq!(result, Some(entity_bits(nearest)));
    }
}
