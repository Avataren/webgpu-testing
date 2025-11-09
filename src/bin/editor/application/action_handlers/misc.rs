use hecs::Entity;
use wgpu_cube::scene::CanCastShadow;

use super::{ActionContext, ActionResult};

/// Handle SetCanCastShadow action
pub fn handle_set_can_cast_shadow(
    ctx: &mut ActionContext,
    entity: Entity,
    casts_shadow: bool,
) -> ActionResult {
    let mut updated = false;
    let world = ctx.scene.main_world_mut();
    let mut needs_insert = false;

    match world.get::<&mut CanCastShadow>(entity) {
        Ok(mut component) => {
            if component.0 != casts_shadow {
                component.0 = casts_shadow;
                updated = true;
            }
        }
        Err(err) => {
            if casts_shadow {
                needs_insert = true;
            } else {
                log::debug!(
                    "CanCastShadow missing for {:?} while disabling shadows: {}",
                    entity,
                    err
                );
            }
        }
    }

    if needs_insert {
        match world.insert(entity, (CanCastShadow(true),)) {
            Ok(_) => {
                updated = true;
            }
            Err(insert_err) => {
                log::warn!(
                    "Failed to add CanCastShadow to {:?}: {}",
                    entity,
                    insert_err
                );
            }
        }
    }

    if updated {
        ActionResult::scene_changed()
    } else {
        ActionResult::no_change()
    }
}

/// Handle RenameEntity action
pub fn handle_rename_entity(
    ctx: &mut ActionContext,
    entity: Entity,
    new_name: String,
) -> ActionResult {
    let updated = {
        let world = ctx.scene.main_world_mut();
        if let Ok(mut name) = world.get::<&mut wgpu_cube::scene::Name>(entity) {
            name.0 = new_name;
            log::info!("Renamed entity {:?}", entity);
            true
        } else {
            log::warn!("Entity {:?} does not have a Name component", entity);
            false
        }
    };

    if updated {
        ActionResult::scene_changed()
    } else {
        ActionResult::no_change()
    }
}
