use glam::{Mat4, Vec3};
use winit::{
    application::ApplicationHandler,
    event::*,
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{Key, NamedKey},
    window::{Window, WindowId},
};

mod renderer;
use renderer::assets::Mesh;
use renderer::Depth;
use renderer::{Assets, DrawItem};

mod scene;
use scene::Camera;

use crate::renderer::Gpu;

struct App {
    gpu: Option<Gpu>,
    window: Option<Window>,
    window_id: Option<WindowId>,
    time: f32,
    last_frame: std::time::Instant,
    assets: Assets,
    cube_mesh: Option<renderer::assets::Handle<Mesh>>,
}

impl App {
    fn new() -> Self {
        Self {
            gpu: None,
            window: None,
            window_id: None,
            time: 0.0,
            last_frame: std::time::Instant::now(),
            assets: Assets::default(),
            cube_mesh: None,
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            let window = event_loop
                .create_window(Window::default_attributes().with_title("wgpu cube"))
                .expect("create window");
            let id = window.id();

            let gpu = pollster::block_on(Gpu::new(&window));
            let (verts, idx) = renderer::cube_mesh();
            let cube = Mesh::from_vertices(gpu.get_device(), &verts, &idx);
            let cube_h = self.assets.meshes.insert(cube);
            self.cube_mesh = Some(cube_h);

            self.window = Some(window);
            self.window_id = Some(id);
            self.gpu = Some(gpu);

            // Kick off the first frame explicitly on platforms that don't auto-redraw
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if Some(id) != self.window_id {
            return;
        }
        let gpu = match self.gpu.as_mut() {
            Some(g) => g,
            None => return,
        };

        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Destroyed => {
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                gpu.resize(size);
            }
            WindowEvent::ScaleFactorChanged {
                scale_factor: _, ..
            } => {
                // On DPI change, reconfigure the surface using the window's current inner size.
                if let Some(w) = &self.window {
                    gpu.resize(w.inner_size());
                }
            }
            WindowEvent::RedrawRequested => {
                // Variable timestep
                let now = std::time::Instant::now();
                let dt = (now - self.last_frame).as_secs_f32();
                self.last_frame = now;
                // Keep time bounded to avoid precision loss
                self.time = (self.time + dt) % std::f32::consts::TAU;

                // --- Camera (CPU-side) ---
                let aspect = gpu.get_config().width as f32 / gpu.get_config().height.max(1) as f32;
                let eye = Vec3::new(self.time.cos() * 2.0, 1.2, self.time.sin() * 2.0);
                let cam = Camera {
                    eye,
                    target: Vec3::ZERO,
                    up: Vec3::Y,
                    fov_y_radians: 60f32.to_radians(),
                    near: 0.01,
                    far: 100.0,
                };
                let vp = cam.view_proj(aspect);
                gpu.set_view_proj(vp);

                // --- Objects (instance transforms) ---
                let t = self.time;
                let mats = [
                    // center
                    Mat4::from_rotation_y(t),
                    // right
                    Mat4::from_translation(Vec3::new(1.6, 0.0, 0.0))
                        * Mat4::from_rotation_y(-t * 0.7),
                    // left
                    Mat4::from_translation(Vec3::new(-1.6, 0.0, 0.0))
                        * Mat4::from_rotation_y(t * 1.2),
                ];
                gpu.write_objects(&mats);

                let cube_h = *self.cube_mesh.as_ref().expect("cube mesh created");
                let draws = [DrawItem {
                    mesh: cube_h,
                    object_range: 0..(mats.len() as u32),
                }];

                match gpu.render_draw_list(&self.assets, &draws) {
                    Ok(()) => {}
                    Err(wgpu::SurfaceError::Lost) => {
                        if let Some(w) = &self.window {
                            gpu.resize(w.inner_size());
                        }
                    }
                    Err(wgpu::SurfaceError::OutOfMemory) => {
                        event_loop.exit();
                    }
                    Err(wgpu::SurfaceError::Timeout) => {}
                    Err(_) => {}
                }

                // Request next frame (for platforms that don't auto-redraw)
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
                // ESC to quit
                event_loop.exit();
            }
            _ => {}
        }
    }
}

fn main() {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();

    let event_loop = EventLoop::new().expect("event loop");
    let mut app = App::new();
    event_loop.run_app(&mut app).unwrap();
}
