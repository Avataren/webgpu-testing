use glam::Vec3;
use hecs::Entity;
use mlua::{Lua, Result as LuaResult, Table as LuaTable};

use crate::scene::TransformComponent;
use crate::scripting::lua::commands::entity_bits;
use crate::scripting::lua::guards::{with_active_registry, with_active_world};

/// Register query API functions with the Lua runtime.
pub(crate) fn register_query_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    // query_entities_with_component(component_name: string) -> table
    globals.set(
        "query_entities_with_component",
        lua.create_function(|lua, component_name: String| {
            with_active_world(|world| {
                with_active_registry(|registry| {
                    let table = lua.create_table()?;

                    // Check if component exists in registry
                    if !registry.is_registered(&component_name) {
                        return Ok(table);
                    }

                    // Iterate all entities and check if they have the component
                    let mut index = 1;
                    for entity_ref in world.iter() {
                        let entity = entity_ref.entity();
                        match registry.has_component(world, entity, &component_name) {
                            Ok(true) => {
                                let handle = entity_bits(entity);
                                table.raw_set(index, handle)?;
                                index += 1;
                            }
                            Ok(false) => {}
                            Err(_) => {}
                        }
                    }

                    Ok(table)
                })
            })
        })?,
    )?;

    // get_entities_in_radius(x: number, y: number, z: number, radius: number) -> table
    globals.set(
        "get_entities_in_radius",
        lua.create_function(|lua, (x, y, z, radius): (f64, f64, f64, f64)| {
            with_active_world(|world| {
                let center = Vec3::new(x as f32, y as f32, z as f32);
                let radius_sq = (radius * radius) as f32;
                let table = lua.create_table()?;

                // Query all entities with TransformComponent
                let mut index = 1;
                for (entity, transform) in world.query::<&TransformComponent>().iter() {
                    let pos = transform.0.translation;
                    let dist_sq = center.distance_squared(pos);

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

    // get_nearest_entity(x: number, y: number, z: number) -> number | nil
    globals.set(
        "get_nearest_entity",
        lua.create_function(|_, (x, y, z): (f64, f64, f64)| {
            with_active_world(|world| {
                let pos = Vec3::new(x as f32, y as f32, z as f32);
                let mut nearest: Option<(Entity, f32)> = None;

                // Query all entities with TransformComponent
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
                with_active_registry(|registry| {
                    let pos = Vec3::new(x as f32, y as f32, z as f32);
                    let mut nearest: Option<(Entity, f32)> = None;

                    // Check if component exists in registry
                    if !registry.is_registered(&component_name) {
                        return Ok(None);
                    }

                    // Query all entities with TransformComponent
                    for (entity, transform) in world.query::<&TransformComponent>().iter() {
                        // Check if entity has the required component
                        match registry.has_component(world, entity, &component_name) {
                            Ok(true) => {
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
                            _ => continue,
                        }
                    }

                    match nearest {
                        Some((entity, _)) => Ok(Some(entity_bits(entity))),
                        None => Ok(None),
                    }
                })
            })
        })?,
    )?;

    // get_entities_in_box(min: table, max: table) -> table
    globals.set(
        "get_entities_in_box",
        lua.create_function(|lua, (min, max): (LuaTable, LuaTable)| {
            with_active_world(|world| {
                // Extract min/max coordinates
                let min_x: f64 = min.get(1)?;
                let min_y: f64 = min.get(2)?;
                let min_z: f64 = min.get(3)?;
                let max_x: f64 = max.get(1)?;
                let max_y: f64 = max.get(2)?;
                let max_z: f64 = max.get(3)?;

                let min_vec = Vec3::new(min_x as f32, min_y as f32, min_z as f32);
                let max_vec = Vec3::new(max_x as f32, max_y as f32, max_z as f32);
                let table = lua.create_table()?;

                // Query all entities with TransformComponent
                let mut index = 1;
                for (entity, transform) in world.query::<&TransformComponent>().iter() {
                    let pos = transform.0.translation;

                    // Check if position is within bounds
                    if pos.x >= min_vec.x
                        && pos.x <= max_vec.x
                        && pos.y >= min_vec.y
                        && pos.y <= max_vec.y
                        && pos.z >= min_vec.z
                        && pos.z <= max_vec.z
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

    Ok(())
}
