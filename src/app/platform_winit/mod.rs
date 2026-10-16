#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

use crate::app::core::frame_timings;
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::ActiveEventLoop,
    keyboard::{Key, NamedKey},
    window::{Window, WindowId},
};

use crate::app::core::{AppBuilder, AppCore, RenderParams, RenderResult, RuntimeTransition};
use crate::renderer::{CustomRenderCallback, CustomRenderStage, RenderRegion, Renderer};
use crate::scene::SceneWorkspace;
use glam::Vec2;

#[cfg(feature = "egui")]
use crate::app::editor::EditorState;
#[cfg(feature = "egui")]
use crate::scene::SceneHandle;
#[cfg(feature = "egui")]
use crate::ui::{
    EnvironmentSettingsHandle, FrameStatsHandle, SceneHierarchyHandle,
    SceneHierarchyRegistryHandle, SceneTabsHandle,
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

fn key_to_string(key: &Key) -> Option<String> {
    match key {
        Key::Character(text) => {
            if text.len() == 1 {
                Some(text.to_ascii_uppercase())
            } else {
                Some(text.to_string())
            }
        }
        Key::Named(named) => {
            let name = match named {
                NamedKey::Space => "Space",
                NamedKey::Escape => "Escape",
                NamedKey::Enter => "Return",
                NamedKey::Backspace => "Backspace",
                NamedKey::Tab => "Tab",
                NamedKey::ArrowUp => "ArrowUp",
                NamedKey::ArrowDown => "ArrowDown",
                NamedKey::ArrowLeft => "ArrowLeft",
                NamedKey::ArrowRight => "ArrowRight",
                NamedKey::Shift => "Shift",
                NamedKey::Control => "Control",
                NamedKey::Alt => "Alt",
                NamedKey::Meta => "Meta",
                _ => return Some(format!("{named:?}")),
            };
            Some(name.to_string())
        }
        _ => None,
    }
}

fn mouse_button_index(button: MouseButton) -> u32 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Right => 1,
        MouseButton::Middle => 2,
        MouseButton::Back => 3,
        MouseButton::Forward => 4,
        MouseButton::Other(value) => value as u32,
    }
}

fn scroll_delta_vec(delta: MouseScrollDelta) -> Vec2 {
    match delta {
        MouseScrollDelta::LineDelta(x, y) => Vec2::new(x, y),
        MouseScrollDelta::PixelDelta(position) => Vec2::new(position.x as f32, position.y as f32),
    }
}

pub struct PlatformState<H: AsRef<Window> + Clone> {
    // CRITICAL: renderer must drop before window!
    // Renderer contains RenderContext with wgpu::Surface which was created from the window.
    // The Surface needs the window to still be alive when it drops.
    renderer: Option<Renderer>,
    renderer_initialized: bool,
    window: Option<H>,
    window_id: Option<WindowId>,
}

impl<H: AsRef<Window> + Clone> PlatformState<H> {
    pub fn new() -> Self {
        Self {
            renderer: None,
            renderer_initialized: false,
            window: None,
            window_id: None,
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
    // CRITICAL: Drop order matters! GPU resources must drop before the Device.
    // EditorState contains egui's GPU resources (egui_wgpu::Renderer)
    // AppCore contains scene GPU resources (Mesh buffers, Textures in Assets)
    // PlatformState contains the Renderer with the wgpu Device
    // All GPU resources must drop before the Device to prevent core dump on exit.
    #[cfg(feature = "egui")]
    editor: EditorState,
    core: AppCore,
    platform: PlatformState<D::WindowHandle>,
    driver: D,
    /// The graphics settings revision the window and surface last applied.
    applied_graphics: Option<u64>,
    /// Reloads shaders when their files change, in builds run from the source tree.
    #[cfg(not(target_arch = "wasm32"))]
    shader_watcher: Option<crate::renderer::shader_source::ShaderWatcher>,
}

impl<D> WinitApp<D>
where
    D: PlatformDriver,
{
    fn on_renderer_ready(&mut self, renderer: &mut Renderer) {
        self.core.on_renderer_ready(renderer);
        // Created here rather than with the app, so its log line is not lost before
        // logging starts.
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.shader_watcher = crate::renderer::shader_source::ShaderWatcher::new();
        }

        #[cfg(feature = "egui")]
        {
            if let Some(window) = self.platform.window() {
                let egui = crate::ui::EguiContext::new(
                    renderer.get_device(),
                    renderer.ui_format(),
                    renderer.sample_count(),
                    window.as_ref(),
                );
                self.editor.install_egui_context(egui);
                log::info!("Egui context initialized");
            }

            self.editor.sync_environment_controls(self.core.workspace());
            self.editor.refresh_scene_hierarchy(self.core.workspace());
        }
    }

    fn ensure_renderer_ready(&mut self) {
        if self.platform.renderer_initialized() {
            return;
        }

        if let Some(mut renderer) = self.platform.take_renderer() {
            // On the web the renderer is created asynchronously from the canvas size at
            // spawn time (0x0 before layout, so a 1x1 surface), and a Resized event that
            // arrives meanwhile finds no renderer and is dropped. Catch up here.
            if let Some(size) = self
                .platform
                .window()
                .map(|window_handle| window_handle.as_ref().inner_size())
            {
                if size.width > 0 && size.height > 0 && size != renderer.surface_size() {
                    renderer.resize(size);
                }
            }
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

    fn update_input_state<F>(&mut self, update: F)
    where
        F: FnOnce(&mut crate::scene::InputState),
    {
        let _ = self
            .core
            .workspace_mut()
            .with_active_scene_mut(|scene| update(scene.input_state_mut()));
    }

    fn handle_keyboard_input(&mut self, event_loop: &ActiveEventLoop, event: &KeyEvent) {
        if let Some(key) = key_to_string(&event.logical_key) {
            let key_name = key.clone();
            self.update_input_state(|input| match event.state {
                winit::event::ElementState::Pressed => input.press_key(key_name),
                winit::event::ElementState::Released => input.release_key(&key),
            });
        }

        if event.state != winit::event::ElementState::Pressed {
            return;
        }

        match event.logical_key {
            // With a UI, Escape opens the settings page instead (it has an Exit button).
            Key::Named(NamedKey::Escape)
                if self.core.exit_on_escape()
                    && !(cfg!(feature = "egui")
                        && crate::graphics_settings::graphics_settings().hotkey_enabled) =>
            {
                event_loop.exit();
            }
            Key::Character(ref c) if c.as_str().eq_ignore_ascii_case("g") => {
                self.core.toggle_editor_gizmos();
            }
            Key::Character(ref c) if c.as_str() == "h" => {
                #[cfg(feature = "egui")]
                EditorState::debug_print_hierarchy(self.core.workspace());
            }
            _ => {}
        }
    }

    /// Bring the window (display mode, size) and the surface (vsync) in line with the
    /// player's graphics settings when they changed.
    fn apply_graphics_settings(&mut self) {
        let (revision, settings) = {
            let graphics = crate::graphics_settings::graphics_settings();
            (graphics.revision(), graphics.settings.clone())
        };
        if self.applied_graphics == Some(revision) || !self.platform.renderer_initialized() {
            return;
        }
        self.applied_graphics = Some(revision);
        if let Some(renderer) = self.platform.renderer_mut() {
            renderer.set_vsync(settings.vsync);
        }
        let manage_window = crate::graphics_settings::graphics_settings().manage_window;
        // The browser decides the canvas size.
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(window) = self.platform.window().filter(|_| manage_window) {
            use crate::graphics_settings::DisplayMode;
            let window = window.as_ref();
            match settings.display_mode {
                DisplayMode::Fullscreen => {
                    if window.fullscreen().is_none() {
                        window.set_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
                    }
                }
                DisplayMode::Windowed => {
                    if window.fullscreen().is_some() {
                        window.set_fullscreen(None);
                    }
                    if let Some([width, height]) = settings.window_size {
                        let size = winit::dpi::PhysicalSize::new(width, height);
                        if window.inner_size() != size {
                            let _ = window.request_inner_size(size);
                        }
                    }
                }
            }
        }
    }

    fn handle_surface_error(
        &mut self,
        renderer: &mut Renderer,
        error: crate::renderer::SurfaceError,
    ) {
        use crate::renderer::SurfaceError;
        match error {
            SurfaceError::Lost | SurfaceError::Outdated => {
                log::warn!("Surface {error}; reconfiguring swapchain");
                if let Some(window) = self.platform.window() {
                    renderer.resize(window.as_ref().inner_size());
                }
            }
            SurfaceError::Timeout | SurfaceError::Occluded => {
                log::debug!("Skipping frame: {error}");
            }
            SurfaceError::Validation => {
                log::error!("Skipping frame: {error}");
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
            #[cfg(feature = "egui")]
            editor,
            core,
            platform: PlatformState::default(),
            driver,
            applied_graphics: None,
            #[cfg(not(target_arch = "wasm32"))]
            shader_watcher: None,
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
    pub fn active_scene_hierarchy(&self) -> Option<(SceneHandle, SceneHierarchyHandle)> {
        self.editor.active_scene_hierarchy()
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
        F: FnMut(&mut crate::ui::egui::Ui) + 'static,
    {
        self.editor.set_egui_ui(callback);
    }

    #[cfg(feature = "egui")]
    pub fn frame_stats_handle(&self) -> FrameStatsHandle {
        self.editor.frame_stats_handle()
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

    #[cfg(feature = "egui")]
    pub fn scene_tabs_handle(&self) -> SceneTabsHandle {
        self.editor.scene_tabs_handle()
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

        if let Some(hub) = self.core.remote_hub() {
            log::info!(
                target: "remote",
                "Remote control listening on 127.0.0.1:{}",
                hub.borrow().port()
            );
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
                self.apply_graphics_settings();

                if let Some(transition) = self.core.sync_runtime_state() {
                    self.handle_runtime_transition(transition);
                }

                // Wait for the GPU before the frame reads the clock (see the method).
                if let Some(renderer) = self.platform.renderer_mut() {
                    frame_timings::time("gpu_wait", || renderer.wait_for_frame_slot());
                }

                let Some(frame) = self.core.begin_frame() else {
                    return;
                };
                let remote = self.core.remote_hub();
                if let Some(hub) = &remote {
                    // Injected input lands before scripts run, like window events.
                    self.update_input_state(|input| hub.borrow_mut().apply_input(input));
                }
                let mut active_handle = self.core.run_update_stage(&frame);

                if let Some(mut renderer) = self.platform.take_renderer() {
                    #[cfg(not(target_arch = "wasm32"))]
                    if self
                        .shader_watcher
                        .as_mut()
                        .is_some_and(|watcher| watcher.poll())
                    {
                        crate::renderer::shader_source::log_reload(&renderer.reload_shaders());
                    }

                    #[cfg(feature = "egui")]
                    if let Some(window_handle) = self.platform.window().cloned() {
                        frame_timings::time("ui_build", || {
                            self.editor.run_ui_frame(window_handle.as_ref())
                        });
                    }

                    active_handle = self
                        .core
                        .run_gpu_systems(&mut renderer, &frame, active_handle);

                    #[cfg(feature = "egui")]
                    {
                        self.editor.refresh_scene_hierarchy(self.core.workspace());
                        let mut scene = self.core.scene_mut();
                        self.editor.apply_environment_settings(&mut scene);
                    }

                    #[cfg(feature = "egui")]
                    let mut render_region = self.editor.render_region();
                    #[cfg(not(feature = "egui"))]
                    let mut render_region: Option<RenderRegion> = None;

                    // A viewport preview lays the game out at another shape.
                    if let Some(region) = {
                        let size = renderer.surface_size();
                        render_region
                            .or_else(|| RenderRegion::full(size.width, size.height))
                            .and_then(crate::remote::preview::preview_region)
                    } {
                        render_region = Some(region);
                    }

                    let capture_override = remote
                        .as_ref()
                        .and_then(|hub| hub.borrow_mut().capture_frame_override());
                    let camera_override = capture_override.and_then(|over| over.camera);
                    if let Some(aspect) = capture_override.and_then(|over| over.aspect) {
                        // Render at the requested shape inside the viewport (or the
                        // whole window when there is no editor viewport).
                        let size = renderer.surface_size();
                        let base =
                            render_region.or_else(|| RenderRegion::full(size.width, size.height));
                        render_region =
                            base.and_then(|base| crate::remote::fit_region(base, aspect));
                    }
                    let post_process_override = remote
                        .as_ref()
                        .and_then(|hub| hub.borrow().post_process_override().cloned());
                    let params = RenderParams {
                        render_region,
                        camera_override,
                        post_process_override,
                        hide_overlays: capture_override.is_some_and(|over| over.hide_overlays),
                    };

                    let result =
                        self.core
                            .render_scene(&mut renderer, &frame, &params, active_handle);

                    match result {
                        Ok(RenderResult::Rendered(render_frame)) => {
                            #[allow(unused_mut)]
                            let mut render_frame = render_frame;
                            #[cfg(feature = "egui")]
                            if let Some(window_handle) = self.platform.window().cloned() {
                                let window = window_handle.as_ref();
                                if let Some(output) = self.editor.take_ui_output() {
                                    frame_timings::time("ui_render", || {
                                        self.editor.render_ui(
                                            &mut renderer,
                                            window,
                                            &mut render_frame,
                                            output,
                                        )
                                    });
                                }
                                if self.editor.take_exit_request() {
                                    self.core.request_exit();
                                }
                            }

                            if let Some(hub) = &remote {
                                if hub.borrow().has_pending_captures() {
                                    let capture =
                                        crate::remote::capture_surface(&renderer, &render_frame);
                                    hub.borrow_mut().complete_captures(capture, render_region);
                                }
                            }

                            frame_timings::time("present", || renderer.present(render_frame));
                        }
                        Ok(RenderResult::Skipped) => {}
                        Err(err) => {
                            self.handle_surface_error(&mut renderer, err);
                        }
                    }

                    #[cfg(feature = "egui")]
                    self.editor.record_frame_stats(&frame, &renderer);

                    self.platform.put_renderer(renderer);

                    self.update_input_state(|input| input.update());
                    frame_timings::end_frame();
                    if let Some(hub) = &remote {
                        hub.borrow_mut().end_frame(frame.dt());
                    }

                    if self.core.exit_requested() {
                        event_loop.exit();
                        return;
                    }
                }

                if let Some(window_handle) = self.platform.window() {
                    window_handle.as_ref().request_redraw();
                }
            }
            WindowEvent::Focused(false) => {
                self.update_input_state(|input| input.reset());
            }
            WindowEvent::CursorMoved { position, .. } => {
                let position = Vec2::new(position.x as f32, position.y as f32);
                self.update_input_state(|input| {
                    let previous = input.mouse_position();
                    input.add_mouse_delta(position - previous);
                    input.set_mouse_position(position);
                });
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let index = mouse_button_index(button);
                self.update_input_state(|input| match state {
                    winit::event::ElementState::Pressed => input.press_mouse_button(index),
                    winit::event::ElementState::Released => input.release_mouse_button(index),
                });
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let delta = scroll_delta_vec(delta);
                self.update_input_state(|input| input.add_scroll_delta(delta));
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.handle_keyboard_input(event_loop, &event);
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_to_string_normalizes_characters() {
        let key = Key::Character("w".into());
        assert_eq!(key_to_string(&key), Some("W".to_string()));
    }

    #[test]
    fn key_to_string_maps_named_keys() {
        let key = Key::Named(NamedKey::Escape);
        assert_eq!(key_to_string(&key), Some("Escape".to_string()));
    }
}
