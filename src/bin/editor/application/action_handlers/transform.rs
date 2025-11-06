use hecs::Entity;
use wgpu_cube::scene::{Transform, TransformComponent};

use super::{ActionContext, ActionResult};

/// Handle UpdateTransform action
pub fn handle_update_transform(
    ctx: &mut ActionContext,
    entity: Entity,
    transform: Transform,
) -> ActionResult {
    let updated = {
        let world = ctx.scene.main_world_mut();
        match world.get::<&mut TransformComponent>(entity) {
            Ok(mut component) => {
                component.0 = transform;
                true
            }
            Err(err) => {
                log::warn!("Failed to update transform for {:?}: {}", entity, err);
                false
            }
        }
    };

    if updated {
        ctx.app.record_scene_change(ctx.scene);
        ActionResult::transforms_changed()
    } else {
        ActionResult::no_change()
    }
}
