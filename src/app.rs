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

#[cfg(feature = "egui")]
use crate::ui::{
    egui, EguiRenderTarget, EguiUiCallback, FrameStatsHandle, FrameStatsHistory,
    PostProcessEffectsHandle, PostProcessWindow,
};

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
            #[cfg(feature = "egui")]
            egui_context: None,
            #[cfg(feature = "egui")]
            egui_pending_ui: None,
            #[cfg(feature = "egui")]
            frame_stats: FrameStatsHistory::handle(),
            #[cfg(feature = "egui")]
            postprocess_effects: PostProcessWindow::handle(),
            window: None,
            window_id: None,
            renderer: None,
        }
    }
}

struct FrameStep {
    dt: f64,
    skip_rendering: bool,
}

impl FrameStep {
    fn dt(&self) -> f64 {
        self.dt
    }

    fn should_render(&self) -> bool {
        !self.skip_rendering
    }
}

pub struct App {
    window: Option<WindowHandle>,
    window_id: Option<WindowId>,
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
    #[cfg(feature = "egui")]
    egui_context: Option<crate::ui::EguiContext>,
    #[cfg(feature = "egui")]
    egui_pending_ui: Option<EguiUiCallback>,
    #[cfg(feature = "egui")]
    frame_stats: FrameStatsHandle,
    #[cfg(feature = "egui")]
    postprocess_effects: PostProcessEffectsHandle,
    scene: Scene,
    renderer: Option<Renderer>,
}

impl App {
    pub fn new() -> Self {
        AppBuilder::default().build()
    }

    #[cfg(feature = "egui")]
    pub fn set_egui_ui<F>(&mut self, callback: F)
    where
        F: FnMut(&egui::Context) + 'static,
    {
        if let Some(egui) = &mut self.egui_context {
            egui.set_ui(callback);
            self.egui_pending_ui = None;
        } else {
            self.egui_pending_ui = Some(Box::new(callback));
        }
    }

    #[cfg(feature = "egui")]
    fn install_egui_context(&mut self, mut egui: crate::ui::EguiContext) {
        if let Some(callback) = self.egui_pending_ui.take() {
            egui.set_ui_box(callback);
        }
        self.egui_context = Some(egui);
    }

    #[cfg(feature = "egui")]
    pub fn frame_stats_handle(&self) -> FrameStatsHandle {
        self.frame_stats.clone()
    }

    #[cfg(feature = "egui")]
    pub fn postprocess_effects_handle(&self) -> PostProcessEffectsHandle {
        self.postprocess_effects.clone()
    }

    #[cfg(feature = "egui")]
    fn apply_postprocess_effects(handle: &PostProcessEffectsHandle, renderer: &mut Renderer) {
        if let Ok(effects) = handle.lock() {
            renderer.set_postprocess_effects(*effects);
        }
    }

    fn begin_frame(&mut self) -> FrameStep {
        self.frame_counter += 1;

        let skip_rendering = if let Some(skip_until) = self.skip_rendering_until_frame {
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

        FrameStep { dt, skip_rendering }
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

            // ADD THIS:
            #[cfg(feature = "egui")]
            {
                if let Some(window) = &self.window {
                    let egui = crate::ui::EguiContext::new(
                        renderer.get_device(),
                        renderer.surface_format(),
                        renderer.sample_count(),
                        window,
                    );
                    self.install_egui_context(egui);
                    log::info!("Egui context initialized (async)");
                }
            }

            self.scene.init_timer();
            self.run_startup_systems(&mut renderer);
            renderer.update_texture_bind_group(&self.scene.assets);

            #[cfg(feature = "egui")]
            Self::apply_postprocess_effects(&self.postprocess_effects, &mut renderer);

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

    fn run_update_stage(&mut self, dt: f64) {
        self.scene.update(dt);

        for system in &mut self.update_systems {
            let mut ctx = UpdateContext {
                scene: &mut self.scene,
                dt,
            };
            (system)(&mut ctx);
        }
    }

    fn run_gpu_systems(
        scene: &mut Scene,
        systems: &mut [GpuUpdateSystem],
        renderer: &mut Renderer,
        dt: f64,
    ) {
        for system in systems {
            let mut ctx = GpuUpdateContext {
                scene,
                renderer,
                dt,
            };
            (system)(&mut ctx);
        }
    }

    fn render_scene(&mut self, renderer: &mut Renderer, frame: &FrameStep) {
        if !frame.should_render() {
            return;
        }

        let aspect = renderer.aspect_ratio();
        renderer.set_camera(self.scene.camera(), aspect);

        #[cfg(feature = "egui")]
        Self::apply_postprocess_effects(&self.postprocess_effects, renderer);

        #[cfg(feature = "egui")]
        let egui_output = {
            if let (Some(egui), Some(window)) = (&mut self.egui_context, &self.window) {
                egui.begin_frame(window);
                egui.run_ui();
                Some(egui.end_frame(window))
            } else {
                None
            }
        };

        match self.scene.render(renderer, &mut self.batcher) {
            Ok(render_frame) => {
                #[cfg(feature = "egui")]
                {
                    if let (Some(egui), Some(window), Some(egui_output)) =
                        (&mut self.egui_context, &self.window, egui_output)
                    {
                        let view = render_frame
                            .frame
                            .texture
                            .create_view(&wgpu::TextureViewDescriptor::default());

                        let mut encoder = renderer.get_device().create_command_encoder(
                            &wgpu::CommandEncoderDescriptor {
                                label: Some("egui_encoder"),
                            },
                        );

                        let surface_size = renderer.surface_size();
                        let mut target = EguiRenderTarget {
                            device: renderer.get_device(),
                            queue: renderer.get_queue(),
                            encoder: &mut encoder,
                            window,
                            view: &view,
                            surface_size: [surface_size.width, surface_size.height],
                        };
                        egui.render(&mut target, egui_output);

                        renderer.get_queue().submit(Some(encoder.finish()));
                    }
                }

                render_frame.frame.present();

                #[cfg(feature = "egui")]
                if let Ok(mut history) = self.frame_stats.lock() {
                    history.record(frame.dt() as f32, renderer.last_frame_stats());
                }
            }
            Err(e) => {
                log::error!("Render error: {:?}", e);
            }
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Winit ApplicationHandler
// ============================================================================

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            log::info!("Initializing application...");

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

                #[cfg(feature = "egui")]
                {
                    let egui = crate::ui::EguiContext::new(
                        renderer.get_device(),
                        renderer.surface_format(),
                        renderer.sample_count(),
                        &window,
                    );
                    self.install_egui_context(egui);
                    log::info!("Egui context initialized");
                }

                self.scene.init_timer();
                self.run_startup_systems(&mut renderer);

                renderer.update_texture_bind_group(&self.scene.assets);
                log::info!(
                    "Bind group updated with {} textures",
                    self.scene.assets.textures.len()
                );

                #[cfg(feature = "egui")]
                Self::apply_postprocess_effects(&self.postprocess_effects, &mut renderer);

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

        // Let egui handle the event first
        #[cfg(feature = "egui")]
        {
            if let (Some(egui), Some(window)) = (&mut self.egui_context, &self.window) {
                if egui.handle_event(window, &event) {
                    return; // Event was consumed by egui
                }
            }
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
                #[cfg(target_arch = "wasm32")]
                self.try_finish_async_initialization();

                let frame = self.begin_frame();

                // --------- 1) Update scene logic first ----------
                self.run_update_stage(frame.dt());

                if let Some(mut renderer) = self.renderer.take() {
                    Self::run_gpu_systems(
                        &mut self.scene,
                        &mut self.gpu_systems,
                        &mut renderer,
                        frame.dt(),
                    );
                    self.render_scene(&mut renderer, &frame);
                    self.renderer = Some(renderer);
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
                    self.debug_print_hierarchy();
                }
                _ => {}
            },

            _ => {}
        }
    }
}
