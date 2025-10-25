use crate::renderer::texture::{
    DEFAULT_CHECKER_TEXTURE_INDEX, DEFAULT_METALLIC_ROUGHNESS_TEXTURE_INDEX,
    DEFAULT_NORMAL_TEXTURE_INDEX, DEFAULT_WHITE_TEXTURE_INDEX,
};
use crate::renderer::{Renderer, Texture};
use crate::scene::Scene;
use crate::time::Instant;

use super::runtime::RuntimeMode;

pub struct StartupContext<'a> {
    pub scene: &'a mut Scene,
    pub renderer: &'a mut Renderer,
}

pub struct UpdateContext<'a> {
    pub scene: &'a mut Scene,
    pub dt: f64,
    pub runtime: RuntimeMode,
}

pub struct GpuUpdateContext<'a> {
    pub scene: &'a mut Scene,
    pub renderer: &'a mut Renderer,
    pub dt: f64,
}

pub type StartupSystem = Box<dyn for<'a> FnMut(&mut StartupContext<'a>) + 'static>;
pub type UpdateSystem = Box<dyn for<'a> FnMut(&mut UpdateContext<'a>) + 'static>;
pub type GpuUpdateSystem = Box<dyn for<'a> FnMut(&mut GpuUpdateContext<'a>) + 'static>;

pub struct FrameStep {
    dt: f64,
    skip_rendering: bool,
}

impl FrameStep {
    pub fn dt(&self) -> f64 {
        self.dt
    }

    pub fn should_render(&self) -> bool {
        !self.skip_rendering
    }
}

pub struct Scheduler {
    startup_systems: Vec<StartupSystem>,
    update_systems: Vec<UpdateSystem>,
    gpu_systems: Vec<GpuUpdateSystem>,
    auto_init_default_textures: bool,
    auto_add_default_lighting: bool,
    startup_ran: bool,
    frame_counter: u32,
    skip_rendering_until_frame: Option<u32>,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self {
            startup_systems: Vec::new(),
            update_systems: Vec::new(),
            gpu_systems: Vec::new(),
            auto_init_default_textures: true,
            auto_add_default_lighting: true,
            startup_ran: false,
            frame_counter: 0,
            skip_rendering_until_frame: None,
        }
    }
}

impl Scheduler {
    pub fn add_startup_system(&mut self, system: StartupSystem) {
        self.startup_systems.push(system);
    }

    pub fn add_update_system(&mut self, system: UpdateSystem) {
        self.update_systems.push(system);
    }

    pub fn add_gpu_system(&mut self, system: GpuUpdateSystem) {
        self.gpu_systems.push(system);
    }

    pub fn disable_default_textures(&mut self) {
        self.auto_init_default_textures = false;
    }

    pub fn disable_default_lighting(&mut self) {
        self.auto_add_default_lighting = false;
    }

    pub fn skip_initial_frames(&mut self, frames: u32) {
        self.skip_rendering_until_frame = Some(frames);
    }

    pub fn begin_frame(&mut self, scene: &mut Scene) -> FrameStep {
        self.frame_counter = self.frame_counter.saturating_add(1);

        let skip_rendering = if let Some(skip_until) = self.skip_rendering_until_frame {
            if self.frame_counter < skip_until {
                true
            } else {
                self.skip_rendering_until_frame = None;
                false
            }
        } else {
            false
        };

        let now = Instant::now();
        let last_frame = match scene.last_frame_instant() {
            Some(last_frame) => last_frame,
            None => {
                scene.set_last_frame(now);
                now
            }
        };
        let dt = (now - last_frame).as_secs_f64();
        scene.set_last_frame(now);

        FrameStep { dt, skip_rendering }
    }

    pub fn run_startup_systems(&mut self, scene: &mut Scene, renderer: &mut Renderer) {
        if self.startup_ran {
            return;
        }

        if self.auto_init_default_textures && scene.assets.textures.is_empty() {
            self.init_default_textures(scene, renderer);
        }

        for system in &mut self.startup_systems {
            let mut ctx = StartupContext { scene, renderer };
            (system)(&mut ctx);
        }

        if self.auto_add_default_lighting {
            let added_lights = scene.add_default_lighting();
            if added_lights > 0 {
                log::info!("Added {} default lights to scene", added_lights);
            }
        }

        log::info!("Running initial transform propagation...");
        scene.set_animation_playback(false);
        scene.update(0.0);
        log::info!("Initial propagation complete");

        self.startup_ran = true;
    }

    pub fn run_update_stage(&mut self, scene: &mut Scene, dt: f64, runtime: RuntimeMode) {
        if runtime == RuntimeMode::Playing {
            scene.update(dt);
        }

        for system in &mut self.update_systems {
            let mut ctx = UpdateContext { scene, dt, runtime };
            (system)(&mut ctx);
        }

        scene.propagate_transforms();
    }

    pub fn run_gpu_systems(&mut self, scene: &mut Scene, renderer: &mut Renderer, dt: f64) {
        Self::run_gpu_systems_impl(scene, &mut self.gpu_systems, renderer, dt);
    }

    fn run_gpu_systems_impl(
        scene: &mut Scene,
        systems: &mut [GpuUpdateSystem],
        renderer: &mut Renderer,
        dt: f64,
    ) {
        for system in systems {
            let mut ctx = GpuUpdateContext {
                scene,
                renderer,
                dt,
            };
            (system)(&mut ctx);
        }

        scene.propagate_transforms();
    }

    fn init_default_textures(&mut self, scene: &mut Scene, renderer: &mut Renderer) {
        let device = renderer.get_device();
        let queue = renderer.get_queue();

        let white = Texture::white(device, queue);
        let white_handle = scene.assets.textures.insert(white);
        debug_assert_eq!(
            white_handle.index() as u32,
            DEFAULT_WHITE_TEXTURE_INDEX,
            "Default white texture index changed; update the constants in renderer::texture",
        );

        let normal = Texture::default_normal(device, queue);
        let normal_handle = scene.assets.textures.insert(normal);
        debug_assert_eq!(
            normal_handle.index() as u32,
            DEFAULT_NORMAL_TEXTURE_INDEX,
            "Default normal texture index changed; update the constants in renderer::texture",
        );

        let mr = Texture::default_metallic_roughness(device, queue);
        let mr_handle = scene.assets.textures.insert(mr);
        debug_assert_eq!(
            mr_handle.index() as u32,
            DEFAULT_METALLIC_ROUGHNESS_TEXTURE_INDEX,
            "Default metallic-roughness texture index changed; update the constants in renderer::texture",
        );

        let checker = Texture::checkerboard(
            device,
            queue,
            128,
            16,
            [255, 255, 255, 255],
            [24, 24, 24, 255],
            Some("DefaultCheckerboard"),
        );
        let checker_handle = scene.assets.textures.insert(checker);
        debug_assert_eq!(
            checker_handle.index() as u32,
            DEFAULT_CHECKER_TEXTURE_INDEX,
            "Default checker texture index changed; update the constants in renderer::texture",
        );

        log::info!(
            "Initialized default textures (white, normal, metallic-roughness, checkerboard)"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_skip_counts_down() {
        let mut scheduler = Scheduler::default();
        scheduler.skip_initial_frames(2);
        let mut scene = Scene::new();

        let first = scheduler.begin_frame(&mut scene);
        assert!(!first.should_render());

        let second = scheduler.begin_frame(&mut scene);
        assert!(second.should_render());
    }
}
