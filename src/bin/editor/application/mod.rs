mod core;
mod gizmo;
mod history;
mod input;
mod picking;
mod project_io;
mod runtime_mode;
mod scripts;
mod selection;
mod setup;
mod ui;

pub use self::core::EditorApplication;
#[allow(unused_imports)]
pub use self::core::EditorApplicationBuilder;
use self::core::{GameViewDisplayMode, PendingScriptAction};

use glam::Vec2;
use wgpu_cube::app::{
    AppBuilder, GpuUpdateContext, RuntimeMode, RuntimeStateHandle, StartupContext, UpdateContext,
};
use wgpu_cube::renderer::{CustomRenderContext, CustomRenderStage, RenderRegion};
use wgpu_cube::scene::{
    CanCastShadow, DirectionalLight, MaterialComponent, PointLight, SpotLight, TransformComponent,
};
use wgpu_cube::scripting::RuneScriptingPlugin;
use wgpu_cube::{DefaultUI, RenderApplication};

use crate::inspector::InspectorAction;
use crate::layout::{EditorBehavior, EditorPane};
use crate::postprocess::ViewportGrid;
use crate::script_editor::{ScriptEditorEvent, ScriptEditorState};

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
        self.ensure_default_scene(ctx);
        self.initialize_history_state(ctx.scene);
    }

    fn update(&mut self, ctx: &mut UpdateContext) {
        // DETECT MODE CHANGES FIRST - saves state BEFORE any animation changes
        let current_mode = ctx.runtime;
        if current_mode != self.last_runtime_mode {
            self.detect_mode_transition(ctx, current_mode);
        }

        // Regular editor updates
        if matches!(ctx.runtime, RuntimeMode::Editor) {
            self.camera_controller.update_camera(ctx);
        }
        self.ensure_editor_entity_ids(ctx.scene);
        self.apply_pending_script_actions(ctx);
        self.apply_pending_inspector_actions(ctx);

        if self.undo_redo.take_undo() {
            self.undo_redo.clear_redo();
            self.perform_undo(ctx);
        } else if self.undo_redo.take_redo() {
            self.perform_redo(ctx);
        }

        ctx.scene
            .set_transform_gizmo_mode(self.transform_tool.gizmo_mode);
        ctx.scene
            .set_transform_gizmo_space(self.transform_tool.gizmo_space);
        self.process_pending_imports(ctx);
        self.process_pending_entity_deletions(ctx);
        self.process_viewport_pick(ctx);
        self.sync_selection_component(ctx);
        self.update_gizmo_drag(ctx);

        let hovered_handle = if let Some(drag) = self.transform_tool.gizmo_drag.as_ref() {
            Some(drag.handle)
        } else if matches!(ctx.runtime, RuntimeMode::Editor) {
            if let (Some(uv), Some(region)) = (
                self.selection.pointer.scene_uv,
                self.viewports.scene_viewport.region(),
            ) {
                let width = region.width().max(1) as f32;
                let height = region.height().max(1) as f32;
                let aspect = width / height;
                let camera = ctx.scene.camera();
                let (origin, direction) = Self::ray_from_uv(camera, uv, aspect);
                ctx.scene.transform_gizmo_hit(origin, direction)
            } else {
                None
            }
        } else {
            None
        };
        ctx.scene.set_transform_gizmo_hover(hovered_handle);
        self.ensure_script_editor_target_valid(ctx.scene);
    }

    fn gpu_update(&mut self, ctx: &mut GpuUpdateContext) {
        // PROCESS MODE TRANSITIONS FIRST
        self.process_pending_mode_transition(ctx);

        if let Some(request) = self.project.take_pending_create() {
            self.handle_project_create(ctx, request);
        }

        if let Some(dir) = self.project.take_pending_load() {
            self.handle_project_load(ctx, dir);
        }

        if let Some(dir) = self.project.take_pending_save() {
            self.handle_project_save(ctx, dir);
        }

        if let Some(request) = self.project.take_pending_build() {
            self.handle_project_build(ctx, request);
        }

        ctx.scene.process_pending_gltf_imports(ctx.renderer);
    }

    fn custom_render(&mut self, ctx: &mut CustomRenderContext) {
        if matches!(self.runtime_state.active_mode(), RuntimeMode::Editor) {
            let grid = self
                .viewports
                .grid_postprocess
                .get_or_insert_with(|| ViewportGrid::new(ctx.renderer.get_device()));
            grid.render(ctx);
        }
    }

    fn custom_render_stage(&self) -> CustomRenderStage {
        CustomRenderStage::AfterPostprocess
    }

    fn ui(&mut self, ctx: &egui::Context, default_ui: &mut DefaultUI) {
        self.viewports.scene_viewport.clear();
        self.viewports.game_viewport.clear();

        self.project.show_startup_dialog(ctx);
        if self.project.is_startup_dialog_visible() {
            return;
        }

        self.show_menu_bar(ctx);

        let active_mode = self.runtime_state.active_mode();
        let is_playing = matches!(active_mode, RuntimeMode::Playing);
        let show_fullscreen_game = is_playing
            && matches!(
                self.viewports.game_view_display,
                GameViewDisplayMode::Fullscreen
            );

        if !show_fullscreen_game {
            self.windows.show(ctx, default_ui);
            self.project.show_settings_window(ctx);
            self.project.show_build_window(ctx);
        }

        let content_root = self.project.content_root();
        let dock_tree = &mut self.dock_tree;
        let scene_viewport = &mut self.viewports.scene_viewport;
        let game_viewport = &mut self.viewports.game_viewport;
        let (scene_hierarchy_window, log_window) = default_ui.scene_hierarchy_and_log_windows_mut();
        if let Some(selection) = self.selection.take_override() {
            scene_hierarchy_window.set_selected_entity(selection);
        }
        let mut inspector_actions = Vec::new();
        let transparent_frame =
            egui::Frame::central_panel(&ctx.style()).fill(egui::Color32::TRANSPARENT);
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
                        asset_browser: &mut self.asset_browser,
                        content_root,
                    };
                    dock_tree.ui(&mut behavior, ui);
                }
            });

        let game_tile_active = self
            .find_pane_tile(EditorPane::GameViewport)
            .map(|id| self.dock_tree.active_tiles().contains(&id))
            .unwrap_or(false);

        let scene_tile_active = self
            .find_pane_tile(EditorPane::SceneViewport)
            .map(|id| self.dock_tree.active_tiles().contains(&id))
            .unwrap_or(false);

        if is_playing && !show_fullscreen_game && !game_tile_active && scene_tile_active {
            self.runtime_state.request_mode(RuntimeMode::Editor);
        }

        if !is_playing
            && !matches!(self.runtime_state.desired_mode(), RuntimeMode::Playing)
            && game_tile_active
        {
            self.runtime_state.request_mode(RuntimeMode::Playing);
        }

        self.selection
            .set_selected(scene_hierarchy_window.selected_entity());

        for action in inspector_actions {
            match action {
                InspectorAction::EditScript { entity, component } => {
                    if let Some(editor) = self.script_editor.as_mut() {
                        if editor.entity() == entity {
                            editor.sync_with_component(&component);
                            continue;
                        }
                    }
                    self.script_editor = Some(ScriptEditorState::new(entity, component));
                }
                other => {
                    self.pending_inspector_actions.push(other);
                }
            }
        }

        if is_playing {
            self.camera_controller.set_viewport_rect(None);
        } else {
            self.camera_controller
                .set_viewport_rect(self.viewports.scene_viewport.rect());
        }

        self.camera_controller.capture_input(ctx);
        if !is_playing {
            self.capture_viewport_pick_input(ctx);
            self.handle_history_shortcuts(ctx);
            self.handle_gizmo_shortcuts(ctx);
            self.handle_general_shortcuts(ctx);
        } else {
            self.selection.clear_pending_pick();
        }

        let script_event = if !show_fullscreen_game {
            if let Some(editor) = self.script_editor.as_mut() {
                editor.show(ctx)
            } else {
                ScriptEditorEvent::None
            }
        } else {
            ScriptEditorEvent::None
        };

        match script_event {
            ScriptEditorEvent::None => {}
            ScriptEditorEvent::Closed => {
                self.script_editor = None;
            }
            ScriptEditorEvent::SaveInline {
                entity,
                name,
                contents,
                message,
            } => {
                self.pending_script_actions
                    .push(PendingScriptAction::SaveInline {
                        entity,
                        name,
                        contents,
                        message,
                    });
            }
            ScriptEditorEvent::SaveFile { entity, message } => {
                self.pending_script_actions
                    .push(PendingScriptAction::ReloadRuntime { entity, message });
            }
        }

        let pointer_uv = if !is_playing && !self.camera_controller.is_looking() {
            self.viewports.scene_viewport.rect().and_then(|rect| {
                ctx.input(|input| input.pointer.hover_pos())
                    .and_then(|pos| {
                        if rect.contains(pos) {
                            let local_x = (pos.x - rect.min.x) / rect.width();
                            let local_y = (pos.y - rect.min.y) / rect.height();
                            if local_x.is_finite() && local_y.is_finite() {
                                Some(Vec2::new(local_x.clamp(0.0, 1.0), local_y.clamp(0.0, 1.0)))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    })
            })
        } else {
            None
        };
        self.selection.pointer.set_scene_uv(pointer_uv);
    }

    fn show_default_ui(&self) -> bool {
        false
    }

    fn render_region(&self) -> Option<RenderRegion> {
        self.render_region_for_mode(self.runtime_state.active_mode())
    }
}

impl EditorApplication {
    fn apply_pending_inspector_actions(&mut self, ctx: &mut UpdateContext) {
        if self.pending_inspector_actions.is_empty() {
            return;
        }

        let actions = std::mem::take(&mut self.pending_inspector_actions);
        let mut transforms_changed = false;

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
                        self.record_scene_change(ctx.scene);
                    }
                }
                InspectorAction::UpdateMaterial { entity, material } => {
                    let mut updated = false;
                    {
                        let world = ctx.scene.main_world_mut();
                        match world.get::<&mut MaterialComponent>(entity) {
                            Ok(mut component) => {
                                component.0 = material;
                                updated = true;
                            }
                            Err(err) => {
                                log::warn!("Failed to update material for {:?}: {}", entity, err);
                            }
                        }
                    }

                    if updated {
                        self.record_scene_change(ctx.scene);
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
                        self.record_scene_change(ctx.scene);
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
                        self.record_scene_change(ctx.scene);
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
                        self.record_scene_change(ctx.scene);
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
                        self.record_scene_change(ctx.scene);
                    }
                }
                InspectorAction::EditScript { .. } => {
                    // Script edits are handled immediately in the UI stage.
                }
            }
        }

        if transforms_changed {
            ctx.scene.propagate_transforms();
        }
    }
}
