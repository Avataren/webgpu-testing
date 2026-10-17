use crate::scene::components::{Children, Parent, TransformComponent, WorldTransform};
use crate::scene::transform::Transform;
use hecs::{Entity, World};

/// Worlds with at least this many transform roots are propagated on the thread pool.
const PARALLEL_ROOTS: usize = 64;

/// Compute every entity's `WorldTransform` from its `TransformComponent` and its
/// parents', walking down from the entities without a `Parent`. An entity without a
/// transform stops the walk: neither it nor anything below it is updated.
///
/// Characters bring thousands of bones, so large worlds are walked in parallel: each
/// root's subtree on one thread, reading through one shared view, and the results
/// written back in parallel as well (every entity is visited once, which a bitset
/// guarantees even for a malformed hierarchy).
pub(crate) fn propagate_transforms(world: &mut World) {
    let roots: Vec<Entity> = world
        .query::<&TransformComponent>()
        .without::<&Parent>()
        .iter()
        .map(|(entity, _)| entity)
        .collect();
    log::trace!("Propagating transforms from {} root entities", roots.len());

    // Walk: every entity's new world transform, by root subtree.
    let computed: Vec<Vec<(Entity, Transform)>> = {
        let view = world.view::<(&TransformComponent, Option<&Children>)>();
        let walk = |root: &Entity| {
            let mut out = Vec::new();
            let mut stack: Vec<(Entity, Transform)> = vec![(*root, Transform::IDENTITY)];
            while let Some((entity, parent_world)) = stack.pop() {
                let Some((local, children)) = view.get(entity) else {
                    continue;
                };
                let transform = parent_world.mul_transform(&local.0);
                out.push((entity, transform));
                if let Some(children) = children {
                    stack.extend(children.0.iter().rev().map(|&child| (child, transform)));
                }
            }
            out
        };
        if roots.len() < PARALLEL_ROOTS {
            roots.iter().map(walk).collect()
        } else {
            use rayon::prelude::*;
            roots.par_iter().map(walk).collect()
        }
    };

    // Write back. Entities that have no `WorldTransform` yet get one afterwards.
    let total: usize = computed.iter().map(Vec::len).sum();
    let mut missing: Vec<(Entity, Transform)> = Vec::new();
    if total < PARALLEL_ROOTS * 8 {
        let mut view = world.view_mut::<&mut WorldTransform>();
        for (entity, transform) in computed.into_iter().flatten() {
            match view.get_mut(entity) {
                Some(world_transform) => world_transform.0 = transform,
                None => missing.push((entity, transform)),
            }
        }
    } else {
        use rayon::prelude::*;
        use std::sync::atomic::{AtomicU64, Ordering};
        let max_id = computed
            .iter()
            .flatten()
            .map(|(entity, _)| entity.id())
            .max()
            .unwrap_or(0) as usize;
        let visited: Vec<AtomicU64> = (0..=max_id / 64).map(|_| AtomicU64::new(0)).collect();
        let view = world.view_mut::<&mut WorldTransform>();
        missing = computed
            .par_iter()
            .flat_map_iter(|subtree| {
                subtree.iter().filter_map(|&(entity, transform)| {
                    // Claim the entity: a second visit (an entity listed under two
                    // parents) is skipped, so no two threads write one component.
                    let id = entity.id() as usize;
                    let bit = 1u64 << (id % 64);
                    if visited[id / 64].fetch_or(bit, Ordering::Relaxed) & bit != 0 {
                        return None;
                    }
                    // SAFETY: `visited` hands each entity to exactly one invocation, so
                    // no component is borrowed twice while the view is alive.
                    match unsafe { view.get_unchecked(entity) } {
                        Some(world_transform) => {
                            world_transform.0 = transform;
                            None
                        }
                        None => Some((entity, transform)),
                    }
                })
            })
            .collect();
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

    /// Enough roots and entities for the parallel walk and write-back: a chain under
    /// each root, all placed correctly, with the world transforms inserted where they
    /// were missing.
    #[test]
    fn test_transform_propagation_parallel_matches_sequential() {
        let mut world = World::new();
        let mut leaves = Vec::new();
        for root_index in 0..(PARALLEL_ROOTS * 2) {
            let root = world.spawn((TransformComponent(Transform::from_translation(Vec3::new(
                root_index as f32,
                0.0,
                0.0,
            ))),));
            let mut parent = root;
            for depth in 1..=8 {
                let child = world.spawn((
                    TransformComponent(Transform::from_translation(Vec3::Y)),
                    Parent(parent),
                ));
                world.insert_one(parent, Children(vec![child])).ok();
                parent = child;
                if depth == 8 {
                    leaves.push((child, root_index as f32));
                }
            }
        }
        propagate_transforms(&mut world);
        for (leaf, x) in leaves {
            let world_transform = world.get::<&WorldTransform>(leaf).unwrap();
            assert!(
                world_transform
                    .0
                    .translation
                    .abs_diff_eq(Vec3::new(x, 8.0, 0.0), 1e-5),
                "{leaf:?}: {}",
                world_transform.0.translation
            );
        }
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
