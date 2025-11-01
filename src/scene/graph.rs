use super::animation::{AnimationClip, AnimationState};
use super::internal::{animations, transforms};
use crate::asset::Assets;
use crate::scene::components::TransformComponent;
use crate::scene::transform::Transform;
use hecs::{Entity, World};
use slotmap::new_key_type;
use std::collections::HashMap;

new_key_type! {
    pub struct SceneNodeId;
}

pub(crate) struct SceneNode {
    name: String,
    parent: Option<SceneNodeId>,
    children: Vec<SceneNodeId>,
    local_transform: Transform,
    world_transform: Transform,
    instance: SceneInstance,
}

impl SceneNode {
    pub(crate) fn new(name: impl Into<String>, instance: SceneInstance) -> Self {
        Self {
            name: name.into(),
            parent: None,
            children: Vec::new(),
            local_transform: Transform::IDENTITY,
            world_transform: Transform::IDENTITY,
            instance,
        }
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
    }

    pub(crate) fn parent(&self) -> Option<SceneNodeId> {
        self.parent
    }

    pub(crate) fn set_parent(&mut self, parent: Option<SceneNodeId>) {
        self.parent = parent;
    }

    pub(crate) fn children(&self) -> &[SceneNodeId] {
        &self.children
    }

    pub(crate) fn add_child(&mut self, child: SceneNodeId) {
        self.children.push(child);
    }

    pub(crate) fn remove_child(&mut self, child: SceneNodeId) {
        if let Some(index) = self.children.iter().position(|&id| id == child) {
            self.children.swap_remove(index);
        }
    }

    pub(crate) fn local_transform(&self) -> &Transform {
        &self.local_transform
    }

    pub(crate) fn local_transform_mut(&mut self) -> &mut Transform {
        &mut self.local_transform
    }

    pub(crate) fn set_local_transform(&mut self, transform: Transform) {
        self.local_transform = transform;
    }

    pub(crate) fn world_transform(&self) -> &Transform {
        &self.world_transform
    }

    pub(crate) fn set_world_transform(&mut self, transform: Transform) {
        self.world_transform = transform;
    }

    pub(crate) fn instance(&self) -> &SceneInstance {
        &self.instance
    }

    pub(crate) fn instance_mut(&mut self) -> &mut SceneInstance {
        &mut self.instance
    }
}

pub(crate) struct SceneInstance {
    world: World,
    animations: Vec<AnimationClip>,
    animation_states: Vec<AnimationState>,
    rest_pose: Option<HashMap<hecs::Entity, Transform>>,
    active_camera: Option<Entity>,
}

impl SceneInstance {
    pub(crate) fn new() -> Self {
        Self {
            world: World::new(),
            animations: Vec::new(),
            animation_states: Vec::new(),
            rest_pose: None,
            active_camera: None,
        }
    }

    pub(crate) fn update(&mut self, assets: &mut Assets, dt: f64, absolute_time: f64) {
        let world = &mut self.world;
        let animations = &self.animations;
        let animation_states = &mut self.animation_states;

        animations::advance_animations(world, assets, animations, animation_states, dt);
        animations::update_rotate_animations(world, dt);
        animations::update_orbit_animations(world, absolute_time);
    }

    pub(crate) fn propagate_transforms(&mut self) {
        transforms::propagate_transforms(&mut self.world);
    }

    pub(crate) fn add_animation_clip(&mut self, clip: AnimationClip) -> usize {
        let index = self.animations.len();
        self.animations.push(clip);
        index
    }

    pub(crate) fn play_animation(&mut self, clip_index: usize, looping: bool) -> Option<usize> {
        if clip_index >= self.animations.len() {
            return None;
        }

        self.capture_rest_pose();

        let mut state = AnimationState::new(clip_index);
        state.looping = looping;
        let index = self.animation_states.len();
        self.animation_states.push(state);
        Some(index)
    }

    pub(crate) fn animations(&self) -> &[AnimationClip] {
        &self.animations
    }

    pub(crate) fn animation_states(&self) -> &[AnimationState] {
        &self.animation_states
    }

    pub(crate) fn animation_states_mut(&mut self) -> &mut [AnimationState] {
        &mut self.animation_states
    }

    pub(crate) fn set_animation_data(
        &mut self,
        animations: Vec<AnimationClip>,
        animation_states: Vec<AnimationState>,
    ) {
        self.animations = animations;
        self.animation_states = animation_states;
    }

    pub(crate) fn take_animation_data(&mut self) -> (Vec<AnimationClip>, Vec<AnimationState>) {
        (
            std::mem::take(&mut self.animations),
            std::mem::take(&mut self.animation_states),
        )
    }

    pub(crate) fn begin_playback(&mut self) {
        self.rest_pose = None;
        self.capture_rest_pose();
    }

    pub(crate) fn capture_rest_pose(&mut self) {
        if let Some(rest) = self.rest_pose.as_mut() {
            for (entity, transform) in self.world.query::<&TransformComponent>().iter() {
                rest.entry(entity).or_insert(transform.0);
            }
            return;
        }

        let mut rest = HashMap::new();
        for (entity, transform) in self.world.query::<&TransformComponent>().iter() {
            rest.insert(entity, transform.0);
        }
        self.rest_pose = Some(rest);
    }

    pub(crate) fn restore_rest_pose(&mut self) {
        let Some(rest) = self.rest_pose.take() else {
            return;
        };

        for (entity, transform) in rest {
            if let Ok(mut component) = self.world.get::<&mut TransformComponent>(entity) {
                component.0 = transform;
            }
        }

        self.propagate_transforms();
    }

    pub(crate) fn push_animation_state(&mut self, state: AnimationState) -> Option<usize> {
        if state.clip_index >= self.animations.len() {
            return None;
        }

        if state.playing {
            self.capture_rest_pose();
        }

        let index = self.animation_states.len();
        self.animation_states.push(state);
        Some(index)
    }

    pub(crate) fn world(&self) -> &World {
        &self.world
    }

    pub(crate) fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    pub(crate) fn active_camera(&self) -> Option<Entity> {
        self.active_camera
    }

    pub(crate) fn set_active_camera(&mut self, camera: Option<Entity>) {
        self.active_camera = camera;
    }

    pub(crate) fn into_world(self) -> World {
        self.world
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::components::TransformComponent;

    #[test]
    fn capture_rest_pose_extends_to_new_entities() {
        let mut instance = SceneInstance::new();

        let entity_a =
            instance
                .world_mut()
                .spawn((TransformComponent(Transform::from_translation(
                    glam::Vec3::new(1.0, 0.0, 0.0),
                )),));

        instance.capture_rest_pose();

        let entity_b =
            instance
                .world_mut()
                .spawn((TransformComponent(Transform::from_translation(
                    glam::Vec3::new(-2.0, 0.0, 0.0),
                )),));

        instance.capture_rest_pose();

        {
            let mut transform_a = instance
                .world_mut()
                .get::<&mut TransformComponent>(entity_a)
                .unwrap();
            transform_a.0.translation = glam::Vec3::new(5.0, 5.0, 5.0);
        }

        {
            let mut transform_b = instance
                .world_mut()
                .get::<&mut TransformComponent>(entity_b)
                .unwrap();
            transform_b.0.translation = glam::Vec3::new(-8.0, 1.0, 3.0);
        }

        instance.restore_rest_pose();

        let restored_a = instance
            .world()
            .get::<&TransformComponent>(entity_a)
            .unwrap()
            .0;
        let restored_b = instance
            .world()
            .get::<&TransformComponent>(entity_b)
            .unwrap()
            .0;

        assert!(restored_a
            .translation
            .abs_diff_eq(glam::Vec3::new(1.0, 0.0, 0.0), 1e-5));
        assert!(restored_b
            .translation
            .abs_diff_eq(glam::Vec3::new(-2.0, 0.0, 0.0), 1e-5));
    }

    #[test]
    fn begin_playback_refreshes_existing_rest_pose() {
        let mut instance = SceneInstance::new();
        let entity = instance
            .world_mut()
            .spawn((TransformComponent(Transform::from_translation(
                glam::Vec3::new(1.0, 2.0, 3.0),
            )),));

        instance.capture_rest_pose();

        {
            let mut transform = instance
                .world_mut()
                .get::<&mut TransformComponent>(entity)
                .unwrap();
            transform.0.translation = glam::Vec3::new(-4.0, 5.0, -6.0);
        }

        instance.begin_playback();

        {
            let mut transform = instance
                .world_mut()
                .get::<&mut TransformComponent>(entity)
                .unwrap();
            transform.0.translation = glam::Vec3::new(42.0, 0.0, 0.0);
        }

        instance.restore_rest_pose();

        let restored = instance
            .world()
            .get::<&TransformComponent>(entity)
            .unwrap()
            .0;

        assert!(restored
            .translation
            .abs_diff_eq(glam::Vec3::new(-4.0, 5.0, -6.0), 1e-5));
    }
}
