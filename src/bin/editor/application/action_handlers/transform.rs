use hecs::Entity;
use wgpu_cube::scene::{Transform, TransformComponent};

use super::{ActionContext, ActionResult};

/// Handle UpdateTransform action
pub fn handle_update_transform(
    ctx: &mut ActionContext,
    entity: Entity,
    transform: Transform,
) -> ActionResult {
    let world = ctx.scene.main_world_mut();
    match world.get::<&mut TransformComponent>(entity) {
        Ok(mut component) => {
            component.0 = transform;
            ctx.app.record_scene_change(ctx.scene);
            ActionResult::transforms_changed()
        }
        Err(err) => {
            log::warn!("Failed to update transform for {:?}: {}", entity, err);
            ActionResult::no_change()
        }
    }
}
