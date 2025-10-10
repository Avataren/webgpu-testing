use crate::scene::components::{Name, TransformComponent, WorldTransform};
use hecs::World;

pub(crate) fn debug_print_transforms(world: &World) {
    log::info!("=== Transform Debug ===");
    for (_entity, (name, local, world_transform)) in world
        .query::<(&Name, &TransformComponent, Option<&WorldTransform>)>()
        .iter()
    {
        log::info!(
            "{}: Local T:{:?} R:{:?} S:{:?}",
            name.0,
            local.0.translation,
            local.0.rotation,
            local.0.scale
        );
        if let Some(world) = world_transform {
            log::info!(
                "    World T:{:?} R:{:?} S:{:?}",
                world.0.translation,
                world.0.rotation,
                world.0.scale
            );
        } else {
            log::info!("    World: NONE (root entity)");
        }
    }
    log::info!("=====================");
}
