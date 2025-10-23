use super::core::FrameStep;
use crate::renderer::{RenderFrame, RenderRegion, Renderer};
use crate::scene::{Children, MeshComponent, Name, Parent, Scene, TransformComponent};
use crate::ui::{
    egui, EguiRenderTarget, EguiUiCallback, EnvironmentSettingsControls, EnvironmentSettingsHandle,
    EnvironmentWindow, FrameStatsHandle, FrameStatsHistory, PostProcessEffectsHandle,
    PostProcessWindow, SceneHierarchyHandle, SceneHierarchyState,
};
use winit::{event::WindowEvent, window::Window};

pub struct EditorState {
    egui_context: Option<crate::ui::EguiContext>,
    egui_pending_ui: Option<EguiUiCallback>,
    frame_stats: FrameStatsHandle,
    postprocess_effects: PostProcessEffectsHandle,
    environment_settings: EnvironmentSettingsHandle,
    scene_hierarchy: SceneHierarchyHandle,
    render_region_query: Option<Box<dyn FnMut() -> Option<RenderRegion>>>,
    exit_requested: bool,
}

impl EditorState {
    pub fn new(scene: &Scene) -> Self {
        Self {
            egui_context: None,
            egui_pending_ui: None,
            frame_stats: FrameStatsHistory::handle(),
            postprocess_effects: PostProcessWindow::handle(),
            environment_settings: EnvironmentWindow::handle_from_environment(scene.environment()),
            scene_hierarchy: SceneHierarchyState::handle(),
            render_region_query: None,
            exit_requested: false,
        }
    }

    pub fn install_egui_context(&mut self, mut egui: crate::ui::EguiContext) {
        if let Some(callback) = self.egui_pending_ui.take() {
            egui.set_ui_box(callback);
        }
        self.egui_context = Some(egui);
    }

    pub fn set_egui_ui<F>(&mut self, callback: F)
    where
        F: FnMut(&egui::Context) + 'static,
    {
        if let Some(ctx) = &mut self.egui_context {
            ctx.set_ui(callback);
            self.egui_pending_ui = None;
        } else {
            self.egui_pending_ui = Some(Box::new(callback));
        }
    }

    pub fn set_render_region_query<F>(&mut self, query: F)
    where
        F: FnMut() -> Option<RenderRegion> + 'static,
    {
        self.render_region_query = Some(Box::new(query));
    }

    pub fn render_region(&mut self) -> Option<RenderRegion> {
        self.render_region_query.as_mut().and_then(|query| query())
    }

    pub fn frame_stats_handle(&self) -> FrameStatsHandle {
        self.frame_stats.clone()
    }

    pub fn postprocess_effects_handle(&self) -> PostProcessEffectsHandle {
        self.postprocess_effects.clone()
    }

    pub fn environment_settings_handle(&self) -> EnvironmentSettingsHandle {
        self.environment_settings.clone()
    }

    pub fn scene_hierarchy_handle(&self) -> SceneHierarchyHandle {
        self.scene_hierarchy.clone()
    }

    pub fn handle_window_event(&mut self, window: &Window, event: &WindowEvent) -> bool {
        if let Some(egui) = &mut self.egui_context {
            egui.handle_event(window, event)
        } else {
            false
        }
    }

    pub fn handle_device_event(&mut self, event: &winit::event::DeviceEvent) {
        if let Some(egui) = &mut self.egui_context {
            egui.handle_device_event(event);
        }
    }

    pub fn begin_ui_frame(&mut self, window: &Window) {
        if let Some(egui) = &mut self.egui_context {
            egui.begin_frame(window);
            egui.run_ui();
        }
    }

    pub fn end_ui_frame(&mut self, window: &Window) -> Option<egui::FullOutput> {
        let egui = self.egui_context.as_mut()?;
        let output = egui.end_frame(window);
        self.exit_requested = egui.take_should_close();
        Some(output)
    }

    pub fn render_ui(
        &mut self,
        renderer: &mut Renderer,
        window: &Window,
        frame: &mut RenderFrame,
        output: egui::FullOutput,
    ) {
        let Some(egui) = &mut self.egui_context else {
            return;
        };

        let view = frame
            .frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder =
            renderer
                .get_device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("egui_encoder"),
                });

        let surface_size = renderer.surface_size();
        let mut target = EguiRenderTarget {
            device: renderer.get_device(),
            queue: renderer.get_queue(),
            encoder: &mut encoder,
            window,
            view: &view,
            surface_size: [surface_size.width, surface_size.height],
        };
        egui.render(&mut target, output);

        renderer.get_queue().submit(Some(encoder.finish()));
    }

    pub fn take_exit_request(&mut self) -> bool {
        let requested = self.exit_requested;
        self.exit_requested = false;
        requested
    }

    pub fn apply_postprocess_effects(&self, renderer: &mut Renderer) {
        if let Ok(effects) = self.postprocess_effects.lock() {
            renderer.set_postprocess_effects(*effects);
        }
    }

    pub fn apply_environment_settings(&self, scene: &mut Scene) {
        if let Ok(mut controls) = self.environment_settings.lock() {
            if controls.dirty {
                let mut environment = scene.environment().clone();
                controls.apply_to_environment(&mut environment);
                scene.set_environment(environment);
            }
            *controls = EnvironmentSettingsControls::from_environment(scene.environment());
        }
    }

    pub fn record_frame_stats(&self, frame: &FrameStep, renderer: &Renderer) {
        if let Ok(mut history) = self.frame_stats.lock() {
            history.record(frame.dt() as f32, renderer.last_frame_stats());
        }
    }

    pub fn refresh_scene_hierarchy(&self, scene: &Scene) {
        if let Ok(mut hierarchy) = self.scene_hierarchy.lock() {
            hierarchy.refresh_from_scene(scene);
        }
    }

    pub fn sync_environment_controls(&self, scene: &Scene) {
        EnvironmentWindow::sync_handle(&self.environment_settings, scene.environment());
    }

    pub fn debug_print_hierarchy(scene: &Scene) {
        log::info!("=== Scene Hierarchy ===");

        let roots: Vec<_> = scene
            .world()
            .query::<()>()
            .without::<&Parent>()
            .iter()
            .map(|(e, _)| e)
            .collect();

        log::info!("Found {} root entities", roots.len());

        for root in roots {
            Self::debug_print_entity(scene, root, 0);
        }

        log::info!("======================");
    }

    fn debug_print_entity(scene: &Scene, entity: hecs::Entity, depth: usize) {
        let indent = "  ".repeat(depth);

        let name = scene
            .world()
            .get::<&Name>(entity)
            .map(|n| n.0.clone())
            .unwrap_or_else(|_| format!("{:?}", entity));

        let local_transform = scene
            .world()
            .get::<&TransformComponent>(entity)
            .map(|t| {
                format!(
                    "T:{:?} R:{:?} S:{:?}",
                    t.0.translation, t.0.rotation, t.0.scale
                )
            })
            .unwrap_or_else(|_| "No local transform".to_string());

        let world_transform = scene
            .world()
            .get::<&crate::scene::components::WorldTransform>(entity)
            .map(|t| format!("WorldT:{:?}", t.0.translation))
            .unwrap_or_else(|_| "No WorldTransform".to_string());

        let has_mesh = scene.world().get::<&MeshComponent>(entity).is_ok();

        log::info!(
            "{}└─ {} [{}]",
            indent,
            name,
            if has_mesh { "Mesh" } else { "Empty" },
        );
        log::info!("{}   Local: {}", indent, local_transform);
        log::info!("{}   {}", indent, world_transform);

        if let Ok(children) = scene.world().get::<&Children>(entity) {
            for child in &children.0 {
                Self::debug_print_entity(scene, *child, depth + 1);
            }
        }
    }
}
