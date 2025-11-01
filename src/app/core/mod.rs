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
use crate::scene::{Scene, SceneWorkspace, SceneWorkspaceSceneMut};
use crate::settings::RenderSettings;

const DEFAULT_SCENE_DOCUMENT_ID: &str = "app://untitled";

fn default_workspace() -> SceneWorkspace {
    let mut workspace = SceneWorkspace::new();
    workspace.open_scene(DEFAULT_SCENE_DOCUMENT_ID.to_string(), Scene::new());
    workspace
}

pub trait Plugin {
    fn build(&self, app: &mut AppBuilder);
}

pub struct AppBuilder {
    scheduler: Scheduler,
    settings: RenderSettings,
    exit_on_escape: bool,
    workspace: Option<SceneWorkspace>,
}

impl Default for AppBuilder {
    fn default() -> Self {
        Self {
            scheduler: Scheduler::default(),
            settings: RenderSettings::load(),
            exit_on_escape: true,
            workspace: None,
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

    pub fn with_workspace(mut self, workspace: SceneWorkspace) -> Self {
        self.workspace = Some(workspace);
        self
    }

    pub fn set_workspace(&mut self, workspace: SceneWorkspace) -> &mut Self {
        self.workspace = Some(workspace);
        self
    }

    pub fn build(self) -> AppCore {
        let Self {
            scheduler,
            settings,
            exit_on_escape,
            mut workspace,
        } = self;

        let workspace = workspace.take().unwrap_or_else(default_workspace);

        AppCore {
            batcher: RenderBatcher::new(),
            scheduler,
            runtime: RuntimeController::new(),
            render_hooks: RenderHooks::new(),
            settings,
            workspace,
            exit_on_escape,
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
    workspace: SceneWorkspace,
    exit_on_escape: bool,
    exit_requested: bool,
}

impl AppCore {
    pub fn new() -> Self {
        AppBuilder::default().build()
    }

    pub fn from_workspace(workspace: SceneWorkspace) -> Self {
        AppBuilder::default().with_workspace(workspace).build()
    }

    pub fn settings(&self) -> &RenderSettings {
        &self.settings
    }

    pub fn workspace(&self) -> &SceneWorkspace {
        &self.workspace
    }

    pub fn workspace_mut(&mut self) -> &mut SceneWorkspace {
        &mut self.workspace
    }

    pub fn active_scene(&self) -> &Scene {
        self.workspace
            .active_scene()
            .expect("AppCore requires an active scene")
    }

    pub fn active_scene_mut(&mut self) -> SceneWorkspaceSceneMut<'_> {
        self.workspace
            .active_scene_mut()
            .expect("AppCore requires an active scene")
    }

    pub fn scene(&self) -> &Scene {
        self.active_scene()
    }

    pub fn scene_mut(&mut self) -> SceneWorkspaceSceneMut<'_> {
        self.active_scene_mut()
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

    pub fn begin_frame(&mut self) -> Option<FrameStep> {
        let Some((handle, mut scene)) = self.workspace.active_scene_handle_and_mut() else {
            log::warn!("Skipping frame: no active scene available");
            return None;
        };

        Some(self.scheduler.begin_frame(handle, &mut scene))
    }

    pub fn sync_runtime_state(&mut self) -> Option<RuntimeTransition> {
        let Some((_, mut scene)) = self.workspace.active_scene_handle_and_mut() else {
            log::warn!("Runtime sync skipped: no active scene available");
            return None;
        };

        self.runtime.sync_runtime_state(&mut scene)
    }

    pub fn toggle_editor_gizmos(&mut self) {
        self.runtime.toggle_editor_gizmos();
    }

    pub fn on_renderer_ready(&mut self, renderer: &mut Renderer) {
        let Some((handle, mut scene)) = self.workspace.active_scene_handle_and_mut() else {
            log::warn!("Renderer ready but no active scene to initialize");
            return;
        };
        scene.init_timer();
        drop(scene);

        if !self
            .scheduler
            .run_startup_systems(&mut self.workspace, handle, renderer)
        {
            return;
        }

        renderer.update_texture_bind_group(self.workspace.assets());
    }

    pub fn run_update_stage(&mut self, frame: &FrameStep) {
        let runtime_mode = self.runtime.mode();
        let handle = frame.scene_handle();
        if !self
            .scheduler
            .run_update_stage(&mut self.workspace, handle, frame.dt(), runtime_mode)
        {
            log::warn!(
                "Update stage skipped: active scene handle {:?} is unavailable",
                handle
            );
        }
    }

    pub fn run_gpu_systems(&mut self, renderer: &mut Renderer, frame: &FrameStep) {
        let handle = frame.scene_handle();
        if !self
            .scheduler
            .run_gpu_systems(&mut self.workspace, handle, renderer, frame.dt())
        {
            log::warn!(
                "GPU systems skipped: active scene handle {:?} is unavailable",
                handle
            );
        }
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
        let gizmos_enabled = self.runtime.gizmos_enabled();
        let handle = frame.scene_handle();
        let mut scene = match self.workspace.scene_mut_by_handle(handle) {
            Some(scene) => scene,
            None => {
                log::warn!(
                    "Render skipped: active scene handle {:?} is unavailable",
                    handle
                );
                return Ok(RenderResult::Skipped);
            }
        };

        let mut render_hooks = RenderHooks::new();
        std::mem::swap(&mut render_hooks, &mut self.render_hooks);
        let mut batcher = RenderBatcher::new();
        std::mem::swap(&mut batcher, &mut self.batcher);

        let mut custom_render_request = render_hooks.prepare_request();
        renderer.set_camera(scene.camera(), aspect);
        renderer.set_render_region(params.render_region);

        let render_result = scene.render(
            renderer,
            &mut batcher,
            custom_render_request.as_mut(),
            gizmos_enabled,
        );

        drop(scene);

        std::mem::swap(&mut batcher, &mut self.batcher);
        std::mem::swap(&mut render_hooks, &mut self.render_hooks);

        let render_frame = render_result?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_creates_active_scene() {
        let core = AppBuilder::new().build();
        assert!(core.workspace().active_scene().is_some());
    }

    #[test]
    fn build_uses_supplied_workspace() {
        let mut workspace = SceneWorkspace::new();
        let handle = workspace.open_scene("test".into(), Scene::new());
        let default_material = workspace.assets().default_material_handle();

        let core = AppBuilder::new().with_workspace(workspace).build();

        assert_eq!(core.workspace().active_scene_handle(), Some(handle));
        assert_eq!(
            core.workspace().assets().default_material_handle(),
            default_material
        );
    }
}
