use glam::Vec3;
use hecs::Entity;
use rune::runtime::VmResult;

use crate::scene::TransformComponent;
use crate::scripting::rune::commands::entity_bits;
use crate::scripting::rune::guards::{with_active_registry, with_active_world};

/// Query all entities that have a specific component.
///
/// # Arguments
/// * `component_name` - The name of the component to search for (e.g., "MeshComponent", "CameraComponent")
///
/// Returns an array of entity handles that have the specified component.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     // Find all entities with cameras
///     let cameras = query_entities_with_component("CameraComponent");
///     log_info(`Found ${cameras.len()} cameras`);
///
///     // Find all entities with meshes
///     let meshes = query_entities_with_component("MeshComponent");
///     for entity in meshes {
///         // Do something with each mesh entity
///     }
/// }
/// ```
#[rune::function]
pub(crate) fn query_entities_with_component(component_name: String) -> VmResult<rune::alloc::Vec<i64>> {
    with_active_world(|world| {
        with_active_registry(|registry| {
            let mut result = rune::alloc::Vec::new();

            // Check if component exists in registry
            if !registry.is_registered(&component_name) {
                return VmResult::Ok(result);
            }

            // Iterate all entities and check if they have the component
            for entity_ref in world.iter() {
                let entity = entity_ref.entity();
                match registry.has_component(world, entity, &component_name) {
                    Ok(true) => {
                        let handle = entity_bits(entity);
                        if let Err(e) = result.try_push(handle) {
                            return VmResult::Err(e.into());
                        }
                    }
                    Ok(false) => {}
                    Err(_) => {}
                }
            }

            VmResult::Ok(result)
        })
    })
}

/// Find all entities within a radius of a point.
///
/// # Arguments
/// * `x`, `y`, `z` - The center position
/// * `radius` - The search radius
///
/// Returns an array of entity handles within the radius.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     // Find all entities within 10 units
///     let nearby = get_entities_in_radius(0.0, 0.0, 0.0, 10.0);
///     log_info(`Found ${nearby.len()} nearby entities`);
///
///     for entity in nearby {
///         // Do something with nearby entities
///     }
/// }
/// ```
#[rune::function]
pub(crate) fn get_entities_in_radius(x: f64, y: f64, z: f64, radius: f64) -> VmResult<rune::alloc::Vec<i64>> {
    with_active_world(|world| {
        let center = Vec3::new(x as f32, y as f32, z as f32);
        let radius_sq = (radius * radius) as f32;
        let mut result = rune::alloc::Vec::new();

        // Query all entities with TransformComponent
        for (entity, transform) in world.query::<&TransformComponent>().iter() {
            let pos = transform.0.translation;
            let dist_sq = center.distance_squared(pos);

            if dist_sq <= radius_sq {
                let handle = entity_bits(entity);
                if let Err(e) = result.try_push(handle) {
                    return VmResult::Err(e.into());
                }
            }
        }

        VmResult::Ok(result)
    })
}

/// Find the nearest entity to a point.
///
/// # Arguments
/// * `x`, `y`, `z` - The position to search from
///
/// Returns the entity handle of the nearest entity, or `None` if no entities found.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     let pos = get_world_translation(self_entity);
///     if pos != None {
///         let nearest = get_nearest_entity(pos[0], pos[1], pos[2]);
///         if nearest != None {
///             log_info(`Nearest entity: ${nearest}`);
///         }
///     }
/// }
/// ```
#[rune::function]
pub(crate) fn get_nearest_entity(x: f64, y: f64, z: f64) -> VmResult<Option<i64>> {
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
            Some((entity, _)) => VmResult::Ok(Some(entity_bits(entity))),
            None => VmResult::Ok(None),
        }
    })
}

/// Find the nearest entity with a specific component.
///
/// # Arguments
/// * `x`, `y`, `z` - The position to search from
/// * `component_name` - The component to filter by
///
/// Returns the entity handle of the nearest entity with the component, or `None` if not found.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     // Find nearest enemy
///     let nearest_enemy = get_nearest_entity_with_component(0.0, 0.0, 0.0, "EnemyTag");
///     if nearest_enemy != None {
///         log_info(`Nearest enemy found!`);
///     }
/// }
/// ```
#[rune::function]
pub(crate) fn get_nearest_entity_with_component(
    x: f64,
    y: f64,
    z: f64,
    component_name: String,
) -> VmResult<Option<i64>> {
    with_active_world(|world| {
        with_active_registry(|registry| {
            let pos = Vec3::new(x as f32, y as f32, z as f32);
            let mut nearest: Option<(Entity, f32)> = None;

            // Check if component exists in registry
            if !registry.is_registered(&component_name) {
                return VmResult::Ok(None);
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
                Some((entity, _)) => VmResult::Ok(Some(entity_bits(entity))),
                None => VmResult::Ok(None),
            }
        })
    })
}

/// Find all entities within an axis-aligned bounding box.
///
/// # Arguments
/// * `min` - Minimum corner of the box as `[x, y, z]`
/// * `max` - Maximum corner of the box as `[x, y, z]`
///
/// Returns an array of entity handles within the box.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     // Find entities in a region
///     let min = [-10.0, 0.0, -10.0];
///     let max = [10.0, 5.0, 10.0];
///     let entities = get_entities_in_box(min, max);
///     log_info(`Found ${entities.len()} entities in box`);
/// }
/// ```
#[rune::function]
pub(crate) fn get_entities_in_box(
    min: rune::alloc::Vec<f64>,
    max: rune::alloc::Vec<f64>,
) -> VmResult<rune::alloc::Vec<i64>> {
    with_active_world(|world| {
        // Validate array sizes
        if min.len() != 3 || max.len() != 3 {
            return VmResult::panic("min and max must be arrays of length 3");
        }

        let min_vec = Vec3::new(min[0] as f32, min[1] as f32, min[2] as f32);
        let max_vec = Vec3::new(max[0] as f32, max[1] as f32, max[2] as f32);
        let mut result = rune::alloc::Vec::new();

        // Query all entities with TransformComponent
        for (entity, transform) in world.query::<&TransformComponent>().iter() {
            let pos = transform.0.translation;

            // Check if position is within bounds
            if pos.x >= min_vec.x && pos.x <= max_vec.x
                && pos.y >= min_vec.y && pos.y <= max_vec.y
                && pos.z >= min_vec.z && pos.z <= max_vec.z
            {
                let handle = entity_bits(entity);
                if let Err(e) = result.try_push(handle) {
                    return VmResult::Err(e.into());
                }
            }
        }

        VmResult::Ok(result)
    })
}
