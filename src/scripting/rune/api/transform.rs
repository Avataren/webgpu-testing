use glam::{EulerRot, Vec3};
use hecs::Entity;
use rune::runtime::VmResult;

use crate::scene::TransformComponent;
use crate::scripting::rune::guards::{with_active_world, ACTIVE_COMMANDS};

/// Translate an entity by a delta vector.
///
/// Adds the delta to the entity's current position.
///
/// # Example
/// ```rune
/// translate(entity, 1.0, 0.0, 0.0);  // Move right by 1 unit
/// ```
#[rune::function]
pub(crate) fn translate(entity_bits: i64, x: f64, y: f64, z: f64) -> VmResult<()> {
    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut().with(|commands| {
            commands.translate(entity_bits, Vec3::new(x as f32, y as f32, z as f32))
        })
    })
}

/// Rotate an entity around an axis by an angle in radians.
///
/// # Example
/// ```rune
/// // Rotate 45 degrees around Y axis
/// rotate(entity, 0.0, 1.0, 0.0, 0.785);
/// ```
#[rune::function]
pub(crate) fn rotate(
    entity_bits: i64,
    axis_x: f64,
    axis_y: f64,
    axis_z: f64,
    angle: f64,
) -> VmResult<()> {
    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut().with(|commands| {
            commands.rotate(
                entity_bits,
                Vec3::new(axis_x as f32, axis_y as f32, axis_z as f32),
                angle as f32,
            )
        })
    })
}

/// Set the scale of an entity.
///
/// # Example
/// ```rune
/// set_scale(entity, 2.0, 2.0, 2.0);  // Double the size
/// ```
#[rune::function]
pub(crate) fn set_scale(entity_bits: i64, x: f64, y: f64, z: f64) -> VmResult<()> {
    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut().with(|commands| {
            commands.set_scale(entity_bits, Vec3::new(x as f32, y as f32, z as f32))
        })
    })
}

/// Make an entity look at a target position.
///
/// # Example
/// ```rune
/// look_at(entity, 0.0, 0.0, 10.0);  // Look at point in front
/// ```
#[rune::function]
pub(crate) fn look_at(
    entity_bits: i64,
    target_x: f64,
    target_y: f64,
    target_z: f64,
) -> VmResult<()> {
    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut().with(|commands| {
            commands.look_at(
                entity_bits,
                Vec3::new(target_x as f32, target_y as f32, target_z as f32),
            )
        })
    })
}

/// Get the world-space translation of an entity.
///
/// Returns an array [x, y, z] of the entity's world position.
/// Note: Currently returns local translation. World transform will be added later.
///
/// # Example
/// ```rune
/// let pos = get_world_translation(entity);
/// if pos != None {
///     log_info("Got position");
/// }
/// ```
#[rune::function]
pub(crate) fn get_world_translation(entity_bits: i64) -> VmResult<Option<rune::alloc::Vec<f64>>> {
    with_active_world(|world| {
        let entity = match Entity::from_bits(entity_bits as u64) {
            Some(e) => e,
            None => return VmResult::Ok(None),
        };

        // Get local transform for now
        if let Ok(transform) = world.get::<&TransformComponent>(entity) {
            let translation = transform.0.translation;
            let mut vec = rune::alloc::Vec::new();
            if let Err(e) = vec.try_push(translation.x as f64) {
                return VmResult::Err(e.into());
            }
            if let Err(e) = vec.try_push(translation.y as f64) {
                return VmResult::Err(e.into());
            }
            if let Err(e) = vec.try_push(translation.z as f64) {
                return VmResult::Err(e.into());
            }
            return VmResult::Ok(Some(vec));
        }

        VmResult::Ok(None)
    })
}

/// Get the world-space rotation of an entity as euler angles.
///
/// Returns an array [yaw, pitch, roll] in radians.
/// Note: Currently returns local rotation. World transform will be added later.
///
/// # Example
/// ```rune
/// let rot = get_world_rotation(entity);
/// if rot != None {
///     log_info("Got rotation");
/// }
/// ```
#[rune::function]
pub(crate) fn get_world_rotation(entity_bits: i64) -> VmResult<Option<rune::alloc::Vec<f64>>> {
    with_active_world(|world| {
        let entity = match Entity::from_bits(entity_bits as u64) {
            Some(e) => e,
            None => return VmResult::Ok(None),
        };

        // Get local transform for now
        if let Ok(transform) = world.get::<&TransformComponent>(entity) {
            let (yaw, pitch, roll) = transform.0.rotation.to_euler(EulerRot::YXZ);
            let mut vec = rune::alloc::Vec::new();
            if let Err(e) = vec.try_push(yaw as f64) {
                return VmResult::Err(e.into());
            }
            if let Err(e) = vec.try_push(pitch as f64) {
                return VmResult::Err(e.into());
            }
            if let Err(e) = vec.try_push(roll as f64) {
                return VmResult::Err(e.into());
            }
            return VmResult::Ok(Some(vec));
        }

        VmResult::Ok(None)
    })
}
