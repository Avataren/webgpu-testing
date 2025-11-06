use hecs::Entity;
use wgpu_cube::scene::{DirectionalLight, PointLight, SpotLight};

use super::{ActionContext, ActionResult};

/// Handle UpdatePointLight action
pub fn handle_update_point_light(
    ctx: &mut ActionContext,
    entity: Entity,
    light: PointLight,
) -> ActionResult {
    let world = ctx.scene.main_world_mut();
    match world.get::<&mut PointLight>(entity) {
        Ok(mut component) => {
            *component = light;
            ctx.app.record_scene_change(ctx.scene);
            ActionResult::scene_changed()
        }
        Err(err) => {
            log::warn!("Failed to update point light for {:?}: {}", entity, err);
            ActionResult::no_change()
        }
    }
}

/// Handle UpdateDirectionalLight action
pub fn handle_update_directional_light(
    ctx: &mut ActionContext,
    entity: Entity,
    light: DirectionalLight,
) -> ActionResult {
    let world = ctx.scene.main_world_mut();
    match world.get::<&mut DirectionalLight>(entity) {
        Ok(mut component) => {
            *component = light;
            ctx.app.record_scene_change(ctx.scene);
            ActionResult::scene_changed()
        }
        Err(err) => {
            log::warn!("Failed to update directional light for {:?}: {}", entity, err);
            ActionResult::no_change()
        }
    }
}

/// Handle UpdateSpotLight action
pub fn handle_update_spot_light(
    ctx: &mut ActionContext,
    entity: Entity,
    light: SpotLight,
) -> ActionResult {
    let world = ctx.scene.main_world_mut();
    match world.get::<&mut SpotLight>(entity) {
        Ok(mut component) => {
            *component = light;
            ctx.app.record_scene_change(ctx.scene);
            ActionResult::scene_changed()
        }
        Err(err) => {
            log::warn!("Failed to update spot light for {:?}: {}", entity, err);
            ActionResult::no_change()
        }
    }
}
