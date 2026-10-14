use super::camera::Camera;
use super::components::{CameraComponent, TransformComponent, WorldTransform};
use super::internal::lights::safe_normalize;
use super::transform::LookAtTarget;
use glam::Vec3;
use hecs::{Entity, World};

/// Wraps the active [`Camera`] for a scene and provides helpers for syncing it
/// from ECS entities.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SceneCamera {
    camera: Camera,
}

impl SceneCamera {
    pub(crate) fn new() -> Self {
        Self {
            camera: Camera::default(),
        }
    }

    pub(crate) fn camera(&self) -> &Camera {
        &self.camera
    }

    pub(crate) fn camera_mut(&mut self) -> &mut Camera {
        &mut self.camera
    }

    pub(crate) fn set_camera(&mut self, camera: Camera) {
        self.camera = camera;
    }

    pub(crate) fn apply_from_entity(&mut self, world: &World, entity: Entity) -> bool {
        if !world.contains(entity) {
            return false;
        }

        let projection = world
            .get::<&CameraComponent>(entity)
            .ok()
            .map(|component| *component);
        let transform = world
            .get::<&WorldTransform>(entity)
            .map(|component| component.0)
            .ok()
            .or_else(|| {
                world
                    .get::<&TransformComponent>(entity)
                    .map(|component| component.0)
                    .ok()
            });

        let mut camera = self.camera;

        if let Some(component) = projection {
            component.apply_to_camera(&mut camera);
        }

        if let Some(transform) = transform {
            let eye = transform.translation;
            let forward = safe_normalize(transform.rotation * Vec3::NEG_Z, Vec3::NEG_Z);
            let up = safe_normalize(transform.rotation * Vec3::Y, Vec3::Y);

            // Only re-derive the camera when the entity actually moved. Rebuilding an
            // unchanged view would reset the focus distance (the orbit pivot) to 1.
            let current_forward = (camera.target - camera.eye).try_normalize();
            let unchanged = camera.eye.abs_diff_eq(eye, 1e-5)
                && current_forward.is_some_and(|current| current.dot(forward) > 1.0 - 1e-6);
            // Stored cameras may use world Y as their up hint. Compare the actual
            // perpendicular view-up vector so an equivalent hint survives undo.
            let current_up = current_forward
                .and_then(|forward| (camera.up - forward * camera.up.dot(forward)).try_normalize());
            let unchanged_up = current_up.is_some_and(|current| current.dot(up) > 1.0 - 1e-6);

            let explicit_target = world
                .get::<&LookAtTarget>(entity)
                .ok()
                .map(|hint| hint.0)
                .filter(|target| {
                    (*target - eye)
                        .try_normalize()
                        .is_some_and(|direction| direction.dot(forward) > 1.0 - 1e-6)
                });
            if let Some(target) = explicit_target {
                camera.eye = eye;
                camera.target = target;
            } else if !unchanged {
                camera.eye = eye;
                camera.target = eye + forward;
            }
            // Roll can change while eye and forward stay fixed. Sync it without
            // shortening the camera's existing focus distance.
            if !unchanged || !unchanged_up {
                camera.up = up;
            }
        }

        self.camera = camera;
        true
    }
}

impl Default for SceneCamera {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::SceneCamera;
    use crate::scene::components::{CameraComponent, TransformComponent};
    use crate::scene::transform::Transform;
    use glam::{Quat, Vec3};
    use hecs::World;

    #[test]
    fn apply_from_entity_updates_camera_transform() {
        let mut world = World::new();
        let entity = world.spawn((
            CameraComponent::perspective(45.0_f32.to_radians(), 0.1, 100.0),
            TransformComponent(Transform {
                translation: Vec3::new(1.0, 2.0, 3.0),
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            }),
        ));

        let mut camera = SceneCamera::new();
        assert!(camera.apply_from_entity(&world, entity));
        let active = camera.camera();
        assert_eq!(active.eye, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(active.target, Vec3::new(1.0, 2.0, 2.0));
        assert_eq!(active.up, Vec3::Y);
    }

    #[test]
    fn apply_from_missing_entity_fails() {
        let mut world = World::new();
        let entity = world.spawn(());
        world
            .despawn(entity)
            .expect("entity just spawned should be removable");

        let mut camera = SceneCamera::new();
        assert!(!camera.apply_from_entity(&world, entity));
    }

    #[test]
    fn apply_from_entity_updates_roll_without_moving_or_turning_forward() {
        let mut world = World::new();
        let entity = world.spawn((TransformComponent(Transform::IDENTITY),));
        let mut camera = SceneCamera::new();
        assert!(camera.apply_from_entity(&world, entity));
        camera.camera_mut().target = Vec3::NEG_Z * 10.0;
        let rotation = Quat::from_rotation_z(0.7);
        world
            .get::<&mut TransformComponent>(entity)
            .unwrap()
            .0
            .rotation = rotation;

        assert!(camera.apply_from_entity(&world, entity));
        assert!(camera.camera().up.abs_diff_eq(rotation * Vec3::Y, 1e-5));
        assert_eq!(camera.camera().target, Vec3::NEG_Z * 10.0);
    }

    #[test]
    fn apply_from_entity_ignores_look_target_after_turning_away() {
        let eye = Vec3::new(0.0, 6.0, 12.0);
        let target = Vec3::ZERO;
        let mut world = World::new();
        let entity = world.spawn((
            TransformComponent(Transform::from_trs(
                eye,
                super::super::transform::look_rotation(eye, target).unwrap(),
                Vec3::ONE,
            )),
            super::LookAtTarget(target),
        ));
        let mut camera = SceneCamera::new();
        camera.apply_from_entity(&world, entity);
        assert_eq!(camera.camera().target, target);

        world
            .get::<&mut TransformComponent>(entity)
            .unwrap()
            .0
            .rotation = Quat::IDENTITY;
        camera.apply_from_entity(&world, entity);
        assert_eq!(camera.camera().eye, eye);
        assert_eq!(camera.camera().target, eye + Vec3::NEG_Z);
    }

    #[test]
    fn apply_from_entity_ignores_look_target_after_moving_sideways() {
        let eye = Vec3::new(0.0, 6.0, 12.0);
        let target = Vec3::ZERO;
        let rotation = super::super::transform::look_rotation(eye, target).unwrap();
        let mut world = World::new();
        let entity = world.spawn((
            TransformComponent(Transform::from_trs(eye, rotation, Vec3::ONE)),
            super::LookAtTarget(target),
        ));
        let mut camera = SceneCamera::new();
        camera.apply_from_entity(&world, entity);
        assert_eq!(camera.camera().target, target);

        let moved = eye + Vec3::X * 4.0;
        world
            .get::<&mut TransformComponent>(entity)
            .unwrap()
            .0
            .translation = moved;
        camera.apply_from_entity(&world, entity);
        assert_eq!(camera.camera().eye, moved);
        assert_eq!(camera.camera().target, moved + rotation * Vec3::NEG_Z);
    }
}
