use hecs::Entity;
use wgpu_cube::scene::CameraComponent;

use super::{ActionContext, ActionResult};

/// Handle UpdateCamera action
pub fn handle_update_camera(
    ctx: &mut ActionContext,
    entity: Entity,
    component: CameraComponent,
) -> ActionResult {
    let world = ctx.scene.main_world_mut();
    match world.get::<&mut CameraComponent>(entity) {
        Ok(mut existing) => {
            if *existing != component {
                *existing = component;

                // Update active camera if this is the active camera or if there's no active camera
                if ctx.app.shared.active_camera_entity.is_none()
                    || ctx.app.shared.active_camera_entity == Some(entity)
                {
                    ctx.scene.set_active_camera_entity(Some(entity));
                    ctx.app.shared.active_camera_entity = ctx.scene.active_camera_entity();
                }

                ctx.app.record_scene_change(ctx.scene);
                ActionResult::scene_changed()
            } else {
                ActionResult::no_change()
            }
        }
        Err(err) => {
            log::warn!("Failed to update camera for {:?}: {}", entity, err);
            ActionResult::no_change()
        }
    }
}
