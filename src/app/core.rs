use std::sync::{Arc, Mutex};

use crate::renderer::{
    texture::{
        DEFAULT_CHECKER_TEXTURE_INDEX, DEFAULT_METALLIC_ROUGHNESS_TEXTURE_INDEX,
        DEFAULT_NORMAL_TEXTURE_INDEX, DEFAULT_WHITE_TEXTURE_INDEX,
    },
    CustomRenderCallback, CustomRenderRequest, CustomRenderStage, RenderBatcher, RenderFrame,
    RenderRegion, Renderer, Texture,
};
use crate::scene::{Scene, SceneSnapshot};
use crate::settings::RenderSettings;
use crate::time::Instant;

const DEFAULT_HDR_ENVIRONMENT: &str = "web/assets/hdr/kloppenheim_06_puresky_4k.hdr";

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeMode {
    Editor,
    Playing,
}

#[derive(Clone)]
pub struct RuntimeStateHandle {
    inner: Arc<Mutex<RuntimeState>>,
}

struct RuntimeState {
    desired_mode: RuntimeMode,
    active_mode: RuntimeMode,
}

impl RuntimeStateHandle {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RuntimeState {
                desired_mode: RuntimeMode::Editor,
                active_mode: RuntimeMode::Editor,
            })),
        }
    }

    pub fn request_mode(&self, mode: RuntimeMode) {
        if let Ok(mut state) = self.inner.lock() {
            state.desired_mode = mode;
        }
    }

    pub fn desired_mode(&self) -> RuntimeMode {
        self.inner
            .lock()
            .map(|state| state.desired_mode)
            .unwrap_or(RuntimeMode::Editor)
    }

    pub fn active_mode(&self) -> RuntimeMode {
        self.inner
            .lock()
            .map(|state| state.active_mode)
            .unwrap_or(RuntimeMode::Editor)
    }
}

impl Default for RuntimeStateHandle {
    fn default() -> Self {
        Self::new()
    }
}

pub type StartupSystem = Box<dyn for<'a> FnMut(&mut StartupContext<'a>) + 'static>;
pub type UpdateSystem = Box<dyn for<'a> FnMut(&mut UpdateContext<'a>) + 'static>;
pub type GpuUpdateSystem = Box<dyn for<'a> FnMut(&mut GpuUpdateContext<'a>) + 'static>;

pub trait Plugin {
    fn build(&self, app: &mut AppBuilder);
}

pub struct AppBuilder {
    startup_systems: Vec<StartupSystem>,
    update_systems: Vec<UpdateSystem>,
    gpu_systems: Vec<GpuUpdateSystem>,
    auto_init_default_textures: bool,
    auto_add_default_lighting: bool,
    skip_initial_frames: Option<u32>,
    settings: RenderSettings,
    exit_on_escape: bool,
}

impl Default for AppBuilder {
    fn default() -> Self {
        Self {
            startup_systems: Vec::new(),
            update_systems: Vec::new(),
            gpu_systems: Vec::new(),
            auto_init_default_textures: true,
            auto_add_default_lighting: true,
            skip_initial_frames: None,
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
        self.startup_systems.push(Box::new(system));
        self
    }

    pub fn add_system<F>(&mut self, system: F) -> &mut Self
    where
        F: for<'a> FnMut(&mut UpdateContext<'a>) + 'static,
    {
        self.update_systems.push(Box::new(system));
        self
    }

    pub fn add_gpu_system<F>(&mut self, system: F) -> &mut Self
    where
        F: for<'a> FnMut(&mut GpuUpdateContext<'a>) + 'static,
    {
        self.gpu_systems.push(Box::new(system));
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
        self.auto_init_default_textures = false;
        self
    }

    pub fn disable_default_lighting(&mut self) -> &mut Self {
        self.auto_add_default_lighting = false;
        self
    }

    pub fn disable_escape_exit(&mut self) -> &mut Self {
        self.exit_on_escape = false;
        self
    }

    pub fn skip_initial_frames(&mut self, frames: u32) -> &mut Self {
        self.skip_initial_frames = Some(frames);
        self
    }

    pub fn build(self) -> AppCore {
        AppCore {
            batcher: RenderBatcher::new(),
            startup_systems: self.startup_systems,
            update_systems: self.update_systems,
            gpu_systems: self.gpu_systems,
            auto_init_default_textures: self.auto_init_default_textures,
            auto_add_default_lighting: self.auto_add_default_lighting,
            startup_ran: false,
            frame_counter: 0,
            skip_rendering_until_frame: self.skip_initial_frames,
            settings: self.settings,
            scene: Scene::new(),
            exit_on_escape: self.exit_on_escape,
            exit_requested: false,
            runtime_mode: RuntimeMode::Editor,
            runtime_state: RuntimeStateHandle::new(),
            gizmos_enabled: true,
            editor_gizmos_enabled: true,
            editor_snapshot: None,
            custom_render_callback: None,
            custom_render_stage: CustomRenderStage::BeforePostprocess,
            custom_render_in_shadows: false,
            custom_render_shadow_query: None,
        }
    }
}

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

pub enum RenderResult {
    Skipped,
    Rendered(RenderFrame),
}

pub struct RenderParams {
    pub render_region: Option<RenderRegion>,
}

pub enum RuntimeTransition {
    EnteredEditor,
    EnteredPlaying,
}

pub struct AppCore {
    batcher: RenderBatcher,
    startup_systems: Vec<StartupSystem>,
    update_systems: Vec<UpdateSystem>,
    gpu_systems: Vec<GpuUpdateSystem>,
    auto_init_default_textures: bool,
    auto_add_default_lighting: bool,
    startup_ran: bool,
    frame_counter: u32,
    skip_rendering_until_frame: Option<u32>,
    settings: RenderSettings,
    scene: Scene,
    exit_on_escape: bool,
    exit_requested: bool,
    runtime_mode: RuntimeMode,
    runtime_state: RuntimeStateHandle,
    gizmos_enabled: bool,
    editor_gizmos_enabled: bool,
    editor_snapshot: Option<SceneSnapshot>,
    custom_render_callback: Option<Box<CustomRenderCallback>>,
    custom_render_stage: CustomRenderStage,
    custom_render_in_shadows: bool,
    custom_render_shadow_query: Option<Box<dyn FnMut() -> bool>>,
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
        self.runtime_mode
    }

    pub fn runtime_state_handle(&self) -> RuntimeStateHandle {
        self.runtime_state.clone()
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

    pub fn set_custom_render_callback(&mut self, callback: Box<CustomRenderCallback>) {
        self.custom_render_callback = Some(callback);
        self.custom_render_stage = CustomRenderStage::BeforePostprocess;
    }

    pub fn set_custom_render_stage(&mut self, stage: CustomRenderStage) {
        self.custom_render_stage = stage;
    }

    pub fn enable_custom_render_shadows(&mut self, enabled: bool) {
        self.custom_render_in_shadows = enabled;
    }

    pub fn set_custom_render_shadow_query<F>(&mut self, query: F)
    where
        F: FnMut() -> bool + 'static,
    {
        self.custom_render_shadow_query = Some(Box::new(query));
    }

    pub fn begin_frame(&mut self) -> FrameStep {
        self.frame_counter += 1;

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
        let last_frame = match self.scene.last_frame_instant() {
            Some(last_frame) => last_frame,
            None => {
                self.scene.set_last_frame(now);
                now
            }
        };
        let dt = (now - last_frame).as_secs_f64();
        self.scene.set_last_frame(now);

        FrameStep { dt, skip_rendering }
    }

    pub fn sync_runtime_state(&mut self) -> Option<RuntimeTransition> {
        let desired_mode = {
            let mut state = match self.runtime_state.inner.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };

            if state.desired_mode == self.runtime_mode {
                if state.active_mode != self.runtime_mode {
                    state.active_mode = self.runtime_mode;
                }
                return None;
            }

            state.desired_mode
        };

        match desired_mode {
            RuntimeMode::Editor => {
                self.stop_playback();
                if let Ok(mut state) = self.runtime_state.inner.lock() {
                    state.active_mode = RuntimeMode::Editor;
                }
                Some(RuntimeTransition::EnteredEditor)
            }
            RuntimeMode::Playing => {
                self.start_playback();
                if let Ok(mut state) = self.runtime_state.inner.lock() {
                    state.active_mode = RuntimeMode::Playing;
                }
                Some(RuntimeTransition::EnteredPlaying)
            }
        }
    }

    pub fn toggle_editor_gizmos(&mut self) {
        if self.runtime_mode == RuntimeMode::Editor {
            self.editor_gizmos_enabled = !self.editor_gizmos_enabled;
        }

        self.gizmos_enabled = match self.runtime_mode {
            RuntimeMode::Editor => self.editor_gizmos_enabled,
            RuntimeMode::Playing => false,
        };
    }

    pub fn on_renderer_ready(&mut self, renderer: &mut Renderer) {
        self.scene.init_timer();
        self.run_startup_systems(renderer);
        renderer.update_texture_bind_group(&self.scene.assets);
    }

    pub fn run_update_stage(&mut self, dt: f64) {
        if self.runtime_mode == RuntimeMode::Playing {
            self.scene.update(dt);
        }

        for system in &mut self.update_systems {
            let mut ctx = UpdateContext {
                scene: &mut self.scene,
                dt,
                runtime: self.runtime_mode,
            };
            (system)(&mut ctx);
        }

        self.scene.propagate_transforms();
    }

    pub fn run_gpu_systems(&mut self, renderer: &mut Renderer, dt: f64) {
        Self::run_gpu_systems_impl(&mut self.scene, &mut self.gpu_systems, renderer, dt);
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

        if let Some(query) = self.custom_render_shadow_query.as_mut() {
            self.custom_render_in_shadows = query();
        }

        renderer.set_render_region(params.render_region);

        let mut custom_render_request =
            self.custom_render_callback
                .as_mut()
                .map(|callback| CustomRenderRequest {
                    callback: &mut **callback,
                    stage: self.custom_render_stage,
                    render_in_shadow_pass: self.custom_render_in_shadows,
                });

        let render_frame = self.scene.render(
            renderer,
            &mut self.batcher,
            &mut custom_render_request,
            self.gizmos_enabled,
        )?;

        Ok(RenderResult::Rendered(render_frame))
    }

    pub fn gizmos_enabled(&self) -> bool {
        self.gizmos_enabled
    }

    fn run_startup_systems(&mut self, renderer: &mut Renderer) {
        if self.startup_ran {
            return;
        }

        self.scene
            .environment_mut()
            .enable_hdr_background(DEFAULT_HDR_ENVIRONMENT);

        if self.auto_init_default_textures && self.scene.assets.textures.is_empty() {
            self.init_default_textures(renderer);
        }

        for system in &mut self.startup_systems {
            let mut ctx = StartupContext {
                scene: &mut self.scene,
                renderer,
            };
            (system)(&mut ctx);
        }

        if self.auto_add_default_lighting {
            let added_lights = self.scene.add_default_lighting();
            if added_lights > 0 {
                log::info!("Added {} default lights to scene", added_lights);
            }
        }

        log::info!("Running initial transform propagation...");
        self.scene.set_animation_playback(false);
        self.scene.update(0.0);
        log::info!("Initial propagation complete");

        self.startup_ran = true;
    }

    fn start_playback(&mut self) {
        if self.runtime_mode == RuntimeMode::Playing {
            return;
        }

        let snapshot = SceneSnapshot::capture(&self.scene);
        self.editor_snapshot = Some(snapshot);
        self.scene.reset_script_runtime();
        self.scene.set_animation_playback(true);
        self.scene.set_time(0.0);
        self.scene.init_timer();
        self.gizmos_enabled = false;
        self.runtime_mode = RuntimeMode::Playing;
    }

    fn stop_playback(&mut self) {
        if self.runtime_mode == RuntimeMode::Editor {
            return;
        }

        if let Some(snapshot) = self.editor_snapshot.take() {
            self.scene = snapshot.into_scene();
            self.scene.init_timer();
            self.scene.reset_script_runtime();
            self.scene.set_animation_playback(false);
            self.scene.update(0.0);
            if !self.scene.has_any_lights() {
                self.scene.add_default_lighting();
            }
        }

        self.gizmos_enabled = self.editor_gizmos_enabled;
        self.runtime_mode = RuntimeMode::Editor;
    }

    fn init_default_textures(&mut self, renderer: &mut Renderer) {
        let device = renderer.get_device();
        let queue = renderer.get_queue();

        let white = Texture::white(device, queue);
        let white_handle = self.scene.assets.textures.insert(white);
        debug_assert_eq!(
            white_handle.index() as u32,
            DEFAULT_WHITE_TEXTURE_INDEX,
            "Default white texture index changed; update the constants in renderer::texture",
        );

        let normal = Texture::default_normal(device, queue);
        let normal_handle = self.scene.assets.textures.insert(normal);
        debug_assert_eq!(
            normal_handle.index() as u32,
            DEFAULT_NORMAL_TEXTURE_INDEX,
            "Default normal texture index changed; update the constants in renderer::texture",
        );

        let mr = Texture::default_metallic_roughness(device, queue);
        let mr_handle = self.scene.assets.textures.insert(mr);
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
        let checker_handle = self.scene.assets.textures.insert(checker);
        debug_assert_eq!(
            checker_handle.index() as u32,
            DEFAULT_CHECKER_TEXTURE_INDEX,
            "Default checker texture index changed; update the constants in renderer::texture",
        );

        log::info!(
            "Initialized default textures (white, normal, metallic-roughness, checkerboard)"
        );
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
}

impl Default for AppCore {
    fn default() -> Self {
        Self::new()
    }
}
