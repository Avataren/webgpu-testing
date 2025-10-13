use super::animation::{AnimationClip, AnimationState};
use super::internal::{animations, transforms};
use hecs::World;

pub(crate) struct SceneInstance {
    world: World,
    animations: Vec<AnimationClip>,
    animation_states: Vec<AnimationState>,
}

impl SceneInstance {
    pub(crate) fn new() -> Self {
        Self {
            world: World::new(),
            animations: Vec::new(),
            animation_states: Vec::new(),
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

    pub(crate) fn world(&self) -> &World {
        &self.world
    }

    pub(crate) fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    pub(crate) fn into_world(self) -> World {
        self.world
    }

    pub(crate) fn set_animations(&mut self, animations: Vec<AnimationClip>) {
        self.animations = animations;
    }

    pub(crate) fn set_animation_states(&mut self, states: Vec<AnimationState>) {
        self.animation_states = states;
    }
}
