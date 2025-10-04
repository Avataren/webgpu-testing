// app.rs
// Pure hecs ECS implementation - no custom entity system

use winit::{
    application::ApplicationHandler,
    event::*,
    event_loop::ActiveEventLoop,
    keyboard::{Key, NamedKey},
    window::{Window, WindowId},
};

use crate::renderer::{Material, RenderBatcher, Renderer, Texture};
use crate::scene::{
    Camera, EntityBuilder, MaterialComponent, MeshComponent, Name, OrbitAnimation, RotateAnimation,
    Scene, SceneLoader, Transform, TransformComponent, Visible,
};
use glam::{Quat, Vec3};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneType {
    Simple,
    Grid,
    Animated,
    MaterialShowcase,
    FromGltf,
}

pub struct App {
    renderer: Option<Renderer>,
    window: Option<Window>,
    window_id: Option<WindowId>,
    scene: Scene,
    batcher: RenderBatcher,
    camera: Camera,
    scene_type: SceneType,
    gltf_path: Option<String>,
    gltf_scale: f32,
}

impl App {
    pub fn new(scene_type: SceneType) -> Self {
        Self {
            renderer: None,
            window: None,
            window_id: None,
            scene: Scene::new(),
            batcher: RenderBatcher::new(),
            camera: Camera::default(),
            scene_type,
            gltf_path: None,
            gltf_scale: 1.0,
        }
    }

    fn init_default_textures(&mut self, renderer: &mut Renderer) {
        // Create default textures that PBR materials can fall back to

        // 0: White texture (default base color)
        let white = Texture::white(renderer.get_device(), renderer.get_queue());
        self.scene.assets.textures.insert(white);

        // 1: Default normal map (pointing up: RGB 128,128,255)
        let normal = Texture::default_normal(renderer.get_device(), renderer.get_queue());
        self.scene.assets.textures.insert(normal);

        // 2: Default metallic-roughness (non-metallic, mid-roughness)
        let mr = Texture::default_metallic_roughness(renderer.get_device(), renderer.get_queue());
        self.scene.assets.textures.insert(mr);

        // Update the bind group with these default textures
        renderer.update_texture_bind_group(&self.scene.assets);

        log::info!("Initialized default PBR textures");
    }

    pub fn with_gltf(path: impl Into<String>, scale: f32) -> Self {
        Self {
            renderer: None,
            window: None,
            window_id: None,
            scene: Scene::new(),
            batcher: RenderBatcher::new(),
            camera: Camera::default(),
            scene_type: SceneType::FromGltf,
            gltf_path: Some(path.into()),
            gltf_scale: scale,
        }
    }

    fn setup_scene(&mut self, renderer: &mut Renderer) {
        // Only initialize default textures if we don't have any yet
        if self.scene.assets.textures.is_empty() {
            self.init_default_textures(renderer);
        }

        match self.scene_type {
            SceneType::Simple => self.create_simple_scene(renderer),
            SceneType::Grid => self.create_grid_scene(renderer, 5),
            SceneType::Animated => self.create_animated_scene(renderer),
            SceneType::MaterialShowcase => self.create_material_showcase(renderer),
            SceneType::FromGltf => {
                if let Some(path) = &self.gltf_path.clone() {
                    self.load_gltf_scene(path, renderer);
                } else {
                    log::error!("No glTF path provided");
                    self.create_simple_scene(renderer);
                }
            }
        }
    }

    // ========================================================================
    // Scene Creation - Pure hecs
    // ========================================================================

    fn create_simple_scene(&mut self, renderer: &mut Renderer) {
        log::info!("Creating simple scene...");

        // Load mesh
        let (verts, idx) = crate::renderer::cube_mesh();
        let cube_mesh = renderer.create_mesh(&verts, &idx);
        let cube_handle = self.scene.assets.meshes.insert(cube_mesh);

        // Create texture
        let texture = Texture::checkerboard(
            renderer.get_device(),
            renderer.get_queue(),
            256,
            32,
            [255, 255, 255, 255],
            [0, 0, 0, 255],
            Some("Checkerboard"),
        );
        self.scene.assets.textures.insert(texture);
        renderer.update_texture_bind_group(&self.scene.assets);

        // Option 1: Using EntityBuilder helper
        EntityBuilder::new(&mut self.scene.world)
            .with_name("Red Cube")
            .with_transform(Transform::from_trs(
                Vec3::new(-2.0, 0.0, 0.0),
                Quat::IDENTITY,
                Vec3::ONE,
            ))
            .with_mesh(cube_handle)
            .with_material(Material::red())
            .visible(true)
            .spawn();

        // Option 2: Using hecs directly (no builder)
        self.scene.world.spawn((
            Name::new("Green Cube"),
            TransformComponent(Transform::from_trs(
                Vec3::new(0.0, 0.0, 0.0),
                Quat::IDENTITY,
                Vec3::ONE,
            )),
            MeshComponent(cube_handle),
            MaterialComponent(Material::green()),
            Visible(true),
        ));

        // Option 3: Builder again
        EntityBuilder::new(&mut self.scene.world)
            .with_name("Blue Cube")
            .with_transform(Transform::from_trs(
                Vec3::new(2.0, 0.0, 0.0),
                Quat::IDENTITY,
                Vec3::ONE,
            ))
            .with_mesh(cube_handle)
            .with_material(Material::blue())
            .visible(true)
            .spawn();

        log::info!("Simple scene: {} entities", self.scene.world.len());
    }

    fn create_grid_scene(&mut self, renderer: &mut Renderer, size: i32) {
        log::info!("Creating grid scene...");

        let (verts, idx) = crate::renderer::cube_mesh();
        let cube_mesh = renderer.create_mesh(&verts, &idx);
        let cube_handle = self.scene.assets.meshes.insert(cube_mesh);

        // Create textures
        let colors = [
            [255, 100, 100, 255],
            [100, 255, 100, 255],
            [100, 100, 255, 255],
            [255, 255, 100, 255],
            [255, 100, 255, 255],
        ];

        for color in colors {
            let texture =
                Texture::from_color(renderer.get_device(), renderer.get_queue(), color, None);
            self.scene.assets.textures.insert(texture);
        }
        renderer.update_texture_bind_group(&self.scene.assets);

        // Spawn grid using pure hecs (no builder)
        let spacing = 2.0;
        for x in -size..=size {
            for z in -size..=size {
                let pos = Vec3::new(x as f32 * spacing, 0.0, z as f32 * spacing);
                let texture_idx = ((x.abs() + z.abs()) % 5) as u32;

                // Pure hecs spawn
                self.scene.world.spawn((
                    Name::new(format!("Cube_{}_{}", x, z)),
                    TransformComponent(Transform::from_trs(pos, Quat::IDENTITY, Vec3::splat(0.4))),
                    MeshComponent(cube_handle),
                    MaterialComponent(Material::white().with_texture(texture_idx)),
                    Visible(true),
                ));
            }
        }

        log::info!("Grid scene: {} entities", self.scene.world.len());
    }

    fn create_animated_scene(&mut self, renderer: &mut Renderer) {
        log::info!("Creating animated scene...");

        let (verts, idx) = crate::renderer::cube_mesh();
        let cube_mesh = renderer.create_mesh(&verts, &idx);
        let cube_handle = self.scene.assets.meshes.insert(cube_mesh);

        let gradient = Texture::gradient(
            renderer.get_device(),
            renderer.get_queue(),
            256,
            [255, 0, 0, 255],
            [0, 0, 255, 255],
            Some("Gradient"),
        );
        self.scene.assets.textures.insert(gradient);
        renderer.update_texture_bind_group(&self.scene.assets);

        // Central rotating sun - using builder
        EntityBuilder::new(&mut self.scene.world)
            .with_name("Sun")
            .with_transform(Transform::from_trs(
                Vec3::ZERO,
                Quat::IDENTITY,
                Vec3::splat(2.0),
            ))
            .with_mesh(cube_handle)
            .with_material(Material::white().with_texture(0))
            .with_rotation_animation(Vec3::Y, 0.5)
            .visible(true)
            .spawn();

        // Orbiting planets - using pure hecs
        let planet_count = 12;
        for i in 0..planet_count {
            let offset = (i as f32) * std::f32::consts::TAU / (planet_count as f32);

            self.scene.world.spawn((
                Name::new(format!("Planet_{}", i)),
                TransformComponent(Transform::default()),
                MeshComponent(cube_handle),
                MaterialComponent(Material::white().with_texture(0)),
                Visible(true),
                OrbitAnimation {
                    center: Vec3::ZERO,
                    radius: 5.0,
                    speed: 0.5,
                    offset,
                },
            ));
        }

        log::info!("Animated scene: {} entities", self.scene.world.len());
    }

    fn create_material_showcase(&mut self, renderer: &mut Renderer) {
        log::info!("Creating material showcase...");

        let (verts, idx) = crate::renderer::cube_mesh();
        let cube_mesh = renderer.create_mesh(&verts, &idx);
        let cube_handle = self.scene.assets.meshes.insert(cube_mesh);

        // Create various textures
        let textures = vec![
            Texture::checkerboard(
                renderer.get_device(),
                renderer.get_queue(),
                256,
                32,
                [255, 255, 255, 255],
                [0, 0, 0, 255],
                Some("Checkerboard"),
            ),
            Texture::gradient(
                renderer.get_device(),
                renderer.get_queue(),
                256,
                [255, 0, 0, 255],
                [255, 255, 0, 255],
                Some("Gradient"),
            ),
            Texture::radial(
                renderer.get_device(),
                renderer.get_queue(),
                256,
                [255, 255, 255, 255],
                [0, 0, 255, 255],
                Some("Radial"),
            ),
            Texture::noise(
                renderer.get_device(),
                renderer.get_queue(),
                256,
                42,
                Some("Noise"),
            ),
        ];

        for texture in textures {
            self.scene.assets.textures.insert(texture);
        }
        renderer.update_texture_bind_group(&self.scene.assets);

        let positions = [
            Vec3::new(-3.0, 0.0, 0.0),
            Vec3::new(-1.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(3.0, 0.0, 0.0),
        ];

        for (i, pos) in positions.iter().enumerate() {
            // Pure hecs spawn with rotation animation
            self.scene.world.spawn((
                Name::new(format!("Cube_{}", i)),
                TransformComponent(Transform::from_trs(*pos, Quat::IDENTITY, Vec3::ONE)),
                MeshComponent(cube_handle),
                MaterialComponent(Material::white().with_texture(i as u32)),
                Visible(true),
                RotateAnimation {
                    axis: Vec3::new(0.3, 1.0, 0.2).normalize(),
                    speed: 0.8,
                },
            ));
        }

        log::info!("Material showcase: {} entities", self.scene.world.len());
    }

fn load_gltf_scene(&mut self, path: &str, renderer: &mut Renderer) {
        log::info!("Loading glTF: {} (scale: {})", path, self.gltf_scale);

        match SceneLoader::load_gltf(path, &mut self.scene, renderer, self.gltf_scale) {
            Ok(_) => {
                renderer.update_texture_bind_group(&self.scene.assets);
                log::info!("glTF loaded: {} entities", self.scene.world.len());
            }
            Err(e) => {
                log::error!("Failed to load glTF: {}", e);
                self.create_simple_scene(renderer);
            }
        }
    }

    fn update_scene(&mut self, dt: f64) {
        // This runs all the ECS systems
        self.scene.update(dt);
    }

   fn update_camera(&mut self) {
        let t = self.scene.time() as f32;
        
        let (radius, height) = match self.scene_type {
            SceneType::FromGltf => {
                // Adjust camera based on scale
                let base_radius = 2.0;
                let base_height = 1.0;
                (base_radius * self.gltf_scale.log10().max(0.5), 
                 base_height * self.gltf_scale.log10().max(0.5))
            },
            SceneType::Simple => (8.0, 4.0),
            SceneType::Grid => (15.0, 8.0),
            SceneType::Animated => (12.0, 6.0),
            SceneType::MaterialShowcase => (8.0, 4.0),
        };

        self.camera.eye = Vec3::new(t.cos() * radius, height, t.sin() * radius);
        self.camera.target = Vec3::ZERO;
        self.camera.up = Vec3::Y;
    }
}

// ============================================================================
// Winit ApplicationHandler
// ============================================================================

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            log::info!("Initializing application...");

            let window = event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("wgpu hecs Renderer")
                        .with_inner_size(winit::dpi::LogicalSize::new(1280, 720)),
                )
                .expect("Failed to create window");
            let id = window.id();

            let mut renderer = pollster::block_on(Renderer::new(&window));
            self.setup_scene(&mut renderer);

            self.window = Some(window);
            self.window_id = Some(id);
            self.renderer = Some(renderer);

            if let Some(w) = &self.window {
                w.request_redraw();
            }

            log::info!("Application initialized");
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
                if let Some(renderer) = self.renderer.as_mut() {
                    if let Some(window) = &self.window {
                        renderer.resize(window.inner_size());
                    }
                }
            }

            WindowEvent::RedrawRequested => {
                // Update time
                let now = std::time::Instant::now();
                let dt = (now - self.scene.last_frame()).as_secs_f64();
                self.scene.set_last_frame(now);

                // Update scene (no borrow of renderer)
                self.update_scene(dt);
                self.update_camera();

                // Now borrow renderer for rendering
                if let Some(renderer) = self.renderer.as_mut() {
                    let aspect = renderer.aspect_ratio();
                    renderer.set_camera(&self.camera, aspect);
                    self.scene.render(renderer, &mut self.batcher);
                }

                // Request next frame
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }

            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key,
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => match logical_key {
                Key::Named(NamedKey::Escape) => {
                    event_loop.exit();
                }
                Key::Character(c) if ('1'..='4').contains(&c.as_str().chars().next().unwrap()) => {
                    let scene_num = c.parse::<usize>().unwrap() - 1;
                    let new_type = match scene_num {
                        0 => SceneType::Simple,
                        1 => SceneType::Grid,
                        2 => SceneType::Animated,
                        3 => SceneType::MaterialShowcase,
                        _ => return,
                    };

                    if new_type != self.scene_type {
                        log::info!("Switching to {:?}", new_type);
                        self.scene_type = new_type;

                        // Create new scene and explicitly drop old one first
                        let old_scene = std::mem::replace(&mut self.scene, Scene::new());
                        drop(old_scene);

                        // Take renderer out temporarily to avoid double borrow
                        if let Some(mut renderer) = self.renderer.take() {
                            self.setup_scene(&mut renderer);
                            self.renderer = Some(renderer); // Put it back
                        }
                    }
                }
                _ => {} // Catch-all for unhandled keys
            },

            _ => {}
        }
    }
}
