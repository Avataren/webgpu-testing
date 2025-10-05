// app.rs - Complete fixed version with hierarchy test scene
use winit::{
    application::ApplicationHandler,
    event::*,
    event_loop::ActiveEventLoop,
    keyboard::{Key, NamedKey},
    window::{Window, WindowId},
};

#[cfg(target_arch = "wasm32")]
use std::{cell::RefCell, rc::Rc};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_futures::spawn_local;

use crate::renderer::{Material, RenderBatcher, Renderer, Texture};

#[cfg(target_arch = "wasm32")]
type WindowHandle = Rc<Window>;
#[cfg(not(target_arch = "wasm32"))]
type WindowHandle = Window;
#[cfg(target_arch = "wasm32")]
type PendingRenderer = Rc<RefCell<Option<Renderer>>>;

use crate::scene::{
    Camera, Children, EntityBuilder, MaterialComponent, MeshComponent, Name, OrbitAnimation,
    Parent, RotateAnimation, Scene, SceneLoader, Transform, TransformComponent, Visible,
};
use crate::time::Instant;
use glam::{Quat, Vec3};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneType {
    Simple,
    Grid,
    Animated,
    MaterialShowcase,
    HierarchyTest,
    PbrTest,
    FromGltf,
}

pub struct App {
    renderer: Option<Renderer>,
    window: Option<WindowHandle>,
    window_id: Option<WindowId>,
    scene: Scene,
    batcher: RenderBatcher,
    camera: Camera,
    scene_type: SceneType,
    gltf_path: Option<String>,
    gltf_scale: f32,
    old_scenes: Vec<Scene>,
    old_renderers: Vec<Renderer>,
    frame_counter: u32,
    skip_rendering_until_frame: Option<u32>,
    #[cfg(target_arch = "wasm32")]
    pending_renderer: Option<PendingRenderer>,
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
            old_scenes: Vec::new(),
            old_renderers: Vec::new(),
            frame_counter: 0,
            skip_rendering_until_frame: None,
            #[cfg(target_arch = "wasm32")]
            pending_renderer: None,
        }
    }

    fn init_default_textures(&mut self, renderer: &mut Renderer) {
        let white = Texture::white(renderer.get_device(), renderer.get_queue());
        self.scene.assets.textures.insert(white);

        let normal = Texture::default_normal(renderer.get_device(), renderer.get_queue());
        self.scene.assets.textures.insert(normal);

        let mr = Texture::default_metallic_roughness(renderer.get_device(), renderer.get_queue());
        self.scene.assets.textures.insert(mr);

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
            old_scenes: Vec::new(),
            old_renderers: Vec::new(),
            frame_counter: 0,
            skip_rendering_until_frame: None,
            #[cfg(target_arch = "wasm32")]
            pending_renderer: None,
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

            self.scene.init_timer();
            self.setup_scene(&mut renderer);
            renderer.update_texture_bind_group(&self.scene.assets);

            self.renderer = Some(renderer);
            self.pending_renderer = None;

            if let Some(window) = &self.window {
                window.request_redraw();
            }

            log::info!("Renderer initialized successfully");
        }
    }

    fn setup_scene(&mut self, renderer: &mut Renderer) {
        if self.scene.assets.textures.is_empty() && self.scene_type != SceneType::FromGltf {
            self.init_default_textures(renderer);
        }

        match self.scene_type {
            SceneType::Simple => self.create_simple_scene(renderer),
            SceneType::Grid => self.create_grid_scene(renderer, 5),
            SceneType::HierarchyTest => self.create_hierarchy_test_scene(renderer),
            SceneType::FromGltf => {
                if let Some(path) = self.gltf_path.clone() {
                    self.load_gltf_scene(&path, renderer);
                } else {
                    log::error!("No glTF path provided");
                    self.create_simple_scene(renderer);
                }
            }
            SceneType::PbrTest => self.create_pbr_test_scene(renderer),
            _ => {}
        }

        // CRITICAL: Propagate transforms immediately after scene creation
        // This ensures WorldTransform components exist before first render
        log::info!("Running initial transform propagation...");
        self.scene.update(0.0);
        log::info!("Initial propagation complete");
    }

    // ========================================================================
    // NEW: Hierarchy Test Scene
    // ========================================================================

    fn create_hierarchy_test_scene(&mut self, renderer: &mut Renderer) {
        log::info!("Creating hierarchy test scene...");

        let (verts, idx) = crate::renderer::cube_mesh();
        let cube_mesh = renderer.create_mesh(&verts, &idx);
        let cube_handle = self.scene.assets.meshes.insert(cube_mesh);

        // Create colored textures
        let colors = [
            [255, 0, 0, 255],   // Red
            [0, 255, 0, 255],   // Green
            [0, 0, 255, 255],   // Blue
            [255, 255, 0, 255], // Yellow
            [255, 0, 255, 255], // Magenta
        ];

        for color in colors {
            let texture =
                Texture::from_color(renderer.get_device(), renderer.get_queue(), color, None);
            self.scene.assets.textures.insert(texture);
        }
        renderer.update_texture_bind_group(&self.scene.assets);

        // Test 1: Simple Parent-Child (should see green cube offset from red)
        log::info!("Creating Test 1: Simple parent-child");
        let parent1 = self.scene.world.spawn((
            Name::new("Parent1 (Red)"),
            TransformComponent(Transform::from_trs(
                Vec3::new(-6.0, 0.0, 0.0), // Parent at -6,0,0
                Quat::IDENTITY,
                Vec3::ONE,
            )),
            MeshComponent(cube_handle),
            MaterialComponent(Material::white().with_texture(0)), // Red
            Visible(true),
        ));

        let child1 = self.scene.world.spawn((
            Name::new("Child1 (Green)"),
            TransformComponent(Transform::from_trs(
                Vec3::new(2.0, 0.0, 0.0), // Offset +2 in X from parent
                Quat::IDENTITY,
                Vec3::splat(0.5), // Half size
            )),
            MeshComponent(cube_handle),
            MaterialComponent(Material::white().with_texture(1)), // Green
            Visible(true),
            Parent(parent1),
        ));

        // Add children list to parent
        self.scene
            .world
            .insert_one(parent1, Children(vec![child1]))
            .ok();

        // Test 2: Three-level hierarchy (Grandparent -> Parent -> Child)
        log::info!("Creating Test 2: Three-level hierarchy");
        let grandparent = self.scene.world.spawn((
            Name::new("Grandparent (Blue)"),
            TransformComponent(Transform::from_trs(
                Vec3::new(0.0, 0.0, 0.0), // Center
                Quat::IDENTITY,
                Vec3::ONE,
            )),
            MeshComponent(cube_handle),
            MaterialComponent(Material::white().with_texture(2)), // Blue
            Visible(true),
        ));

        let parent2 = self.scene.world.spawn((
            Name::new("Parent2 (Yellow)"),
            TransformComponent(Transform::from_trs(
                Vec3::new(0.0, 2.0, 0.0), // Offset +2 in Y
                Quat::IDENTITY,
                Vec3::splat(0.8),
            )),
            MeshComponent(cube_handle),
            MaterialComponent(Material::white().with_texture(3)), // Yellow
            Visible(true),
            Parent(grandparent),
        ));

        let child2 = self.scene.world.spawn((
            Name::new("Child2 (Magenta)"),
            TransformComponent(Transform::from_trs(
                Vec3::new(0.0, 1.5, 0.0), // Offset +1.5 in Y from parent
                Quat::IDENTITY,
                Vec3::splat(0.6),
            )),
            MeshComponent(cube_handle),
            MaterialComponent(Material::white().with_texture(4)), // Magenta
            Visible(true),
            Parent(parent2),
        ));

        self.scene
            .world
            .insert_one(grandparent, Children(vec![parent2]))
            .ok();
        self.scene
            .world
            .insert_one(parent2, Children(vec![child2]))
            .ok();

        // Test 3: Rotation hierarchy
        log::info!("Creating Test 3: Rotation hierarchy");
        let rotating_parent = self.scene.world.spawn((
            Name::new("Rotating Parent (Red)"),
            TransformComponent(Transform::from_trs(
                Vec3::new(6.0, 0.0, 0.0),
                Quat::IDENTITY,
                Vec3::ONE,
            )),
            MeshComponent(cube_handle),
            MaterialComponent(Material::white().with_texture(0)), // Red
            Visible(true),
            RotateAnimation {
                axis: Vec3::Y,
                speed: 1.0,
            },
        ));

        let rotating_child = self.scene.world.spawn((
            Name::new("Rotating Child (Green)"),
            TransformComponent(Transform::from_trs(
                Vec3::new(3.0, 0.0, 0.0), // Should orbit around parent
                Quat::IDENTITY,
                Vec3::splat(0.5),
            )),
            MeshComponent(cube_handle),
            MaterialComponent(Material::white().with_texture(1)), // Green
            Visible(true),
            Parent(rotating_parent),
        ));

        self.scene
            .world
            .insert_one(rotating_parent, Children(vec![rotating_child]))
            .ok();

        // Test 4: Scale hierarchy
        log::info!("Creating Test 4: Scale hierarchy");
        let scaled_parent = self.scene.world.spawn((
            Name::new("Scaled Parent (Blue)"),
            TransformComponent(Transform::from_trs(
                Vec3::new(0.0, -3.0, 0.0),
                Quat::IDENTITY,
                Vec3::splat(2.0), // 2x scale
            )),
            MeshComponent(cube_handle),
            MaterialComponent(Material::white().with_texture(2)), // Blue
            Visible(true),
        ));

        let scaled_child = self.scene.world.spawn((
            Name::new("Scaled Child (Yellow)"),
            TransformComponent(Transform::from_trs(
                Vec3::new(1.5, 0.0, 0.0), // Translation should also be scaled
                Quat::IDENTITY,
                Vec3::splat(0.5), // 0.5x scale (net 1.0x when combined)
            )),
            MeshComponent(cube_handle),
            MaterialComponent(Material::white().with_texture(3)), // Yellow
            Visible(true),
            Parent(scaled_parent),
        ));

        self.scene
            .world
            .insert_one(scaled_parent, Children(vec![scaled_child]))
            .ok();

        log::info!(
            "Hierarchy test scene created: {} entities",
            self.scene.world.len()
        );
        log::info!("Expected layout:");
        log::info!("  Test 1 (left): Red cube at (-6,0,0), green smaller cube at (-4,0,0)");
        log::info!("  Test 2 (center): Blue->Yellow->Magenta vertical stack");
        log::info!("  Test 3 (right): Red cube at (6,0,0) with green orbiting child");
        log::info!("  Test 4 (bottom): Large blue cube with yellow child offset");
    }

    fn create_simple_scene(&mut self, renderer: &mut Renderer) {
        log::info!("Creating simple scene...");

        let (verts, idx) = crate::renderer::cube_mesh();
        let cube_mesh = renderer.create_mesh(&verts, &idx);
        let cube_handle = self.scene.assets.meshes.insert(cube_mesh);

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

        let spacing = 2.0;
        for x in -size..=size {
            for z in -size..=size {
                let pos = Vec3::new(x as f32 * spacing, 0.0, z as f32 * spacing);
                let texture_idx = ((x.abs() + z.abs()) % 5) as u32;

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

    fn create_pbr_test_scene(&mut self, renderer: &mut Renderer) {
        log::info!("Creating PBR test scene...");

        // Create sphere mesh (higher resolution for better PBR visualization)
        let (verts, idx) = crate::renderer::sphere_mesh(32, 16);
        let sphere_mesh = renderer.create_mesh(&verts, &idx);
        let sphere_handle = self.scene.assets.meshes.insert(sphere_mesh);

        // Create a white base texture
        let white = Texture::white(renderer.get_device(), renderer.get_queue());
        self.scene.assets.textures.insert(white);
        renderer.update_texture_bind_group(&self.scene.assets);

        // Create a grid of spheres with varying metallic and roughness values
        let grid_size = 5; // 5x5 grid
        let spacing = 2.5;
        let start_offset = -((grid_size - 1) as f32 * spacing) / 2.0;

        log::info!(
            "Creating {}x{} grid of PBR test spheres",
            grid_size,
            grid_size
        );

        for row in 0..grid_size {
            for col in 0..grid_size {
                let x = start_offset + col as f32 * spacing;
                let z = start_offset + row as f32 * spacing;

                // Vary metallic along X axis (0.0 to 1.0)
                let metallic = col as f32 / (grid_size - 1) as f32;

                // Vary roughness along Z axis (0.0 to 1.0)
                let roughness = row as f32 / (grid_size - 1) as f32;

                // Color based on position for visual distinction
                let color = if col == 0 && row == 0 {
                    [200, 200, 200, 255] // Light gray for (0,0)
                } else if col == grid_size - 1 && row == 0 {
                    [200, 150, 100, 255] // Copper tone for metallic
                } else if col == 0 && row == grid_size - 1 {
                    [180, 180, 200, 255] // Bluish for rough
                } else {
                    [220, 220, 220, 255] // Nearly white
                };

                let material = Material::new(color)
                    .with_metallic(metallic)
                    .with_roughness(roughness)
                    .with_base_color_texture(0); // Use white texture

                self.scene.world.spawn((
                    Name::new(format!("Sphere_M{:.2}_R{:.2}", metallic, roughness)),
                    TransformComponent(Transform::from_trs(
                        Vec3::new(x, 0.0, z),
                        Quat::IDENTITY,
                        Vec3::splat(0.8),
                    )),
                    MeshComponent(sphere_handle),
                    MaterialComponent(material),
                    Visible(true),
                ));
            }
        }

        // Add labels using small cubes at the edges
        let (label_verts, label_idx) = crate::renderer::cube_mesh();
        let label_mesh = renderer.create_mesh(&label_verts, &label_idx);
        let label_handle = self.scene.assets.meshes.insert(label_mesh);

        // Label colors
        let red_texture = Texture::from_color(
            renderer.get_device(),
            renderer.get_queue(),
            [255, 100, 100, 255],
            Some("Red"),
        );
        let blue_texture = Texture::from_color(
            renderer.get_device(),
            renderer.get_queue(),
            [100, 100, 255, 255],
            Some("Blue"),
        );
        self.scene.assets.textures.insert(red_texture);
        self.scene.assets.textures.insert(blue_texture);
        renderer.update_texture_bind_group(&self.scene.assets);

        // Metallic axis labels (red) - along X axis
        for i in 0..3 {
            let x = start_offset + i as f32 * spacing * 2.0;
            self.scene.world.spawn((
                Name::new(format!("MetallicLabel_{}", i)),
                TransformComponent(Transform::from_trs(
                    Vec3::new(x, 2.0, start_offset - 1.5),
                    Quat::IDENTITY,
                    Vec3::new(0.2, 0.2, 0.2),
                )),
                MeshComponent(label_handle),
                MaterialComponent(Material::white().with_texture(1)), // Red
                Visible(true),
            ));
        }

        // Roughness axis labels (blue) - along Z axis
        for i in 0..3 {
            let z = start_offset + i as f32 * spacing * 2.0;
            self.scene.world.spawn((
                Name::new(format!("RoughnessLabel_{}", i)),
                TransformComponent(Transform::from_trs(
                    Vec3::new(start_offset - 1.5, 2.0, z),
                    Quat::IDENTITY,
                    Vec3::new(0.2, 0.2, 0.2),
                )),
                MeshComponent(label_handle),
                MaterialComponent(Material::white().with_texture(2)), // Blue
                Visible(true),
            ));
        }

        log::info!("PBR test scene: {} entities", self.scene.world.len());
        log::info!("Grid layout:");
        log::info!("  X-axis (red labels): Metallic 0.0 → 1.0");
        log::info!("  Z-axis (blue labels): Roughness 0.0 → 1.0");
        log::info!("  Front-left: Non-metallic, smooth");
        log::info!("  Front-right: Metallic, smooth (mirror-like)");
        log::info!("  Back-left: Non-metallic, rough (matte)");
        log::info!("  Back-right: Metallic, rough");
    }

    fn load_gltf_scene(&mut self, path: &str, renderer: &mut Renderer) {
        log::info!("Loading glTF: {} (scale: {})", path, self.gltf_scale);

        match SceneLoader::load_gltf(path, &mut self.scene, renderer, self.gltf_scale) {
            Ok(_) => {
                renderer.update_texture_bind_group(&self.scene.assets);
                log::info!("glTF loaded: {} entities", self.scene.world.len());

                // Debug: Print hierarchy
                self.debug_print_hierarchy();
            }
            Err(e) => {
                log::error!("Failed to load glTF: {}", e);
                self.create_simple_scene(renderer);
            }
        }
    }

    fn debug_print_hierarchy(&self) {
        log::info!("=== Scene Hierarchy ===");

        // Find roots
        let roots: Vec<_> = self
            .scene
            .world
            .query::<()>()
            .without::<&Parent>()
            .iter()
            .map(|(e, _)| e)
            .collect();

        log::info!("Found {} root entities", roots.len());

        for root in roots {
            self.debug_print_entity(root, 0);
        }

        log::info!("======================");
    }

    fn debug_print_entity(&self, entity: hecs::Entity, depth: usize) {
        let indent = "  ".repeat(depth);

        let name = self
            .scene
            .world
            .get::<&Name>(entity)
            .map(|n| n.0.clone())
            .unwrap_or_else(|_| format!("{:?}", entity));

        let local_transform = self
            .scene
            .world
            .get::<&TransformComponent>(entity)
            .map(|t| {
                format!(
                    "T:{:?} R:{:?} S:{:?}",
                    t.0.translation, t.0.rotation, t.0.scale
                )
            })
            .unwrap_or_else(|_| "No local transform".to_string());

        let world_transform = self
            .scene
            .world
            .get::<&crate::scene::components::WorldTransform>(entity)
            .map(|t| format!("WorldT:{:?}", t.0.translation))
            .unwrap_or_else(|_| "No WorldTransform".to_string());

        let has_mesh = self.scene.world.get::<&MeshComponent>(entity).is_ok();

        log::info!(
            "{}└─ {} [{}]",
            indent,
            name,
            if has_mesh { "Mesh" } else { "Empty" },
        );
        log::info!("{}   Local: {}", indent, local_transform);
        log::info!("{}   {}", indent, world_transform);

        // Print children
        if let Ok(children) = self.scene.world.get::<&Children>(entity) {
            for child in &children.0 {
                self.debug_print_entity(*child, depth + 1);
            }
        }
    }

    fn update_scene(&mut self, dt: f64) {
        self.scene.update(dt);
    }

    fn update_camera(&mut self) {
        let t = self.scene.time() as f32;

        let (radius, height) = match self.scene_type {
            SceneType::HierarchyTest => (15.0, 8.0),
            SceneType::FromGltf => {
                let base_radius = 5.0;
                let base_height = 2.0;
                (
                    base_radius * self.gltf_scale.log10().max(0.5),
                    base_height * self.gltf_scale.log10().max(0.5),
                )
            }
            SceneType::Simple => (8.0, 4.0),
            SceneType::Grid => (15.0, 8.0),
            SceneType::Animated => (12.0, 6.0),
            SceneType::PbrTest => (8.0, 2.0),
            _ => (8.0, 4.0),
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

            #[cfg(not(target_arch = "wasm32"))]
            {
                let mut renderer = pollster::block_on(Renderer::new(&window));

                self.scene.init_timer();
                self.setup_scene(&mut renderer);

                renderer.update_texture_bind_group(&self.scene.assets);
                log::info!(
                    "Bind group updated with {} textures",
                    self.scene.assets.textures.len()
                );

                self.window = Some(window);
                self.window_id = Some(id);
                self.renderer = Some(renderer);

                if let Some(w) = &self.window {
                    w.request_redraw();
                }

                log::info!("Application initialized");
            }

            #[cfg(target_arch = "wasm32")]
            {
                let window_handle = Rc::new(window);
                let pending_renderer: PendingRenderer = Rc::new(RefCell::new(None));
                let renderer_cell = pending_renderer.clone();
                let window_for_renderer = window_handle.clone();

                log::info!("Spawning asynchronous renderer initialization");

                spawn_local(async move {
                    let renderer = Renderer::new(&window_for_renderer).await;
                    renderer_cell.borrow_mut().replace(renderer);
                    window_for_renderer.request_redraw();
                });

                self.window = Some(window_handle);
                self.window_id = Some(id);
                self.pending_renderer = Some(pending_renderer);

                log::info!("Waiting for renderer to finish initializing");
            }
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
                #[cfg(target_arch = "wasm32")]
                self.try_finish_async_initialization();

                self.frame_counter += 1;

                let should_skip = if let Some(skip_until) = self.skip_rendering_until_frame {
                    if self.frame_counter < skip_until {
                        true
                    } else {
                        self.skip_rendering_until_frame = None;
                        false
                    }
                } else {
                    false
                };

                let now = Instant::now();
                let dt = (now - self.scene.last_frame()).as_secs_f64();
                self.scene.set_last_frame(now);

                self.update_scene(dt);
                self.update_camera();

                if !should_skip {
                    if let Some(renderer) = self.renderer.as_mut() {
                        let aspect = renderer.aspect_ratio();
                        renderer.set_camera(&self.camera, aspect);
                        self.scene.render(renderer, &mut self.batcher);
                    }
                }

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
                Key::Character(c) if c.as_str() == "h" => {
                    // Print hierarchy on 'h' key
                    self.debug_print_hierarchy();
                }
                _ => {}
            },

            _ => {}
        }
    }
}
