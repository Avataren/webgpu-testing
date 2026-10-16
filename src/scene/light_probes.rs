//! The light probe grid a scene's [`LightProbeVolume`] asks the renderer for.

use glam::{Mat4, Vec3};
use hecs::{Entity, World};

use crate::renderer::gizmo_shapes::{GizmoColor, GizmoShape};
use crate::renderer::LightProbeGrid;
use crate::scene::components::{
    LightProbeVolume, SelectedInEditor, TransformComponent, WorldTransform,
};
use crate::scene::transform::Transform;

/// Where the probes sit, as the selected volume's outline shows it.
const PROBE_EXTENT_COLOR: GizmoColor = [0.94, 0.66, 0.35, 1.0];
/// The box the volume asks for, where the probes overshoot it (spacing rounds up).
const VOLUME_BOX_COLOR: GizmoColor = [0.94, 0.66, 0.35, 0.45];

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
            (entity, volume, volume_matrix(world_transform, local))
        })
        .min_by_key(|(entity, _, _)| entity.to_bits())?;
    Some(ActiveLightProbeVolume {
        entity,
        grid: volume_grid(volume, transform),
        baked: volume.baked.clone(),
    })
}

/// The entity's placement: its world transform, else its local one.
fn volume_matrix(world: Option<&WorldTransform>, local: Option<&TransformComponent>) -> Mat4 {
    world
        .map(|t| t.0.matrix())
        .or_else(|| local.map(|t| t.0.matrix()))
        .unwrap_or(Mat4::IDENTITY)
}

/// The grid `volume` asks for at `transform`: centred on it, sized by its scale.
fn volume_grid(volume: &LightProbeVolume, transform: Mat4) -> LightProbeGrid {
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
    grid
}

/// Outlines of the light probe volumes selected in the editor, in a world placed by
/// `node`: the box the probes fill (solid), and the box the volume asks for where the
/// probes overshoot it (faint), so a box fitted to a room shows its outer probes sitting
/// in the walls.
pub(crate) fn selected_volume_gizmos(world: &World, node: Transform) -> Vec<GizmoShape> {
    let mut shapes = Vec::new();
    let mut query = world.query::<(
        &LightProbeVolume,
        &SelectedInEditor,
        Option<&WorldTransform>,
        Option<&TransformComponent>,
    )>();
    for (_, (volume, _, world_transform, local)) in query.iter() {
        let transform = volume_matrix(world_transform, local);
        let grid = volume_grid(volume, transform);
        let extent = (Vec3::new(
            grid.counts[0] as f32,
            grid.counts[1] as f32,
            grid.counts[2] as f32,
        ) - Vec3::ONE)
            * grid.spacing;
        let node = node.matrix();
        let place = |p: Vec3| node.transform_point3(p);
        push_box(
            &mut shapes,
            place(grid.origin),
            place(grid.origin + extent),
            2.0,
            PROBE_EXTENT_COLOR,
        );
        let (scale, _, center) = transform.to_scale_rotation_translation();
        let half = volume.world_size(scale.to_array()) * 0.5;
        if (extent * 0.5 - half).abs().max_element() > 0.01 {
            push_box(
                &mut shapes,
                place(center - half),
                place(center + half),
                1.0,
                VOLUME_BOX_COLOR,
            );
        }
    }
    shapes
}

/// The twelve edges of the axis-aligned box from `min` to `max`.
fn push_box(shapes: &mut Vec<GizmoShape>, min: Vec3, max: Vec3, width: f32, color: GizmoColor) {
    let corner = |i: u32| {
        Vec3::new(
            if i & 1 == 0 { min.x } else { max.x },
            if i & 2 == 0 { min.y } else { max.y },
            if i & 4 == 0 { min.z } else { max.z },
        )
    };
    for i in 0..8u32 {
        for bit in [1u32, 2, 4] {
            if i & bit == 0 {
                shapes.push(GizmoShape::line(corner(i), corner(i | bit), width, color));
            }
        }
    }
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
    fn selected_volumes_outline_their_probes_and_any_overshoot() {
        let mut world = World::new();
        let fitted = world.spawn((
            LightProbeVolume::default(),
            TransformComponent(Transform::default()),
        ));
        assert!(selected_volume_gizmos(&world, Transform::default()).is_empty());
        world.insert_one(fitted, SelectedInEditor).unwrap();
        // 10 x 4 x 10 at 0.5 m: the probes fill the box exactly, one outline.
        assert_eq!(
            selected_volume_gizmos(&world, Transform::default()).len(),
            12
        );

        // The study's 4.6 m box: 11 probes span 5 m, so both boxes show.
        world.remove_one::<SelectedInEditor>(fitted).unwrap();
        world.spawn((
            LightProbeVolume {
                size: [4.6, 2.8, 4.8],
                ..Default::default()
            },
            TransformComponent(Transform::default()),
            SelectedInEditor,
        ));
        assert_eq!(
            selected_volume_gizmos(&world, Transform::default()).len(),
            24
        );
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
