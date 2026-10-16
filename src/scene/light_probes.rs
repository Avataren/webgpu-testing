//! The light probe grid a scene's [`LightProbeVolume`] asks the renderer for.

use glam::Mat4;
use hecs::{Entity, World};

use crate::renderer::LightProbeGrid;
use crate::scene::components::{LightProbeVolume, TransformComponent, WorldTransform};

/// The light probe volume the renderer uses, as [`active_light_probe_volume`] finds it.
#[derive(Clone, Debug, PartialEq)]
pub struct ActiveLightProbeVolume {
    pub entity: Entity,
    pub grid: LightProbeGrid,
    /// Project path of its baked data.
    pub baked: Option<String>,
}

/// The grid of the first enabled light probe volume in `world` (lowest entity id first,
/// so the choice is stable), placed at its entity: centred on it, sized by its scale.
pub fn light_probe_grid(world: &World) -> Option<LightProbeGrid> {
    active_light_probe_volume(world).map(|volume| volume.grid)
}

/// The first enabled light probe volume in `world` (lowest entity id first), with the
/// grid it asks for and its baked data.
pub fn active_light_probe_volume(world: &World) -> Option<ActiveLightProbeVolume> {
    let mut query = world.query::<(
        &LightProbeVolume,
        Option<&WorldTransform>,
        Option<&TransformComponent>,
    )>();
    let (entity, volume, transform) = query
        .iter()
        .filter(|(_, (volume, _, _))| volume.enabled)
        .map(|(entity, (volume, world_transform, local))| {
            let transform = world_transform
                .map(|t| t.0.matrix())
                .or_else(|| local.map(|t| t.0.matrix()))
                .unwrap_or(Mat4::IDENTITY);
            (entity, volume, transform)
        })
        .min_by_key(|(entity, _, _)| entity.to_bits())?;
    let (scale, _, center) = transform.to_scale_rotation_translation();
    let scale = scale.abs().to_array();
    let mut grid = LightProbeGrid::centered(
        center,
        volume.effective_spacing(scale),
        volume.probe_counts(scale),
        volume.rays_per_probe,
        volume.hysteresis,
        volume.intensity,
    );
    grid.live = !volume.use_baked;
    grid.relocate = volume.relocate;
    Some(ActiveLightProbeVolume {
        entity,
        grid,
        baked: volume.baked.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Transform;
    use glam::Vec3;

    #[test]
    fn grid_follows_the_volume_entity() {
        let mut world = World::new();
        assert!(light_probe_grid(&world).is_none());
        let mut transform = Transform::default();
        transform.translation = Vec3::new(0.0, 1.5, 0.0);
        world.spawn((
            LightProbeVolume {
                size: [5.0, 3.0, 5.0],
                ..Default::default()
            },
            TransformComponent(transform),
        ));
        let grid = light_probe_grid(&world).unwrap();
        assert_eq!(grid.counts, [11, 7, 11]);
        assert_eq!(grid.origin, Vec3::new(-2.5, 0.0, -2.5));
    }

    #[test]
    fn disabled_volumes_give_no_grid() {
        let mut world = World::new();
        world.spawn((LightProbeVolume {
            enabled: false,
            ..Default::default()
        },));
        assert!(light_probe_grid(&world).is_none());
    }
}
