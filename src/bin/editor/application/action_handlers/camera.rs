use hecs::Entity;
use wgpu_cube::scene::CameraComponent;

use super::{ActionContext, ActionResult};

/// Handle UpdateCamera action
pub fn handle_update_camera(
    ctx: &mut ActionContext,
    entity: Entity,
    component: CameraComponent,
) -> ActionResult {
    let (updated, should_update_active) = {
        let world = ctx.scene.main_world_mut();
        match world.get::<&mut CameraComponent>(entity) {
            Ok(mut existing) => {
                if *existing != component {
                    *existing = component;

                    // Check if we should update active camera
                    let should_update = ctx.app.shared.active_camera_entity.is_none()
                        || ctx.app.shared.active_camera_entity == Some(entity);

                    (true, should_update)
                } else {
                    (false, false)
                }
            }
            Err(err) => {
                log::warn!("Failed to update camera for {:?}: {}", entity, err);
                (false, false)
            }
        }
    };

    if updated {
        if should_update_active {
            ctx.scene.set_active_camera_entity(Some(entity));
            ctx.app.shared.active_camera_entity = ctx.scene.active_camera_entity();
        }
        ActionResult::scene_changed()
    } else {
        ActionResult::no_change()
    }
}
