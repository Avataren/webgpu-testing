#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, KeyEvent, WindowEvent},
    event_loop::ActiveEventLoop,
    keyboard::{Key, NamedKey},
    window::{Window, WindowId},
};

use crate::app::core::{AppBuilder, AppCore, RenderParams, RenderResult, RuntimeTransition};
use crate::renderer::{CustomRenderCallback, CustomRenderStage, RenderRegion, Renderer};
use crate::scene::SceneWorkspace;

#[cfg(feature = "egui")]
use crate::app::editor::EditorState;
#[cfg(feature = "egui")]
use crate::ui::{
    EnvironmentSettingsHandle, FrameStatsHandle, PostProcessEffectsHandle, SceneHierarchyHandle,
    SceneHierarchyRegistryHandle,
};

#[cfg(not(target_arch = "wasm32"))]
pub use native::NativeWinitDriver;
#[cfg(target_arch = "wasm32")]
pub use web::WebWinitDriver;

#[cfg(target_arch = "wasm32")]
pub type DefaultDriver = WebWinitDriver;
#[cfg(not(target_arch = "wasm32"))]
pub type DefaultDriver = NativeWinitDriver;

/// Abstraction over platform specific winit behaviours.
pub trait PlatformDriver {
    type WindowHandle: AsRef<Window> + Clone;

    fn initialize(
        &mut self,
        state: &mut PlatformState<Self::WindowHandle>,
        event_loop: &ActiveEventLoop,
        settings: &crate::settings::RenderSettings,
    );

    fn handle_event(
        &mut self,
        state: &mut PlatformState<Self::WindowHandle>,
        event_loop: &ActiveEventLoop,
        event: &WindowEvent,
    );

    /// Ensures the renderer is ready for the upcoming frame.
    /// Returns `true` when rendering should proceed.
    fn render_frame(
        &mut self,
        state: &mut PlatformState<Self::WindowHandle>,
        event_loop: &ActiveEventLoop,
    ) -> bool;

    fn shutdown(
        &mut self,
        _state: &mut PlatformState<Self::WindowHandle>,
        _event_loop: &ActiveEventLoop,
    ) {
    }
}

pub struct PlatformState<H: AsRef<Window> + Clone> {
    window: Option<H>,
    window_id: Option<WindowId>,
    renderer: Option<Renderer>,
    renderer_initialized: bool,
}

impl<H: AsRef<Window> + Clone> PlatformState<H> {
    pub fn new() -> Self {
        Self {
            window: None,
            window_id: None,
            renderer: None,
            renderer_initialized: false,
        }
    }

    pub fn window(&self) -> Option<&H> {
        self.window.as_ref()
    }

    pub fn window_id(&self) -> Option<WindowId> {
        self.window_id
    }

    pub fn set_window(&mut self, handle: H) {
        self.window_id = Some(handle.as_ref().id());
        self.window = Some(handle);
    }

    pub fn take_renderer(&mut self) -> Option<Renderer> {
        self.renderer.take()
    }

    pub fn put_renderer(&mut self, renderer: Renderer) {
        self.renderer = Some(renderer);
    }

    pub fn renderer_mut(&mut self) -> Option<&mut Renderer> {
        self.renderer.as_mut()
    }

    pub fn has_renderer(&self) -> bool {
        self.renderer.is_some()
    }

    fn mark_renderer_initialized(&mut self) {
        self.renderer_initialized = true;
    }

    fn renderer_initialized(&self) -> bool {
        self.renderer_initialized
    }
}

impl<H: AsRef<Window> + Clone> Default for PlatformState<H> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct WinitApp<D: PlatformDriver = DefaultDriver> {
    core: AppCore,
    platform: PlatformState<D::WindowHandle>,
    driver: D,
    #[cfg(feature = "egui")]
    editor: EditorState,
}

impl<D> WinitApp<D>
where
    D: PlatformDriver,
{
    fn on_renderer_ready(&mut self, renderer: &mut Renderer) {
        self.core.on_renderer_ready(renderer);

        #[cfg(feature = "egui")]
        {
            if let Some(window) = self.platform.window() {
                let egui = crate::ui::EguiContext::new(
                    renderer.get_device(),
                    renderer.surface_format(),
                    renderer.sample_count(),
                    window.as_ref(),
                );
                self.editor.install_egui_context(egui);
                log::info!("Egui context initialized");
            }

            self.editor.apply_postprocess_effects(renderer);
            self.editor.sync_environment_controls(self.core.workspace());
            self.editor.refresh_scene_hierarchy(self.core.workspace());
        }
    }

    fn ensure_renderer_ready(&mut self) {
        if self.platform.renderer_initialized() {
            return;
        }

        if let Some(mut renderer) = self.platform.take_renderer() {
            self.on_renderer_ready(&mut renderer);
            self.platform.mark_renderer_initialized();
            self.platform.put_renderer(renderer);
            log::info!("Renderer initialized successfully");
        }
    }

    fn handle_runtime_transition(&mut self, transition: RuntimeTransition) {
        #[cfg(feature = "egui")]
        match transition {
            RuntimeTransition::EnteredEditor => {
                self.editor.sync_environment_controls(self.core.workspace());
                self.editor.refresh_scene_hierarchy(self.core.workspace());
            }
            RuntimeTransition::EnteredPlaying => {}
        }

        #[cfg(not(feature = "egui"))]
        let _ = transition;
    }

    fn handle_surface_error(
        &mut self,
        event_loop: &ActiveEventLoop,
        renderer: &mut Renderer,
        error: wgpu::SurfaceError,
    ) -> bool {
        match error {
            wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated => {
                log::warn!("Surface lost/outdated; resizing swapchain");
                if let Some(window) = self.platform.window() {
                    renderer.resize(window.as_ref().inner_size());
                }
                true
            }
            wgpu::SurfaceError::Timeout => {
                log::warn!("Surface timeout; will retry next frame");
                true
            }
            wgpu::SurfaceError::OutOfMemory => {
                log::error!("Surface out of memory; shutting down");
                event_loop.exit();
                false
            }
            err @ wgpu::SurfaceError::Other => {
                log::error!("Unexpected surface error: {err:?}");
                true
            }
        }
    }
}

impl<D> WinitApp<D>
where
    D: PlatformDriver + Default,
{
    pub fn new() -> Self {
        Self::from_core(AppBuilder::new().build())
    }

    pub fn new_with_workspace(workspace: SceneWorkspace) -> Self {
        Self::from_core(AppBuilder::new().with_workspace(workspace).build())
    }

    pub fn from_core(core: AppCore) -> Self {
        Self::from_core_with_driver(core, D::default())
    }
}

impl<D> WinitApp<D>
where
    D: PlatformDriver,
{
    pub fn from_core_with_driver(core: AppCore, driver: D) -> Self {
        #[cfg(feature = "egui")]
        let editor = EditorState::new(core.workspace());

        Self {
            core,
            platform: PlatformState::default(),
            driver,
            #[cfg(feature = "egui")]
            editor,
        }
    }

    pub fn core(&self) -> &AppCore {
        &self.core
    }

    pub fn core_mut(&mut self) -> &mut AppCore {
        &mut self.core
    }

    pub fn settings(&self) -> &crate::settings::RenderSettings {
        self.core.settings()
    }

    pub fn runtime_state_handle(&self) -> crate::app::core::RuntimeStateHandle {
        self.core.runtime_state_handle()
    }

    pub fn set_custom_render_callback(&mut self, callback: Box<CustomRenderCallback>) {
        self.core.set_custom_render_callback(callback);
    }

    pub fn set_custom_render_stage(&mut self, stage: CustomRenderStage) {
        self.core.set_custom_render_stage(stage);
    }

    pub fn enable_custom_render_shadows(&mut self, enabled: bool) {
        self.core.enable_custom_render_shadows(enabled);
    }

    pub fn set_custom_render_shadow_query<F>(&mut self, query: F)
    where
        F: FnMut() -> bool + 'static,
    {
        self.core.set_custom_render_shadow_query(query);
    }

    #[cfg(feature = "egui")]
    pub fn set_render_region_query<F>(&mut self, query: F)
    where
        F: FnMut() -> Option<RenderRegion> + 'static,
    {
        self.editor.set_render_region_query(query);
    }

    #[cfg(feature = "egui")]
    pub fn set_egui_ui<F>(&mut self, callback: F)
    where
        F: FnMut(&crate::ui::egui::Context) + 'static,
    {
        self.editor.set_egui_ui(callback);
    }

    #[cfg(feature = "egui")]
    pub fn frame_stats_handle(&self) -> FrameStatsHandle {
        self.editor.frame_stats_handle()
    }

    #[cfg(feature = "egui")]
    pub fn postprocess_effects_handle(&self) -> PostProcessEffectsHandle {
        self.editor.postprocess_effects_handle()
    }

    #[cfg(feature = "egui")]
    pub fn environment_settings_handle(&self) -> EnvironmentSettingsHandle {
        self.editor.environment_settings_handle()
    }

    #[cfg(feature = "egui")]
    pub fn scene_hierarchy_handle(&self) -> Option<SceneHierarchyHandle> {
        self.editor.scene_hierarchy_handle()
    }

    #[cfg(feature = "egui")]
    pub fn scene_hierarchy_registry(&self) -> SceneHierarchyRegistryHandle {
        self.editor.scene_hierarchy_registry()
    }
}

impl<D> Default for WinitApp<D>
where
    D: PlatformDriver + Default,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<D> ApplicationHandler for WinitApp<D>
where
    D: PlatformDriver,
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.platform.window().is_some() {
            return;
        }

        log::info!("Initializing application...");

        let settings = self.settings().clone();

        self.driver
            .initialize(&mut self.platform, event_loop, &settings);
        self.ensure_renderer_ready();

        if let Some(window) = self.platform.window() {
            window.as_ref().request_redraw();
        }

        if self.platform.renderer_initialized() {
            log::info!("Application initialized");
        } else {
            log::info!("Waiting for renderer to finish initializing");
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if Some(window_id) != self.platform.window_id() {
            return;
        }

        self.driver
            .handle_event(&mut self.platform, event_loop, &event);
        self.ensure_renderer_ready();

        #[cfg(feature = "egui")]
        if let Some(window_handle) = self.platform.window().cloned() {
            let consumed = self
                .editor
                .handle_window_event(window_handle.as_ref(), &event);
            if consumed {
                if let WindowEvent::RedrawRequested = event {
                    window_handle.as_ref().request_redraw();
                }
                return;
            }
        }

        match event {
            WindowEvent::CloseRequested | WindowEvent::Destroyed => {
                log::info!("Closing application");
                event_loop.exit();
            }
            WindowEvent::Resized(new_size) => {
                if let Some(renderer) = self.platform.renderer_mut() {
                    renderer.resize(new_size);
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(size) = self
                    .platform
                    .window()
                    .map(|window_handle| window_handle.as_ref().inner_size())
                {
                    if let Some(renderer) = self.platform.renderer_mut() {
                        renderer.resize(size);
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                if !self.driver.render_frame(&mut self.platform, event_loop) {
                    return;
                }

                self.ensure_renderer_ready();

                if let Some(transition) = self.core.sync_runtime_state() {
                    self.handle_runtime_transition(transition);
                }

                let Some(frame) = self.core.begin_frame() else {
                    return;
                };
                let mut active_handle = self.core.run_update_stage(&frame);

                if let Some(mut renderer) = self.platform.take_renderer() {
                    #[cfg(feature = "egui")]
                    if let Some(window_handle) = self.platform.window().cloned() {
                        self.editor.begin_ui_frame(window_handle.as_ref());
                    }

                    active_handle = self
                        .core
                        .run_gpu_systems(&mut renderer, &frame, active_handle);

                    #[cfg(feature = "egui")]
                    {
                        self.editor.refresh_scene_hierarchy(self.core.workspace());
                        {
                            let mut scene = self.core.scene_mut();
                            self.editor.apply_environment_settings(&mut scene);
                        }
                        self.editor.apply_postprocess_effects(&mut renderer);
                    }

                    #[cfg(feature = "egui")]
                    let render_region = self.editor.render_region();
                    #[cfg(not(feature = "egui"))]
                    let render_region: Option<RenderRegion> = None;

                    let params = RenderParams { render_region };

                    let result =
                        self.core
                            .render_scene(&mut renderer, &frame, &params, active_handle);

                    let mut should_continue = true;

                    match result {
                        Ok(RenderResult::Rendered(render_frame)) => {
                            #[allow(unused_mut)]
                            let mut render_frame = render_frame;
                            #[cfg(feature = "egui")]
                            if let Some(window_handle) = self.platform.window().cloned() {
                                let window = window_handle.as_ref();
                                if let Some(output) = self.editor.end_ui_frame(window) {
                                    self.editor.render_ui(
                                        &mut renderer,
                                        window,
                                        &mut render_frame,
                                        output,
                                    );
                                }
                                if self.editor.take_exit_request() {
                                    self.core.request_exit();
                                }
                            }

                            render_frame.frame.present();
                        }
                        Ok(RenderResult::Skipped) => {}
                        Err(err) => {
                            should_continue =
                                self.handle_surface_error(event_loop, &mut renderer, err);
                        }
                    }

                    #[cfg(feature = "egui")]
                    self.editor.record_frame_stats(&frame, &renderer);

                    self.platform.put_renderer(renderer);

                    if self.core.exit_requested() {
                        event_loop.exit();
                        return;
                    }

                    if !should_continue {
                        return;
                    }
                }

                if let Some(window_handle) = self.platform.window() {
                    window_handle.as_ref().request_redraw();
                }
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key,
                        state: winit::event::ElementState::Pressed,
                        ..
                    },
                ..
            } => match logical_key {
                Key::Named(NamedKey::Escape) => {
                    if self.core.exit_on_escape() {
                        event_loop.exit();
                    }
                }
                Key::Character(c) if c.as_str().eq_ignore_ascii_case("g") => {
                    self.core.toggle_editor_gizmos();
                }
                Key::Character(c) if c.as_str() == "h" => {
                    #[cfg(feature = "egui")]
                    EditorState::debug_print_hierarchy(self.core.workspace());
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        _event: DeviceEvent,
    ) {
        #[cfg(feature = "egui")]
        {
            self.editor.handle_device_event(&_event);
        }
    }
}
