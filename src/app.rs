// app.rs
use winit::{
    application::ApplicationHandler,
    event::*,
    event_loop::ActiveEventLoop,
    keyboard::{Key, NamedKey},
    window::{Window, WindowId},
};

use crate::renderer::Renderer;
use crate::scene::Scene;

pub struct App {
    renderer: Option<Renderer>,
    window: Option<Window>,
    window_id: Option<WindowId>,
    scene: Scene,
}

impl App {
    pub fn new() -> Self {
        Self {
            renderer: None,
            window: None,
            window_id: None,
            scene: Scene::new(),
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            let window = event_loop
                .create_window(Window::default_attributes().with_title("wgpu renderer"))
                .expect("create window");
            let id = window.id();

            let renderer = pollster::block_on(Renderer::new(&window));
            self.scene.setup(&renderer);

            self.window = Some(window);
            self.window_id = Some(id);
            self.renderer = Some(renderer);

            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if Some(id) != self.window_id {
            return;
        }

        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };

        match event {
            WindowEvent::CloseRequested | WindowEvent::Destroyed => {
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                renderer.resize(size);
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(w) = &self.window {
                    renderer.resize(w.inner_size());
                }
            }
            WindowEvent::RedrawRequested => {
                let now = std::time::Instant::now();
                let dt = (now - self.scene.last_frame()).as_secs_f64();
                self.scene.set_last_frame(now);
                
                self.scene.update(dt);
                self.scene.render(renderer);

                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key: Key::Named(NamedKey::Escape),
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => {
                event_loop.exit();
            }
            _ => {}
        }
    }
}