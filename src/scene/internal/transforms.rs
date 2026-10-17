use crate::scene::components::{Children, Parent, TransformComponent, WorldTransform};
use crate::scene::transform::Transform;
use hecs::{Entity, World};

/// Compute every entity's `WorldTransform` from its `TransformComponent` and its
/// parents', walking down from the entities without a `Parent`. An entity without a
/// transform stops the walk: neither it nor anything below it is updated.
pub(crate) fn propagate_transforms(world: &mut World) {
    let roots: Vec<Entity> = world
        .query::<&TransformComponent>()
        .without::<&Parent>()
        .iter()
        .map(|(entity, _)| entity)
        .collect();
    log::trace!("Propagating transforms from {} root entities", roots.len());

    // One view for the whole walk rather than a lookup per component access: characters
    // bring thousands of bones.
    let mut missing: Vec<(Entity, Transform)> = Vec::new();
    let mut view = world.view_mut::<(
        &TransformComponent,
        Option<&mut WorldTransform>,
        Option<&Children>,
    )>();
    let mut stack: Vec<(Entity, Transform)> = Vec::new();
    for root in roots {
        stack.push((root, Transform::IDENTITY));
        while let Some((entity, parent_world)) = stack.pop() {
            let Some((local, world_transform, children)) = view.get_mut(entity) else {
                continue;
            };
            let transform = parent_world.mul_transform(&local.0);
            match world_transform {
                Some(world_transform) => world_transform.0 = transform,
                None => missing.push((entity, transform)),
            }
            if let Some(children) = children {
                stack.extend(children.0.iter().rev().map(|&child| (child, transform)));
            }
        }
    }
    for (entity, transform) in missing {
        if let Err(e) = world.insert_one(entity, WorldTransform(transform)) {
            log::error!("Failed to insert WorldTransform for entity {entity:?}: {e:?}");
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
