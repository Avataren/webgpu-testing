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

use crate::renderer::{RenderBatcher, Renderer, Texture};
use crate::settings::RenderSettings;

#[cfg(target_arch = "wasm32")]
type WindowHandle = Rc<Window>;
#[cfg(not(target_arch = "wasm32"))]
type WindowHandle = Window;
#[cfg(target_arch = "wasm32")]
type PendingRenderer = Rc<RefCell<Option<Renderer>>>;

use crate::scene::{Children, MeshComponent, Name, Parent, Scene, TransformComponent};
use crate::time::Instant;

pub struct StartupContext<'a> {
    pub scene: &'a mut Scene,
    pub renderer: &'a mut Renderer,
}

pub struct UpdateContext<'a> {
    pub scene: &'a mut Scene,
    pub dt: f64,
}

pub struct GpuUpdateContext<'a> {
    pub scene: &'a mut Scene,
    pub renderer: &'a mut Renderer,
    pub dt: f64,
}

pub type StartupSystem = Box<dyn for<'a> FnMut(&mut StartupContext<'a>) + 'static>;
pub type UpdateSystem = Box<dyn for<'a> FnMut(&mut UpdateContext<'a>) + 'static>;
pub type GpuUpdateSystem = Box<dyn for<'a> FnMut(&mut GpuUpdateContext<'a>) + 'static>;

pub trait Plugin {
    fn build(&self, app: &mut AppBuilder);
}

pub struct AppBuilder {
    startup_systems: Vec<StartupSystem>,
    update_systems: Vec<UpdateSystem>,
    gpu_systems: Vec<GpuUpdateSystem>,
    auto_init_default_textures: bool,
    auto_add_default_lighting: bool,
    skip_initial_frames: Option<u32>,
    settings: RenderSettings,
}

impl Default for AppBuilder {
    fn default() -> Self {
        Self {
            startup_systems: Vec::new(),
            update_systems: Vec::new(),
            gpu_systems: Vec::new(),
            auto_init_default_textures: true,
            auto_add_default_lighting: true,
            skip_initial_frames: None,
            settings: RenderSettings::load(),
        }
    }
}

impl AppBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_startup_system<F>(&mut self, system: F) -> &mut Self
    where
        F: for<'a> FnMut(&mut StartupContext<'a>) + 'static,
    {
        self.startup_systems.push(Box::new(system));
        self
    }

    pub fn add_system<F>(&mut self, system: F) -> &mut Self
    where
        F: for<'a> FnMut(&mut UpdateContext<'a>) + 'static,
    {
        self.update_systems.push(Box::new(system));
        self
    }

    pub fn add_gpu_system<F>(&mut self, system: F) -> &mut Self
    where
        F: for<'a> FnMut(&mut GpuUpdateContext<'a>) + 'static,
    {
        self.gpu_systems.push(Box::new(system));
        self
    }

    pub fn add_plugin<P: Plugin>(&mut self, plugin: P) -> &mut Self {
        plugin.build(self);
        self
    }

    pub fn set_settings(&mut self, settings: RenderSettings) -> &mut Self {
        self.settings = settings;
        self
    }

    pub fn disable_default_textures(&mut self) -> &mut Self {
        self.auto_init_default_textures = false;
        self
    }

    pub fn disable_default_lighting(&mut self) -> &mut Self {
        self.auto_add_default_lighting = false;
        self
    }

    pub fn skip_initial_frames(&mut self, frames: u32) -> &mut Self {
        self.skip_initial_frames = Some(frames);
        self
    }

    pub fn build(self) -> App {
        App {
            renderer: None,
            window: None,
            window_id: None,
            scene: Scene::new(),
            batcher: RenderBatcher::new(),
            startup_systems: self.startup_systems,
            update_systems: self.update_systems,
            gpu_systems: self.gpu_systems,
            auto_init_default_textures: self.auto_init_default_textures,
            auto_add_default_lighting: self.auto_add_default_lighting,
            startup_ran: false,
            frame_counter: 0,
            skip_rendering_until_frame: self.skip_initial_frames,
            settings: self.settings,
            #[cfg(target_arch = "wasm32")]
            pending_renderer: None,
        }
    }
}

pub struct App {
    renderer: Option<Renderer>,
    window: Option<WindowHandle>,
    window_id: Option<WindowId>,
    scene: Scene,
    batcher: RenderBatcher,
    startup_systems: Vec<StartupSystem>,
    update_systems: Vec<UpdateSystem>,
    gpu_systems: Vec<GpuUpdateSystem>,
    auto_init_default_textures: bool,
    auto_add_default_lighting: bool,
    startup_ran: bool,
    frame_counter: u32,
    skip_rendering_until_frame: Option<u32>,
    settings: RenderSettings,
    #[cfg(target_arch = "wasm32")]
    pending_renderer: Option<PendingRenderer>,
}

impl App {
    pub fn new() -> Self {
        AppBuilder::default().build()
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
            self.run_startup_systems(&mut renderer);
            renderer.update_texture_bind_group(&self.scene.assets);

            self.renderer = Some(renderer);
            self.pending_renderer = None;

            if let Some(window) = &self.window {
                window.request_redraw();
            }

            log::info!("Renderer initialized successfully");
        }
    }

    fn run_startup_systems(&mut self, renderer: &mut Renderer) {
        if self.startup_ran {
            return;
        }

        if self.auto_init_default_textures && self.scene.assets.textures.is_empty() {
            self.init_default_textures(renderer);
        }

        for system in &mut self.startup_systems {
            let mut ctx = StartupContext {
                scene: &mut self.scene,
                renderer,
            };
            (system)(&mut ctx);
        }

        if self.auto_add_default_lighting {
            let added_lights = self.scene.add_default_lighting();
            if added_lights > 0 {
                log::info!("Added {} default lights to scene", added_lights);
            }
        }

        log::info!("Running initial transform propagation...");
        self.scene.update(0.0);
        log::info!("Initial propagation complete");

        self.startup_ran = true;
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

        for system in &mut self.update_systems {
            let mut ctx = UpdateContext {
                scene: &mut self.scene,
                dt,
            };
            (system)(&mut ctx);
        }
    }
}

// ============================================================================
// Winit ApplicationHandler
// ============================================================================

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            log::info!("Initializing application...");

            // Build window attributes with web-specific configuration
            let base_window_attrs = Window::default_attributes()
                .with_title("wgpu hecs Renderer")
                .with_inner_size(winit::dpi::LogicalSize::new(
                    f64::from(self.settings.resolution.width),
                    f64::from(self.settings.resolution.height),
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
            let id = window.id();

            #[cfg(not(target_arch = "wasm32"))]
            {
                let mut renderer =
                    pollster::block_on(Renderer::new(&window, self.settings.clone()));

                self.scene.init_timer();
                self.run_startup_systems(&mut renderer);

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
                let settings = self.settings.clone();

                log::info!("Spawning asynchronous renderer initialization");

                spawn_local(async move {
                    let renderer = Renderer::new(&window_for_renderer, settings).await;
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

                if let Some(renderer) = self.renderer.as_mut() {
                    {
                        let scene = &mut self.scene;
                        for system in &mut self.gpu_systems {
                            let mut ctx = GpuUpdateContext {
                                scene,
                                renderer,
                                dt,
                            };
                            (system)(&mut ctx);
                        }
                    }

                    if !should_skip {
                        let aspect = renderer.aspect_ratio();
                        renderer.set_camera(self.scene.camera(), aspect);
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
