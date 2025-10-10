use super::animation::{AnimationClip, AnimationState};
use super::internal::{animations, composition, debug, lights, rendering, transforms};
use crate::asset::Assets;
use crate::environment::Environment;
use crate::renderer::{RenderBatcher, Renderer};
use crate::scene::Camera;
use crate::time::Instant;
use hecs::World;

pub struct Scene {
    pub world: World,
    pub assets: Assets,
    time: f64,
    last_frame: Option<Instant>,
    animations: Vec<AnimationClip>,
    animation_states: Vec<AnimationState>,
    camera: Camera,
    environment: Environment,
}

impl Scene {
    pub fn new() -> Self {
        Self {
            world: World::new(),
            assets: Assets::default(),
            time: 0.0,
            last_frame: None,
            animations: Vec::new(),
            animation_states: Vec::new(),
            camera: Camera::default(),
            environment: Environment::default(),
        }
    }

    pub fn init_timer(&mut self) {
        self.last_frame = Some(Instant::now());
    }

    pub fn time(&self) -> f64 {
        self.time
    }

    pub fn last_frame(&self) -> Instant {
        self.last_frame
            .expect("Scene timer not initialized - call init_timer() first")
    }

    pub fn set_last_frame(&mut self, instant: Instant) {
        self.last_frame = Some(instant);
    }

    pub fn animations(&self) -> &[AnimationClip] {
        &self.animations
    }

    pub(crate) fn animations_mut(&mut self) -> &mut Vec<AnimationClip> {
        &mut self.animations
    }

    pub fn animation_states(&self) -> &[AnimationState] {
        &self.animation_states
    }

    pub fn animation_states_mut(&mut self) -> &mut Vec<AnimationState> {
        &mut self.animation_states
    }

    pub fn camera(&self) -> &Camera {
        &self.camera
    }

    pub fn camera_mut(&mut self) -> &mut Camera {
        &mut self.camera
    }

    pub fn set_camera(&mut self, camera: Camera) {
        self.camera = camera;
    }

    pub fn environment(&self) -> &Environment {
        &self.environment
    }

    pub fn environment_mut(&mut self) -> &mut Environment {
        &mut self.environment
    }

    pub fn set_environment(&mut self, environment: Environment) {
        self.environment = environment;
    }

    pub fn add_animation_clip(&mut self, clip: AnimationClip) -> usize {
        let index = self.animations.len();
        self.animations.push(clip);
        index
    }

    pub fn play_animation(&mut self, clip_index: usize, looping: bool) -> Option<usize> {
        if clip_index >= self.animations.len() {
            return None;
        }

        let mut state = AnimationState::new(clip_index);
        state.looping = looping;
        let index = self.animation_states.len();
        self.animation_states.push(state);
        Some(index)
    }

    pub fn update(&mut self, dt: f64) {
        self.time += dt;

        animations::advance_animations(
            &mut self.world,
            &self.animations,
            &mut self.animation_states,
            dt,
        );
        animations::update_rotate_animations(&mut self.world, dt);
        animations::update_orbit_animations(&mut self.world, self.time);

        transforms::propagate_transforms(&mut self.world);
    }

    pub fn render(
        &mut self,
        renderer: &mut Renderer,
        batcher: &mut RenderBatcher,
    ) -> Result<crate::renderer::RenderFrame, wgpu::SurfaceError> {
        batcher.clear();
        let camera = rendering::CameraVectors::from_renderer(renderer);

        for object in rendering::build_render_objects(&self.world, camera) {
            batcher.add(object);
        }

        let lights = lights::collect_lights(&self.world, camera);
        renderer.set_lights(&lights);

        renderer.render(&self.assets, batcher, &lights, &self.environment)
    }

    pub fn add_default_lighting(&mut self) -> usize {
        lights::add_default_lighting(&mut self.world)
    }

    pub fn has_any_lights(&self) -> bool {
        lights::has_any_lights(&self.world)
    }

    pub fn merge_as_child(&mut self, parent_entity: hecs::Entity, other: Scene) {
        composition::merge_as_child(self, parent_entity, other);
    }

    pub fn debug_print_transforms(&self) {
        debug::debug_print_transforms(&self.world);
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        World,
        Assets,
        Environment,
        Vec<AnimationClip>,
        Vec<AnimationState>,
    ) {
        (
            self.world,
            self.assets,
            self.environment,
            self.animations,
            self.animation_states,
        )
    }
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}
