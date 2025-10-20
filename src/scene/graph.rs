use super::animation::{AnimationClip, AnimationState};
use super::internal::{animations, transforms};
use crate::scene::components::TransformComponent;
use crate::scene::transform::Transform;
use hecs::World;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SceneNodeId(u32);

impl SceneNodeId {
    pub(crate) fn new(value: u32) -> Self {
        SceneNodeId(value)
    }

    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
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
    pub(crate) fn new(_id: SceneNodeId, name: impl Into<String>, instance: SceneInstance) -> Self {
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
}

impl SceneInstance {
    pub(crate) fn new() -> Self {
        Self {
            world: World::new(),
            animations: Vec::new(),
            animation_states: Vec::new(),
            rest_pose: None,
        }
    }

    pub(crate) fn update(&mut self, dt: f64, absolute_time: f64) {
        let world = &mut self.world;
        let animations = &self.animations;
        let animation_states = &mut self.animation_states;

        animations::advance_animations(world, animations, animation_states, dt);
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

    pub(crate) fn capture_rest_pose(&mut self) {
        if self.rest_pose.is_some() {
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

    pub(crate) fn into_world(self) -> World {
        self.world
    }
}
