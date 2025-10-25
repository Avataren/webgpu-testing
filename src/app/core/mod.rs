mod render_hooks;
mod runtime;
mod scheduler;

use render_hooks::RenderHooks;
use runtime::RuntimeController;
use scheduler::Scheduler;

pub use render_hooks::{RenderParams, RenderResult};
pub use runtime::{RuntimeMode, RuntimeStateHandle, RuntimeTransition};
pub use scheduler::{
    FrameStep, GpuUpdateContext, GpuUpdateSystem, StartupContext, StartupSystem, UpdateContext,
    UpdateSystem,
};

use crate::renderer::{RenderBatcher, Renderer};
use crate::scene::Scene;
use crate::settings::RenderSettings;

pub trait Plugin {
    fn build(&self, app: &mut AppBuilder);
}

pub struct AppBuilder {
    scheduler: Scheduler,
    settings: RenderSettings,
    exit_on_escape: bool,
}

impl Default for AppBuilder {
    fn default() -> Self {
        Self {
            scheduler: Scheduler::default(),
            settings: RenderSettings::load(),
            exit_on_escape: true,
        }
    }
}

impl AppBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_startup_system<F>(&mut self, system: F) -> &mut Self
    where
        F: for<'a> FnMut(&mut StartupContext<'a>) + 'static,
    {
        self.scheduler.add_startup_system(Box::new(system));
        self
    }

    pub fn add_system<F>(&mut self, system: F) -> &mut Self
    where
        F: for<'a> FnMut(&mut UpdateContext<'a>) + 'static,
    {
        self.scheduler.add_update_system(Box::new(system));
        self
    }

    pub fn add_gpu_system<F>(&mut self, system: F) -> &mut Self
    where
        F: for<'a> FnMut(&mut GpuUpdateContext<'a>) + 'static,
    {
        self.scheduler.add_gpu_system(Box::new(system));
        self
    }

    pub fn add_plugin<P: Plugin>(&mut self, plugin: P) -> &mut Self {
        plugin.build(self);
        self
    }

    pub fn set_settings(&mut self, settings: RenderSettings) -> &mut Self {
        self.settings = settings;
        self
    }

    pub fn disable_default_textures(&mut self) -> &mut Self {
        self.scheduler.disable_default_textures();
        self
    }

    pub fn disable_default_lighting(&mut self) -> &mut Self {
        self.scheduler.disable_default_lighting();
        self
    }

    pub fn disable_escape_exit(&mut self) -> &mut Self {
        self.exit_on_escape = false;
        self
    }

    pub fn skip_initial_frames(&mut self, frames: u32) -> &mut Self {
        self.scheduler.skip_initial_frames(frames);
        self
    }

    pub fn build(self) -> AppCore {
        AppCore {
            batcher: RenderBatcher::new(),
            scheduler: self.scheduler,
            runtime: RuntimeController::new(),
            render_hooks: RenderHooks::new(),
            settings: self.settings,
            scene: Scene::new(),
            exit_on_escape: self.exit_on_escape,
            exit_requested: false,
        }
    }
}

pub struct AppCore {
    batcher: RenderBatcher,
    scheduler: Scheduler,
    runtime: RuntimeController,
    render_hooks: RenderHooks,
    settings: RenderSettings,
    scene: Scene,
    exit_on_escape: bool,
    exit_requested: bool,
}

impl AppCore {
    pub fn new() -> Self {
        AppBuilder::default().build()
    }

    pub fn settings(&self) -> &RenderSettings {
        &self.settings
    }

    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    pub fn scene_mut(&mut self) -> &mut Scene {
        &mut self.scene
    }

    pub fn exit_on_escape(&self) -> bool {
        self.exit_on_escape
    }

    pub fn runtime_mode(&self) -> RuntimeMode {
        self.runtime.mode()
    }

    pub fn runtime_state_handle(&self) -> RuntimeStateHandle {
        self.runtime.state_handle()
    }

    pub fn request_exit(&mut self) {
        self.exit_requested = true;
    }

    pub fn clear_exit_request(&mut self) {
        self.exit_requested = false;
    }

    pub fn exit_requested(&self) -> bool {
        self.exit_requested
    }

    pub fn set_custom_render_callback(
        &mut self,
        callback: Box<crate::renderer::CustomRenderCallback>,
    ) {
        self.render_hooks.set_custom_render_callback(callback);
    }

    pub fn set_custom_render_stage(&mut self, stage: crate::renderer::CustomRenderStage) {
        self.render_hooks.set_custom_render_stage(stage);
    }

    pub fn enable_custom_render_shadows(&mut self, enabled: bool) {
        self.render_hooks.enable_custom_render_shadows(enabled);
    }

    pub fn set_custom_render_shadow_query<F>(&mut self, query: F)
    where
        F: FnMut() -> bool + 'static,
    {
        self.render_hooks.set_custom_render_shadow_query(query);
    }

    pub fn begin_frame(&mut self) -> FrameStep {
        self.scheduler.begin_frame(&mut self.scene)
    }

    pub fn sync_runtime_state(&mut self) -> Option<RuntimeTransition> {
        self.runtime.sync_runtime_state(&mut self.scene)
    }

    pub fn toggle_editor_gizmos(&mut self) {
        self.runtime.toggle_editor_gizmos();
    }

    pub fn on_renderer_ready(&mut self, renderer: &mut Renderer) {
        self.scene.init_timer();
        self.scheduler
            .run_startup_systems(&mut self.scene, renderer);
        renderer.update_texture_bind_group(&self.scene.assets);
    }

    pub fn run_update_stage(&mut self, dt: f64) {
        let runtime_mode = self.runtime.mode();
        self.scheduler
            .run_update_stage(&mut self.scene, dt, runtime_mode);
    }

    pub fn run_gpu_systems(&mut self, renderer: &mut Renderer, dt: f64) {
        self.scheduler
            .run_gpu_systems(&mut self.scene, renderer, dt);
    }

    pub fn render_scene(
        &mut self,
        renderer: &mut Renderer,
        frame: &FrameStep,
        params: &RenderParams,
    ) -> Result<RenderResult, wgpu::SurfaceError> {
        if !frame.should_render() {
            return Ok(RenderResult::Skipped);
        }

        let aspect = params
            .render_region
            .map(|region| region.width() as f32 / region.height() as f32)
            .unwrap_or_else(|| renderer.aspect_ratio());
        renderer.set_camera(self.scene.camera(), aspect);

        renderer.set_render_region(params.render_region);

        let mut custom_render_request = self.render_hooks.prepare_request();

        let render_frame = self.scene.render(
            renderer,
            &mut self.batcher,
            &mut custom_render_request,
            self.runtime.gizmos_enabled(),
        )?;

        Ok(RenderResult::Rendered(render_frame))
    }

    pub fn gizmos_enabled(&self) -> bool {
        self.runtime.gizmos_enabled()
    }
}

impl Default for AppCore {
    fn default() -> Self {
        Self::new()
    }
}
