use super::camera::Camera;
use super::components::{CameraComponent, TransformComponent, WorldTransform};
use super::internal::lights::safe_normalize;
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

            camera.eye = eye;
            camera.target = eye + forward;
            camera.up = up;
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
}
