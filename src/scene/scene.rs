// scene/scene.rs
use glam::{Quat, Vec3};

use crate::asset::Assets;
use crate::renderer::{Material, RenderBatcher, RenderObject, Renderer};
use crate::scene::transform::Transform;
use crate::scene::Camera;

pub struct Scene {
    time: f64,
    last_frame: std::time::Instant,
    pub assets: Assets,
    pub batcher: RenderBatcher,
}

impl Scene {
    pub fn new() -> Self {
        Self {
            time: 0.0,
            last_frame: std::time::Instant::now(),
            assets: Assets::default(),
            batcher: RenderBatcher::new(),
        }
    }

    pub fn setup(&mut self, renderer: &Renderer) {
        // Load cube mesh
        let (verts, idx) = crate::renderer::cube_mesh();
        let cube_mesh = renderer.create_mesh(&verts, &idx);
        self.assets.meshes.insert(cube_mesh);

        // Future: Load textures and models here
        // let loader = AssetLoader::new(device, queue);
        // let texture = loader.load_texture("path/to/texture.png");
        // self.assets.textures.insert(texture);
    }

    pub fn update(&mut self, dt: f64) {
        self.time += dt;
    }

    pub fn time(&self) -> f64 {
        self.time
    }

    pub fn last_frame(&self) -> std::time::Instant {
        self.last_frame
    }

    pub fn set_last_frame(&mut self, instant: std::time::Instant) {
        self.last_frame = instant;
    }

    pub fn render(&mut self, renderer: &mut Renderer) {
        let t = self.time as f32;
        
        self.batcher.clear();

        // Setup camera (unchanged)
        let aspect = renderer.aspect_ratio();
        let eye = Vec3::new(t.cos() * 3.0, 2.0, t.sin() * 3.0);
        let cam = Camera {
            eye,
            target: Vec3::ZERO,
            up: Vec3::Y,
            fov_y_radians: 60f32.to_radians(),
            near: 0.01,
            far: 100.0,
        };
        renderer.set_camera(&cam, aspect);

        if let Some(cube_handle) = self.get_cube_handle() {
            // Center spinning cube - red
            self.batcher.add(RenderObject {
                mesh: cube_handle,
                material: Material::red(),
                transform: Transform::from_trs(
                    Vec3::ZERO,
                    Quat::from_rotation_x(t * 0.5) 
                        * Quat::from_rotation_y(t * 1.2)
                        * Quat::from_rotation_z(-t * 0.2),
                    Vec3::ONE,
                ),
            });

            // Right cube - green
            self.batcher.add(RenderObject {
                mesh: cube_handle,
                material: Material::green(),
                transform: Transform::from_trs(
                    Vec3::new(1.6, 0.0, 0.0),
                    Quat::from_rotation_y(-t * 1.0),
                    Vec3::ONE,
                ),
            });

            // Left cube - blue
            self.batcher.add(RenderObject {
                mesh: cube_handle,
                material: Material::blue(),
                transform: Transform::from_trs(
                    Vec3::new(-1.6, 0.0, 0.0),
                    Quat::from_rotation_y(t * 1.5),
                    Vec3::ONE,
                ),
            });

            // Ring of small cubes
            for i in 0..50 {
                let angle = (i as f32) * std::f32::consts::TAU / 50.0;
                let radius = 2.5;
                self.batcher.add(RenderObject {
                    mesh: cube_handle,
                    material: Material::checker(),
                    transform: Transform::from_trs(
                        Vec3::new(
                            angle.cos() * radius,
                            (t + i as f32).sin() * 0.5,
                            angle.sin() * radius,
                        ),
                        Quat::IDENTITY,
                        Vec3::splat(0.3),
                    ),
                });
            }
        }

        if let Err(e) = renderer.render(&self.assets, &self.batcher) {
            log::error!("Render error: {:?}", e);
        }
    }

    fn get_cube_handle(&self) -> Option<crate::asset::Handle<crate::asset::Mesh>> {
        if self.assets.meshes.len() > 0 {
            Some(crate::asset::Handle::new(0))
        } else {
            None
        }
    }
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}