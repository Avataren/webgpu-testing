use glam::{Mat4, Vec3};
use winit::{
    application::ApplicationHandler,
    event::*,
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{Key, NamedKey},
    window::{Window, WindowId},
};

mod renderer;
use renderer::assets::{Handle, Mesh};
use renderer::{Assets, Material, RenderBatcher, RenderObject};
use renderer::Depth;

mod scene;
use scene::Camera;

use crate::renderer::Gpu;

struct App {
    gpu: Option<Gpu>,
    window: Option<Window>,
    window_id: Option<WindowId>,
    time: f64,
    last_frame: std::time::Instant,
    assets: Assets,
    cube_mesh: Option<Handle<Mesh>>,
    batcher: RenderBatcher,
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
            batcher: RenderBatcher::new(),
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
                if let Some(w) = &self.window {
                    gpu.resize(w.inner_size());
                }
            }
            WindowEvent::RedrawRequested => {
                let now = std::time::Instant::now();
                let dt = (now - self.last_frame).as_secs_f64();
                self.last_frame = now;
                self.time += dt;
                let t = self.time as f32;

                // Camera setup
                let aspect = gpu.get_config().width as f32 / gpu.get_config().height.max(1) as f32;
                let eye = Vec3::new(t.cos() * 3.0, 2.0, t.sin() * 3.0);
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

                // Clear batches from previous frame
                self.batcher.clear();

                let cube_h = *self.cube_mesh.as_ref().expect("cube mesh created");

                // Add multiple objects with different materials
                // Center spinning cube - red
                self.batcher.add(RenderObject {
                    mesh: cube_h,
                    material: Material::red(),
                    transform: Mat4::from_rotation_x(t * 0.5)
                        * Mat4::from_rotation_y(t * 1.2)
                        * Mat4::from_rotation_z(-t * 0.2),
                });

                // Right cube - green
                self.batcher.add(RenderObject {
                    mesh: cube_h,
                    material: Material::green(),
                    transform: Mat4::from_translation(Vec3::new(1.6, 0.0, 0.0))
                        * Mat4::from_rotation_y(-t * 1.0),
                });

                // Left cube - blue
                self.batcher.add(RenderObject {
                    mesh: cube_h,
                    material: Material::blue(),
                    transform: Mat4::from_translation(Vec3::new(-1.6, 0.0, 0.0))
                        * Mat4::from_rotation_y(t * 1.5),
                });

                // Add some extra cubes for demonstration - they'll batch automatically!
                for i in 0..50 {
                    let angle = (i as f32) * std::f32::consts::TAU / 50.0;
                    let radius = 2.5;
                    self.batcher.add(RenderObject {
                        mesh: cube_h,
                        material: Material::white(),
                        transform: Mat4::from_translation(Vec3::new(
                            angle.cos() * radius,
                            (t + i as f32).sin() * 0.5,
                            angle.sin() * radius,
                        )) * Mat4::from_scale(Vec3::splat(0.3)),
                    });
                }

                match gpu.render_batched(&self.assets, &self.batcher) {
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

fn main() {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();

    let event_loop = EventLoop::new().expect("event loop");
    let mut app = App::new();
    event_loop.run_app(&mut app).unwrap();
}