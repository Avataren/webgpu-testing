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
mod remote;
mod runtime_mode;
mod scene_creation_system;
mod scene_tabs_panel;
mod script_editor_system;
mod scripts;
mod selection_system;
mod setup;
mod shader_editor_system;
mod ui_plugin_manager;

#[cfg(not(target_arch = "wasm32"))]
mod script_watcher;
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
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Manifest name of the built-in welcome screen plugin.
const WELCOME_SCREEN_PLUGIN: &str = "Welcome Screen";
/// Manifest name of the built-in text editor plugin.
const TEXT_EDITOR_PLUGIN: &str = "Text Editor";

/// Widget responses for one script, keyed by widget id.
type ScriptUiResponses = HashMap<String, UiResponse>;

/// Store this pass's script UI responses for the next frame.
///
/// egui can run several passes per frame but only the first receives input, so later
/// passes are merged in rather than overwriting clicks from the first.
fn store_ui_responses(
    ctx: &egui::Context,
    target: &mut HashMap<Entity, ScriptUiResponses>,
    responses: HashMap<Entity, ScriptUiResponses>,
) {
    if ctx.current_pass_index() == 0 {
        *target = responses;
    } else {
        merge_ui_responses(target, responses);
    }
}
use wgpu_cube::app::{
    AppBuilder, GpuUpdateContext, RuntimeMode, RuntimeStateHandle, StartupContext, UpdateContext,
};
use wgpu_cube::asset::{Handle, MaterialAsset, Mesh};
use wgpu_cube::gpu_particles::ParticleEmitter;
use wgpu_cube::renderer::primitives::{quad_mesh, PrimitiveMeshDescriptor};
use wgpu_cube::renderer::{CustomRenderContext, CustomRenderStage, Material, RenderRegion};
use wgpu_cube::scene::components::{
    Billboard, BillboardOrientation, DepthState, PrimitiveMeshComponent,
};
use wgpu_cube::scene::workspace::SceneDocumentId;
use wgpu_cube::scene::{
    CameraComponent, CanCastShadow, DirectionalLight, EntityBuilder, EnvironmentComponent,
    MeshBounds, ParticleBehaviorPreset, ParticleEmitterComponent, ParticleSystemComponent,
    PointLight, Scene, SceneNodeId, ScenePrefabOverrides, ScenePrefabRef, SceneTreeAsset,
    SceneTreeAssetNode, SceneWorkspaceSceneMut, SpotLight, Transform,
};
use wgpu_cube::scripting::lua::api::enqueue_text_editor_request;
use wgpu_cube::scripting::lua::api::ui::{merge_ui_responses, UiCommand, UiResponse};
use wgpu_cube::scripting::LuaScriptingPlugin;
use wgpu_cube::{
    DefaultUI, RenderApplication, SceneHierarchyEvent, SceneHierarchySceneDescriptor,
    ScenePrimitivePreset,
};

use crate::inspector::InspectorAction;
use crate::layout::{EditorBehavior, EditorPane};
use crate::postprocess::ViewportGrid;

impl RenderApplication for EditorApplication {
    fn initial_runtime_mode(&self) -> RuntimeMode {
        RuntimeMode::Editor
    }

    fn name(&self) -> &str {
        "Engine Editor"
    }

    fn install_runtime_state_handle(&mut self, handle: RuntimeStateHandle) {
        self.set_runtime_state_handle(handle);
    }

    fn configure(&self, builder: &mut AppBuilder) {
        builder.add_plugin(LuaScriptingPlugin::new());
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

    fn ui(&mut self, ui: &mut egui::Ui, default_ui: &mut DefaultUI) {
        self.run_ui_impl(ui, default_ui);
    }

    fn show_default_ui(&self) -> bool {
        false
    }

    fn render_region(&self) -> Option<RenderRegion> {
        self.render_region_for_mode(self.shared.runtime_state.active_mode())
    }

    fn remote_control_port(&self) -> Option<u16> {
        Some(wgpu_cube::remote::DEFAULT_EDITOR_PORT)
    }

    fn remote_command(
        &mut self,
        method: &str,
        params: &serde_json::Value,
        ctx: &mut wgpu_cube::remote::RemoteContext<'_, '_>,
    ) -> Option<wgpu_cube::remote::RemoteResult> {
        self.handle_remote_command(method, params, ctx)
    }

    fn remote_methods(&self) -> Vec<&'static str> {
        remote::EDITOR_METHODS.to_vec()
    }

    fn remote_scene_modified(&mut self, ctx: &mut GpuUpdateContext) {
        self.handle_remote_scene_modified(ctx);
    }
}

impl EditorApplication {
    fn run_update_impl(&mut self, ctx: &mut UpdateContext) {
        // Detect mode changes first so the rest of the update sees the new mode.
        let current_mode = ctx.runtime;
        if current_mode != self.shared.last_runtime_mode {
            self.detect_mode_transition(current_mode);
        }

        self.sync_active_scene_state(ctx);

        // Process Lua editor commands from plugins
        self.process_lua_editor_commands();

        // Regular editor updates
        self.drain_update_commands(ctx);
        self.run_system_updates(ctx);
    }

    fn run_gpu_update_impl(&mut self, ctx: &mut GpuUpdateContext) {
        #[cfg(not(target_arch = "wasm32"))]
        self.process_shader_file_changes(ctx);

        #[cfg(not(target_arch = "wasm32"))]
        self.process_script_file_changes(ctx);

        let scene_changed = self.shared.active_scene_handle != Some(ctx.scene_handle);

        self.shared.set_active_scene_handle(ctx.scene_handle);
        self.selection_system_mut()
            .set_active_scene(ctx.scene_handle);

        if scene_changed {
            ctx.renderer.update_texture_bind_group(&ctx.scene.assets);
            self.shared.mark_scene_hierarchy_dirty(ctx.scene_handle);
        }

        // Load UI plugins on first update (no project required - welcome screen needs this)
        if !self.shared.ui_plugins_loaded {
            // Plugin scripts are started by the regular per-frame update.
            Self::load_ui_plugins(&mut self.shared, &mut ctx.scene);
            self.shared.ui_plugins_loaded = true;
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

        // Collect UI commands from scripts that implement on_ui()
        // Pass editor_mode based on actual runtime state
        let editor_mode = matches!(self.shared.runtime_state.active_mode(), RuntimeMode::Editor);
        log::debug!(target: "editor_app", "Calling process_script_ui with editor_mode={}, runtime={:?}",
            editor_mode, ctx.runtime);

        // Feed last frame's widget responses back to scripts. Always replace them,
        // even when empty, so a click is only ever seen once.
        ctx.scene
            .set_ui_responses(std::mem::take(&mut self.shared.script_ui_responses));

        // Viewport information for in-game UI positioning
        let viewport = self.shared.viewports.game_viewport.rect().map(|rect| {
            (
                rect.width(),
                rect.height(),
                self.shared.last_pixels_per_point.unwrap_or(1.0),
            )
        });

        self.shared.script_ui_commands = ctx.scene.process_script_ui(editor_mode, viewport);
        self.shared.script_ui_titles = self
            .shared
            .script_ui_commands
            .keys()
            .map(|&entity| {
                let title = ctx
                    .scene
                    .main_world()
                    .get::<&wgpu_cube::scene::Name>(entity)
                    .map(|name| name.0.clone())
                    .unwrap_or_else(|_| format!("Entity {}", entity.id()));
                (entity, title)
            })
            .collect();

        // UI plugins run in their own world and Lua runtime, independent of the scene.
        if let Some(manager) = self.shared.ui_plugin_manager.as_mut() {
            manager.set_ui_responses(std::mem::take(&mut self.shared.plugin_ui_responses));
            manager.run_scripts(ctx.dt, editor_mode, ctx.scene.input_state());
            self.shared.plugin_ui_commands = manager.process_ui(
                editor_mode,
                viewport,
                &self.shared.plugin_window_sizes,
                ctx.scene.input_state(),
            );
        } else {
            self.shared.plugin_ui_commands.clear();
        }
    }

    fn run_ui_impl(&mut self, ui: &mut egui::Ui, default_ui: &mut DefaultUI) {
        let ctx = &ui.ctx().clone();
        if let Some(handle) = self.shared.active_scene_handle {
            self.selection_system_mut().set_active_scene(handle);
        }

        self.shared.viewports.scene_viewport.clear();
        self.shared.viewports.game_viewport.clear();
        self.shared.last_pixels_per_point = Some(ctx.pixels_per_point());

        self.project_system_mut().show_startup_dialog(ctx);

        if self.project_system().is_startup_dialog_visible() {
            return;
        }

        self.show_menu_bar(ui);
        self.show_rename_scene_dialog(ctx);

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
            self.scene_tabs_panel.show(&tabs, ui)
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
            egui::Frame::central_panel(ui.style()).fill(egui::Color32::TRANSPARENT);
        {
            let (shared, asset_browser_state) = self.shared_and_asset_browser_state_mut();
            let dock_tree = &mut shared.dock_tree;
            let scene_viewport = &mut shared.viewports.scene_viewport;
            let game_viewport = &mut shared.viewports.game_viewport;
            let workspace_info = scene_hierarchy_window.workspace_info();

            egui::CentralPanel::default_margins()
                .frame(transparent_frame)
                .show(ui, |ui| {
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

        // Render reload notifications as toasts
        self.render_reload_notifications(ctx);

        self.run_system_ui(ctx, default_ui);
    }
}

impl EditorApplication {
    /// Render UI from scripts that implement on_ui()
    fn render_script_ui(&mut self, ctx: &egui::Context) {
        let project_loaded = self.project_system().controller().current_dir().is_some();

        self.apply_welcome_screen_visibility(project_loaded);
        let mut welcome_override = self.shared.welcome_plugin_override;

        let welcome_screen_visible =
            self.render_plugin_ui(ctx, project_loaded, &mut welcome_override);

        // Scene scripts are hidden while the welcome modal is active.
        let scene_responses = if welcome_screen_visible {
            HashMap::new()
        } else {
            self.render_scene_script_ui(ctx)
        };

        self.shared.welcome_plugin_override = welcome_override;
        store_ui_responses(ctx, &mut self.shared.script_ui_responses, scene_responses);
    }

    /// Render UI for scene scripts.
    ///
    /// Top-level anchored panels are HUD overlays positioned inside the game viewport;
    /// all other widgets go into a window titled after the entity.
    fn render_scene_script_ui(&self, ctx: &egui::Context) -> HashMap<Entity, ScriptUiResponses> {
        let game_viewport_rect = self.shared.viewports.game_viewport.rect();
        let mut scene_responses = HashMap::new();

        for (entity, commands) in &self.shared.script_ui_commands {
            let mut responses = HashMap::new();
            let (overlays, widgets): (Vec<&UiCommand>, Vec<&UiCommand>) = commands
                .iter()
                .partition(|command| matches!(command, UiCommand::AnchoredPanel { .. }));

            if let (Some(rect), false) = (game_viewport_rect, overlays.is_empty()) {
                // Host area for the panels; each anchored panel creates its own layer.
                egui::Area::new(egui::Id::new(("script_hud", *entity)))
                    .fixed_pos(rect.min)
                    .interactable(false)
                    .show(ctx, |ui| {
                        for command in &overlays {
                            command.render_and_collect(ui, &mut responses, Some(rect));
                        }
                    });
            }

            if !widgets.is_empty() {
                let title = self
                    .shared
                    .script_ui_titles
                    .get(entity)
                    .cloned()
                    .unwrap_or_else(|| format!("Entity {}", entity.id()));
                egui::Window::new(title)
                    .id(egui::Id::new(("script_ui", *entity)))
                    .resizable(true)
                    .default_size([300.0, 200.0])
                    .show(ctx, |ui| {
                        for command in &widgets {
                            command.render_and_collect(ui, &mut responses, game_viewport_rect);
                        }
                    });
            }

            if !responses.is_empty() {
                scene_responses.insert(*entity, responses);
            }
        }

        scene_responses
    }

    /// Render the plugin manager and plugin windows. Returns whether the welcome
    /// screen is showing as a modal.
    fn render_plugin_ui(
        &mut self,
        ctx: &egui::Context,
        project_loaded: bool,
        welcome_override: &mut Option<bool>,
    ) -> bool {
        let shared = &mut self.shared;
        let Some(manager) = shared.ui_plugin_manager.as_mut() else {
            shared.plugin_window_sizes.clear();
            shared.plugin_ui_responses.clear();
            return false;
        };

        egui::Window::new("Plugin Manager")
            .open(&mut shared.windows.plugin_manager)
            .resizable(true)
            .default_size([350.0, 500.0])
            .show(ctx, |ui| {
                use ui_plugin_manager::PluginType;

                let (mut editor_plugins, mut user_plugins): (Vec<_>, Vec<_>) = manager
                    .plugins_mut()
                    .iter_mut()
                    .filter(|plugin| plugin.metadata.enabled)
                    .partition(|plugin| plugin.metadata.plugin_type == PluginType::Editor);

                let mut section =
                    |ui: &mut egui::Ui,
                     heading: &str,
                     subtitle: &str,
                     empty: &str,
                     plugins: &mut [&mut ui_plugin_manager::LoadedPlugin]| {
                        ui.heading(heading);
                        ui.label(egui::RichText::new(subtitle).small().italics());
                        ui.separator();
                        if plugins.is_empty() {
                            ui.label(egui::RichText::new(empty).weak());
                        }
                        for plugin in plugins.iter_mut() {
                            ui.horizontal(|ui| {
                                if ui
                                    .checkbox(&mut plugin.visible, &plugin.metadata.name)
                                    .changed()
                                    && plugin.metadata.name == WELCOME_SCREEN_PLUGIN
                                {
                                    // Only override while the choice differs from the default.
                                    let default_visible = !project_loaded;
                                    *welcome_override = (plugin.visible != default_visible)
                                        .then_some(plugin.visible);
                                }
                                ui.label(egui::RichText::new(&plugin.metadata.category).weak());
                            });
                            ui.label(
                                egui::RichText::new(format!("  {}", plugin.metadata.description))
                                    .small(),
                            );
                            ui.add_space(4.0);
                        }
                    };

                section(
                    ui,
                    "Editor Plugins",
                    "Built-in editor functionality",
                    "No editor plugins installed",
                    &mut editor_plugins,
                );
                ui.add_space(8.0);
                section(
                    ui,
                    "User Plugins",
                    "Custom user-created plugins",
                    "No user plugins installed",
                    &mut user_plugins,
                );
            });

        // The welcome screen may hide itself from Lua, so it only counts as visible
        // while it is enabled and actually producing UI.
        let welcome_screen_visible = shared.plugin_ui_commands.iter().any(|(entity, commands)| {
            !commands.is_empty()
                && manager
                    .get_plugin(*entity)
                    .is_some_and(|p| p.visible && p.metadata.name == WELCOME_SCREEN_PLUGIN)
        });

        if welcome_screen_visible {
            egui::Area::new("welcome_modal_overlay".into())
                .fixed_pos(egui::pos2(0.0, 0.0))
                .interactable(false)
                .show(ctx, |ui| {
                    ui.painter().rect_filled(
                        ctx.viewport_rect(),
                        0.0,
                        egui::Color32::from_black_alpha(180),
                    );
                });
            ctx.set_cursor_icon(egui::CursorIcon::Default);
        }

        let mut plugin_responses = HashMap::new();
        let entities: Vec<Entity> = manager
            .plugins()
            .iter()
            .filter(|plugin| plugin.metadata.enabled)
            .map(|plugin| plugin.entity)
            .collect();

        for entity in entities {
            let Some(plugin) = manager.get_plugin(entity) else {
                continue;
            };
            let commands = shared
                .plugin_ui_commands
                .get(&entity)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let error = manager.last_error(entity).map(str::to_owned);
            let is_welcome_screen = plugin.metadata.name == WELCOME_SCREEN_PLUGIN;
            let is_text_editor = plugin.metadata.name == TEXT_EDITOR_PLUGIN;

            let hidden_by_modal = welcome_screen_visible && !is_welcome_screen && !is_text_editor;
            if !plugin.visible || hidden_by_modal || (commands.is_empty() && error.is_none()) {
                shared.plugin_window_sizes.remove(&entity);
                continue;
            }

            let mut open_flag = true;
            let mut window = egui::Window::new(&plugin.metadata.name)
                .id(egui::Id::new(("plugin_window", entity)))
                .resizable(true)
                .default_size(if is_text_editor {
                    [800.0, 800.0]
                } else {
                    [500.0, 400.0]
                })
                .collapsible(true)
                .open(&mut open_flag);

            if is_welcome_screen {
                let screen_rect = ctx.viewport_rect();
                window = window
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .collapsible(false)
                    .resizable(false)
                    .title_bar(false)
                    .fixed_size([screen_rect.width() * 0.8, screen_rect.height() * 0.8])
                    .interactable(true);
            } else if is_text_editor {
                window = window.vscroll(true).hscroll(true);
            }

            let mut responses = HashMap::new();
            let window_response = window.show(ctx, |ui| {
                if let Some(error) = &error {
                    ui.colored_label(ui.visuals().error_fg_color, format!("⚠ {error}"));
                    ui.separator();
                }
                if is_welcome_screen {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for command in commands {
                                command.render_and_collect(ui, &mut responses, None);
                            }
                        });
                } else {
                    for command in commands {
                        command.render_and_collect(ui, &mut responses, None);
                    }
                }
            });

            if let Some(window_response) = &window_response {
                // Keep the modal welcome screen focused so it cannot lose input.
                if is_welcome_screen {
                    let clicked_outside = ctx.input(|input| {
                        input.pointer.any_click()
                            && input
                                .pointer
                                .interact_pos()
                                .is_some_and(|pos| !window_response.response.rect.contains(pos))
                    });
                    let nothing_focused = ctx.memory(|mem| mem.focused().is_none());
                    if clicked_outside || nothing_focused {
                        window_response.response.request_focus();
                    }
                }

                let rect = window_response.response.rect;
                let ppp = ctx.pixels_per_point();
                shared
                    .plugin_window_sizes
                    .insert(entity, (rect.width() / ppp, rect.height() / ppp, ppp));
            }

            if !open_flag {
                shared.plugin_window_sizes.remove(&entity);
                if let Some(plugin) = manager.get_plugin_mut(entity) {
                    plugin.visible = false;
                }
            }

            if !responses.is_empty() {
                plugin_responses.insert(entity, responses);
            }
        }

        store_ui_responses(ctx, &mut shared.plugin_ui_responses, plugin_responses);
        welcome_screen_visible
    }

    pub(super) fn open_text_editor_with_path(&mut self, path: &Path) -> bool {
        let Some(manager) = self.shared.ui_plugin_manager.as_mut() else {
            log::warn!(
                "Cannot open Text Editor plugin for {} - plugin manager not initialized",
                path.display()
            );
            return false;
        };

        let mut plugin_found = false;
        for plugin in manager.plugins_mut() {
            if plugin.metadata.name == TEXT_EDITOR_PLUGIN {
                plugin.visible = true;
                plugin_found = true;
                break;
            }
        }

        if !plugin_found {
            log::warn!(
                "Text Editor plugin not registered or disabled (needed for {}).",
                path.display()
            );
            return false;
        }

        enqueue_text_editor_request(path);
        log::info!("Queued {} for the Text Editor plugin", path.display());
        true
    }

    fn apply_welcome_screen_visibility(&mut self, project_loaded: bool) {
        let Some(manager) = self.shared.ui_plugin_manager.as_mut() else {
            return;
        };

        let target_visible = self
            .shared
            .welcome_plugin_override
            .unwrap_or(!project_loaded);

        for plugin in manager.plugins_mut() {
            if plugin.metadata.name == WELCOME_SCREEN_PLUGIN {
                plugin.visible = target_visible;
                break;
            }
        }
    }

    /// Render reload notifications as toasts in the bottom-right corner
    fn render_reload_notifications(&mut self, ctx: &egui::Context) {
        use core::NotificationSeverity;

        // Remove expired notifications
        self.shared
            .reload_notifications
            .retain(|notif| !notif.should_dismiss());

        let screen_rect = ctx.viewport_rect();
        let toast_width = 300.0;
        let toast_height = 60.0;
        let margin = 10.0;
        let spacing = 5.0;

        // Render toasts from bottom to top
        for (index, notification) in self.shared.reload_notifications.iter().enumerate() {
            let y_offset = margin + (toast_height + spacing) * index as f32;
            let pos = egui::pos2(
                screen_rect.max.x - toast_width - margin,
                screen_rect.max.y - y_offset - toast_height,
            );

            let (bg_color, text_color) = match notification.severity {
                NotificationSeverity::Success => {
                    (egui::Color32::from_rgb(40, 180, 40), egui::Color32::WHITE)
                }
                NotificationSeverity::Warning => {
                    (egui::Color32::from_rgb(220, 180, 20), egui::Color32::BLACK)
                }
                NotificationSeverity::Error => {
                    (egui::Color32::from_rgb(200, 40, 40), egui::Color32::WHITE)
                }
            };

            egui::Area::new(format!("reload_notification_{}", index).into())
                .fixed_pos(pos)
                .show(ctx, |ui| {
                    egui::Frame::default()
                        .fill(bg_color)
                        .corner_radius(4.0)
                        .inner_margin(8.0)
                        .show(ui, |ui| {
                            ui.set_width(toast_width - 16.0);
                            ui.set_height(toast_height - 16.0);

                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.add_space(4.0);
                                    ui.label(
                                        egui::RichText::new(&notification.message)
                                            .color(text_color)
                                            .size(14.0),
                                    );

                                    // Show age of notification
                                    let age_secs = notification.timestamp.elapsed().as_secs();
                                    if age_secs > 0 {
                                        ui.label(
                                            egui::RichText::new(format!("{}s ago", age_secs))
                                                .color(text_color.gamma_multiply(0.7))
                                                .size(11.0),
                                        );
                                    }
                                });
                            });
                        });
                });
        }
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
            SceneTabAction::Rename(document_id) => {
                self.shared.pending_scene_rename = Some(document_id);
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
        let mut scene_changed = false;

        // Route all actions through the dispatcher
        use action_handlers::dispatch_action;

        for action in actions {
            let result = dispatch_action(&mut ctx.scene, self, action);

            if result.transforms_changed {
                transforms_changed = true;
            }
            if result.scene_changed {
                scene_changed = true;
            }
        }

        // END OF OLD IMPLEMENTATION

        if transforms_changed {
            ctx.scene.propagate_transforms();
        }

        if scene_changed {
            self.record_scene_change(&mut ctx.scene);
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
