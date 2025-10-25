#![cfg(target_arch = "wasm32")]

use std::{cell::RefCell, rc::Rc};

use wasm_bindgen_futures::spawn_local;
use winit::{event_loop::ActiveEventLoop, window::Window};

use crate::renderer::Renderer;

use super::{PlatformDriver, PlatformState};

#[derive(Default)]
pub struct WebWinitDriver {
    pending_renderer: Option<Rc<RefCell<Option<Renderer>>>>,
}

impl WebWinitDriver {
    fn window_attributes(
        settings: &crate::settings::RenderSettings,
    ) -> winit::window::WindowAttributes {
        use winit::platform::web::WindowAttributesExtWebSys;

        Window::default_attributes()
            .with_title("wgpu hecs Renderer")
            .with_inner_size(winit::dpi::LogicalSize::new(
                f64::from(settings.resolution.width),
                f64::from(settings.resolution.height),
            ))
            .with_append(true)
    }

    fn try_finish_async_initialization(&mut self, state: &mut PlatformState<Rc<Window>>) {
        if state.has_renderer() {
            return;
        }

        let Some(pending) = self.pending_renderer.as_ref() else {
            return;
        };

        let renderer = {
            let mut pending_ref = pending.borrow_mut();
            pending_ref.take()
        };

        if let Some(renderer) = renderer {
            log::info!("Completing asynchronous renderer initialization");
            state.put_renderer(renderer);
            self.pending_renderer = None;

            if let Some(window) = state.window() {
                window.as_ref().request_redraw();
            }
        }
    }
}

impl PlatformDriver for WebWinitDriver {
    type WindowHandle = Rc<Window>;

    fn initialize(
        &mut self,
        state: &mut PlatformState<Self::WindowHandle>,
        event_loop: &ActiveEventLoop,
        settings: &crate::settings::RenderSettings,
    ) {
        let window = event_loop
            .create_window(Self::window_attributes(settings))
            .expect("Failed to create window");

        let window_handle = Rc::new(window);
        let pending_renderer: Rc<RefCell<Option<Renderer>>> = Rc::new(RefCell::new(None));
        let renderer_cell = pending_renderer.clone();
        let window_for_renderer = window_handle.clone();
        let settings = settings.clone();

        log::info!("Spawning asynchronous renderer initialization");

        spawn_local(async move {
            let renderer = Renderer::new(window_for_renderer.clone(), settings).await;
            renderer_cell.borrow_mut().replace(renderer);
            window_for_renderer.request_redraw();
        });

        state.set_window(window_handle);
        self.pending_renderer = Some(pending_renderer);
    }

    fn handle_event(
        &mut self,
        state: &mut PlatformState<Self::WindowHandle>,
        _event_loop: &ActiveEventLoop,
        _event: &winit::event::WindowEvent,
    ) {
        self.try_finish_async_initialization(state);
    }

    fn render_frame(
        &mut self,
        state: &mut PlatformState<Self::WindowHandle>,
        _event_loop: &ActiveEventLoop,
    ) -> bool {
        self.try_finish_async_initialization(state);
        state.has_renderer()
    }

    fn shutdown(
        &mut self,
        _state: &mut PlatformState<Self::WindowHandle>,
        _event_loop: &ActiveEventLoop,
    ) {
        self.pending_renderer = None;
    }
}
