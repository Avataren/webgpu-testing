use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, KeyEvent, WindowEvent},
    event_loop::ActiveEventLoop,
    keyboard::{Key, NamedKey},
    window::{Window, WindowId},
};

#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;
#[cfg(target_arch = "wasm32")]
use std::{cell::RefCell, rc::Rc};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_futures::spawn_local;

use crate::app::core::{AppBuilder, AppCore, RenderParams, RenderResult, RuntimeTransition};
use crate::renderer::{CustomRenderCallback, CustomRenderStage, RenderRegion, Renderer};

#[cfg(feature = "egui")]
use crate::app::editor::EditorState;

#[cfg(feature = "egui")]
use crate::ui::{
    EnvironmentSettingsHandle, FrameStatsHandle, PostProcessEffectsHandle, SceneHierarchyHandle,
};

#[cfg(target_arch = "wasm32")]
type WindowHandle = Rc<Window>;
#[cfg(not(target_arch = "wasm32"))]
type WindowHandle = Arc<Window>;
#[cfg(target_arch = "wasm32")]
type PendingRenderer = Rc<RefCell<Option<Renderer>>>;

pub struct WinitApp {
    core: AppCore,
    window: Option<WindowHandle>,
    window_id: Option<WindowId>,
    renderer: Option<Renderer>,
    #[cfg(target_arch = "wasm32")]
    pending_renderer: Option<PendingRenderer>,
    #[cfg(feature = "egui")]
    editor: EditorState,
}

impl WinitApp {
    pub fn new() -> Self {
        Self::from_core(AppBuilder::new().build())
    }

    pub fn from_core(core: AppCore) -> Self {
        #[cfg(feature = "egui")]
        let editor = EditorState::new(core.scene());

        Self {
            core,
            window: None,
            window_id: None,
            renderer: None,
            #[cfg(target_arch = "wasm32")]
            pending_renderer: None,
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
    pub fn scene_hierarchy_handle(&self) -> SceneHierarchyHandle {
        self.editor.scene_hierarchy_handle()
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
                if let Some(window) = &self.window {
                    renderer.resize(window.inner_size());
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

    #[cfg(target_arch = "wasm32")]
    fn try_finish_async_initialization(&mut self) {
        if self.renderer.is_some() {
            return;
        }

        let Some(pending) = self.pending_renderer.clone() else {
            return;
        };

        let renderer_opt = {
            let mut pending_ref = pending.borrow_mut();
            pending_ref.take()
        };

        drop(pending);

        if let Some(mut renderer) = renderer_opt {
            log::info!("Completing asynchronous renderer initialization");

            self.on_renderer_ready(&mut renderer);

            self.renderer = Some(renderer);
            self.pending_renderer = None;

            if let Some(window) = &self.window {
                window.request_redraw();
            }

            log::info!("Renderer initialized successfully");
        }
    }

    fn on_renderer_ready(&mut self, renderer: &mut Renderer) {
        self.core.on_renderer_ready(renderer);

        #[cfg(feature = "egui")]
        {
            if let Some(window) = &self.window {
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
            self.editor.sync_environment_controls(self.core.scene());
            self.editor.refresh_scene_hierarchy(self.core.scene());
        }
    }

    fn handle_runtime_transition(&mut self, transition: RuntimeTransition) {
        #[cfg(feature = "egui")]
        match transition {
            RuntimeTransition::EnteredEditor => {
                self.editor.sync_environment_controls(self.core.scene());
                self.editor.refresh_scene_hierarchy(self.core.scene());
            }
            RuntimeTransition::EnteredPlaying => {}
        }

        #[cfg(not(feature = "egui"))]
        let _ = transition;
    }

    fn window(&self) -> Option<&Window> {
        self.window.as_ref().map(|handle| handle.as_ref())
    }
}

impl Default for WinitApp {
    fn default() -> Self {
        Self::new()
    }
}

impl ApplicationHandler for WinitApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        log::info!("Initializing application...");

        let base_window_attrs = Window::default_attributes()
            .with_title("wgpu hecs Renderer")
            .with_inner_size(winit::dpi::LogicalSize::new(
                f64::from(self.settings().resolution.width),
                f64::from(self.settings().resolution.height),
            ));

        #[cfg(target_arch = "wasm32")]
        let window_attrs = {
            use winit::platform::web::WindowAttributesExtWebSys;
            base_window_attrs.with_append(true)
        };

        #[cfg(not(target_arch = "wasm32"))]
        let window_attrs = base_window_attrs;

        let window = event_loop
            .create_window(window_attrs)
            .expect("Failed to create window");

        #[cfg(not(target_arch = "wasm32"))]
        {
            let window = Arc::new(window);
            let id = window.id();

            let mut renderer =
                pollster::block_on(Renderer::new(window.clone(), self.settings().clone()));

            self.window = Some(window);
            self.window_id = Some(id);

            self.on_renderer_ready(&mut renderer);

            self.renderer = Some(renderer);

            if let Some(w) = self.window() {
                w.request_redraw();
            }

            log::info!("Application initialized");
        }

        #[cfg(target_arch = "wasm32")]
        {
            let id = window.id();
            let window_handle = Rc::new(window);
            let pending_renderer: PendingRenderer = Rc::new(RefCell::new(None));
            let renderer_cell = pending_renderer.clone();
            let window_for_renderer = window_handle.clone();
            let settings = self.settings().clone();

            log::info!("Spawning asynchronous renderer initialization");

            spawn_local(async move {
                let renderer = Renderer::new(window_for_renderer.clone(), settings).await;
                renderer_cell.borrow_mut().replace(renderer);
                window_for_renderer.request_redraw();
            });

            self.window = Some(window_handle);
            self.window_id = Some(id);
            self.pending_renderer = Some(pending_renderer);

            log::info!("Waiting for renderer to finish initializing");
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if Some(window_id) != self.window_id {
            return;
        }

        #[cfg(target_arch = "wasm32")]
        self.try_finish_async_initialization();

        #[cfg(feature = "egui")]
        if let Some(window_handle) = self.window.as_ref().cloned() {
            let consumed = self
                .editor
                .handle_window_event(window_handle.as_ref(), &event);
            if consumed {
                if let WindowEvent::RedrawRequested = event {
                    window_handle.request_redraw();
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
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(new_size);
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                if let (Some(renderer), Some(window_handle)) =
                    (self.renderer.as_mut(), self.window.as_ref().cloned())
                {
                    renderer.resize(window_handle.inner_size());
                }
            }
            WindowEvent::RedrawRequested => {
                #[cfg(target_arch = "wasm32")]
                self.try_finish_async_initialization();

                if let Some(transition) = self.core.sync_runtime_state() {
                    self.handle_runtime_transition(transition);
                }

                let frame = self.core.begin_frame();
                self.core.run_update_stage(frame.dt());

                if let Some(mut renderer) = self.renderer.take() {
                    #[cfg(feature = "egui")]
                    if let Some(window_handle) = self.window.as_ref().cloned() {
                        self.editor.begin_ui_frame(window_handle.as_ref());
                    }

                    self.core.run_gpu_systems(&mut renderer, frame.dt());

                    #[cfg(feature = "egui")]
                    {
                        self.editor.refresh_scene_hierarchy(self.core.scene());
                        {
                            let scene = self.core.scene_mut();
                            self.editor.apply_environment_settings(scene);
                        }
                        self.editor.apply_postprocess_effects(&mut renderer);
                    }

                    #[cfg(feature = "egui")]
                    let render_region = self.editor.render_region();
                    #[cfg(not(feature = "egui"))]
                    let render_region: Option<RenderRegion> = None;

                    let params = RenderParams { render_region };

                    let result = self.core.render_scene(&mut renderer, &frame, &params);

                    let mut should_continue = true;

                    match result {
                        Ok(RenderResult::Rendered(render_frame)) => {
                            #[allow(unused_mut)]
                            let mut render_frame = render_frame;
                            #[cfg(feature = "egui")]
                            if let Some(window_handle) = self.window.as_ref().cloned() {
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

                    self.renderer = Some(renderer);

                    if self.core.exit_requested() {
                        event_loop.exit();
                        return;
                    }

                    if !should_continue {
                        return;
                    }
                }

                if let Some(window_handle) = self.window.as_ref() {
                    window_handle.request_redraw();
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
                    EditorState::debug_print_hierarchy(self.core.scene());
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
