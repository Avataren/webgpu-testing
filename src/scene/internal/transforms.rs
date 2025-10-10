use crate::scene::components::{Children, Parent, TransformComponent, WorldTransform};
use crate::scene::transform::Transform;
use hecs::World;

pub(crate) fn propagate_transforms(world: &mut World) {
    let roots: Vec<hecs::Entity> = world
        .query::<&TransformComponent>()
        .without::<&Parent>()
        .iter()
        .map(|(entity, _)| entity)
        .collect();

    log::trace!("Propagating transforms from {} root entities", roots.len());

    let mut stack: Vec<(hecs::Entity, Transform)> = Vec::new();

    for root in roots {
        stack.push((root, Transform::IDENTITY));

        while let Some((entity, parent_world)) = stack.pop() {
            let local = match world.get::<&TransformComponent>(entity) {
                Ok(t) => t.0,
                Err(_) => {
                    log::trace!("Entity {:?} has no TransformComponent, skipping", entity);
                    continue;
                }
            };

            let world_transform = parent_world.mul_transform(&local);

            log::trace!(
                "Entity {:?}: local T:{:?}, world T:{:?}",
                entity,
                local.translation,
                world_transform.translation
            );

            let mut has_world_transform = false;
            if let Ok(mut wt) = world.get::<&mut WorldTransform>(entity) {
                wt.0 = world_transform;
                has_world_transform = true;
            }

            if !has_world_transform {
                if let Err(e) = world.insert_one(entity, WorldTransform(world_transform)) {
                    log::error!(
                        "Failed to insert WorldTransform for entity {:?}: {:?}",
                        entity,
                        e
                    );
                    continue;
                } else {
                    log::trace!("Inserted WorldTransform for entity {:?}", entity);
                }
            }

            if let Ok(children) = world.get::<&Children>(entity) {
                for &child in children.0.iter().rev() {
                    stack.push((child, world_transform));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::components::{Name, Parent};
    use glam::Vec3;

    #[test]
    fn test_transform_propagation_simple() {
        let mut world = World::new();

        let parent = world.spawn((
            Name::new("Parent"),
            TransformComponent(Transform::from_trs(
                Vec3::new(5.0, 0.0, 0.0),
                glam::Quat::IDENTITY,
                Vec3::ONE,
            )),
        ));

        let child = world.spawn((
            Name::new("Child"),
            TransformComponent(Transform::from_trs(
                Vec3::new(2.0, 0.0, 0.0),
                glam::Quat::IDENTITY,
                Vec3::ONE,
            )),
            Parent(parent),
        ));

        world.insert_one(parent, Children(vec![child])).ok();

        propagate_transforms(&mut world);

        let parent_world = world.get::<&WorldTransform>(parent).unwrap();
        assert_eq!(parent_world.0.translation, Vec3::new(5.0, 0.0, 0.0));

        let child_world = world.get::<&WorldTransform>(child).unwrap();
        assert_eq!(child_world.0.translation, Vec3::new(7.0, 0.0, 0.0));
    }

    #[test]
    fn test_transform_propagation_scale() {
        let mut world = World::new();

        let parent = world.spawn((
            Name::new("Parent"),
            TransformComponent(Transform::from_trs(
                Vec3::ZERO,
                glam::Quat::IDENTITY,
                Vec3::splat(2.0),
            )),
        ));

        let child = world.spawn((
            Name::new("Child"),
            TransformComponent(Transform::from_trs(
                Vec3::new(1.0, 0.0, 0.0),
                glam::Quat::IDENTITY,
                Vec3::splat(0.5),
            )),
            Parent(parent),
        ));

        world.insert_one(parent, Children(vec![child])).ok();

        propagate_transforms(&mut world);

        let child_world = world.get::<&WorldTransform>(child).unwrap();
        assert_eq!(child_world.0.translation, Vec3::new(2.0, 0.0, 0.0));
        assert_eq!(child_world.0.scale, Vec3::splat(1.0));
    }

    #[test]
    fn test_transform_propagation_rotation() {
        let mut world = World::new();

        let parent = world.spawn((
            Name::new("Parent"),
            TransformComponent(Transform::from_trs(
                Vec3::ZERO,
                glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
                Vec3::ONE,
            )),
        ));

        let child = world.spawn((
            Name::new("Child"),
            TransformComponent(Transform::from_trs(
                Vec3::new(1.0, 0.0, 0.0),
                glam::Quat::IDENTITY,
                Vec3::ONE,
            )),
            Parent(parent),
        ));

        world.insert_one(parent, Children(vec![child])).ok();

        propagate_transforms(&mut world);

        let parent_world = world.get::<&WorldTransform>(parent).unwrap();
        assert!(parent_world.0.translation.abs_diff_eq(Vec3::ZERO, 1e-5));

        let child_world = world.get::<&WorldTransform>(child).unwrap();
        assert!(child_world
            .0
            .translation
            .abs_diff_eq(Vec3::new(0.0, 0.0, -1.0), 1e-5));
    }

    #[test]
    fn test_transform_propagation_updates_existing_world_transform() {
        let mut world = World::new();

        let parent = world.spawn((
            Name::new("Parent"),
            TransformComponent(Transform::from_trs(
                Vec3::ZERO,
                glam::Quat::IDENTITY,
                Vec3::ONE,
            )),
        ));

        let child = world.spawn((
            Name::new("Child"),
            TransformComponent(Transform::from_trs(
                Vec3::new(2.0, 0.0, 0.0),
                glam::Quat::IDENTITY,
                Vec3::ONE,
            )),
            Parent(parent),
        ));

        world.insert_one(parent, Children(vec![child])).ok();

        propagate_transforms(&mut world);

        {
            let child_world = world.get::<&WorldTransform>(child).unwrap();
            assert_eq!(child_world.0.translation, Vec3::new(2.0, 0.0, 0.0));
        }

        {
            let mut parent_transform = world.get::<&mut TransformComponent>(parent).unwrap();
            parent_transform.0.translation = Vec3::new(1.0, 0.0, 0.0);
        }

        propagate_transforms(&mut world);

        let child_world = world.get::<&WorldTransform>(child).unwrap();
        assert_eq!(child_world.0.translation, Vec3::new(3.0, 0.0, 0.0));
    }
}
