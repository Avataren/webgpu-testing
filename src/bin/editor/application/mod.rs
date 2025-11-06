mod action_handlers;
mod asset_browser_system;
mod system;
pub(crate) use system::*;
mod camera_system;
mod core;
mod history_system;
mod input;
mod particle_system;
mod picking;
mod project_system;
mod runtime_mode;
mod scene_creation_system;
mod scene_tabs_panel;
mod script_editor_system;
mod shader_editor_system;
mod scripts;
mod selection_system;
mod setup;
mod ui_plugin_manager;

#[cfg(not(target_arch = "wasm32"))]
mod shader_watcher;
mod ui;
use self::asset_browser_system::AssetBrowserSystem;
#[allow(unused_imports)]
pub use self::core::EditorApplicationBuilder;
use self::core::GameViewDisplayMode;
pub use self::core::{EditorApplication, EditorSharedState};
use self::project_system::ProjectSystem;
use self::scene_tabs_panel::SceneTabAction;
use crate::asset_browser::AssetBrowserState;
use glam::{Quat, Vec3};
use hecs::Entity;
use std::path::PathBuf;
use wgpu_cube::app::{
    AppBuilder, GpuUpdateContext, RuntimeMode, RuntimeStateHandle, StartupContext, UpdateContext,
};
use wgpu_cube::asset::{
    Handle, MaterialAsset, Mesh,
};
use wgpu_cube::gpu_particles::ParticleEmitter;
use wgpu_cube::renderer::primitives::{quad_mesh, PrimitiveMeshDescriptor};
use wgpu_cube::renderer::{CustomRenderContext, CustomRenderStage, Material, RenderRegion};
use wgpu_cube::scene::components::{
    Billboard, BillboardOrientation, DepthState, PrimitiveMeshComponent,
};
use wgpu_cube::scene::workspace::SceneDocumentId;
use wgpu_cube::scene::{
    CameraComponent, CanCastShadow, DirectionalLight, EntityBuilder, EnvironmentComponent,
    MeshBounds, ParticleBehaviorPreset, ParticleEmitterComponent,
    ParticleSystemComponent, PointLight, Scene, SceneNodeId, ScenePrefabOverrides, ScenePrefabRef,
    SceneTreeAsset, SceneTreeAssetNode, SceneWorkspaceSceneMut, SpotLight, Transform,
};
use wgpu_cube::scripting::RuneScriptingPlugin;
use wgpu_cube::{
    DefaultUI, RenderApplication, SceneHierarchyEvent, SceneHierarchySceneDescriptor,
    ScenePrimitivePreset,
};

use crate::inspector::InspectorAction;
use crate::layout::{EditorBehavior, EditorPane};
use crate::postprocess::ViewportGrid;

impl RenderApplication for EditorApplication {
    fn name(&self) -> &str {
        "Engine Editor"
    }

    fn install_runtime_state_handle(&mut self, handle: RuntimeStateHandle) {
        self.set_runtime_state_handle(handle);
    }

    fn configure(&self, builder: &mut AppBuilder) {
        builder.add_plugin(RuneScriptingPlugin::new());
        builder.disable_default_textures();
        builder.disable_default_lighting();
        builder.disable_escape_exit();
    }

    fn setup(&mut self, ctx: &mut StartupContext) {
        Self::ensure_editor_scene_basics(&mut ctx.scene, ctx.renderer);
        // Don't load UI plugins at startup - wait until a project is opened/created
        // Self::load_ui_plugins(&mut self.shared, &mut ctx.scene);
        self.shared.mark_scene_hierarchy_dirty(ctx.scene_handle);
        self.initialize_history_state(&mut ctx.scene);
    }

    fn update(&mut self, ctx: &mut UpdateContext) {
        self.run_update_impl(ctx);
    }

    fn gpu_update(&mut self, ctx: &mut GpuUpdateContext) {
        self.run_gpu_update_impl(ctx);
    }

    fn custom_render(&mut self, ctx: &mut CustomRenderContext) {
        if !matches!(ctx.stage, CustomRenderStage::Shadow(_))
            && matches!(self.shared.runtime_state.active_mode(), RuntimeMode::Editor)
        {
            let grid = self
                .shared
                .viewports
                .grid_postprocess
                .get_or_insert_with(|| ViewportGrid::new(ctx.renderer.get_device()));
            grid.render(ctx);
        }

        self.particle_system_mut().render(ctx);
    }

    fn custom_render_stage(&self) -> CustomRenderStage {
        CustomRenderStage::AfterPostprocess
    }

    fn custom_render_includes_shadows(&self) -> bool {
        self.particle_system().has_shadow_casters()
    }

    fn ui(&mut self, ctx: &egui::Context, default_ui: &mut DefaultUI) {
        self.run_ui_impl(ctx, default_ui);
    }

    fn show_default_ui(&self) -> bool {
        false
    }

    fn render_region(&self) -> Option<RenderRegion> {
        self.render_region_for_mode(self.shared.runtime_state.active_mode())
    }
}

impl EditorApplication {
    fn run_update_impl(&mut self, ctx: &mut UpdateContext) {
        // DETECT MODE CHANGES FIRST - saves state BEFORE any animation changes
        let current_mode = ctx.runtime;
        if current_mode != self.shared.last_runtime_mode {
            self.detect_mode_transition(ctx, current_mode);
        }

        self.sync_active_scene_state(ctx);

        // Regular editor updates
        self.drain_update_commands(ctx);
        self.run_system_updates(ctx);
    }

    fn run_gpu_update_impl(&mut self, ctx: &mut GpuUpdateContext) {
        #[cfg(not(target_arch = "wasm32"))]
        self.process_shader_file_changes(ctx);

        let scene_changed = self.shared.active_scene_handle != Some(ctx.scene_handle);

        self.shared.set_active_scene_handle(ctx.scene_handle);
        self.selection_system_mut()
            .set_active_scene(ctx.scene_handle);

        if scene_changed {
            ctx.renderer.update_texture_bind_group(&ctx.scene.assets);
            self.shared.mark_scene_hierarchy_dirty(ctx.scene_handle);
        }

        // Load UI plugins if a project is open and plugins haven't been loaded yet
        if !self.shared.ui_plugins_loaded {
            let has_project = self.project_system().controller().current_dir().is_some();
            if has_project {
                Self::load_ui_plugins(&mut self.shared, &mut ctx.scene);
                self.shared.ui_plugins_loaded = true;

                // Initialize the newly loaded scripts by calling update(0.0) in editor mode
                // This ensures on_created() is called and script instances are created
                let is_editor_mode = matches!(self.shared.runtime_state.active_mode(), RuntimeMode::Editor);
                if is_editor_mode {
                    log::info!("Initializing UI plugin scripts in editor mode");
                    ctx.scene.update(0.0);
                }
            }
        }

        self.process_pending_mode_transition(ctx);

        self.process_pending_new_scenes(ctx);

        self.run_system_gpu_updates(ctx);
    }

    fn process_pending_new_scenes(&mut self, ctx: &mut GpuUpdateContext) {
        if self.shared.pending_new_scenes.is_empty() {
            return;
        }

        let requests = std::mem::take(&mut self.shared.pending_new_scenes);
        for _request in requests {
            let document_id = self.allocate_new_scene_id();
            let mut scene = Scene::new();
            Self::ensure_editor_scene_basics(&mut scene, ctx.renderer);

            ctx.request_open_scene(document_id.clone(), scene);
            ctx.request_active_scene(document_id);

            self.history_system_mut().reset();
            self.selection_system_mut().reset_workspace();
        }
    }

    fn allocate_new_scene_id(&mut self) -> String {
        loop {
            let index = self.shared.next_untitled_scene_index;
            self.shared.next_untitled_scene_index = index + 1;
            let candidate = if index == 1 {
                "untitled.scene".to_string()
            } else {
                format!("untitled_{index}.scene")
            };

            if self
                .scene_tabs()
                .iter()
                .all(|tab| tab.document_id != candidate)
            {
                return candidate;
            }
        }
    }

    fn sync_active_scene_state(&mut self, ctx: &mut UpdateContext) {
        let handle = ctx.scene_handle;
        let scene_changed = self.shared.active_scene_handle != Some(handle);
        self.shared.set_active_scene_handle(handle);

        {
            let selection = self.selection_system_mut();
            selection.set_active_scene(handle);
        }

        if scene_changed {
            self.shared.active_camera_entity = ctx.scene.active_camera_entity();
            self.shared.mark_scene_hierarchy_dirty(handle);
            let (selected, highlighted) = {
                let selection = self.selection_system();
                (selection.selected(), selection.highlighted())
            };
            self.history_system_mut()
                .initialize_state(&mut ctx.scene, selected, highlighted);

            match ctx.runtime {
                RuntimeMode::Playing => ctx.scene.set_animation_playback(true),
                RuntimeMode::Editor => {
                    ctx.scene.set_animation_playback(false);
                    ctx.scene.update(0.0);
                }
            }
        }

        // Feed back responses from the previous frame before collecting new commands
        if !self.shared.script_ui_responses.is_empty() {
            ctx.scene.set_ui_responses(std::mem::take(&mut self.shared.script_ui_responses));
        }

        // Collect UI commands from scripts that implement on_ui()
        // Pass editor_mode based on actual runtime state
        let editor_mode = matches!(self.shared.runtime_state.active_mode(), RuntimeMode::Editor);
        log::debug!(target: "editor_app", "Calling process_script_ui with editor_mode={}, runtime={:?}",
            editor_mode, ctx.runtime);
        self.shared.script_ui_commands = ctx.scene.process_script_ui(editor_mode);
    }

    fn run_ui_impl(&mut self, ctx: &egui::Context, default_ui: &mut DefaultUI) {
        if let Some(handle) = self.shared.active_scene_handle {
            self.selection_system_mut().set_active_scene(handle);
        }

        self.shared.viewports.scene_viewport.clear();
        self.shared.viewports.game_viewport.clear();

        self.project_system_mut().show_startup_dialog(ctx);

        if self.project_system().is_startup_dialog_visible() {
            return;
        }

        self.show_menu_bar(ctx);

        let active_mode = self.shared.runtime_state.active_mode();
        let is_playing = matches!(active_mode, RuntimeMode::Playing);
        let show_fullscreen_game = is_playing
            && matches!(
                self.shared.viewports.game_view_display,
                GameViewDisplayMode::Fullscreen
            );

        self.project_system_mut()
            .set_auxiliary_windows_enabled(!show_fullscreen_game);

        if !show_fullscreen_game {
            self.shared.windows.show(ctx, default_ui);
        }

        let content_root = self.project_system().content_root();
        let override_selection = { self.selection_system_mut().take_override() };
        let hierarchy_registry = default_ui.scene_hierarchy_registry();
        self.shared
            .set_scene_hierarchy_registry(hierarchy_registry.clone());
        let scene_tabs_handle = default_ui.scene_tabs_handle();
        self.shared.set_scene_tabs_handle(scene_tabs_handle);

        let tabs = self.scene_tabs();
        let active_tab_handle = tabs.iter().find(|tab| tab.active).map(|tab| tab.handle);

        let available_scene_docs: Vec<_> = self
            .project_system()
            .scene_documents()
            .iter()
            .map(|doc| {
                let display_name = if doc.name.is_empty() {
                    doc.id.clone()
                } else {
                    doc.name.clone()
                };
                SceneHierarchySceneDescriptor::new(doc.id.clone(), display_name)
            })
            .collect();

        let mut scene_tab_actions = if !show_fullscreen_game {
            self.scene_tabs_panel.show(&tabs, ctx)
        } else {
            Vec::new()
        };
        let (scene_hierarchy_window, log_window) = default_ui.scene_hierarchy_and_log_windows_mut();
        scene_hierarchy_window.set_available_scenes(available_scene_docs);
        match active_tab_handle {
            Some(scene_handle) => {
                if let Some(handle) = self.shared.scene_hierarchy_handle_for_scene(scene_handle) {
                    scene_hierarchy_window.set_active_scene(scene_handle, handle);
                } else {
                    scene_hierarchy_window.clear_active_scene();
                }
            }
            None => scene_hierarchy_window.clear_active_scene(),
        }

        let current_selection = self.selection_system().selected();
        scene_hierarchy_window.sync_selected_entity(current_selection);
        if let Some(selection) = override_selection {
            scene_hierarchy_window.set_selected_entity(selection);
        }
        let mut inspector_actions = Vec::new();
        let mut hierarchy_events = Vec::new();
        let transparent_frame =
            egui::Frame::central_panel(&ctx.style()).fill(egui::Color32::TRANSPARENT);
        {
            let (shared, asset_browser_state) = self.shared_and_asset_browser_state_mut();
            let dock_tree = &mut shared.dock_tree;
            let scene_viewport = &mut shared.viewports.scene_viewport;
            let game_viewport = &mut shared.viewports.game_viewport;
            let workspace_info = scene_hierarchy_window.workspace_info();

            egui::CentralPanel::default()
                .frame(transparent_frame)
                .show(ctx, |ui| {
                    if show_fullscreen_game {
                        crate::layout::show_fullscreen_viewport(ui, game_viewport);
                    } else {
                        let mut behavior = EditorBehavior {
                            scene_viewport,
                            game_viewport,
                            scene_hierarchy: scene_hierarchy_window,
                            log_window,
                            is_playing,
                            inspector_actions: &mut inspector_actions,
                            scene_events: &mut hierarchy_events,
                            asset_browser: asset_browser_state,
                            content_root,
                            workspace_info,
                        };
                        dock_tree.ui(&mut behavior, ui);
                    }
                });
        }

        let game_tile_active = self
            .find_pane_tile(EditorPane::GameViewport)
            .map(|id| self.shared.dock_tree.active_tiles().contains(&id))
            .unwrap_or(false);

        let scene_tile_active = self
            .find_pane_tile(EditorPane::SceneViewport)
            .map(|id| self.shared.dock_tree.active_tiles().contains(&id))
            .unwrap_or(false);

        let has_pending_transition = self.shared.pending_mode_transition.is_some();

        if !has_pending_transition
            && is_playing
            && !show_fullscreen_game
            && !game_tile_active
            && scene_tile_active
        {
            self.shared.runtime_state.request_mode(RuntimeMode::Editor);
        }

        if !has_pending_transition
            && !is_playing
            && !matches!(
                self.shared.runtime_state.desired_mode(),
                RuntimeMode::Playing
            )
            && game_tile_active
        {
            self.shared.runtime_state.request_mode(RuntimeMode::Playing);
        }

        self.selection_system_mut()
            .set_selected(scene_hierarchy_window.selected_entity());

        for action in inspector_actions {
            match action {
                InspectorAction::EditScript { entity, component } => {
                    self.script_editor_system_mut()
                        .open_script_editor(entity, component);
                }
                InspectorAction::EditShader {
                    entity,
                    handle,
                    metadata,
                } => {
                    self.shader_editor_system_mut()
                        .open_shader_editor(entity, handle, &metadata);
                }
                other => {
                    self.enqueue_command(EditorCommand::Inspector(other));
                }
            }
        }

        for event in hierarchy_events {
            match event {
                SceneHierarchyEvent::Create(action) => {
                    self.enqueue_command(EditorCommand::CreateScene(action));
                }
                SceneHierarchyEvent::OpenScene(document_id) => {
                    scene_tab_actions.push(SceneTabAction::Activate(document_id));
                }
                SceneHierarchyEvent::AddScriptToEntity(entity) => {
                    self.enqueue_command(EditorCommand::Inspector(InspectorAction::AddScript {
                        entity,
                    }));
                }
            }
        }

        for action in scene_tab_actions.drain(..) {
            self.handle_scene_tab_action(action);
        }

        if !is_playing {
            self.capture_viewport_pick_input(ctx);
            self.handle_history_shortcuts(ctx);
            self.handle_gizmo_shortcuts(ctx);
            self.handle_general_shortcuts(ctx);
        } else {
            self.selection_system_mut().clear_pending_pick();
        }

        self.script_editor_system_mut()
            .set_window_enabled(!show_fullscreen_game);
        self.shader_editor_system_mut()
            .set_window_enabled(!show_fullscreen_game);

        // Render script UI
        self.render_script_ui(ctx);

        self.run_system_ui(ctx, default_ui);
    }
}

impl EditorApplication {
    /// Render UI from scripts that implement on_ui()
    fn render_script_ui(&mut self, ctx: &egui::Context) {
        use std::collections::HashMap;
        use wgpu_cube::scripting::rune::api::ui::UiCommand;

        let mut all_responses: HashMap<hecs::Entity, HashMap<String, wgpu_cube::scripting::rune::api::ui::UiResponse>> = HashMap::new();

        // Render plugin manager window if we have plugins
        if let Some(ref mut manager) = self.shared.ui_plugin_manager {
            egui::Window::new("UI Plugins")
                .resizable(true)
                .default_size([300.0, 400.0])
                .show(ctx, |ui| {
                    ui.heading("Installed Plugins");
                    ui.separator();

                    let plugins = manager.plugins_mut();
                    for plugin in plugins {
                        if !plugin.metadata.enabled {
                            continue;
                        }

                        ui.horizontal(|ui| {
                            let was_visible = plugin.visible;
                            if ui.checkbox(&mut plugin.visible, &plugin.metadata.name).changed() {
                                log::info!(
                                    "Plugin '{}' visibility changed: {} -> {}",
                                    plugin.metadata.name,
                                    was_visible,
                                    plugin.visible
                                );
                            }
                            ui.label(&plugin.metadata.category);
                        });

                        ui.label(format!("  {}", plugin.metadata.description));
                        ui.add_space(4.0);
                    }
                });

            // Render each plugin's UI in a separate window (only if visible)
            for (entity, commands) in &self.shared.script_ui_commands {
                // Check if this entity is a plugin and if it's visible
                if let Some(plugin) = manager.get_plugin(*entity) {
                    if !plugin.visible {
                        continue;
                    }

                    let mut responses = HashMap::new();
                    let window_title = plugin.metadata.name.clone();

                    egui::Window::new(window_title)
                        .resizable(true)
                        .default_size([500.0, 400.0])
                        .collapsible(true)
                        .show(ctx, |ui| {
                            for command in commands {
                                if let Some(response) = command.render(ui) {
                                    // Extract the ID from the command and store the response
                                    let id = match command {
                                        UiCommand::Button { text } => format!("button_{}", text),
                                        UiCommand::TextEdit { id, .. } => id.clone(),
                                        UiCommand::TextEditMultiline { id, .. } => id.clone(),
                                        UiCommand::Slider { id, .. } => id.clone(),
                                        UiCommand::DragValue { id, .. } => id.clone(),
                                        UiCommand::Checkbox { id, .. } => id.clone(),
                                        UiCommand::ColorEdit { id, .. } => id.clone(),
                                        _ => continue,
                                    };
                                    responses.insert(id, response);
                                }
                            }
                        });

                    if !responses.is_empty() {
                        all_responses.insert(*entity, responses);
                    }
                } else {
                    // Not a plugin, render with default title
                    let mut responses = HashMap::new();

                    egui::Window::new(format!("Script UI - Entity {:?}", entity))
                        .resizable(true)
                        .default_size([300.0, 200.0])
                        .show(ctx, |ui| {
                            for command in commands {
                                if let Some(response) = command.render(ui) {
                                    // Extract the ID from the command and store the response
                                    let id = match command {
                                        UiCommand::Button { text } => format!("button_{}", text),
                                        UiCommand::TextEdit { id, .. } => id.clone(),
                                        UiCommand::TextEditMultiline { id, .. } => id.clone(),
                                        UiCommand::Slider { id, .. } => id.clone(),
                                        UiCommand::DragValue { id, .. } => id.clone(),
                                        UiCommand::Checkbox { id, .. } => id.clone(),
                                        UiCommand::ColorEdit { id, .. } => id.clone(),
                                        _ => continue,
                                    };
                                    responses.insert(id, response);
                                }
                            }
                        });

                    if !responses.is_empty() {
                        all_responses.insert(*entity, responses);
                    }
                }
            }
        } else {
            // No plugin manager, render all script UIs with default titles
            for (entity, commands) in &self.shared.script_ui_commands {
                let mut responses = HashMap::new();

                egui::Window::new(format!("Script UI - Entity {:?}", entity))
                    .resizable(true)
                    .default_size([300.0, 200.0])
                    .show(ctx, |ui| {
                        for command in commands {
                            if let Some(response) = command.render(ui) {
                                // Extract the ID from the command and store the response
                                let id = match command {
                                    UiCommand::Button { text } => format!("button_{}", text),
                                    UiCommand::TextEdit { id, .. } => id.clone(),
                                    UiCommand::Slider { id, .. } => id.clone(),
                                    UiCommand::DragValue { id, .. } => id.clone(),
                                    UiCommand::Checkbox { id, .. } => id.clone(),
                                    UiCommand::ColorEdit { id, .. } => id.clone(),
                                    _ => continue,
                                };
                                responses.insert(id, response);
                            }
                        }
                    });

                if !responses.is_empty() {
                    all_responses.insert(*entity, responses);
                }
            }
        }

        // Store responses for the next frame
        self.shared.script_ui_responses = all_responses;
    }

    fn handle_scene_tab_action(&mut self, action: SceneTabAction) {
        match action {
            SceneTabAction::Activate(document_id) => {
                self.shared.runtime_state.request_mode(RuntimeMode::Editor);
                self.enqueue_command(EditorCommand::ActivateScene(document_id));
            }
            SceneTabAction::Close(document_id) => {
                self.enqueue_command(EditorCommand::CloseScene(document_id));
            }
            SceneTabAction::NewScene => {
                self.shared.runtime_state.request_mode(RuntimeMode::Editor);
                self.enqueue_command(EditorCommand::NewScene);
            }
        }
    }

    fn shared_and_asset_browser_state_mut(
        &mut self,
    ) -> (&mut EditorSharedState, &mut AssetBrowserState) {
        let asset_browser_index = self.asset_browser_system_index;
        let (_, tail) = self.systems.split_at_mut(asset_browser_index);
        let (asset_browser_system, _) = tail
            .split_first_mut()
            .expect("asset browser system registered");
        let asset_browser_system = asset_browser_system
            .as_any_mut()
            .downcast_mut::<AssetBrowserSystem>()
            .expect("asset browser system registered");

        (&mut self.shared, asset_browser_system.state_mut())
    }

    fn apply_pending_inspector_actions(
        &mut self,
        ctx: &mut UpdateContext,
        actions: Vec<InspectorAction>,
    ) {
        if actions.is_empty() {
            return;
        }

        self.resolve_active_camera_entity(&mut ctx.scene);
        let mut transforms_changed = false;

        // Route all actions through the dispatcher
        use action_handlers::dispatch_action;

        for action in actions {
            // SAFETY: We need to use unsafe here to work around a limitation in the borrow checker.
            // The borrow checker thinks that the mutable borrow of ctx.scene from the previous
            // iteration is still active, but in reality each iteration's borrows are completely
            // independent and don't overlap. We use raw pointers to explicitly control the borrow
            // lifetime and make it clear to the compiler that each call to dispatch_action is
            // independent.
            let result = unsafe {
                let scene_ptr: *mut SceneWorkspaceSceneMut = &mut ctx.scene;
                let app_ptr: *mut EditorApplication = self as *mut _;
                dispatch_action(&mut *scene_ptr, &mut *app_ptr, action)
            };

            if result.transforms_changed {
                transforms_changed = true;
            }
        }

        /* OLD IMPLEMENTATION REMOVED - kept for reference during transition
        for action in actions {
            match action {
                InspectorAction::UpdateTransform { entity, transform } => {
                    let mut updated = false;
                    {
                        let world = ctx.scene.main_world_mut();
                        match world.get::<&mut TransformComponent>(entity) {
                            Ok(mut component) => {
                                component.0 = transform;
                                updated = true;
                            }
                            Err(err) => {
                                log::warn!("Failed to update transform for {:?}: {}", entity, err);
                            }
                        }
                    }

                    if updated {
                        transforms_changed = true;
                        self.record_scene_change(&mut ctx.scene);
                    }
                }
                InspectorAction::UpdateCamera { entity, component } => {
                    let mut updated = false;
                    {
                        let world = ctx.scene.main_world_mut();
                        match world.get::<&mut CameraComponent>(entity) {
                            Ok(mut existing) => {
                                if *existing != component {
                                    *existing = component;
                                    updated = true;
                                }
                            }
                            Err(err) => {
                                log::warn!("Failed to update camera for {:?}: {}", entity, err);
                            }
                        }
                    }

                    if updated {
                        if self.shared.active_camera_entity.is_none()
                            || self.shared.active_camera_entity == Some(entity)
                        {
                            ctx.scene.set_active_camera_entity(Some(entity));
                            self.shared.active_camera_entity = ctx.scene.active_camera_entity();
                        }

                        self.record_scene_change(&mut ctx.scene);
                    }
                }
                InspectorAction::UpdateMaterial {
                    entity,
                    handle,
                    material,
                } => {
                    let mut can_update = false;
                    {
                        let world = ctx.scene.main_world_mut();
                        match world.get::<&MaterialComponent>(entity) {
                            Ok(component) => {
                                if component.0 == handle {
                                    can_update = true;
                                } else {
                                    log::warn!(
                                        "Inspector material handle mismatch for {:?}: {:?} != {:?}",
                                        entity,
                                        component.0.index(),
                                        handle.index()
                                    );
                                }
                            }
                            Err(err) => {
                                log::warn!("Failed to access material for {:?}: {}", entity, err);
                            }
                        }
                    }

                    if can_update {
                        if let Some(asset) = ctx.scene.assets.material_mut(handle) {
                            *asset.material_mut() = material;
                            self.record_scene_change(&mut ctx.scene);
                        } else {
                            log::warn!(
                                "Inspector material asset missing for handle {:?}",
                                handle.index()
                            );
                        }
                    }
                }
                InspectorAction::SetMaterialKind {
                    entity,
                    handle,
                    kind,
                } => {
                    let mut can_update = false;
                    {
                        let world = ctx.scene.main_world_mut();
                        match world.get::<&MaterialComponent>(entity) {
                            Ok(component) => {
                                if component.0 == handle {
                                    can_update = true;
                                } else {
                                    log::warn!(
                                        "Inspector material handle mismatch for {:?}: {:?} != {:?}",
                                        entity,
                                        component.0.index(),
                                        handle.index()
                                    );
                                }
                            }
                            Err(err) => {
                                log::warn!("Failed to access material for {:?}: {}", entity, err);
                            }
                        }
                    }

                    if !can_update {
                        continue;
                    }

                    if let Some(asset) = ctx.scene.assets.material_mut(handle) {
                        asset.set_kind(kind.clone());
                        match kind {
                            MaterialKind::Shader(_) => {
                                asset.set_asset_type(AssetTypeTag::new("shader_material"));
                            }
                            MaterialKind::Pbr => {
                                asset.set_asset_type(AssetTypeTag::default());
                            }
                        }
                        self.record_scene_change(&mut ctx.scene);
                    } else {
                        log::warn!(
                            "Inspector material asset missing for handle {:?}",
                            handle.index()
                        );
                    }
                }
                InspectorAction::AssignShaderSource {
                    entity,
                    handle,
                    shader_path,
                } => {
                    let mut can_update = false;
                    {
                        let world = ctx.scene.main_world_mut();
                        match world.get::<&MaterialComponent>(entity) {
                            Ok(component) => {
                                if component.0 == handle {
                                    can_update = true;
                                } else {
                                    log::warn!(
                                        "Inspector material handle mismatch for {:?}: {:?} != {:?}",
                                        entity,
                                        component.0.index(),
                                        handle.index()
                                    );
                                }
                            }
                            Err(err) => {
                                log::warn!("Failed to access material for {:?}: {}", entity, err);
                            }
                        }
                    }

                    if !can_update {
                        continue;
                    }

                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        let Some(content_root) = self.project_system().content_root() else {
                            log::warn!(
                                "Cannot assign shader without an open project for handle {:?}",
                                handle.index()
                            );
                            continue;
                        };

                        let canonical = shader_path.canonicalize().unwrap_or(shader_path.clone());

                        if !canonical.starts_with(&content_root) {
                            log::warn!(
                                "Shader {:?} is outside of the project content directory",
                                canonical
                            );
                            continue;
                        }

                        let source = match fs::read_to_string(&canonical) {
                            Ok(contents) => contents,
                            Err(err) => {
                                log::warn!("Failed to read shader source {:?}: {}", canonical, err);
                                continue;
                            }
                        };

                        if let Some(asset) = ctx.scene.assets.material_mut(handle) {
                            let mut metadata = match asset.kind().clone() {
                                MaterialKind::Shader(existing) => existing,
                                MaterialKind::Pbr => ShaderMaterialMetadata::default_template(),
                            };
                            metadata.set_wgsl_source(source);
                            metadata.set_source_path(Some(canonical));
                            asset.set_kind(MaterialKind::Shader(metadata));
                            asset.set_asset_type(AssetTypeTag::new("shader_material"));
                            self.record_scene_change(&mut ctx.scene);
                        } else {
                            log::warn!(
                                "Inspector material asset missing for handle {:?}",
                                handle.index()
                            );
                        }
                    }
                    #[cfg(target_arch = "wasm32")]
                    {
                        let _ = (entity, handle, shader_path);
                        log::warn!("Assigning shader files is unavailable on WebAssembly builds.");
                    }
                }
                InspectorAction::CreateShaderSource {
                    entity,
                    handle,
                    suggested_stem,
                } => {
                    let mut can_update = false;
                    {
                        let world = ctx.scene.main_world_mut();
                        match world.get::<&MaterialComponent>(entity) {
                            Ok(component) => {
                                if component.0 == handle {
                                    can_update = true;
                                } else {
                                    log::warn!(
                                        "Inspector material handle mismatch for {:?}: {:?} != {:?}",
                                        entity,
                                        component.0.index(),
                                        handle.index()
                                    );
                                }
                            }
                            Err(err) => {
                                log::warn!("Failed to access material for {:?}: {}", entity, err);
                            }
                        }
                    }

                    if !can_update {
                        continue;
                    }

                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        let Some(content_root) = self.project_system().content_root() else {
                            log::warn!(
                                "Cannot create shader without an open project for handle {:?}",
                                handle.index()
                            );
                            continue;
                        };

                        let shader_dir = content_root.join("shaders");
                        if let Err(err) = fs::create_dir_all(&shader_dir) {
                            log::warn!(
                                "Failed to create shader directory {:?}: {}",
                                shader_dir,
                                err
                            );
                            continue;
                        }

                        let base_stem = if suggested_stem.trim().is_empty() {
                            format!("material_{}", handle.index())
                        } else {
                            suggested_stem.clone()
                        };

                        let mut candidate = shader_dir.join(format!("{base_stem}.wgsl"));
                        let mut counter = 1usize;
                        while candidate.exists() {
                            candidate = shader_dir.join(format!("{base_stem}_{counter:02}.wgsl"));
                            counter += 1;
                        }

                        if let Err(err) =
                            fs::write(&candidate, ShaderMaterialMetadata::DEFAULT_TEMPLATE)
                        {
                            log::warn!("Failed to write shader template {:?}: {}", candidate, err);
                            continue;
                        }

                        let canonical = candidate.canonicalize().unwrap_or(candidate.clone());

                        if let Some(asset) = ctx.scene.assets.material_mut(handle) {
                            let mut metadata = match asset.kind().clone() {
                                MaterialKind::Shader(existing) => existing,
                                MaterialKind::Pbr => ShaderMaterialMetadata::default_template(),
                            };
                            metadata.set_wgsl_source(ShaderMaterialMetadata::DEFAULT_TEMPLATE);
                            metadata.set_source_path(Some(canonical));
                            asset.set_kind(MaterialKind::Shader(metadata));
                            asset.set_asset_type(AssetTypeTag::new("shader_material"));
                            self.record_scene_change(&mut ctx.scene);
                        } else {
                            log::warn!(
                                "Inspector material asset missing for handle {:?}",
                                handle.index()
                            );
                        }
                    }
                    #[cfg(target_arch = "wasm32")]
                    {
                        let _ = (entity, handle, suggested_stem);
                        log::warn!("Creating shader files is unavailable on WebAssembly builds.");
                    }
                }
                InspectorAction::CreateShaderMaterial { entity, source } => {
                    let Some(source_asset) = ctx.scene.assets.material(source).cloned() else {
                        log::warn!(
                            "Inspector shader material source missing for handle {:?}",
                            source.index()
                        );
                        continue;
                    };

                    let mut shader_asset = source_asset;
                    shader_asset.set_kind(MaterialKind::Shader(
                        ShaderMaterialMetadata::default_template(),
                    ));
                    shader_asset.set_asset_type(AssetTypeTag::new("shader_material"));
                    shader_asset.set_canonical_path(PathBuf::new());

                    let new_handle = ctx.scene.assets.insert_material_asset(shader_asset);

                    match ctx
                        .scene
                        .main_world_mut()
                        .insert_one(entity, MaterialComponent(new_handle))
                    {
                        Ok(_) => {
                            self.record_scene_change(&mut ctx.scene);
                        }
                        Err(err) => {
                            log::warn!("Failed to assign shader material to {:?}: {}", entity, err);
                        }
                    }
                }
                InspectorAction::UpdatePointLight { entity, light } => {
                    let mut updated = false;
                    {
                        let world = ctx.scene.main_world_mut();
                        match world.get::<&mut PointLight>(entity) {
                            Ok(mut component) => {
                                *component = light;
                                updated = true;
                            }
                            Err(err) => {
                                log::warn!(
                                    "Failed to update point light for {:?}: {}",
                                    entity,
                                    err
                                );
                            }
                        }
                    }

                    if updated {
                        self.record_scene_change(&mut ctx.scene);
                    }
                }
                InspectorAction::UpdateDirectionalLight { entity, light } => {
                    let mut updated = false;
                    {
                        let world = ctx.scene.main_world_mut();
                        match world.get::<&mut DirectionalLight>(entity) {
                            Ok(mut component) => {
                                *component = light;
                                updated = true;
                            }
                            Err(err) => {
                                log::warn!(
                                    "Failed to update directional light for {:?}: {}",
                                    entity,
                                    err
                                );
                            }
                        }
                    }

                    if updated {
                        self.record_scene_change(&mut ctx.scene);
                    }
                }
                InspectorAction::UpdateSpotLight { entity, light } => {
                    let mut updated = false;
                    {
                        let world = ctx.scene.main_world_mut();
                        match world.get::<&mut SpotLight>(entity) {
                            Ok(mut component) => {
                                *component = light;
                                updated = true;
                            }
                            Err(err) => {
                                log::warn!("Failed to update spot light for {:?}: {}", entity, err);
                            }
                        }
                    }

                    if updated {
                        self.record_scene_change(&mut ctx.scene);
                    }
                }
                InspectorAction::UpdateEnvironment { entity, component } => {
                    let previous = ctx
                        .scene
                        .main_world()
                        .get::<&EnvironmentComponent>(entity)
                        .ok()
                        .map(|existing| EnvironmentComponent::clone(&*existing));

                    let mut component = component;
                    let should_enable_hdr = {
                        let new_path = component
                            .hdr
                            .as_ref()
                            .and_then(|hdr| hdr.path.as_ref())
                            .map(|path| path.as_path());
                        let previous_path = previous
                            .as_ref()
                            .and_then(|prev| prev.hdr.as_ref())
                            .and_then(|hdr| hdr.path.as_ref())
                            .map(|path| path.as_path());

                        match (previous_path, new_path) {
                            (Some(prev), Some(new)) => prev != new,
                            (None, Some(_)) => true,
                            _ => false,
                        }
                    };

                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        if let Err(err) =
                            self.copy_environment_asset_if_needed(&mut component, previous.as_ref())
                        {
                            log::warn!("{err}");
                            self.asset_browser_state_mut().report_error(err);
                        }
                    }

                    if let Some(hdr) = component.hdr.as_mut() {
                        if should_enable_hdr && hdr.path.is_some() {
                            hdr.enabled = true;
                        }
                    }

                    let mut updated = false;
                    {
                        let world = ctx.scene.main_world_mut();
                        match world.get::<&mut EnvironmentComponent>(entity) {
                            Ok(mut existing) => {
                                if *existing != component {
                                    *existing = component.clone();
                                    updated = true;
                                }
                            }
                            Err(err) => {
                                log::warn!(
                                    "Failed to update environment for {:?}: {}",
                                    entity,
                                    err
                                );
                            }
                        }
                    }

                    if updated {
                        ctx.scene.set_environment(component.to_environment());
                        self.record_scene_change(&mut ctx.scene);
                    }
                }
                InspectorAction::UpdateParticleSystem { entity, component } => {
                    let mut updated = false;
                    let new_spawn_rate = component.spawn_rate;
                    let mut spawn_rate_changed = false;
                    {
                        let world = ctx.scene.main_world_mut();
                        match world.get::<&mut ParticleSystemComponent>(entity) {
                            Ok(mut existing) => {
                                spawn_rate_changed =
                                    (existing.spawn_rate - new_spawn_rate).abs() > f32::EPSILON;
                                if *existing != component {
                                    *existing = component;
                                    updated = true;
                                }
                            }
                            Err(err) => {
                                log::warn!(
                                    "Failed to update particle system for {:?}: {}",
                                    entity,
                                    err
                                );
                            }
                        }

                        if spawn_rate_changed {
                            match world.get::<&mut ParticleEmitterComponent>(entity) {
                                Ok(mut emitter) => {
                                    if (emitter.spawn_rate - new_spawn_rate).abs() > f32::EPSILON {
                                        emitter.spawn_rate = new_spawn_rate;
                                        updated = true;
                                    }
                                }
                                Err(err) => {
                                    log::warn!(
                                        "Failed to update particle emitter spawn rate for {:?}: {}",
                                        entity,
                                        err
                                    );
                                }
                            }
                        }
                    }

                    if updated {
                        self.record_scene_change(&mut ctx.scene);
                    }
                }
                InspectorAction::UpdateParticleEmitter { entity, component } => {
                    let mut updated = false;
                    {
                        let world = ctx.scene.main_world_mut();
                        match world.get::<&mut ParticleEmitterComponent>(entity) {
                            Ok(mut existing) => {
                                if *existing != component {
                                    *existing = component;
                                    updated = true;
                                }
                            }
                            Err(err) => {
                                log::warn!(
                                    "Failed to update particle emitter for {:?}: {}",
                                    entity,
                                    err
                                );
                            }
                        }
                    }

                    if updated {
                        self.record_scene_change(&mut ctx.scene);
                    }
                }
                InspectorAction::UpdateParticleBehavior {
                    entity,
                    behavior,
                    config,
                } => {
                    let mut updated = false;
                    {
                        let world = ctx.scene.main_world_mut();
                        match world.get::<&mut ParticleSystemComponent>(entity) {
                            Ok(mut existing) => {
                                let config = config.ensure_variant(behavior);
                                if existing.behavior != behavior
                                    || existing.behavior_config != config
                                {
                                    existing.behavior = behavior;
                                    existing.behavior_config = config;
                                    updated = true;
                                }
                            }
                            Err(err) => {
                                log::warn!(
                                    "Failed to update particle behavior for {:?}: {}",
                                    entity,
                                    err
                                );
                            }
                        }
                    }

                    if updated {
                        self.record_scene_change(&mut ctx.scene);
                    }
                }
                InspectorAction::SetBillboard { entity, billboard } => {
                    let mut updated = false;
                    {
                        let world = ctx.scene.main_world_mut();
                        match billboard {
                            Some(component) => {
                                let mut needs_insert = false;
                                match world.get::<&mut Billboard>(entity) {
                                    Ok(mut existing) => {
                                        *existing = component;
                                        updated = true;
                                    }
                                    Err(err) => {
                                        log::debug!(
                                            "Billboard missing for {:?} while enabling: {}",
                                            entity,
                                            err
                                        );
                                        needs_insert = true;
                                    }
                                }

                                if needs_insert {
                                    match world.insert(entity, (component,)) {
                                        Ok(_) => {
                                            updated = true;
                                        }
                                        Err(insert_err) => {
                                            log::warn!(
                                                "Failed to add Billboard to {:?}: {}",
                                                entity,
                                                insert_err
                                            );
                                        }
                                    }
                                }
                            }
                            None => match world.remove_one::<Billboard>(entity) {
                                Ok(_) => {
                                    updated = true;
                                }
                                Err(err) => {
                                    log::debug!(
                                        "Billboard missing for {:?} while disabling: {}",
                                        entity,
                                        err
                                    );
                                }
                            },
                        }
                    }

                    if updated {
                        self.record_scene_change(&mut ctx.scene);
                    }
                }
                InspectorAction::SetCanCastShadow {
                    entity,
                    casts_shadow,
                } => {
                    let mut updated = false;
                    {
                        let world = ctx.scene.main_world_mut();
                        let mut needs_insert = false;
                        match world.get::<&mut CanCastShadow>(entity) {
                            Ok(mut component) => {
                                if component.0 != casts_shadow {
                                    component.0 = casts_shadow;
                                    updated = true;
                                }
                            }
                            Err(err) => {
                                if casts_shadow {
                                    needs_insert = true;
                                } else {
                                    log::debug!(
                                        "CanCastShadow missing for {:?} while disabling shadows: {}",
                                        entity,
                                        err
                                    );
                                }
                            }
                        }

                        if needs_insert {
                            match world.insert(entity, (CanCastShadow(true),)) {
                                Ok(_) => {
                                    updated = true;
                                }
                                Err(insert_err) => {
                                    log::warn!(
                                        "Failed to add CanCastShadow to {:?}: {}",
                                        entity,
                                        insert_err
                                    );
                                }
                            }
                        }
                    }

                    if updated {
                        self.record_scene_change(&mut ctx.scene);
                    }
                }
                InspectorAction::AddScript { entity } => {
                    let mut updated = false;
                    {
                        let world = ctx.scene.main_world_mut();
                        // Check if entity already has a script component
                        if world.get::<&RuneScriptComponent>(entity).is_ok() {
                            log::warn!("Entity {:?} already has a script component", entity);
                        } else {
                            // Create a default inline script
                            let default_script = RuneScriptComponent::new(RuneScriptSource::inline(
                                "new_script",
                                "// New script\n\npub fn on_update(dt) {\n    // Your code here\n}\n",
                            ));
                            match world.insert(entity, (default_script,)) {
                                Ok(_) => {
                                    updated = true;
                                    log::info!("Added script component to entity {:?}", entity);
                                }
                                Err(err) => {
                                    log::warn!(
                                        "Failed to add script component to {:?}: {}",
                                        entity,
                                        err
                                    );
                                }
                            }
                        }
                    }

                    if updated {
                        self.record_scene_change(&mut ctx.scene);
                    }
                }
                InspectorAction::ChangeScriptSource {
                    entity,
                    script_path,
                } => {
                    let mut updated = false;
                    {
                        let world = ctx.scene.main_world_mut();
                        // Check if entity has a script component
                        if let Ok(mut script) = world.get::<&mut RuneScriptComponent>(entity) {
                            // Create new script source from file
                            let new_source = RuneScriptSource::File { path: script_path.clone() };
                            *script = RuneScriptComponent::new(new_source);
                            updated = true;
                            log::info!(
                                "Changed script source for entity {:?} to {:?}",
                                entity,
                                script_path
                            );
                        } else {
                            log::warn!("Entity {:?} does not have a script component", entity);
                        }
                    }

                    if updated {
                        self.record_scene_change(&mut ctx.scene);
                        // Reload the script runtime to pick up the new script
                        ctx.scene.reset_script_runtime();
                    }
                }
                InspectorAction::AddCamera { entity } => {
                    let world = ctx.scene.main_world_mut();
                    if world.get::<&CameraComponent>(entity).is_ok() {
                        log::warn!("Entity {:?} already has a camera component", entity);
                    } else {
                        let camera = CameraComponent::default();
                        match world.insert(entity, (camera,)) {
                            Ok(_) => {
                                log::info!("Added camera component to entity {:?}", entity);
                                self.record_scene_change(&mut ctx.scene);
                            }
                            Err(err) => {
                                log::warn!("Failed to add camera component to {:?}: {}", entity, err);
                            }
                        }
                    }
                }
                InspectorAction::AddMesh { entity } => {
                    // Check if entity already has a mesh component
                    let has_mesh = {
                        let world = ctx.scene.main_world_mut();
                        world.get::<&MeshComponent>(entity).is_ok()
                    };

                    if has_mesh {
                        log::warn!("Entity {:?} already has a mesh component", entity);
                    } else {
                        // Try to use existing cube mesh (created during setup or previous usage)
                        let descriptor = PrimitiveMeshDescriptor::from(ScenePrimitivePreset::Cube);
                        if let Some(mesh_handle) = ctx.scene.assets.primitive_mesh_handle(descriptor) {
                            let material_handle = ctx
                                .scene
                                .assets
                                .materials
                                .insert(MaterialAsset::from_material(
                                    Material::pbr(),
                                    PathBuf::new(),
                                ));
                            let world = ctx.scene.main_world_mut();
                            match world.insert(entity, (MeshComponent(mesh_handle), MaterialComponent(material_handle))) {
                                Ok(_) => {
                                    log::info!("Added mesh component to entity {:?}", entity);
                                    self.record_scene_change(&mut ctx.scene);
                                }
                                Err(err) => {
                                    log::warn!("Failed to add mesh component to {:?}: {}", entity, err);
                                }
                            }
                        } else {
                            log::warn!("Cannot add mesh component: cube primitive not yet created. Try creating a cube primitive first.");
                        }
                    }
                }
                InspectorAction::AddPointLight { entity } => {
                    let world = ctx.scene.main_world_mut();
                    if world.get::<&PointLight>(entity).is_ok() {
                        log::warn!("Entity {:?} already has a point light component", entity);
                    } else {
                        let light = PointLight::default();
                        match world.insert(entity, (light,)) {
                            Ok(_) => {
                                log::info!("Added point light component to entity {:?}", entity);
                                self.record_scene_change(&mut ctx.scene);
                            }
                            Err(err) => {
                                log::warn!("Failed to add point light component to {:?}: {}", entity, err);
                            }
                        }
                    }
                }
                InspectorAction::AddDirectionalLight { entity } => {
                    let world = ctx.scene.main_world_mut();
                    if world.get::<&DirectionalLight>(entity).is_ok() {
                        log::warn!("Entity {:?} already has a directional light component", entity);
                    } else {
                        let light = DirectionalLight::default();
                        match world.insert(entity, (light,)) {
                            Ok(_) => {
                                log::info!("Added directional light component to entity {:?}", entity);
                                self.record_scene_change(&mut ctx.scene);
                            }
                            Err(err) => {
                                log::warn!("Failed to add directional light component to {:?}: {}", entity, err);
                            }
                        }
                    }
                }
                InspectorAction::AddSpotLight { entity } => {
                    let world = ctx.scene.main_world_mut();
                    if world.get::<&SpotLight>(entity).is_ok() {
                        log::warn!("Entity {:?} already has a spot light component", entity);
                    } else {
                        let light = SpotLight::default();
                        match world.insert(entity, (light,)) {
                            Ok(_) => {
                                log::info!("Added spot light component to entity {:?}", entity);
                                self.record_scene_change(&mut ctx.scene);
                            }
                            Err(err) => {
                                log::warn!("Failed to add spot light component to {:?}: {}", entity, err);
                            }
                        }
                    }
                }
                InspectorAction::AddEnvironment { entity } => {
                    // Check if entity already has an environment component
                    let has_environment = {
                        let world = ctx.scene.main_world_mut();
                        world.get::<&EnvironmentComponent>(entity).is_ok()
                    };

                    if has_environment {
                        log::warn!("Entity {:?} already has an environment component", entity);
                    } else {
                        let component = EnvironmentComponent::from_environment(ctx.scene.environment());
                        {
                            let world = ctx.scene.main_world_mut();
                            match world.insert(entity, (component.clone(),)) {
                                Ok(_) => {
                                    log::info!("Added environment component to entity {:?}", entity);
                                }
                                Err(err) => {
                                    log::warn!("Failed to add environment component to {:?}: {}", entity, err);
                                    return;
                                }
                            }
                        }
                        ctx.scene.set_environment(component.to_environment());
                        self.record_scene_change(&mut ctx.scene);
                    }
                }
                InspectorAction::AddParticleSystem { entity } => {
                    let world = ctx.scene.main_world_mut();
                    if world.get::<&ParticleSystemComponent>(entity).is_ok() {
                        log::warn!("Entity {:?} already has a particle system component", entity);
                    } else {
                        let component = ParticleSystemComponent::default();
                        let emitter = ParticleEmitterComponent::default();
                        match world.insert(entity, (component, emitter)) {
                            Ok(_) => {
                                log::info!("Added particle system component to entity {:?}", entity);
                                self.record_scene_change(&mut ctx.scene);
                            }
                            Err(err) => {
                                log::warn!("Failed to add particle system component to {:?}: {}", entity, err);
                            }
                        }
                    }
                }
                InspectorAction::RenameEntity { entity, new_name } => {
                    let renamed = {
                        let world = ctx.scene.main_world_mut();
                        if let Ok(mut name) = world.get::<&mut wgpu_cube::scene::Name>(entity) {
                            name.0 = new_name;
                            true
                        } else {
                            false
                        }
                    };

                    if renamed {
                        log::info!("Renamed entity {:?}", entity);
                        self.record_scene_change(&mut ctx.scene);
                    } else {
                        log::warn!("Entity {:?} does not have a Name component", entity);
                    }
                }
                InspectorAction::EditScript { .. } => {
                    // Script edits are handled immediately in the UI stage.
                }
                InspectorAction::EditShader { .. } => {
                    // Shader edits are handled immediately in the UI stage.
                }
            }
        }
        */ // END OF OLD IMPLEMENTATION

        if transforms_changed {
            ctx.scene.propagate_transforms();
        }
    }

    fn resolve_active_camera_entity(&mut self, scene: &mut SceneWorkspaceSceneMut) {
        if let Some(entity) = self.shared.active_camera_entity {
            if scene.main_world().contains(entity) {
                return;
            }
            scene.set_active_camera_entity(None);
            self.shared.active_camera_entity = scene.active_camera_entity();
        }

        let target_projection = scene.camera().projection();
        let candidate = {
            let world = scene.main_world();
            world
                .query::<&CameraComponent>()
                .iter()
                .find(|(_, component)| component.projection == target_projection)
                .map(|(entity, _)| entity)
        };

        if let Some(entity) = candidate {
            scene.set_active_camera_entity(Some(entity));
            self.shared.active_camera_entity = scene.active_camera_entity();
        }
    }

    pub(super) fn create_empty_entity(ctx: &mut GpuUpdateContext) -> Option<Entity> {
        let entity = EntityBuilder::new(&mut ctx.scene)
            .with_name("Empty")
            .with_transform(Transform::default())
            .visible(true)
            .spawn();
        Some(entity)
    }

    pub(super) fn create_primitive(
        ctx: &mut GpuUpdateContext,
        preset: ScenePrimitivePreset,
    ) -> Option<Entity> {
        let descriptor = PrimitiveMeshDescriptor::from(preset);
        let mesh_handle = ctx
            .scene
            .assets
            .ensure_primitive_mesh(ctx.renderer, descriptor);
        let bounds = ctx
            .scene
            .assets
            .meshes
            .get(mesh_handle)
            .and_then(|mesh| MeshBounds::from_vertices(&mesh.data().vertices));

        let material_handle = ctx
            .scene
            .assets
            .materials
            .insert(MaterialAsset::from_material(
                Material::pbr(),
                PathBuf::new(),
            ));
        let mut builder = EntityBuilder::new(&mut ctx.scene);
        builder = builder
            .with_name(preset.display_name())
            .with_transform(Transform::default())
            .with_mesh(mesh_handle)
            .with_material(material_handle)
            .visible(true);

        builder = builder.with_component(PrimitiveMeshComponent { descriptor });

        if let Some(bounds) = bounds {
            builder = builder.with_component(bounds);
        }

        Some(builder.spawn())
    }

    pub(super) fn create_particle_system(
        shared: &mut EditorSharedState,
        ctx: &mut GpuUpdateContext,
        preset: ParticleBehaviorPreset,
    ) -> Option<Entity> {
        let name = format!("{} Particle System", preset.display_name());
        let spawn_rate = Self::default_particle_spawn_rate(preset);
        let (mesh_handle, bounds) = Self::ensure_particle_mesh(shared, ctx);
        let material = Self::default_particle_material(preset);
        let material_handle = ctx
            .scene
            .assets
            .materials
            .insert(MaterialAsset::from_material(material, PathBuf::new()));
        let system_component = ParticleSystemComponent::new(spawn_rate, preset);
        let emitter_component = Self::default_particle_emitter(spawn_rate);

        let mut builder = EntityBuilder::new(&mut ctx.scene)
            .with_name(name)
            .with_transform(Transform::default())
            .with_mesh(mesh_handle)
            .with_material(material_handle)
            .with_particle_system(system_component)
            .with_particle_emitter(emitter_component)
            .visible(true);

        if let Some(bounds) = bounds {
            builder = builder.with_component(bounds);
        }

        if let Some(billboard) = Self::default_particle_billboard(preset) {
            builder = builder.with_component(billboard);
        }

        if let Some(depth_state) = Self::default_particle_depth_state(preset) {
            builder = builder.with_component(depth_state);
        }

        Some(builder.spawn())
    }

    fn ensure_particle_mesh(
        shared: &mut EditorSharedState,
        ctx: &mut GpuUpdateContext,
    ) -> (Handle<Mesh>, Option<MeshBounds>) {
        if let Some(handle) = shared.particle_mesh {
            if ctx.scene.assets.meshes.get(handle).is_some() {
                return (handle, shared.particle_mesh_bounds);
            }

            shared.particle_mesh = None;
            shared.particle_mesh_bounds = None;
        }

        let (vertices, indices) = quad_mesh();
        let mesh = ctx.renderer.create_mesh(&vertices, &indices);
        let handle = ctx.scene.assets.meshes.insert(mesh);
        let bounds = MeshBounds::from_vertices(&vertices);

        shared.particle_mesh = Some(handle);
        shared.particle_mesh_bounds = bounds;

        (handle, bounds)
    }

    fn default_particle_material(preset: ParticleBehaviorPreset) -> Material {
        match preset {
            ParticleBehaviorPreset::Starfield => Material::new([255, 255, 255, 255])
                .with_unlit()
                .with_billboarding(),
            _ => Material::pbr(),
        }
    }

    fn default_particle_emitter(spawn_rate: f32) -> ParticleEmitterComponent {
        let emitter = ParticleEmitter::new(Vec3::ZERO, spawn_rate);
        ParticleEmitterComponent::from(&emitter)
    }

    fn default_particle_billboard(preset: ParticleBehaviorPreset) -> Option<Billboard> {
        match preset {
            ParticleBehaviorPreset::Starfield => {
                Some(Billboard::new(BillboardOrientation::FaceCamera))
            }
            _ => None,
        }
    }

    fn default_particle_depth_state(preset: ParticleBehaviorPreset) -> Option<DepthState> {
        match preset {
            ParticleBehaviorPreset::Starfield => Some(DepthState::new(true, false)),
            _ => None,
        }
    }

    const fn default_particle_spawn_rate(_preset: ParticleBehaviorPreset) -> f32 {
        120.0
    }

    pub(super) fn create_point_light(ctx: &mut GpuUpdateContext) -> Option<Entity> {
        let transform = Transform::from_trs(Vec3::new(3.0, 3.0, 2.0), Quat::IDENTITY, Vec3::ONE);
        let light = PointLight {
            color: Vec3::splat(1.0),
            intensity: 120.0,
            range: 12.0,
        };

        let entity = EntityBuilder::new(&mut ctx.scene)
            .with_name("Point Light")
            .with_transform(transform)
            .with_component(light)
            .with_component(CanCastShadow(true))
            .spawn();

        Some(entity)
    }

    pub(super) fn create_directional_light(ctx: &mut GpuUpdateContext) -> Option<Entity> {
        let direction = Vec3::new(0.35, -1.0, -0.85).normalize();
        let rotation = Quat::from_rotation_arc(Vec3::NEG_Z, direction);
        let transform = Transform::from_trs(Vec3::new(0.0, 6.0, 0.0), rotation, Vec3::ONE);
        let light = DirectionalLight::new(Vec3::new(0.9, 0.95, 1.0), 3.0);

        let entity = EntityBuilder::new(&mut ctx.scene)
            .with_name("Directional Light")
            .with_transform(transform)
            .with_component(light)
            .with_component(CanCastShadow(true))
            .spawn();

        Some(entity)
    }

    pub(super) fn create_spot_light(ctx: &mut GpuUpdateContext) -> Option<Entity> {
        let position = Vec3::new(-4.0, 5.0, -3.0);
        let target = Vec3::new(0.0, 1.0, 0.0);
        let direction = (target - position).normalize();
        let rotation = Quat::from_rotation_arc(Vec3::NEG_Z, direction);
        let transform = Transform::from_trs(position, rotation, Vec3::ONE);
        let light = SpotLight {
            color: Vec3::new(1.0, 0.95, 0.9),
            intensity: 15.0,
            inner_angle: 0.35,
            outer_angle: 0.6,
            range: 18.0,
        };

        let entity = EntityBuilder::new(&mut ctx.scene)
            .with_name("Spot Light")
            .with_transform(transform)
            .with_component(light)
            .with_component(CanCastShadow(true))
            .spawn();

        Some(entity)
    }

    pub(super) fn create_camera(
        shared: &mut EditorSharedState,
        ctx: &mut GpuUpdateContext,
    ) -> Option<Entity> {
        let position = Vec3::new(0.0, 3.0, 6.0);
        let target = Vec3::ZERO;
        let direction = {
            let dir = target - position;
            if dir.length_squared() > 1e-6 {
                dir.normalize()
            } else {
                Vec3::NEG_Z
            }
        };
        let rotation = Quat::from_rotation_arc(Vec3::NEG_Z, direction);
        let transform = Transform::from_trs(position, rotation, Vec3::ONE);

        let current_camera = *ctx.scene.camera();
        let camera_component = CameraComponent::from(current_camera);
        let entity = EntityBuilder::new(&mut ctx.scene)
            .with_name("Camera")
            .with_transform(transform)
            .with_component(camera_component)
            .spawn();

        ctx.scene.set_active_camera_entity(Some(entity));
        shared.active_camera_entity = ctx.scene.active_camera_entity();

        Some(entity)
    }

    pub(super) fn create_environment(ctx: &mut GpuUpdateContext) -> Option<Entity> {
        let component = EnvironmentComponent::from_environment(ctx.scene.environment());
        let entity = EntityBuilder::new(&mut ctx.scene)
            .with_name("Environment")
            .with_component(component.clone())
            .spawn();

        ctx.scene.set_environment(component.to_environment());

        Some(entity)
    }

    pub(super) fn instance_scene_prefab(
        selected_entity: Option<Entity>,
        project_system: &mut ProjectSystem,
        ctx: &mut GpuUpdateContext,
        document_id: SceneDocumentId,
        node_path: Vec<String>,
    ) -> Option<Entity> {
        let parent_node = selected_entity
            .and_then(|entity| ctx.scene.node_for_entity(entity))
            .unwrap_or_else(|| ctx.scene.main_scene());

        let host_document = ctx.scene.document_id().to_string();

        let display_name = {
            project_system
                .scene_document(&document_id)
                .map(|doc| {
                    if doc.name.is_empty() {
                        doc.id.clone()
                    } else {
                        doc.name.clone()
                    }
                })
                .unwrap_or_else(|| document_id.clone())
        };

        let library = project_system.scene_library_mut();
        let resolved_path = {
            let asset = match library.asset(&document_id) {
                Some(asset) => asset,
                None => {
                    log::warn!(
                        "Scene document '{}' is not loaded; cannot instance into hierarchy.",
                        document_id
                    );
                    return None;
                }
            };

            if node_path.is_empty() {
                asset
                    .entities
                    .iter()
                    .find(|entity| entity.parent.is_none())
                    .and_then(|entity| entity.name.clone())
                    .map(|name| vec![name])
                    .unwrap_or_default()
            } else {
                node_path
            }
        };

        if resolved_path.is_empty() {
            log::warn!(
                "Scene document '{}' has no identifiable root node to instance.",
                document_id
            );
            return None;
        }

        let prefab_ref = ScenePrefabRef {
            document: document_id.clone(),
            node_path: resolved_path,
            overrides: ScenePrefabOverrides::default(),
        };

        let tree_node = SceneTreeAssetNode::builder(display_name.clone())
            .with_scene_ref(prefab_ref)
            .build();
        let tree_asset = SceneTreeAsset::new(display_name, tree_node);

        let node_id = ctx.scene.instantiate_tree_asset(
            library,
            &tree_asset,
            Some(parent_node),
            Some(host_document.as_str()),
        );

        fn first_entity(scene: &Scene, node: SceneNodeId) -> Option<Entity> {
            let mut index = 0;
            loop {
                if let Some(entity) = scene.node_asset_entity(node, index) {
                    return Some(entity);
                }
                index += 1;
                if index > 1000 {
                    break;
                }
            }

            for &child in scene.node_children(node) {
                if let Some(entity) = first_entity(scene, child) {
                    return Some(entity);
                }
            }

            None
        }

        first_entity(&ctx.scene, node_id)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn copy_environment_asset_if_needed(
        &mut self,
        component: &mut EnvironmentComponent,
        previous: Option<&EnvironmentComponent>,
    ) -> Result<(), String> {
        let Some(hdr) = component.hdr.as_mut() else {
            return Ok(());
        };
        let Some(requested_path) = hdr.path.clone() else {
            return Ok(());
        };

        let previous_path = previous
            .and_then(|prev| prev.hdr.as_ref())
            .and_then(|hdr| hdr.path.clone());

        if previous_path.as_ref() == Some(&requested_path) {
            return Ok(());
        }

        let Some(content_root) = self.project_system().content_root() else {
            return Err(
                "Open or create a project before assigning environment HDR files.".to_string(),
            );
        };

        let absolute_source = if requested_path.is_absolute() {
            requested_path
        } else {
            std::env::current_dir()
                .map_err(|err| format!("Failed to resolve current directory: {err}"))?
                .join(&requested_path)
        };

        if !absolute_source.exists() {
            return Err(format!(
                "Selected HDR file {:?} does not exist",
                absolute_source
            ));
        }

        let environment_dir = content_root.join("environment");
        std::fs::create_dir_all(&environment_dir).map_err(|err| {
            format!(
                "Failed to create environment asset folder {:?}: {err}",
                environment_dir
            )
        })?;

        let file_name = absolute_source
            .file_name()
            .map(|name| name.to_owned())
            .ok_or_else(|| {
                format!(
                    "HDR file {:?} is missing a valid file name",
                    absolute_source
                )
            })?;

        let destination = environment_dir.join(&file_name);

        if destination != absolute_source {
            std::fs::copy(&absolute_source, &destination).map_err(|err| {
                format!(
                    "Failed to copy HDR file from {:?} to {:?}: {err}",
                    absolute_source, destination
                )
            })?;
            self.asset_browser_state_mut().report_info(format!(
                "Copied environment HDR to {}",
                destination.display()
            ));
        }

        let resolved = destination.canonicalize().unwrap_or(destination);
        hdr.path = Some(resolved);

        Ok(())
    }
}
