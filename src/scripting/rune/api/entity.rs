use glam::{EulerRot, Quat, Vec3};
use rune::runtime::VmResult;

use crate::scene::Name;
use crate::scripting::rune::commands::entity_bits;
use crate::scripting::rune::guards::{with_active_world, ACTIVE_COMMANDS};

#[rune::function]
pub(crate) fn spawn_entity(name: Option<String>) -> VmResult<i64> {
    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut()
            .with(|commands| VmResult::Ok(commands.spawn_entity(name)))
    })
}

#[rune::function]
pub(crate) fn set_name(handle: i64, name: String) -> VmResult<()> {
    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut()
            .with(|commands| commands.set_name(handle, name))
    })
}

#[rune::function]
pub(crate) fn set_translation(handle: i64, x: f64, y: f64, z: f64) -> VmResult<()> {
    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut().with(|commands| {
            commands.set_translation(handle, Vec3::new(x as f32, y as f32, z as f32))
        })
    })
}

#[rune::function]
pub(crate) fn set_rotation(handle: i64, yaw: f64, pitch: f64, roll: f64) -> VmResult<()> {
    let rotation = Quat::from_euler(EulerRot::YXZ, yaw as f32, pitch as f32, roll as f32);
    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut()
            .with(|commands| commands.set_rotation(handle, rotation))
    })
}

#[rune::function]
pub(crate) fn import_gltf(handle: i64, path: String, scale: f64) -> VmResult<()> {
    let scale = scale as f32;
    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut()
            .with(|commands| commands.import_gltf(handle, path, scale))
    })
}

#[rune::function(path = attach_inline_script)]
pub(crate) fn attach_inline_script(handle: i64, name: String, source: String) -> VmResult<()> {
    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut()
            .with(|commands| commands.attach_inline_script(handle, name, source))
    })
}

#[rune::function(path = attach_script)]
pub(crate) fn attach_script_file(handle: i64, path: String) -> VmResult<()> {
    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut()
            .with(|commands| commands.attach_file_script(handle, path))
    })
}

/// Find an entity by name.
///
/// Returns the entity handle if found, or None if no entity with that name exists.
///
/// # Example
/// ```rune
/// let player = find_entity_by_name("Player");
/// if player != None {
///     log_info("Found the player!");
/// }
/// ```
#[rune::function]
pub(crate) fn find_entity_by_name(name: String) -> VmResult<Option<i64>> {
    with_active_world(|world| {
        for (entity, entity_name) in world.query::<&Name>().iter() {
            if entity_name.0 == name {
                return VmResult::Ok(Some(entity_bits(entity)));
            }
        }
        VmResult::Ok(None)
    })
}
