use super::core::FrameStep;
use crate::environment::Environment;
use crate::renderer::{RenderFrame, RenderRegion, Renderer};
use crate::scene::{
    Children, MeshComponent, Name, Parent, Scene, SceneHandle, SceneWorkspace,
    SceneWorkspaceSceneMut, TransformComponent,
};
use crate::ui::{
    egui, EguiRenderTarget, EguiUiCallback, EnvironmentSettingsControls, EnvironmentSettingsHandle,
    EnvironmentWindow, FrameStatsHandle, FrameStatsHistory, PostProcessEffectsHandle,
    PostProcessWindow, SceneHierarchyHandle, SceneHierarchyRegistryHandle, SceneHierarchyState,
    SceneTabsHandle, SceneTabsState,
};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use winit::{event::WindowEvent, window::Window};

pub struct EditorState {
    egui_context: Option<crate::ui::EguiContext>,
    egui_pending_ui: Option<EguiUiCallback>,
    frame_stats: FrameStatsHandle,
    postprocess_effects: PostProcessEffectsHandle,
    environment_settings: EnvironmentSettingsHandle,
    scene_hierarchies: SceneHierarchyRegistryHandle,
    scene_tabs: SceneTabsHandle,
    render_region_query: Option<Box<dyn FnMut() -> Option<RenderRegion>>>,
    exit_requested: bool,
    active_scene_handle: Option<SceneHandle>,
}

impl EditorState {
    pub fn new(workspace: &SceneWorkspace) -> Self {
        let active_scene = workspace.active_scene_handle();
        let environment_source = workspace
            .active_scene()
            .map(|scene| scene.environment().clone())
            .unwrap_or_default();

        let scene_hierarchies = Arc::new(Mutex::new(HashMap::new()));
        let scene_tabs = SceneTabsState::handle();

        Self {
            egui_context: None,
            egui_pending_ui: None,
            frame_stats: FrameStatsHistory::handle(),
            postprocess_effects: PostProcessWindow::handle(),
            environment_settings: EnvironmentWindow::handle_from_environment(&environment_source),
            scene_hierarchies,
            scene_tabs,
            render_region_query: None,
            exit_requested: false,
            active_scene_handle: active_scene,
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

    pub fn scene_hierarchy_registry(&self) -> SceneHierarchyRegistryHandle {
        self.scene_hierarchies.clone()
    }

    pub fn scene_tabs_handle(&self) -> SceneTabsHandle {
        self.scene_tabs.clone()
    }

    pub fn scene_hierarchy_handle(&self) -> Option<SceneHierarchyHandle> {
        let active = self.active_scene_handle?;
        let Ok(mut registry) = self.scene_hierarchies.lock() else {
            return None;
        };
        let handle = registry
            .entry(active)
            .or_insert_with(SceneHierarchyState::handle)
            .clone();
        Some(handle)
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

    pub fn apply_environment_settings(&self, scene: &mut SceneWorkspaceSceneMut<'_>) {
        if let Ok(mut controls) = self.environment_settings.lock() {
            if controls.dirty {
                let mut environment = scene.environment().clone();
                controls.apply_to_environment(&mut environment);
                scene.set_environment(environment);
                scene.mark_dirty();
            }
            *controls = EnvironmentSettingsControls::from_environment(scene.environment());
        }
    }

    pub fn record_frame_stats(&self, frame: &FrameStep, renderer: &Renderer) {
        if let Ok(mut history) = self.frame_stats.lock() {
            history.record(frame.dt() as f32, renderer.last_frame_stats());
        }
    }

    pub fn refresh_scene_hierarchy(&mut self, workspace: &SceneWorkspace) {
        self.active_scene_handle = workspace.active_scene_handle();

        let open_handles: HashSet<_> = workspace.scene_handles().collect();
        let mut handle_to_refresh: Option<SceneHierarchyHandle> = None;

        if let Ok(mut registry) = self.scene_hierarchies.lock() {
            registry.retain(|handle, _| open_handles.contains(handle));

            if let Some(active) = self.active_scene_handle {
                let handle = registry
                    .entry(active)
                    .or_insert_with(SceneHierarchyState::handle)
                    .clone();
                handle_to_refresh = Some(handle);
            }
        }

        if let (Some(scene), Some(handle)) = (workspace.active_scene(), handle_to_refresh) {
            if let Ok(mut hierarchy) = handle.lock() {
                hierarchy.refresh_from_scene(scene);
            }
        }

        if let Ok(mut tabs) = self.scene_tabs.lock() {
            tabs.refresh_from_workspace(workspace);
        }
    }

    pub fn sync_environment_controls(&mut self, workspace: &SceneWorkspace) {
        self.active_scene_handle = workspace.active_scene_handle();
        if let Some(scene) = workspace.active_scene() {
            EnvironmentWindow::sync_handle(&self.environment_settings, scene.environment());
        } else {
            EnvironmentWindow::sync_handle(&self.environment_settings, &Environment::default());
        }
    }

    pub fn debug_print_hierarchy(workspace: &SceneWorkspace) {
        let Some(scene) = workspace.active_scene() else {
            log::info!("No active scene available for hierarchy dump");
            return;
        };
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
