#![cfg(feature = "egui")]

mod camera;
mod history;
mod inspector;
mod layout;
mod postprocess;
mod windows;

use std::fs;
use std::path::{Path, PathBuf};

use egui_tiles::{Tile, TileId, Tree};
use glam::{Quat, Vec2, Vec3, Vec4};
use hecs::Entity;
use log::{error, warn};
use wgpu_cube::app::{
    AppBuilder, GpuUpdateContext, RuntimeMode, RuntimeStateHandle, StartupContext, UpdateContext,
};
use wgpu_cube::renderer::{
    cube_mesh, CustomRenderContext, CustomRenderStage, Material, RenderRegion,
};
use wgpu_cube::scene::components::{DirectionalLight, PointLight, SpotLight};
use wgpu_cube::scene::{
    Children, EditorEntityId, EntityBuilder, MaterialComponent, MeshBounds, MeshComponent, Name,
    Parent, SelectedInEditor, Transform, TransformComponent, TransformGizmoAxis,
    TransformGizmoHandle, TransformGizmoMode, TransformGizmoSpace, Visible, WorldTransform,
};
use wgpu_cube::scripting::{RuneScriptSource, RuneScriptingPlugin};
use wgpu_cube::{run_application, DefaultUI, RenderApplication};

use camera::EditorCameraController;
use history::{EditorHistory, HistorySelection};
use layout::{create_editor_layout, EditorBehavior, EditorPane, ViewportState};
use postprocess::ViewportGrid;
use windows::WindowToggles;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_application(EditorApplication::new())?;
    Ok(())
}

struct EditorApplication {
    dock_tree: Tree<EditorPane>,
    scene_viewport: ViewportState,
    game_viewport: ViewportState,
    camera_controller: EditorCameraController,
    grid_postprocess: Option<ViewportGrid>,
    pending_imports: Vec<PathBuf>,
    pending_entity_deletions: Vec<Entity>,
    windows: WindowToggles,
    selected_entity: Option<Entity>,
    highlighted_entity: Option<Entity>,
    pending_pick: Option<ViewportPick>,
    selection_override: Option<Option<Entity>>,
    runtime_state: RuntimeStateHandle,
    last_runtime_mode: RuntimeMode,
    transform_gizmo_mode: TransformGizmoMode,
    transform_gizmo_space: TransformGizmoSpace,
    scene_pointer_uv: Option<Vec2>,
    gizmo_drag: Option<GizmoDragState>,
    pointer_primary_down: bool,
    pointer_press_uv: Option<Vec2>,
    selection_press_uv: Option<Vec2>,
    history: EditorHistory,
    next_editor_entity_id: u128,
    pending_undo: bool,
    pending_redo: bool,
}

struct ViewportPick {
    uv: Vec2,
}

struct GizmoDragState {
    entity: Entity,
    handle: TransformGizmoHandle,
    initial_local: Transform,
    parent_world: Transform,
    initial_world: Transform,
    last_pointer_uv: Vec2,
    any_change: bool,
    kind: GizmoDragKind,
}

enum GizmoDragKind {
    TranslateAxis {
        axis: TransformGizmoAxis,
        axis_dir: Vec3,
        origin: Vec3,
        start_param: f32,
    },
    TranslatePlane {
        plane_normal: Vec3,
        origin: Vec3,
        start_point: Vec3,
    },
    Rotate {
        axis_dir: Vec3,
        origin: Vec3,
        start_vector: Vec3,
    },
    ScaleAxis {
        axis: TransformGizmoAxis,
        axis_dir: Vec3,
        origin: Vec3,
        start_param: f32,
        initial_scale: Vec3,
    },
    ScaleUniform {
        plane_normal: Vec3,
        origin: Vec3,
        right: Vec3,
        up: Vec3,
        base_scale: f32,
        initial_scale: Vec3,
    },
}

const LIGHT_ICON_SCREEN_FRACTION: f32 = 0.055;
const LIGHT_ICON_MIN_DISTANCE: f32 = 0.1;
const LIGHT_ICON_ORTHO_WORLD_SIZE: f32 = 0.65;
const LIGHT_ICON_PICK_PADDING: f32 = 0.15;

impl EditorApplication {
    fn new() -> Self {
        Self {
            dock_tree: create_editor_layout(),
            scene_viewport: ViewportState::default(),
            game_viewport: ViewportState::default(),
            camera_controller: EditorCameraController::default(),
            grid_postprocess: None,
            pending_imports: Vec::new(),
            pending_entity_deletions: Vec::new(),
            windows: WindowToggles::new(),
            selected_entity: None,
            highlighted_entity: None,
            pending_pick: None,
            selection_override: None,
            runtime_state: RuntimeStateHandle::new(),
            last_runtime_mode: RuntimeMode::Editor,
            transform_gizmo_mode: TransformGizmoMode::Translate,
            transform_gizmo_space: TransformGizmoSpace::Local,
            scene_pointer_uv: None,
            gizmo_drag: None,
            pointer_primary_down: false,
            pointer_press_uv: None,
            selection_press_uv: None,
            history: EditorHistory::new(),
            next_editor_entity_id: 1,
            pending_undo: false,
            pending_redo: false,
        }
    }

    fn set_runtime_state_handle(&mut self, handle: RuntimeStateHandle) {
        self.runtime_state = handle;
    }

    fn find_pane_tile(&self, pane: EditorPane) -> Option<TileId> {
        self.dock_tree
            .tiles
            .iter()
            .find_map(|(id, tile)| match tile {
                Tile::Pane(current) if *current == pane => Some(*id),
                _ => None,
            })
    }

    fn ensure_viewport_tab_for_mode(&mut self, mode: RuntimeMode) {
        let target = match mode {
            RuntimeMode::Editor => EditorPane::SceneViewport,
            RuntimeMode::Playing => EditorPane::GameViewport,
        };

        if let Some(tile_id) = self.find_pane_tile(target) {
            let _ = self.dock_tree.make_active(|id, _| id == tile_id);
        }
    }

    fn load_script_text(path: &str) -> Option<String> {
        let full_path = PathBuf::from("scripts").join(path);
        match fs::read_to_string(&full_path) {
            Ok(src) => Some(src),
            Err(err) => {
                error!("Failed to read script {:?}: {err}", full_path);
                None
            }
        }
    }

    fn load_script(path: &str) -> Option<RuneScriptSource> {
        Self::load_script_text(path).map(|src| RuneScriptSource::inline(path, src))
    }

    fn create_import_script(path: &Path) -> Option<RuneScriptSource> {
        let template = Self::load_script_text("editor_import_gltf.rn")?;
        let path_string = path.to_string_lossy();
        let encoded_path = match serde_json::to_string(path_string.as_ref()) {
            Ok(value) => value,
            Err(err) => {
                error!("Failed to encode glTF path {path:?}: {err}");
                return None;
            }
        };

        let script_source = template.replace("__GLTF_PATH__", &encoded_path);

        let script_name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "imported_gltf".to_string());

        Some(RuneScriptSource::inline(
            format!("editor_import_gltf::{script_name}"),
            script_source,
        ))
    }

    fn process_pending_imports(&mut self, ctx: &mut UpdateContext) {
        if self.pending_imports.is_empty() {
            return;
        }

        let imports = std::mem::take(&mut self.pending_imports);
        let mut any_spawned = false;
        for path in imports {
            let Some(script_source) = Self::create_import_script(&path) else {
                continue;
            };

            let entity_name = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "Imported glTF".to_string());

            let mut builder = EntityBuilder::new(ctx.scene.main_world_mut())
                .with_name(format!("{entity_name} (glTF)"))
                .with_transform(Transform::default())
                .with_script(script_source);
            builder.spawn();
            any_spawned = true;
        }

        if matches!(ctx.runtime, RuntimeMode::Editor) {
            ctx.scene.set_animation_playback(false);
            ctx.scene.update(0.0);
        }

        if any_spawned {
            self.record_scene_change(ctx.scene);
        }
    }

    fn process_pending_entity_deletions(&mut self, ctx: &mut UpdateContext) {
        if self.pending_entity_deletions.is_empty() {
            return;
        }

        let pending = std::mem::take(&mut self.pending_entity_deletions);
        let mut removed_entities = Vec::new();

        {
            let world = ctx.scene.main_world_mut();
            for entity in pending {
                if let Some(mut removed) = Self::remove_entity_subtree(world, entity) {
                    removed_entities.append(&mut removed);
                }
            }
        }

        if removed_entities.is_empty() {
            return;
        }

        if let Some(selected) = self.selected_entity {
            if removed_entities.iter().any(|&entity| entity == selected) {
                self.selected_entity = None;
            }
        }

        if let Some(highlighted) = self.highlighted_entity {
            if removed_entities.iter().any(|&entity| entity == highlighted) {
                self.highlighted_entity = None;
            }
        }

        if self
            .gizmo_drag
            .as_ref()
            .is_some_and(|drag| removed_entities.iter().any(|&entity| entity == drag.entity))
        {
            self.gizmo_drag = None;
        }

        self.selection_override = Some(self.selected_entity);
        ctx.scene.propagate_transforms();
        self.record_scene_change(ctx.scene);
    }

    fn remove_entity_subtree(world: &mut hecs::World, root: Entity) -> Option<Vec<Entity>> {
        if !world.contains(root) {
            return None;
        }

        let parent_entity = world.get::<&Parent>(root).map(|parent| parent.0).ok();
        if let Some(parent_entity) = parent_entity {
            let mut remove_children_component = false;
            if let Ok(mut siblings) = world.get::<&mut Children>(parent_entity) {
                siblings.0.retain(|&child| child != root);
                remove_children_component = siblings.0.is_empty();
            }
            if remove_children_component {
                let _ = world.remove_one::<Children>(parent_entity);
            }
        }

        let mut entities = Vec::new();
        let mut stack = vec![root];

        while let Some(entity) = stack.pop() {
            if !world.contains(entity) {
                continue;
            }

            if let Ok(children) = world.get::<&Children>(entity) {
                stack.extend(children.0.iter().copied());
            }

            entities.push(entity);
        }

        for entity in entities.iter().rev() {
            let _ = world.despawn(*entity);
        }

        Some(entities)
    }

    fn sync_selection_component(&mut self, ctx: &mut UpdateContext) {
        if self.selected_entity == self.highlighted_entity {
            if let Some(entity) = self.selected_entity {
                let missing_marker = ctx
                    .scene
                    .main_world()
                    .get::<&SelectedInEditor>(entity)
                    .is_err();

                if missing_marker {
                    if let Err(err) = ctx
                        .scene
                        .main_world_mut()
                        .insert_one(entity, SelectedInEditor)
                    {
                        warn!(
                            "failed to reapply editor selection marker to {:?}: {err}",
                            entity
                        );
                        self.selected_entity = None;
                        self.highlighted_entity = None;
                    }
                }
            }
            return;
        }

        let mut new_highlight = None;
        {
            let world = ctx.scene.main_world_mut();

            if let Some(previous) = self.highlighted_entity.take() {
                let _ = world.remove_one::<SelectedInEditor>(previous);
            }

            if let Some(entity) = self.selected_entity {
                match world.insert_one(entity, SelectedInEditor) {
                    Ok(()) => new_highlight = Some(entity),
                    Err(err) => {
                        warn!("failed to mark entity {:?} as selected: {err}", entity);
                        self.selected_entity = None;
                    }
                }
            }
        }

        self.highlighted_entity = new_highlight;
        self.update_history_selection(ctx.scene);
    }

    fn capture_viewport_pick_input(&mut self, ctx: &egui::Context) {
        if matches!(self.runtime_state.active_mode(), RuntimeMode::Playing) {
            self.pending_pick = None;
            self.pointer_primary_down = false;
            self.pointer_press_uv = None;
            self.selection_press_uv = None;
            return;
        }

        if self.camera_controller.is_looking() {
            self.pointer_primary_down = false;
            self.pointer_press_uv = None;
            self.selection_press_uv = None;
            return;
        }

        let Some(rect) = self.scene_viewport.rect() else {
            self.pointer_primary_down = false;
            self.pointer_press_uv = None;
            self.selection_press_uv = None;
            return;
        };
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            self.pointer_primary_down = false;
            self.pointer_press_uv = None;
            self.selection_press_uv = None;
            return;
        }

        let mut pressed_uv: Option<Vec2> = None;
        let mut released_uv: Option<Vec2> = None;
        let mut pointer_down = false;

        ctx.input(|input| {
            pointer_down = input.pointer.button_down(egui::PointerButton::Primary);

            if input.pointer.button_pressed(egui::PointerButton::Primary) {
                if let Some(pos) = input.pointer.latest_pos() {
                    if rect.contains(pos) {
                        let uv = Self::viewport_uv(rect, pos);
                        if uv.is_finite() {
                            pressed_uv = Some(uv);
                        }
                    }
                }
            }

            if input.pointer.button_released(egui::PointerButton::Primary) {
                if let Some(pos) = input.pointer.latest_pos() {
                    if rect.contains(pos) {
                        let uv = Self::viewport_uv(rect, pos);
                        if uv.is_finite() {
                            released_uv = Some(uv);
                        }
                    }
                }
            }
        });

        self.pointer_primary_down = pointer_down;

        if let Some(uv) = pressed_uv {
            self.pointer_press_uv = Some(uv);
            self.selection_press_uv = Some(uv);
        }

        if let Some(uv) = released_uv {
            if self.gizmo_drag.is_none() && self.selection_press_uv.take().is_some() {
                self.pending_pick = Some(ViewportPick { uv });
            }
        } else if !self.pointer_primary_down {
            self.selection_press_uv = None;
        }
    }

    fn handle_gizmo_shortcuts(&mut self, ctx: &egui::Context) {
        if self.camera_controller.is_looking() {
            return;
        }
        ctx.input_mut(|input| {
            if input.consume_key(egui::Modifiers::NONE, egui::Key::W) {
                self.transform_gizmo_mode = TransformGizmoMode::Translate;
            } else if input.consume_key(egui::Modifiers::NONE, egui::Key::E) {
                self.transform_gizmo_mode = TransformGizmoMode::Rotate;
            } else if input.consume_key(egui::Modifiers::NONE, egui::Key::R) {
                self.transform_gizmo_mode = TransformGizmoMode::Scale;
            }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Delete) {
                if let Some(entity) = self.selected_entity {
                    self.pending_entity_deletions.push(entity);
                    self.gizmo_drag = None;
                }
            }
        });
    }

    fn handle_history_shortcuts(&mut self, ctx: &egui::Context) {
        if self.camera_controller.is_looking() {
            return;
        }

        ctx.input_mut(|input| {
            let undo_shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Z);
            if input.consume_shortcut(&undo_shortcut) {
                self.pending_undo = true;
            }

            let mut redo_mods = egui::Modifiers::COMMAND;
            redo_mods.shift = true;
            let redo_shortcut = egui::KeyboardShortcut::new(redo_mods, egui::Key::Z);
            let redo_variants = [
                redo_shortcut,
                egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Y),
            ];
            if redo_variants
                .iter()
                .any(|shortcut| input.consume_shortcut(shortcut))
            {
                self.pending_redo = true;
            }
        });
    }

    fn process_viewport_pick(&mut self, ctx: &mut UpdateContext) {
        if !matches!(ctx.runtime, RuntimeMode::Editor) {
            self.pending_pick = None;
            return;
        };

        let Some(request) = self.pending_pick.take() else {
            return;
        };

        let Some(region) = self.scene_viewport.region() else {
            self.selected_entity = None;
            self.selection_override = Some(None);
            self.update_history_selection(ctx.scene);
            return;
        };

        let picked = self.pick_entity(ctx, request.uv, region);
        self.selected_entity = picked;
        self.selection_override = Some(picked);
        self.update_history_selection(ctx.scene);
    }

    fn update_gizmo_drag(&mut self, ctx: &mut UpdateContext) {
        if !matches!(ctx.runtime, RuntimeMode::Editor) {
            self.gizmo_drag = None;
            self.pointer_press_uv = None;
            return;
        }

        if let Some(uv) = self.pointer_press_uv.take() {
            if self.try_begin_gizmo_drag(ctx, uv) {
                self.selection_press_uv = None;
            }
        }

        let mut transforms_dirty = false;
        let mut end_drag = false;

        if let Some(drag) = self.gizmo_drag.as_mut() {
            if !self.pointer_primary_down {
                end_drag = true;
            } else if let Some(region) = self.scene_viewport.region() {
                let width = region.width().max(1) as f32;
                let height = region.height().max(1) as f32;
                if width > 0.0 && height > 0.0 {
                    let aspect = width / height;
                    let camera = ctx.scene.camera();
                    let uv = self
                        .scene_pointer_uv
                        .filter(|uv| uv.is_finite())
                        .unwrap_or(drag.last_pointer_uv);
                    let (origin, direction) = Self::ray_from_uv(camera, uv, aspect);
                    match Self::apply_gizmo_drag(ctx, drag, origin, direction) {
                        Ok(updated) => {
                            if updated {
                                drag.last_pointer_uv = uv;
                                drag.any_change = true;
                                transforms_dirty = true;
                            }
                        }
                        Err(_) => {
                            end_drag = true;
                        }
                    }
                }
            }
        }

        if transforms_dirty {
            ctx.scene.propagate_transforms();
        }

        let mut record_history = false;
        if end_drag {
            if let Some(drag) = self.gizmo_drag.take() {
                record_history = drag.any_change;
            }
        }

        if record_history {
            self.record_scene_change(ctx.scene);
        }
    }

    fn try_begin_gizmo_drag(&mut self, ctx: &mut UpdateContext, press_uv: Vec2) -> bool {
        let Some(entity) = self.selected_entity else {
            return false;
        };

        let Some(region) = self.scene_viewport.region() else {
            return false;
        };
        let width = region.width().max(1) as f32;
        let height = region.height().max(1) as f32;
        if width <= 0.0 || height <= 0.0 {
            return false;
        }
        let aspect = width / height;

        let camera = *ctx.scene.camera();
        let (origin, direction) = Self::ray_from_uv(&camera, press_uv, aspect);
        let Some(handle) = ctx.scene.transform_gizmo_hit(origin, direction) else {
            return false;
        };

        {
            let world = ctx.scene.main_world();
            if world.entity(entity).is_err() {
                return false;
            }
        }

        ctx.scene.propagate_transforms();

        let (initial_local, parent_world, initial_world_opt) = {
            let world = ctx.scene.main_world();
            let local = world
                .get::<&TransformComponent>(entity)
                .map(|c| c.0)
                .unwrap_or(Transform::IDENTITY);
            let parent_world = world
                .get::<&Parent>(entity)
                .ok()
                .and_then(|parent| world.get::<&WorldTransform>(parent.0).ok())
                .map(|wt| wt.0)
                .unwrap_or(Transform::IDENTITY);
            let world_transform = world.get::<&WorldTransform>(entity).ok().map(|wt| wt.0);
            (local, parent_world, world_transform)
        };

        let initial_world =
            initial_world_opt.unwrap_or_else(|| parent_world.mul_transform(&initial_local));

        let mut camera_forward = camera.target - camera.eye;
        camera_forward = Self::safe_normalize(camera_forward, Vec3::NEG_Z);
        let mut camera_up = camera.up;
        camera_up = Self::safe_normalize(camera_up, Vec3::Y);

        let origin_point = initial_world.translation;
        let gizmo_rotation = match self.transform_gizmo_space {
            TransformGizmoSpace::Local => initial_world.rotation,
            TransformGizmoSpace::World => Quat::IDENTITY,
        };
        let kind = match handle {
            TransformGizmoHandle::TranslateAxis(axis) => {
                let axis_dir = Self::axis_direction(gizmo_rotation, axis);
                let mut start_param =
                    Self::ray_axis_parameter(origin, direction, origin_point, axis_dir)
                        .unwrap_or(0.0);
                if start_param.abs() < 1e-4 {
                    let Some(plane_normal) =
                        Self::translation_plane_normal(axis_dir, camera_forward, camera_up)
                    else {
                        return false;
                    };
                    let Some(point) =
                        Self::ray_plane_intersection(origin, direction, origin_point, plane_normal)
                    else {
                        return false;
                    };
                    start_param = (point - origin_point).dot(axis_dir);
                }
                if start_param.abs() < 1e-4 {
                    start_param = 1.0;
                }
                GizmoDragKind::TranslateAxis {
                    axis,
                    axis_dir,
                    origin: origin_point,
                    start_param,
                }
            }
            TransformGizmoHandle::TranslatePlane(axis_a, axis_b) => {
                let axis_a_dir = Self::axis_direction(gizmo_rotation, axis_a);
                let axis_b_dir = Self::axis_direction(gizmo_rotation, axis_b);
                let mut plane_normal = axis_a_dir.cross(axis_b_dir);
                if plane_normal.length_squared() < 1e-6 {
                    return false;
                }
                plane_normal = plane_normal.normalize();
                let Some(point) =
                    Self::ray_plane_intersection(origin, direction, origin_point, plane_normal)
                else {
                    return false;
                };
                GizmoDragKind::TranslatePlane {
                    plane_normal,
                    origin: origin_point,
                    start_point: point,
                }
            }
            TransformGizmoHandle::TranslateCenter => {
                let plane_normal = camera_forward;
                let Some(point) =
                    Self::ray_plane_intersection(origin, direction, origin_point, plane_normal)
                else {
                    return false;
                };
                GizmoDragKind::TranslatePlane {
                    plane_normal,
                    origin: origin_point,
                    start_point: point,
                }
            }
            TransformGizmoHandle::RotateAxis(axis) => {
                let axis_dir = Self::axis_direction(gizmo_rotation, axis);
                let Some(point) =
                    Self::ray_plane_intersection(origin, direction, origin_point, axis_dir)
                else {
                    return false;
                };
                let start_vector = point - origin_point;
                if start_vector.length_squared() < 1e-6 {
                    return false;
                }
                GizmoDragKind::Rotate {
                    axis_dir,
                    origin: origin_point,
                    start_vector,
                }
            }
            TransformGizmoHandle::RotateScreen => {
                let axis_dir = -camera_forward;
                let Some(point) =
                    Self::ray_plane_intersection(origin, direction, origin_point, axis_dir)
                else {
                    return false;
                };
                let start_vector = point - origin_point;
                if start_vector.length_squared() < 1e-6 {
                    return false;
                }
                GizmoDragKind::Rotate {
                    axis_dir,
                    origin: origin_point,
                    start_vector,
                }
            }
            TransformGizmoHandle::ScaleAxis(axis) => {
                let axis_dir = Self::axis_direction(gizmo_rotation, axis);
                let mut start_param =
                    Self::ray_axis_parameter(origin, direction, origin_point, axis_dir)
                        .unwrap_or(0.0);

                if start_param.abs() < 1e-4 {
                    let plane_normal =
                        match Self::translation_plane_normal(axis_dir, camera_forward, camera_up) {
                            Some(normal) => normal,
                            None => return false,
                        };
                    let Some(point) =
                        Self::ray_plane_intersection(origin, direction, origin_point, plane_normal)
                    else {
                        return false;
                    };
                    start_param = (point - origin_point).dot(axis_dir);
                    if start_param.abs() < 1e-4 {
                        start_param = 1.0;
                    }
                }
                if start_param.abs() < 1e-4 {
                    start_param = 1.0;
                }

                GizmoDragKind::ScaleAxis {
                    axis,
                    axis_dir,
                    origin: origin_point,
                    start_param,
                    initial_scale: initial_world.scale,
                }
            }
            TransformGizmoHandle::ScaleUniform => {
                let plane_normal = camera_forward;
                let right = {
                    let r = camera_forward.cross(camera_up);
                    if r.length_squared() < 1e-6 {
                        Vec3::X
                    } else {
                        r.normalize()
                    }
                };
                let up = {
                    let u = right.cross(camera_forward);
                    if u.length_squared() < 1e-6 {
                        camera_up
                    } else {
                        u.normalize()
                    }
                };
                let base_scale = Self::gizmo_screen_scale(&camera, origin_point);
                GizmoDragKind::ScaleUniform {
                    plane_normal,
                    origin: origin_point,
                    right,
                    up,
                    base_scale,
                    initial_scale: initial_world.scale,
                }
            }
        };

        self.gizmo_drag = Some(GizmoDragState {
            entity,
            handle,
            initial_local,
            parent_world,
            initial_world,
            last_pointer_uv: press_uv,
            any_change: false,
            kind,
        });

        true
    }

    fn apply_gizmo_drag(
        ctx: &mut UpdateContext,
        drag: &mut GizmoDragState,
        ray_origin: Vec3,
        ray_dir: Vec3,
    ) -> Result<bool, ()> {
        let mut new_world = drag.initial_world;
        let mut updated = false;

        match &mut drag.kind {
            GizmoDragKind::TranslateAxis {
                axis_dir,
                origin,
                start_param,
                ..
            } => {
                let Some(param) = Self::ray_axis_parameter(ray_origin, ray_dir, *origin, *axis_dir)
                else {
                    return Ok(false);
                };
                let delta = param - *start_param;
                if delta.is_finite() {
                    new_world.translation = drag.initial_world.translation + *axis_dir * delta;
                    updated = true;
                }
            }
            GizmoDragKind::TranslatePlane {
                plane_normal,
                origin,
                start_point,
            } => {
                let Some(point) =
                    Self::ray_plane_intersection(ray_origin, ray_dir, *origin, *plane_normal)
                else {
                    return Ok(false);
                };
                let delta = point - *start_point;
                if delta.is_finite() {
                    new_world.translation = drag.initial_world.translation + delta;
                    updated = true;
                }
            }
            GizmoDragKind::Rotate {
                axis_dir,
                origin,
                start_vector,
            } => {
                let Some(point) =
                    Self::ray_plane_intersection(ray_origin, ray_dir, *origin, *axis_dir)
                else {
                    return Ok(false);
                };
                let current_vector = point - *origin;
                let Some(angle) = Self::signed_angle(*start_vector, current_vector, *axis_dir)
                else {
                    return Ok(false);
                };
                let rotation = Quat::from_axis_angle(*axis_dir, angle);
                new_world.rotation = rotation * drag.initial_world.rotation;
                updated = true;
            }
            GizmoDragKind::ScaleAxis {
                axis,
                axis_dir,
                origin,
                start_param,
                initial_scale,
            } => {
                let Some(param) = Self::ray_axis_parameter(ray_origin, ray_dir, *origin, *axis_dir)
                else {
                    return Ok(false);
                };
                let mut ratio = if start_param.abs() < 1e-4 {
                    1.0 + (param - *start_param)
                } else {
                    param / *start_param
                };
                if !ratio.is_finite() {
                    return Ok(false);
                }
                if ratio.abs() < 0.01 {
                    ratio = 0.01 * ratio.signum();
                    if !ratio.is_finite() || ratio == 0.0 {
                        ratio = 0.01;
                    }
                }

                let mut scale = *initial_scale;
                match axis {
                    TransformGizmoAxis::X => scale.x = initial_scale.x * ratio,
                    TransformGizmoAxis::Y => scale.y = initial_scale.y * ratio,
                    TransformGizmoAxis::Z => scale.z = initial_scale.z * ratio,
                }

                new_world.scale = scale;
                updated = true;
            }
            GizmoDragKind::ScaleUniform {
                plane_normal,
                origin,
                right,
                up,
                base_scale,
                initial_scale,
            } => {
                let Some(point) =
                    Self::ray_plane_intersection(ray_origin, ray_dir, *origin, *plane_normal)
                else {
                    return Ok(false);
                };
                let delta = point - *origin;
                let right_amt = delta.dot(*right);
                let up_amt = delta.dot(*up);
                let dominant = if right_amt.abs() >= up_amt.abs() {
                    right_amt
                } else {
                    up_amt
                };
                let scale_ref = base_scale.max(1e-3);
                let mut ratio = 1.0 + dominant / scale_ref;
                if !ratio.is_finite() {
                    return Ok(false);
                }
                if ratio.abs() < 0.01 {
                    ratio = 0.01 * ratio.signum();
                    if !ratio.is_finite() || ratio == 0.0 {
                        ratio = 0.01;
                    }
                }
                new_world.scale = *initial_scale * ratio;
                updated = true;
            }
        }

        if !updated {
            return Ok(false);
        }

        let new_local = Self::world_to_local(drag.parent_world, new_world);

        {
            let world = ctx.scene.main_world_mut();
            let updated_existing = {
                if let Ok(mut transform) = world.get::<&mut TransformComponent>(drag.entity) {
                    transform.0 = new_local;
                    true
                } else {
                    false
                }
            };

            if !updated_existing {
                if let Err(err) = world.insert_one(drag.entity, TransformComponent(new_local)) {
                    warn!(
                        "failed to insert TransformComponent for {:?}: {err}",
                        drag.entity
                    );
                    return Err(());
                }
            }
        }

        Ok(true)
    }

    fn pick_entity(&self, ctx: &UpdateContext, uv: Vec2, region: RenderRegion) -> Option<Entity> {
        let width = region.width().max(1) as f32;
        let height = region.height().max(1) as f32;
        let aspect = width / height;
        let camera = ctx.scene.camera();
        let (origin, direction) = Self::ray_from_uv(camera, uv, aspect);

        let world = ctx.scene.main_world();
        let mut best: Option<(Entity, f32)> = None;

        for (entity, (bounds, world_transform, local_transform, visible)) in world
            .query::<(
                &MeshBounds,
                Option<&WorldTransform>,
                Option<&TransformComponent>,
                Option<&Visible>,
            )>()
            .iter()
        {
            if visible.is_some_and(|v| !v.0) {
                continue;
            }

            let transform = world_transform
                .map(|wt| wt.0)
                .or_else(|| local_transform.map(|lt| lt.0))
                .unwrap_or(Transform::IDENTITY);

            let Some(distance) = Self::entity_hit_distance(transform, *bounds, origin, direction)
            else {
                continue;
            };

            match best {
                Some((_, best_distance)) if distance >= best_distance => {}
                _ => best = Some((entity, distance)),
            }
        }

        self.consider_light_picks(
            world,
            camera.eye,
            camera.up,
            camera.fov_y_radians,
            origin,
            direction,
            &mut best,
        );

        best.map(|(entity, _)| entity)
    }

    fn consider_light_picks(
        &self,
        world: &hecs::World,
        camera_eye: Vec3,
        camera_up: Vec3,
        camera_fov_y: f32,
        ray_origin: Vec3,
        ray_dir: Vec3,
        best: &mut Option<(Entity, f32)>,
    ) {
        let mut consider = |entity: Entity, distance: f32| {
            if let Some((_, best_distance)) = best.as_ref() {
                if distance >= *best_distance {
                    return;
                }
            }
            *best = Some((entity, distance));
        };

        for (entity, (_light, world_transform, local_transform)) in world
            .query::<(
                &PointLight,
                Option<&WorldTransform>,
                Option<&TransformComponent>,
            )>()
            .iter()
        {
            let transform = world_transform
                .map(|wt| wt.0)
                .or_else(|| local_transform.map(|lt| lt.0))
                .unwrap_or(Transform::IDENTITY);
            if let Some(distance) = Self::light_icon_hit_distance(
                camera_eye,
                camera_up,
                camera_fov_y,
                transform.translation,
                ray_origin,
                ray_dir,
            ) {
                consider(entity, distance);
            }
        }

        for (entity, (_light, world_transform, local_transform)) in world
            .query::<(
                &SpotLight,
                Option<&WorldTransform>,
                Option<&TransformComponent>,
            )>()
            .iter()
        {
            let transform = world_transform
                .map(|wt| wt.0)
                .or_else(|| local_transform.map(|lt| lt.0))
                .unwrap_or(Transform::IDENTITY);
            if let Some(distance) = Self::light_icon_hit_distance(
                camera_eye,
                camera_up,
                camera_fov_y,
                transform.translation,
                ray_origin,
                ray_dir,
            ) {
                consider(entity, distance);
            }
        }

        for (entity, (_light, world_transform, local_transform)) in world
            .query::<(
                &DirectionalLight,
                Option<&WorldTransform>,
                Option<&TransformComponent>,
            )>()
            .iter()
        {
            let transform = world_transform
                .map(|wt| wt.0)
                .or_else(|| local_transform.map(|lt| lt.0))
                .unwrap_or(Transform::IDENTITY);
            if let Some(distance) = Self::light_icon_hit_distance(
                camera_eye,
                camera_up,
                camera_fov_y,
                transform.translation,
                ray_origin,
                ray_dir,
            ) {
                consider(entity, distance);
            }
        }
    }

    fn viewport_uv(rect: egui::Rect, pos: egui::Pos2) -> Vec2 {
        let width = rect.width();
        let height = rect.height();
        if width <= 0.0 || height <= 0.0 {
            return Vec2::ZERO;
        }

        let local_x = (pos.x - rect.min.x) / width;
        let local_y = (pos.y - rect.min.y) / height;

        if !local_x.is_finite() || !local_y.is_finite() {
            Vec2::ZERO
        } else {
            Vec2::new(local_x.clamp(0.0, 1.0), local_y.clamp(0.0, 1.0))
        }
    }

    fn ray_from_uv(camera: &wgpu_cube::scene::Camera, uv: Vec2, aspect: f32) -> (Vec3, Vec3) {
        let ndc_x = uv.x * 2.0 - 1.0;
        let ndc_y = 1.0 - uv.y * 2.0;

        let view = camera.view();
        let proj = camera.proj(aspect);
        let inv = (proj * view).inverse();

        let near = inv * Vec4::new(ndc_x, ndc_y, -1.0, 1.0);
        let far = inv * Vec4::new(ndc_x, ndc_y, 1.0, 1.0);

        let near_point = if near.w.abs() > f32::EPSILON {
            near.truncate() / near.w
        } else {
            near.truncate()
        };
        let far_point = if far.w.abs() > f32::EPSILON {
            far.truncate() / far.w
        } else {
            far.truncate()
        };

        let origin = camera.eye;
        let mut direction = far_point - origin;
        if direction.length_squared() < 1e-12 {
            direction = (far_point - near_point).normalize_or_zero();
        } else {
            direction = direction.normalize();
        }

        (origin, direction)
    }

    fn entity_hit_distance(
        transform: Transform,
        bounds: MeshBounds,
        origin: Vec3,
        direction: Vec3,
    ) -> Option<f32> {
        let world_matrix = transform.matrix();
        let inverse = world_matrix.inverse();

        let origin_local = (inverse * origin.extend(1.0)).truncate();
        let direction_local = (inverse * direction.extend(0.0)).truncate();

        if !direction_local.is_finite() || direction_local.length_squared() < 1e-12 {
            return None;
        }

        let local_t =
            Self::ray_aabb_intersection(origin_local, direction_local, bounds.min, bounds.max)?;

        let hit_local = origin_local + direction_local * local_t;
        let hit_world = (world_matrix * hit_local.extend(1.0)).truncate();
        let distance = (hit_world - origin).length();

        distance.is_finite().then_some(distance)
    }

    fn ray_aabb_intersection(origin: Vec3, direction: Vec3, min: Vec3, max: Vec3) -> Option<f32> {
        let mut t_min = f32::NEG_INFINITY;
        let mut t_max = f32::INFINITY;

        for axis in 0..3 {
            let o = origin[axis];
            let d = direction[axis];
            let min_bound = min[axis];
            let max_bound = max[axis];

            if d.abs() < 1e-6 {
                if o < min_bound || o > max_bound {
                    return None;
                }
                continue;
            }

            let inv = 1.0 / d;
            let mut t1 = (min_bound - o) * inv;
            let mut t2 = (max_bound - o) * inv;

            if t1 > t2 {
                std::mem::swap(&mut t1, &mut t2);
            }

            t_min = t_min.max(t1);
            t_max = t_max.min(t2);

            if t_min > t_max {
                return None;
            }
        }

        if t_max < 0.0 {
            return None;
        }

        let hit = if t_min >= 0.0 { t_min } else { t_max };
        (hit.is_finite() && hit >= 0.0).then_some(hit)
    }

    fn light_icon_hit_distance(
        camera_eye: Vec3,
        camera_up: Vec3,
        camera_fov_y: f32,
        position: Vec3,
        ray_origin: Vec3,
        ray_dir: Vec3,
    ) -> Option<f32> {
        let icon_scale = Self::light_icon_world_scale(camera_eye, camera_fov_y, position);
        let forward = Self::safe_normalize(camera_eye - position, Vec3::Z);
        let up_hint = Self::safe_normalize(camera_up, Vec3::Y);
        let (right, up) = Self::basis_from_up_forward(up_hint, forward);
        let normal = forward;
        let denom = ray_dir.dot(normal);
        if denom.abs() < 1e-6 {
            return None;
        }

        let to_center = position - ray_origin;
        let t = to_center.dot(normal) / denom;
        if !t.is_finite() || t < 0.0 {
            return None;
        }

        let hit_point = ray_origin + ray_dir * t;
        let offset = hit_point - position;
        let half_extent = 0.5 * icon_scale;
        let padding = half_extent * LIGHT_ICON_PICK_PADDING;
        let limit = half_extent + padding;

        let proj_right = offset.dot(right);
        let proj_up = offset.dot(up);
        if proj_right.abs() <= limit && proj_up.abs() <= limit {
            Some(t)
        } else {
            None
        }
    }

    fn light_icon_world_scale(camera_eye: Vec3, camera_fov_y: f32, position: Vec3) -> f32 {
        let distance = (camera_eye - position)
            .length()
            .max(LIGHT_ICON_MIN_DISTANCE);
        let half_fov = (camera_fov_y * 0.5).max(1e-4);
        if half_fov <= 1e-3 {
            LIGHT_ICON_ORTHO_WORLD_SIZE
        } else {
            let vertical_extent = half_fov.tan();
            2.0 * distance * vertical_extent * LIGHT_ICON_SCREEN_FRACTION
        }
    }

    fn basis_from_up_forward(up_hint: Vec3, forward: Vec3) -> (Vec3, Vec3) {
        let mut right = up_hint.cross(forward);
        if right.length_squared() < 1e-6 {
            right = Vec3::X;
        } else {
            right = right.normalize();
        }

        let mut up = forward.cross(right);
        if up.length_squared() < 1e-6 {
            up = Vec3::Y;
        } else {
            up = up.normalize();
        }

        (right, up)
    }

    fn safe_normalize(vec: Vec3, fallback: Vec3) -> Vec3 {
        if vec.length_squared() < 1e-6 {
            fallback
        } else {
            vec.normalize()
        }
    }

    fn axis_basis(axis: TransformGizmoAxis) -> Vec3 {
        match axis {
            TransformGizmoAxis::X => Vec3::X,
            TransformGizmoAxis::Y => Vec3::Y,
            TransformGizmoAxis::Z => Vec3::Z,
        }
    }

    fn axis_direction(rotation: Quat, axis: TransformGizmoAxis) -> Vec3 {
        let dir = rotation * Self::axis_basis(axis);
        if dir.length_squared() < 1e-6 {
            Self::axis_basis(axis)
        } else {
            dir.normalize()
        }
    }

    fn translation_plane_normal(axis_dir: Vec3, view_dir: Vec3, view_up: Vec3) -> Option<Vec3> {
        let mut normal = axis_dir * axis_dir.dot(view_dir) - view_dir;
        if normal.length_squared() < 1e-6 {
            normal = axis_dir.cross(view_up);
        }
        if normal.length_squared() < 1e-6 {
            let fallback = if axis_dir.x.abs() < 0.9 {
                Vec3::X
            } else {
                Vec3::Y
            };
            normal = axis_dir.cross(fallback);
        }
        if normal.length_squared() < 1e-6 {
            normal = axis_dir.cross(Vec3::Z);
        }
        if normal.length_squared() < 1e-6 {
            return None;
        }
        Some(normal.normalize())
    }

    fn ray_plane_intersection(
        ray_origin: Vec3,
        ray_dir: Vec3,
        plane_origin: Vec3,
        plane_normal: Vec3,
    ) -> Option<Vec3> {
        let denom = ray_dir.dot(plane_normal);
        if denom.abs() < 1e-6 {
            return None;
        }
        let t = (plane_origin - ray_origin).dot(plane_normal) / denom;
        if !t.is_finite() || t < 0.0 {
            return None;
        }
        Some(ray_origin + ray_dir * t)
    }

    fn ray_axis_parameter(
        ray_origin: Vec3,
        ray_dir: Vec3,
        axis_origin: Vec3,
        axis_dir: Vec3,
    ) -> Option<f32> {
        if axis_dir.length_squared() < 1e-6 {
            return None;
        }
        let axis_dir = axis_dir.normalize();
        let a = ray_dir.dot(ray_dir);
        if a < 1e-6 {
            return None;
        }
        let b = ray_dir.dot(axis_dir);
        let w0 = ray_origin - axis_origin;
        let d = ray_dir.dot(w0);
        let e = axis_dir.dot(w0);
        let denom = a - b * b;
        let t = if denom.abs() > 1e-6 {
            (a * e - b * d) / denom
        } else {
            e
        };
        t.is_finite().then_some(t)
    }

    fn gizmo_screen_scale(camera: &wgpu_cube::scene::Camera, position: Vec3) -> f32 {
        let distance = (camera.eye - position).length().max(0.1);
        let half_fov = (camera.fov_y_radians * 0.5).max(1e-4);
        if half_fov <= 1e-3 {
            1.0
        } else {
            2.0 * distance * half_fov.tan() * 0.16
        }
    }

    fn signed_angle(start: Vec3, current: Vec3, axis: Vec3) -> Option<f32> {
        if start.length_squared() < 1e-6
            || current.length_squared() < 1e-6
            || axis.length_squared() < 1e-6
        {
            return None;
        }

        let start_norm = start.normalize();
        let current_norm = current.normalize();
        let axis_norm = axis.normalize();

        let cross = start_norm.cross(current_norm);
        let sin = cross.dot(axis_norm);
        let cos = start_norm.dot(current_norm).clamp(-1.0, 1.0);
        Some(sin.atan2(cos))
    }

    fn world_to_local(parent_world: Transform, world: Transform) -> Transform {
        let parent_matrix = parent_world.matrix();
        let parent_inverse = parent_matrix.inverse();
        let world_matrix = world.matrix();
        let local_matrix = parent_inverse * world_matrix;
        let (scale, rotation, translation) = local_matrix.to_scale_rotation_translation();
        Transform::from_trs(translation, rotation, scale)
    }

    fn ensure_editor_entity_ids(&mut self, scene: &mut wgpu_cube::scene::Scene) {
        let missing: Vec<Entity> = {
            let world = scene.main_world();
            world
                .iter()
                .filter(|entity_ref| entity_ref.get::<&EditorEntityId>().is_none())
                .map(|entity_ref| entity_ref.entity())
                .collect()
        };

        if missing.is_empty() {
            return;
        }

        let world = scene.main_world_mut();
        for entity in missing {
            let id = self.allocate_editor_entity_id();
            let _ = world.insert_one(entity, EditorEntityId(id));
        }
    }

    fn allocate_editor_entity_id(&mut self) -> u128 {
        let id = self.next_editor_entity_id.max(1);
        self.next_editor_entity_id = id.saturating_add(1);
        id
    }

    fn refresh_next_editor_entity_id(&mut self, scene: &wgpu_cube::scene::Scene) {
        let world = scene.main_world();
        let mut max_seen = 0u128;
        for (_, editor_id) in world.query::<&EditorEntityId>().iter() {
            max_seen = max_seen.max(editor_id.0);
        }
        self.next_editor_entity_id = max_seen.saturating_add(1).max(1);
    }

    fn editor_id_for_entity(
        scene: &wgpu_cube::scene::Scene,
        entity: Entity,
    ) -> Option<EditorEntityId> {
        let world = scene.main_world();
        if !world.contains(entity) {
            return None;
        }
        world.get::<&EditorEntityId>(entity).ok().map(|id| *id)
    }

    fn entity_by_editor_id(
        scene: &wgpu_cube::scene::Scene,
        target: EditorEntityId,
    ) -> Option<Entity> {
        scene
            .main_world()
            .query::<&EditorEntityId>()
            .iter()
            .find_map(|(entity, id)| (id.0 == target.0).then_some(entity))
    }

    fn current_selection_ids(
        &self,
        scene: &wgpu_cube::scene::Scene,
    ) -> (Option<EditorEntityId>, Option<EditorEntityId>) {
        let selected = self
            .selected_entity
            .and_then(|entity| Self::editor_id_for_entity(scene, entity));
        let highlighted = self
            .highlighted_entity
            .and_then(|entity| Self::editor_id_for_entity(scene, entity));
        (selected, highlighted)
    }

    fn initialize_history_state(&mut self, scene: &mut wgpu_cube::scene::Scene) {
        self.ensure_editor_entity_ids(scene);
        self.refresh_next_editor_entity_id(scene);
        let (selected, highlighted) = self.current_selection_ids(scene);
        self.history.initialize(scene, selected, highlighted);
    }

    fn record_scene_change(&mut self, scene: &mut wgpu_cube::scene::Scene) {
        self.ensure_editor_entity_ids(scene);
        let (selected, highlighted) = self.current_selection_ids(scene);
        self.history.record_change(scene, selected, highlighted);
    }

    fn update_history_selection(&mut self, scene: &wgpu_cube::scene::Scene) {
        if !self.history.is_initialized() {
            return;
        }
        let (selected, highlighted) = self.current_selection_ids(scene);
        self.history.update_selection(selected, highlighted);
    }

    fn apply_history_selection(
        &mut self,
        scene: &wgpu_cube::scene::Scene,
        selection: HistorySelection,
    ) {
        let selected = selection
            .selected
            .and_then(|id| Self::entity_by_editor_id(scene, id));
        let highlighted = selection
            .highlighted
            .and_then(|id| Self::entity_by_editor_id(scene, id))
            .or(selected);
        self.selected_entity = selected;
        self.highlighted_entity = highlighted;
        self.selection_override = Some(selected);
    }

    fn perform_undo(&mut self, ctx: &mut UpdateContext) {
        self.gizmo_drag = None;
        self.pending_entity_deletions.clear();
        if let Some(selection) = self.history.undo(ctx.scene) {
            self.refresh_next_editor_entity_id(ctx.scene);
            self.apply_history_selection(ctx.scene, selection);
            ctx.scene.propagate_transforms();
            self.sync_selection_component(ctx);
            self.update_history_selection(ctx.scene);
        }
    }

    fn perform_redo(&mut self, ctx: &mut UpdateContext) {
        self.gizmo_drag = None;
        self.pending_entity_deletions.clear();
        if let Some(selection) = self.history.redo(ctx.scene) {
            self.refresh_next_editor_entity_id(ctx.scene);
            self.apply_history_selection(ctx.scene, selection);
            ctx.scene.propagate_transforms();
            self.sync_selection_component(ctx);
            self.update_history_selection(ctx.scene);
        }
    }

    fn ensure_default_scene(&mut self, ctx: &mut StartupContext) {
        let has_editor_cube = {
            ctx.scene
                .main_world()
                .query::<&Name>()
                .iter()
                .any(|(_, name)| name.0 == "Editor Cube")
        };

        if !has_editor_cube {
            let startup_script = Self::load_script("editor_startup.rn");

            if let Some(script) = startup_script {
                let world = ctx.scene.main_world_mut();
                EntityBuilder::new(world)
                    .with_name("Editor Startup Script")
                    .with_script(script)
                    .spawn();

                ctx.scene.update(0.0);
            } else {
                error!("Failed to load editor startup script");
                return;
            }
        }

        let cube_entity = {
            let world = ctx.scene.main_world();
            world
                .query::<&Name>()
                .iter()
                .find(|(_, name)| name.0 == "Editor Cube")
                .map(|(entity, _)| entity)
        };

        if let Some(entity) = cube_entity {
            let missing_mesh = {
                let world = ctx.scene.main_world();
                world.get::<&MeshComponent>(entity).is_err()
            };
            let missing_bounds = {
                let world = ctx.scene.main_world();
                world.get::<&MeshBounds>(entity).is_err()
            };
            let mut cached_bounds = None;
            if missing_mesh {
                let (vertices, indices) = cube_mesh();
                cached_bounds = MeshBounds::from_vertices(&vertices);
                let mesh = ctx.renderer.create_mesh(&vertices, &indices);
                let mesh_handle = ctx.scene.assets.meshes.insert(mesh);
                if let Err(err) = ctx
                    .scene
                    .main_world_mut()
                    .insert_one(entity, MeshComponent(mesh_handle))
                {
                    error!("failed to attach mesh to Editor Cube: {err}");
                }
            }
            if missing_bounds {
                let bounds = cached_bounds
                    .unwrap_or_else(|| MeshBounds::new(Vec3::splat(-0.5), Vec3::splat(0.5)));
                if let Err(err) = ctx.scene.main_world_mut().insert_one(entity, bounds) {
                    error!("failed to attach bounds to Editor Cube: {err}");
                }
            }

            let missing_material = {
                let world = ctx.scene.main_world();
                world.get::<&MaterialComponent>(entity).is_err()
            };
            if missing_material {
                if let Err(err) = ctx
                    .scene
                    .main_world_mut()
                    .insert_one(entity, MaterialComponent(Material::pbr()))
                {
                    error!("failed to attach material to Editor Cube: {err}");
                }
            }

            let missing_visibility = {
                let world = ctx.scene.main_world();
                world.get::<&Visible>(entity).is_err()
            };
            if missing_visibility {
                if let Err(err) = ctx.scene.main_world_mut().insert_one(entity, Visible(true)) {
                    error!("failed to mark Editor Cube visible: {err}");
                }
            }
        }

        if !ctx.scene.has_any_lights() {
            ctx.scene.add_default_lighting();
        }

        let camera = ctx.scene.camera_mut();
        camera.eye = Vec3::new(6.0, 4.0, 6.0);
        camera.target = Vec3::new(0.0, 0.5, 0.0);
        camera.up = Vec3::Y;
    }

    fn show_menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("editor_top_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        if ui.button("Import glTF...").clicked() {
                            ui.close();
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("glTF", &["gltf", "glb"])
                                .pick_file()
                            {
                                self.pending_imports.push(path);
                            }
                        }
                    }

                    #[cfg(target_arch = "wasm32")]
                    {
                        ui.add_enabled(false, egui::Button::new("Import glTF..."));
                    }

                    if ui.button("Exit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        ui.close();
                    }
                });

                ui.menu_button("Window", |ui| {
                    self.windows.window_menu(ui);
                });
            });

            ui.separator();
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Toolbar").strong());
                ui.separator();
                let desired = self.runtime_state.desired_mode();
                let active = self.runtime_state.active_mode();
                let requesting_play = matches!(desired, RuntimeMode::Playing);
                let active_play = matches!(active, RuntimeMode::Playing);

                if ui
                    .add_enabled(!requesting_play, egui::Button::new("▶ Play"))
                    .clicked()
                {
                    self.runtime_state.request_mode(RuntimeMode::Playing);
                }

                ui.add_enabled(false, egui::Button::new("Pause"));

                if ui
                    .add_enabled(requesting_play || active_play, egui::Button::new("⏹ Stop"))
                    .clicked()
                {
                    self.runtime_state.request_mode(RuntimeMode::Editor);
                }

                ui.separator();

                let (status_text, status_color) = match (active_play, requesting_play) {
                    (true, true) => ("Play Mode", egui::Color32::from_rgb(120, 200, 120)),
                    (true, false) => ("Stopping...", egui::Color32::from_rgb(220, 190, 0)),
                    (false, true) => ("Starting...", egui::Color32::from_rgb(220, 190, 0)),
                    (false, false) => ("Editor Mode", egui::Color32::from_gray(180)),
                };

                ui.label(egui::RichText::new(status_text).color(status_color));

                ui.separator();
                ui.label("Space:");
                ui.selectable_value(
                    &mut self.transform_gizmo_space,
                    TransformGizmoSpace::Local,
                    "Local",
                );
                ui.selectable_value(
                    &mut self.transform_gizmo_space,
                    TransformGizmoSpace::World,
                    "World",
                );
            });
        });
    }
}

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
    }

    fn setup(&mut self, ctx: &mut StartupContext) {
        self.ensure_default_scene(ctx);
        self.initialize_history_state(ctx.scene);
    }

    fn update(&mut self, ctx: &mut UpdateContext) {
        if matches!(ctx.runtime, RuntimeMode::Editor) {
            self.camera_controller.update_camera(ctx);
        }
        self.ensure_editor_entity_ids(ctx.scene);

        if self.pending_undo {
            self.pending_undo = false;
            self.pending_redo = false;
            self.perform_undo(ctx);
        } else if self.pending_redo {
            self.pending_redo = false;
            self.perform_redo(ctx);
        }

        ctx.scene
            .set_transform_gizmo_mode(self.transform_gizmo_mode);
        ctx.scene
            .set_transform_gizmo_space(self.transform_gizmo_space);
        self.process_pending_imports(ctx);
        self.process_pending_entity_deletions(ctx);
        self.process_viewport_pick(ctx);
        self.sync_selection_component(ctx);
        self.update_gizmo_drag(ctx);

        let hovered_handle = if let Some(drag) = self.gizmo_drag.as_ref() {
            Some(drag.handle)
        } else if matches!(ctx.runtime, RuntimeMode::Editor) {
            if let (Some(uv), Some(region)) = (self.scene_pointer_uv, self.scene_viewport.region())
            {
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
    }

    fn gpu_update(&mut self, ctx: &mut GpuUpdateContext) {
        ctx.scene.process_pending_gltf_imports(ctx.renderer);
    }

    fn custom_render(&mut self, ctx: &mut CustomRenderContext) {
        if matches!(self.runtime_state.active_mode(), RuntimeMode::Editor) {
            let grid = self
                .grid_postprocess
                .get_or_insert_with(|| ViewportGrid::new(ctx.renderer.get_device()));
            grid.render(ctx);
        }
    }

    fn custom_render_stage(&self) -> CustomRenderStage {
        CustomRenderStage::AfterPostprocess
    }

    fn ui(&mut self, ctx: &egui::Context, default_ui: &mut DefaultUI) {
        self.scene_viewport.clear();
        self.game_viewport.clear();
        self.show_menu_bar(ctx);

        self.windows.show(ctx, default_ui);

        let runtime_mode = self.runtime_state.active_mode();
        if runtime_mode != self.last_runtime_mode {
            self.last_runtime_mode = runtime_mode;
            self.ensure_viewport_tab_for_mode(runtime_mode);
        }

        let dock_tree = &mut self.dock_tree;
        let scene_viewport = &mut self.scene_viewport;
        let game_viewport = &mut self.game_viewport;
        let scene_hierarchy_window = default_ui.scene_hierarchy_window_mut();
        if let Some(selection) = self.selection_override.take() {
            scene_hierarchy_window.set_selected_entity(selection);
        }
        let is_playing = matches!(runtime_mode, RuntimeMode::Playing);
        let transparent_frame =
            egui::Frame::central_panel(&ctx.style()).fill(egui::Color32::TRANSPARENT);
        egui::CentralPanel::default()
            .frame(transparent_frame)
            .show(ctx, |ui| {
                let mut behavior = EditorBehavior {
                    scene_viewport,
                    game_viewport,
                    scene_hierarchy: scene_hierarchy_window,
                    is_playing,
                };
                dock_tree.ui(&mut behavior, ui);
            });

        self.selected_entity = scene_hierarchy_window.selected_entity();

        if is_playing {
            self.camera_controller.set_viewport_rect(None);
        } else {
            self.camera_controller
                .set_viewport_rect(self.scene_viewport.rect());
        }

        self.camera_controller.capture_input(ctx);
        if !is_playing {
            self.capture_viewport_pick_input(ctx);
            self.handle_history_shortcuts(ctx);
            self.handle_gizmo_shortcuts(ctx);
        } else {
            self.pending_pick = None;
        }

        let pointer_uv = if !is_playing && !self.camera_controller.is_looking() {
            self.scene_viewport.rect().and_then(|rect| {
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
        self.scene_pointer_uv = pointer_uv;
    }

    fn show_default_ui(&self) -> bool {
        false
    }

    fn render_region(&self) -> Option<RenderRegion> {
        if matches!(self.runtime_state.active_mode(), RuntimeMode::Playing) {
            self.game_viewport.region()
        } else {
            self.scene_viewport.region()
        }
    }
}
