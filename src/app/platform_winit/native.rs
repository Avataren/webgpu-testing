#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;

use winit::{
    event_loop::ActiveEventLoop,
    window::{Window, WindowAttributes},
};

use crate::renderer::Renderer;

use super::{PlatformDriver, PlatformState};

#[derive(Default)]
pub struct NativeWinitDriver;

impl NativeWinitDriver {
    fn window_attributes(settings: &crate::settings::RenderSettings) -> WindowAttributes {
        Window::default_attributes()
            .with_title("wgpu hecs Renderer")
            .with_inner_size(winit::dpi::LogicalSize::new(
                f64::from(settings.resolution.width),
                f64::from(settings.resolution.height),
            ))
    }
}

impl PlatformDriver for NativeWinitDriver {
    type WindowHandle = Arc<Window>;

    fn initialize(
        &mut self,
        state: &mut PlatformState<Self::WindowHandle>,
        event_loop: &ActiveEventLoop,
        settings: &crate::settings::RenderSettings,
    ) {
        let window = event_loop
            .create_window(Self::window_attributes(settings))
            .expect("Failed to create window");

        let handle = Arc::new(window);
        let renderer = pollster::block_on(Renderer::new(handle.clone(), settings.clone()));

        state.set_window(handle);
        state.put_renderer(renderer);
    }

    fn handle_event(
        &mut self,
        _state: &mut PlatformState<Self::WindowHandle>,
        _event_loop: &ActiveEventLoop,
        _event: &winit::event::WindowEvent,
    ) {
    }

    fn render_frame(
        &mut self,
        state: &mut PlatformState<Self::WindowHandle>,
        _event_loop: &ActiveEventLoop,
    ) -> bool {
        state.has_renderer()
    }
}
