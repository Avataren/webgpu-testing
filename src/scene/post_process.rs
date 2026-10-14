//! Blending [`PostProcessVolume`]s into the settings for one camera position.

use glam::{Mat4, Vec3};
use hecs::World;

use crate::renderer::postprocess::PostProcessSettings;
use crate::scene::components::{
    PostProcessVolume, TransformComponent, VolumeShape, WorldTransform,
};

/// How much a volume applies at `point`, from 0 to 1, before its weight: 1 inside the
/// shape (always, for an infinite volume), fading to 0 over `blend_distance` outside it.
pub fn volume_influence(volume: &PostProcessVolume, transform: Mat4, point: Vec3) -> f32 {
    let distance = match volume.shape {
        VolumeShape::Infinite => return 1.0,
        VolumeShape::Box { size } => {
            let Some(inverse) = invertible(transform) else {
                return 0.0;
            };
            let half = Vec3::from(size).abs() * 0.5;
            let local = inverse.transform_point3(point);
            let closest = local.clamp(-half, half);
            if closest == local {
                return 1.0;
            }
            // Measured in world space, so scaling the entity does not stretch the fade.
            transform.transform_point3(closest).distance(point)
        }
        VolumeShape::Sphere { radius } => {
            let (scale, _, center) = transform.to_scale_rotation_translation();
            let world_radius = radius.abs() * scale.abs().max_element();
            point.distance(center) - world_radius
        }
    };
    if distance <= 0.0 {
        1.0
    } else if volume.blend_distance > 0.0 {
        (1.0 - distance / volume.blend_distance).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn invertible(matrix: Mat4) -> Option<Mat4> {
    (matrix.determinant().abs() > 1e-12).then(|| matrix.inverse())
}

/// The post-processing settings at `camera`: every enabled volume in `world` blended in
/// order of priority (infinite volumes before bounded ones of the same priority), starting
/// from the defaults. Each volume only changes the settings it overrides. `None` when the world has no enabled volume.
pub fn blend_post_process_volumes(world: &World, camera: Vec3) -> Option<PostProcessSettings> {
    let mut volumes: Vec<(&PostProcessVolume, Mat4)> = Vec::new();
    let mut query = world.query::<(
        &PostProcessVolume,
        Option<&WorldTransform>,
        Option<&TransformComponent>,
    )>();
    for (_, (volume, world_transform, local)) in query.iter() {
        if !volume.enabled {
            continue;
        }
        let transform = world_transform
            .map(|t| t.0.matrix())
            .or_else(|| local.map(|t| t.0.matrix()))
            .unwrap_or(Mat4::IDENTITY);
        volumes.push((volume, transform));
    }
    if volumes.is_empty() {
        return None;
    }
    volumes.sort_by_key(|(volume, _)| (volume.priority, !volume.shape.is_infinite()));

    let mut settings = PostProcessSettings::default();
    for (volume, transform) in volumes {
        let amount = volume.weight.clamp(0.0, 1.0) * volume_influence(volume, transform, camera);
        if amount > 0.0 {
            settings = volume.apply(&settings, amount);
        }
    }
    Some(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer::postprocess::Tonemapper;
    use crate::scene::Transform;
    use glam::Quat;

    fn bounded(shape: VolumeShape, compensation: f32) -> PostProcessVolume {
        let mut volume = PostProcessVolume {
            shape,
            priority: 1,
            blend_distance: 2.0,
            ..PostProcessVolume::default()
        };
        volume.settings.exposure.compensation = compensation;
        volume
    }

    #[test]
    fn no_volumes_means_no_settings() {
        let world = World::new();
        assert!(blend_post_process_volumes(&world, Vec3::ZERO).is_none());
    }

    #[test]
    fn infinite_volume_applies_everywhere() {
        let mut world = World::new();
        let mut volume = PostProcessVolume::default();
        volume.settings.tonemapping = Tonemapper::Aces;
        world.spawn((volume,));
        let far = blend_post_process_volumes(&world, Vec3::splat(1.0e6)).unwrap();
        assert_eq!(far.tonemapping, Tonemapper::Aces);
    }

    #[test]
    fn box_volume_fades_over_blend_distance() {
        let volume = bounded(
            VolumeShape::Box {
                size: [2.0, 2.0, 2.0],
            },
            2.0,
        );
        let transform = Mat4::IDENTITY;
        assert_eq!(volume_influence(&volume, transform, Vec3::ZERO), 1.0);
        assert_eq!(volume_influence(&volume, transform, Vec3::X), 1.0);
        let halfway = volume_influence(&volume, transform, Vec3::new(2.0, 0.0, 0.0));
        assert!((halfway - 0.5).abs() < 1e-5, "{halfway}");
        assert_eq!(
            volume_influence(&volume, transform, Vec3::new(4.0, 0.0, 0.0)),
            0.0
        );
    }

    #[test]
    fn box_follows_the_entity_transform() {
        let volume = bounded(
            VolumeShape::Box {
                size: [1.0, 1.0, 1.0],
            },
            0.0,
        );
        let transform = Transform::from_trs(
            Vec3::new(10.0, 0.0, 0.0),
            Quat::from_rotation_y(0.7),
            Vec3::splat(4.0),
        )
        .matrix();
        assert_eq!(
            volume_influence(&volume, transform, Vec3::new(11.5, 0.0, 0.0)),
            1.0
        );
        assert_eq!(volume_influence(&volume, transform, Vec3::ZERO), 0.0);
    }

    #[test]
    fn sphere_scales_with_largest_axis() {
        let volume = bounded(VolumeShape::Sphere { radius: 1.0 }, 0.0);
        let transform = Mat4::from_scale(Vec3::new(1.0, 3.0, 1.0));
        assert_eq!(
            volume_influence(&volume, transform, Vec3::new(2.9, 0.0, 0.0)),
            1.0
        );
        let fading = volume_influence(&volume, transform, Vec3::new(4.0, 0.0, 0.0));
        assert!((fading - 0.5).abs() < 1e-5);
    }

    #[test]
    fn higher_priority_local_volume_blends_over_infinite() {
        let mut world = World::new();
        world.spawn((PostProcessVolume::default(),));
        world.spawn((
            bounded(
                VolumeShape::Box {
                    size: [2.0, 2.0, 2.0],
                },
                2.0,
            ),
            WorldTransform(Transform::IDENTITY),
        ));
        let inside = blend_post_process_volumes(&world, Vec3::ZERO).unwrap();
        assert_eq!(inside.exposure.compensation, 2.0);
        let edge = blend_post_process_volumes(&world, Vec3::new(2.0, 0.0, 0.0)).unwrap();
        assert!((edge.exposure.compensation - 1.0).abs() < 1e-5);
        let outside = blend_post_process_volumes(&world, Vec3::new(9.0, 0.0, 0.0)).unwrap();
        assert_eq!(outside.exposure.compensation, 0.0);
    }

    #[test]
    fn disabled_and_weightless_volumes_do_nothing() {
        let mut world = World::new();
        let mut disabled = PostProcessVolume {
            enabled: false,
            ..PostProcessVolume::default()
        };
        disabled.settings.exposure.compensation = 3.0;
        world.spawn((disabled,));
        assert!(blend_post_process_volumes(&world, Vec3::ZERO).is_none());

        let mut weightless = PostProcessVolume {
            weight: 0.0,
            ..PostProcessVolume::default()
        };
        weightless.settings.exposure.compensation = 3.0;
        world.spawn((weightless,));
        let settings = blend_post_process_volumes(&world, Vec3::ZERO).unwrap();
        assert_eq!(settings.exposure.compensation, 0.0);
    }

    #[test]
    fn partial_volume_changes_only_its_overrides() {
        let mut world = World::new();
        let mut global = PostProcessVolume::default();
        global.settings.color.saturation = 1.5;
        global.settings.exposure.compensation = -1.0;
        world.spawn((global,));
        let mut local = bounded(
            VolumeShape::Box {
                size: [2.0, 2.0, 2.0],
            },
            3.0,
        );
        local.overrides = Some(["exposure.compensation".to_string()].into());
        world.spawn((local, WorldTransform(Transform::IDENTITY)));

        let inside = blend_post_process_volumes(&world, Vec3::ZERO).unwrap();
        assert_eq!(inside.exposure.compensation, 3.0);
        // The local volume's own (default) saturation does not leak in.
        assert_eq!(inside.color.saturation, 1.5);
    }

    #[test]
    fn shape_names_accept_the_global_alias() {
        let shape: VolumeShape = serde_json::from_str(r#"{"kind": "global"}"#).unwrap();
        assert_eq!(shape, VolumeShape::Infinite);
        let shape: VolumeShape =
            serde_json::from_str(r#"{"kind": "box", "size": [1, 2, 3]}"#).unwrap();
        assert_eq!(
            shape,
            VolumeShape::Box {
                size: [1.0, 2.0, 3.0]
            }
        );
    }
}
