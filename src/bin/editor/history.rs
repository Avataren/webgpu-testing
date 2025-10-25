use std::collections::VecDeque;

use glam::{Mat3, Quat, Vec3};
use hecs::Entity;
use wgpu_cube::scene::{
    Camera, EditorEntityId, Parent, Scene, SceneStateSnapshot, Transform, TransformComponent,
    WorldTransform,
};

const DEFAULT_HISTORY_CAPACITY: usize = 64;

pub struct EditorHistory {
    current: Option<HistoryEntry>,
    undo_stack: VecDeque<HistoryEntry>,
    redo_stack: VecDeque<HistoryEntry>,
    capacity: usize,
}

impl EditorHistory {
    pub fn new() -> Self {
        Self {
            current: None,
            undo_stack: VecDeque::new(),
            redo_stack: VecDeque::new(),
            capacity: DEFAULT_HISTORY_CAPACITY,
        }
    }

    pub fn is_initialized(&self) -> bool {
        self.current.is_some()
    }

    pub fn initialize(
        &mut self,
        scene: &Scene,
        selected: Option<EditorEntityId>,
        highlighted: Option<EditorEntityId>,
    ) {
        let snapshot = SceneStateSnapshot::capture(scene);
        self.current = Some(HistoryEntry::new(snapshot, selected, highlighted));
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    pub fn update_selection(
        &mut self,
        selected: Option<EditorEntityId>,
        highlighted: Option<EditorEntityId>,
    ) {
        if let Some(current) = self.current.as_mut() {
            current.set_selection(selected, highlighted);
        }
    }

    pub fn record_change(
        &mut self,
        scene: &Scene,
        selected: Option<EditorEntityId>,
        highlighted: Option<EditorEntityId>,
    ) {
        if !self.is_initialized() {
            self.initialize(scene, selected, highlighted);
            return;
        }

        if let Some(current) = self.current.take() {
            self.push_undo_entry(current);
        }

        let snapshot = SceneStateSnapshot::capture(scene);
        self.current = Some(HistoryEntry::new(snapshot, selected, highlighted));
        self.redo_stack.clear();
    }

    pub fn undo(&mut self, scene: &mut Scene) -> Option<HistorySelection> {
        let current = self.current.take()?;
        let mut entry = self.undo_stack.pop_back()?;
        self.push_redo_entry(current);
        let selection = entry.restore(scene);
        self.current = Some(entry);
        Some(selection)
    }

    pub fn redo(&mut self, scene: &mut Scene) -> Option<HistorySelection> {
        let current = self.current.take()?;
        let mut entry = self.redo_stack.pop_back()?;
        self.push_undo_entry(current);
        let selection = entry.restore(scene);
        self.current = Some(entry);
        Some(selection)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    fn push_undo_entry(&mut self, entry: HistoryEntry) {
        if self.undo_stack.len() == self.capacity {
            self.undo_stack.pop_front();
        }
        self.undo_stack.push_back(entry);
    }

    fn push_redo_entry(&mut self, entry: HistoryEntry) {
        if self.redo_stack.len() == self.capacity {
            self.redo_stack.pop_front();
        }
        self.redo_stack.push_back(entry);
    }
}

#[derive(Clone, Copy, Debug)]
pub struct HistorySelection {
    pub selected: Option<EditorEntityId>,
    pub highlighted: Option<EditorEntityId>,
}

struct HistoryEntry {
    snapshot: Option<SceneStateSnapshot>,
    selected: Option<EditorEntityId>,
    highlighted: Option<EditorEntityId>,
}

impl HistoryEntry {
    fn new(
        snapshot: SceneStateSnapshot,
        selected: Option<EditorEntityId>,
        highlighted: Option<EditorEntityId>,
    ) -> Self {
        Self {
            snapshot: Some(snapshot),
            selected,
            highlighted,
        }
    }

    fn set_selection(
        &mut self,
        selected: Option<EditorEntityId>,
        highlighted: Option<EditorEntityId>,
    ) {
        self.selected = selected;
        self.highlighted = highlighted;
    }

    fn restore(&mut self, scene: &mut Scene) -> HistorySelection {
        let current_camera = *scene.camera();
        let snapshot = self
            .snapshot
            .take()
            .expect("history entry snapshot must be present");
        *scene = snapshot.into_scene();
        scene.set_camera(current_camera);
        if let Some(entity) = scene.active_camera_entity() {
            Self::apply_camera_to_entity(scene, entity, &current_camera);
        }
        let selection = self.selection();
        self.snapshot = Some(SceneStateSnapshot::capture(scene));
        selection
    }

    fn selection(&self) -> HistorySelection {
        HistorySelection {
            selected: self.selected,
            highlighted: self.highlighted,
        }
    }

    fn apply_camera_to_entity(scene: &mut Scene, entity: Entity, camera: &Camera) {
        if !scene.main_world().contains(entity) {
            return;
        }

        let parent_world = {
            let world = scene.main_world();
            world
                .get::<&Parent>(entity)
                .ok()
                .and_then(|parent| world.get::<&WorldTransform>(parent.0).ok().map(|wt| wt.0))
        };

        {
            let world = scene.main_world_mut();

            if let Ok(mut transform) = world.get::<&mut TransformComponent>(entity) {
                Self::update_local_transform(&mut transform.0, parent_world, camera);
            }

            if let Ok(mut world_transform) = world.get::<&mut WorldTransform>(entity) {
                Self::update_world_transform(&mut world_transform.0, camera);
            }
        }
    }

    fn update_local_transform(
        transform: &mut Transform,
        parent_world: Option<Transform>,
        camera: &Camera,
    ) {
        let desired_rotation = Self::camera_rotation(camera);
        let desired_translation = camera.eye;

        let (target_translation, target_rotation) = if let Some(parent) = parent_world {
            let parent_rotation = parent.rotation;
            let parent_inverse = parent_rotation.conjugate();
            let offset = desired_translation - parent.translation;
            let rotated = parent_inverse * offset;
            let local_translation = Self::safe_divide(rotated, parent.scale);
            let local_rotation = parent_inverse * desired_rotation;
            (local_translation, local_rotation)
        } else {
            (desired_translation, desired_rotation)
        };

        if !transform.translation.abs_diff_eq(target_translation, 1e-5) {
            transform.translation = target_translation;
        }

        if !transform.rotation.abs_diff_eq(target_rotation, 1e-5) {
            transform.rotation = target_rotation;
        }
    }

    fn update_world_transform(transform: &mut Transform, camera: &Camera) {
        let desired_rotation = Self::camera_rotation(camera);

        if !transform.translation.abs_diff_eq(camera.eye, 1e-5) {
            transform.translation = camera.eye;
        }

        if !transform.rotation.abs_diff_eq(desired_rotation, 1e-5) {
            transform.rotation = desired_rotation;
        }
    }

    fn camera_rotation(camera: &Camera) -> Quat {
        let forward = (camera.target - camera.eye)
            .try_normalize()
            .unwrap_or(Vec3::NEG_Z);
        let raw_up = camera.up.try_normalize().unwrap_or(Vec3::Y);
        let right = forward.cross(raw_up).try_normalize().unwrap_or(Vec3::X);
        let up = right.cross(forward).try_normalize().unwrap_or(Vec3::Y);
        Quat::from_mat3(&Mat3::from_cols(right, up, -forward))
    }

    fn safe_divide(value: Vec3, divisor: Vec3) -> Vec3 {
        Vec3::new(
            if divisor.x.abs() > f32::EPSILON {
                value.x / divisor.x
            } else {
                value.x
            },
            if divisor.y.abs() > f32::EPSILON {
                value.y / divisor.y
            } else {
                value.y
            },
            if divisor.z.abs() > f32::EPSILON {
                value.z / divisor.z
            } else {
                value.z
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use wgpu::Color;
    use wgpu_cube::scene::{CameraComponent, Transform, TransformComponent, WorldTransform};

    #[test]
    fn undo_restores_scene_while_preserving_camera() {
        let mut scene = Scene::new();
        let mut history = EditorHistory::new();
        history.initialize(&scene, None, None);

        let initial_clear_color = scene.environment().clear_color();

        {
            let camera = scene.camera_mut();
            camera.eye = Vec3::new(1.0, 2.0, 3.0);
            camera.target = Vec3::new(-4.0, 0.5, 0.25);
        }
        let moved_camera = *scene.camera();

        scene.environment_mut().set_clear_color(Color {
            r: 0.2,
            g: 0.4,
            b: 0.6,
            a: 1.0,
        });
        history.record_change(&scene, None, None);

        assert_ne!(scene.environment().clear_color(), initial_clear_color);

        history
            .undo(&mut scene)
            .expect("undo should restore previous scene state");

        let restored_camera = scene.camera();
        assert_eq!(restored_camera.eye, moved_camera.eye);
        assert_eq!(restored_camera.target, moved_camera.target);
        assert_eq!(restored_camera.up, moved_camera.up);
        assert_eq!(restored_camera.projection(), moved_camera.projection());
        assert_eq!(scene.environment().clear_color(), initial_clear_color);
    }

    #[test]
    fn undo_preserves_active_camera_entity_transform() {
        let mut scene = Scene::new();

        let camera_entity = {
            let components = (
                TransformComponent(Transform::IDENTITY),
                WorldTransform(Transform::IDENTITY),
                CameraComponent::default(),
            );
            scene.main_world_mut().spawn(components)
        };

        scene.set_active_camera_entity(Some(camera_entity));
        scene.propagate_transforms();

        let mut history = EditorHistory::new();
        history.initialize(&scene, None, None);

        scene.environment_mut().set_clear_color(Color {
            r: 0.1,
            g: 0.2,
            b: 0.3,
            a: 1.0,
        });
        history.record_change(&scene, None, None);

        let mut camera = *scene.camera();
        camera.eye = Vec3::new(2.0, 3.0, -4.0);
        camera.target = Vec3::new(1.5, 3.5, -4.5);
        camera.up = Vec3::Y;
        scene.set_camera(camera);

        let expected_rotation = HistoryEntry::camera_rotation(&camera);

        {
            let world = scene.main_world_mut();
            if let Ok(mut transform) = world.get::<&mut TransformComponent>(camera_entity) {
                transform.0.translation = camera.eye;
                transform.0.rotation = expected_rotation;
                transform.0.scale = Vec3::ONE;
            }
        }

        {
            let world = scene.main_world_mut();
            if let Ok(mut world_transform) = world.get::<&mut WorldTransform>(camera_entity) {
                world_transform.0.translation = camera.eye;
                world_transform.0.rotation = expected_rotation;
                world_transform.0.scale = Vec3::ONE;
            }
        }

        let camera_before = *scene.camera();

        history.undo(&mut scene).expect("undo should succeed");
        scene.propagate_transforms();

        let camera_after = scene.camera();
        assert_eq!(camera_after.eye, camera_before.eye);
        assert_eq!(camera_after.target, camera_before.target);
        assert_eq!(camera_after.up, camera_before.up);

        let active_after = scene
            .active_camera_entity()
            .expect("active camera entity should persist after undo");

        let world = scene.main_world();

        let transform = world
            .get::<&TransformComponent>(active_after)
            .expect("camera transform component should exist");
        assert!(transform.0.translation.abs_diff_eq(camera_before.eye, 1e-5));
        assert!(transform.0.rotation.abs_diff_eq(expected_rotation, 1e-5));

        let world_transform = world
            .get::<&WorldTransform>(active_after)
            .expect("camera world transform should exist");
        assert!(world_transform
            .0
            .translation
            .abs_diff_eq(camera_before.eye, 1e-5));
        assert!(world_transform
            .0
            .rotation
            .abs_diff_eq(expected_rotation, 1e-5));
    }
}
