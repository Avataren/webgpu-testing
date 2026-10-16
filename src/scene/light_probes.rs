//! The light probe volumes a scene's [`LightProbeVolume`]s ask the renderer for.

use glam::{Mat4, Quat, Vec3};
use hecs::{Entity, World};

use crate::renderer::gizmo_shapes::{GizmoColor, GizmoShape};
use crate::renderer::{LightProbeGrid, MAX_LIGHT_PROBE_VOLUMES};
use crate::scene::components::{
    LightProbeVolume, SelectedInEditor, TransformComponent, WorldTransform,
};
use crate::scene::transform::Transform;

/// Where the probes sit, as the selected volume's outline shows it.
const PROBE_EXTENT_COLOR: GizmoColor = [0.94, 0.66, 0.35, 1.0];
/// The box the volume asks for, where the probes overshoot it (spacing rounds up).
const VOLUME_BOX_COLOR: GizmoColor = [0.94, 0.66, 0.35, 0.45];

/// A light probe volume the renderer uses, as [`light_probe_volumes`] finds it.
#[derive(Clone, Debug, PartialEq)]
pub struct ActiveLightProbeVolume {
    pub entity: Entity,
    /// The grid it asks for (its `id` is the entity's bits).
    pub grid: LightProbeGrid,
    /// Project path of its baked data.
    pub baked: Option<String>,
}

/// The enabled light probe volumes in `world` in blending order, at most
/// [`MAX_LIGHT_PROBE_VOLUMES`]: higher `priority` first, then smaller volumes first (so a
/// small volume inside a big one takes over there), then lower entity ids (so the order
/// is stable). Each grid is placed at its entity: centred on it, turned with it, sized by
/// its scale.
pub fn light_probe_volumes(world: &World) -> Vec<ActiveLightProbeVolume> {
    let mut query = world.query::<(
        &LightProbeVolume,
        Option<&WorldTransform>,
        Option<&TransformComponent>,
    )>();
    let mut volumes: Vec<(i32, f32, ActiveLightProbeVolume)> = query
        .iter()
        .filter(|(_, (volume, _, _))| volume.enabled)
        .map(|(entity, (volume, world_transform, local))| {
            let transform = volume_matrix(world_transform, local);
            let mut grid = volume_grid(volume, transform);
            grid.id = entity.to_bits().get();
            let (scale, _, _) = transform.to_scale_rotation_translation();
            let size = volume.world_size(scale.abs().to_array());
            let active = ActiveLightProbeVolume {
                entity,
                grid,
                baked: volume.baked.clone(),
            };
            (volume.priority, size.x * size.y * size.z, active)
        })
        .collect();
    volumes.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(a.1.total_cmp(&b.1))
            .then(a.2.entity.to_bits().cmp(&b.2.entity.to_bits()))
    });
    volumes.truncate(MAX_LIGHT_PROBE_VOLUMES);
    volumes.into_iter().map(|(_, _, volume)| volume).collect()
}

/// The entity's placement: its world transform, else its local one.
fn volume_matrix(world: Option<&WorldTransform>, local: Option<&TransformComponent>) -> Mat4 {
    world
        .map(|t| t.0.matrix())
        .or_else(|| local.map(|t| t.0.matrix()))
        .unwrap_or(Mat4::IDENTITY)
}

/// The grid `volume` asks for at `transform`: centred on it, turned with it, sized by its
/// scale.
fn volume_grid(volume: &LightProbeVolume, transform: Mat4) -> LightProbeGrid {
    let (scale, rotation, center) = transform.to_scale_rotation_translation();
    let scale = scale.abs().to_array();
    let mut grid = LightProbeGrid::centered(
        center,
        rotation.normalize(),
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
    let node = node.matrix();
    let mut query = world.query::<(
        &LightProbeVolume,
        &SelectedInEditor,
        Option<&WorldTransform>,
        Option<&TransformComponent>,
    )>();
    for (_, (volume, _, world_transform, local)) in query.iter() {
        let transform = volume_matrix(world_transform, local);
        let grid = volume_grid(volume, transform);
        let extent = grid.extent();
        let center = grid.to_world(extent * 0.5);
        push_box(
            &mut shapes,
            &node,
            center,
            grid.rotation,
            extent * 0.5,
            2.0,
            PROBE_EXTENT_COLOR,
        );
        let (scale, _, _) = transform.to_scale_rotation_translation();
        let half = volume.world_size(scale.to_array()) * 0.5;
        if (extent * 0.5 - half).abs().max_element() > 0.01 {
            push_box(
                &mut shapes,
                &node,
                center,
                grid.rotation,
                half,
                1.0,
                VOLUME_BOX_COLOR,
            );
        }
    }
    shapes
}

/// The twelve edges of the box of half size `half` around `center`, turned by `rotation`,
/// placed by `node`.
fn push_box(
    shapes: &mut Vec<GizmoShape>,
    node: &Mat4,
    center: Vec3,
    rotation: Quat,
    half: Vec3,
    width: f32,
    color: GizmoColor,
) {
    let corner = |i: u32| {
        let local = Vec3::new(
            if i & 1 == 0 { -half.x } else { half.x },
            if i & 2 == 0 { -half.y } else { half.y },
            if i & 4 == 0 { -half.z } else { half.z },
        );
        node.transform_point3(center + rotation * local)
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

    fn at(translation: Vec3) -> TransformComponent {
        TransformComponent(Transform::from_translation(translation))
    }

    #[test]
    fn grid_follows_the_volume_entity() {
        let mut world = World::new();
        assert!(light_probe_volumes(&world).is_empty());
        let entity = world.spawn((
            LightProbeVolume {
                size: [5.0, 3.0, 5.0],
                ..Default::default()
            },
            at(Vec3::new(0.0, 1.5, 0.0)),
        ));
        let volumes = light_probe_volumes(&world);
        assert_eq!(volumes.len(), 1);
        let grid = volumes[0].grid;
        assert_eq!(grid.id, entity.to_bits().get());
        assert_eq!(grid.counts, [11, 7, 11]);
        assert_eq!(grid.origin, Vec3::new(-2.5, 0.0, -2.5));
    }

    #[test]
    fn grids_turn_with_their_entity() {
        let mut world = World::new();
        let rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        world.spawn((
            LightProbeVolume {
                size: [4.0, 2.0, 2.0],
                spacing: 1.0,
                ..Default::default()
            },
            TransformComponent(Transform::from_trs(Vec3::ZERO, rotation, Vec3::ONE)),
        ));
        let grid = light_probe_volumes(&world)[0].grid;
        assert_eq!(grid.counts, [5, 3, 3]);
        // The long side runs along the entity's x axis, which now points along -z.
        let far = grid.to_world(Vec3::new(grid.extent().x, 0.0, 0.0));
        assert!((far - grid.origin).normalize().distance(Vec3::NEG_Z) < 1e-5);
    }

    #[test]
    fn volumes_blend_by_priority_then_smaller_first() {
        let mut world = World::new();
        let hall = world.spawn((
            LightProbeVolume {
                size: [20.0, 4.0, 20.0],
                spacing: 1.0,
                ..Default::default()
            },
            at(Vec3::ZERO),
        ));
        let alcove = world.spawn((
            LightProbeVolume {
                size: [2.0, 2.0, 2.0],
                ..Default::default()
            },
            at(Vec3::ZERO),
        ));
        let order = |world: &World| {
            light_probe_volumes(world)
                .iter()
                .map(|volume| volume.entity)
                .collect::<Vec<_>>()
        };
        assert_eq!(order(&world), vec![alcove, hall]);
        world.get::<&mut LightProbeVolume>(hall).unwrap().priority = 1;
        assert_eq!(order(&world), vec![hall, alcove]);
    }

    #[test]
    fn no_more_volumes_than_the_renderer_takes() {
        let mut world = World::new();
        for i in 0..MAX_LIGHT_PROBE_VOLUMES + 3 {
            world.spawn((
                LightProbeVolume {
                    size: [2.0, 2.0, 2.0],
                    ..Default::default()
                },
                at(Vec3::new(i as f32 * 3.0, 0.0, 0.0)),
            ));
        }
        assert_eq!(light_probe_volumes(&world).len(), MAX_LIGHT_PROBE_VOLUMES);
    }

    #[test]
    fn selected_volumes_outline_their_probes_and_any_overshoot() {
        let mut world = World::new();
        let fitted = world.spawn((LightProbeVolume::default(), at(Vec3::ZERO)));
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
            at(Vec3::ZERO),
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
        assert!(light_probe_volumes(&world).is_empty());
    }
}
