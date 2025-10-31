use glam::{Quat, Vec2, Vec3};
use hecs::Entity;
use log::warn;
use std::collections::VecDeque;
use wgpu_cube::app::{RuntimeMode, UpdateContext};
use wgpu_cube::scene::{
    EditorEntityId, Parent, Scene, Transform, TransformComponent, TransformGizmoAxis,
    TransformGizmoHandle, TransformGizmoMode, TransformGizmoSpace, WorldTransform,
};

use super::core::EditorApplication;
use super::system::{EditorAppAccess, EditorCommand, EditorContext, EditorSystem};
use crate::history::{EditorHistory, HistorySelection};
use crate::layout::ViewportState;

pub(crate) struct HistorySystem {
    history: EditorHistory,
    next_editor_entity_id: u128,
    transform_tool: TransformToolSystem,
}

impl HistorySystem {
    pub(crate) fn new(history: EditorHistory, transform_tool: TransformToolSystem) -> Self {
        Self {
            history,
            next_editor_entity_id: 1,
            transform_tool,
        }
    }

    pub(crate) fn history(&self) -> &EditorHistory {
        &self.history
    }

    pub(crate) fn history_mut(&mut self) -> &mut EditorHistory {
        &mut self.history
    }

    pub(crate) fn transform_tool_mut(&mut self) -> &mut TransformToolSystem {
        &mut self.transform_tool
    }

    pub(crate) fn gizmo_drag(&self) -> Option<&GizmoDragState> {
        self.transform_tool.gizmo_drag.as_ref()
    }

    pub(crate) fn clear_gizmo_drag(&mut self) {
        self.transform_tool.gizmo_drag = None;
    }

    pub(crate) fn clear_redo(&mut self) {
        // Retained for compatibility; redo requests are routed through explicit
        // command variants and are processed immediately, so there is no
        // deferred redo state to clear here.
    }

    pub(crate) fn reset(&mut self) {
        self.history = EditorHistory::new();
        self.next_editor_entity_id = 1;
        self.clear_gizmo_drag();
    }

    pub(crate) fn initialize_state(
        &mut self,
        scene: &mut Scene,
        selected: Option<Entity>,
        highlighted: Option<Entity>,
    ) {
        self.ensure_editor_entity_ids(scene);
        self.refresh_next_editor_entity_id(scene);
        let (selected_id, highlighted_id) =
            self.current_selection_ids(scene, selected, highlighted);
        self.history.initialize(scene, selected_id, highlighted_id);
    }

    pub(crate) fn record_scene_change(
        &mut self,
        scene: &mut Scene,
        selected: Option<Entity>,
        highlighted: Option<Entity>,
    ) {
        self.ensure_editor_entity_ids(scene);
        let (selected_id, highlighted_id) =
            self.current_selection_ids(scene, selected, highlighted);
        self.history
            .record_change(scene, selected_id, highlighted_id);
    }

    pub(crate) fn update_history_selection(
        &mut self,
        scene: &Scene,
        selected: Option<Entity>,
        highlighted: Option<Entity>,
    ) {
        if !self.history.is_initialized() {
            return;
        }
        let (selected_id, highlighted_id) =
            self.current_selection_ids(scene, selected, highlighted);
        self.history.update_selection(selected_id, highlighted_id);
    }

    fn process_history_commands(&mut self, ctx: &mut EditorContext) {
        use EditorCommand::{DeleteEntity, HistoryCommitTransforms, HistoryRedo, HistoryUndo};

        let mut queue = {
            let queue_ref = ctx.command_queue();
            std::mem::take(queue_ref)
        };

        let mut remaining = VecDeque::new();
        let mut undo_or_redo_occurred = false;

        while let Some(command) = queue.pop_front() {
            match command {
                HistoryUndo => {
                    self.perform_undo(ctx);
                    undo_or_redo_occurred = true;
                }
                HistoryRedo => {
                    self.perform_redo(ctx);
                    undo_or_redo_occurred = true;
                }
                HistoryCommitTransforms => {
                    self.commit_transform_snapshot(ctx);
                }
                other => remaining.push_back(other),
            }
        }

        if undo_or_redo_occurred {
            remaining.retain(|command| !matches!(command, DeleteEntity(_)));
        }

        {
            let queue_ref = ctx.command_queue();
            queue_ref.extend(remaining);
        }
    }

    fn commit_transform_snapshot(&mut self, ctx: &mut EditorContext) {
        let Some(()) = ctx.with_update_app(|app, update_ctx| {
            let (selected, highlighted) = {
                let selection = app.selection_system();
                (selection.selected(), selection.highlighted())
            };
            self.record_scene_change(update_ctx.scene, selected, highlighted);
        }) else {
            return;
        };
    }

    fn ensure_editor_entity_ids(&mut self, scene: &mut Scene) {
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

    fn refresh_next_editor_entity_id(&mut self, scene: &Scene) {
        let world = scene.main_world();
        let mut max_seen = 0u128;
        for (_, editor_id) in world.query::<&EditorEntityId>().iter() {
            max_seen = max_seen.max(editor_id.0);
        }
        self.next_editor_entity_id = max_seen.saturating_add(1).max(1);
    }

    fn current_selection_ids(
        &self,
        scene: &Scene,
        selected: Option<Entity>,
        highlighted: Option<Entity>,
    ) -> (Option<EditorEntityId>, Option<EditorEntityId>) {
        let selected = selected.and_then(|entity| Self::editor_id_for_entity(scene, entity));
        let highlighted = highlighted.and_then(|entity| Self::editor_id_for_entity(scene, entity));
        (selected, highlighted)
    }

    fn apply_history_selection(
        &mut self,
        app: &mut EditorAppAccess,
        scene: &Scene,
        selection: HistorySelection,
    ) {
        let selected = selection
            .selected
            .and_then(|id| Self::entity_by_editor_id(scene, id));
        let highlighted = selection
            .highlighted
            .and_then(|id| Self::entity_by_editor_id(scene, id))
            .or(selected);
        let selection_system = app.selection_system_mut();
        selection_system.set_selected(selected);
        selection_system.set_highlighted(highlighted);
        selection_system.request_override(selected);
    }

    fn editor_id_for_entity(scene: &Scene, entity: Entity) -> Option<EditorEntityId> {
        let world = scene.main_world();
        if !world.contains(entity) {
            return None;
        }
        world.get::<&EditorEntityId>(entity).ok().map(|id| *id)
    }

    fn entity_by_editor_id(scene: &Scene, target: EditorEntityId) -> Option<Entity> {
        scene
            .main_world()
            .query::<&EditorEntityId>()
            .iter()
            .find_map(|(entity, id)| (id.0 == target.0).then_some(entity))
    }

    fn perform_undo(&mut self, ctx: &mut EditorContext) {
        self.clear_gizmo_drag();

        let Some(()) = ctx.with_update_app(|app, update_ctx| {
            if let Some(selection) = self.history.undo(update_ctx.scene) {
                self.refresh_next_editor_entity_id(update_ctx.scene);
                self.apply_history_selection(app, update_ctx.scene, selection);
                update_ctx.scene.propagate_transforms();
                {
                    let selection = app.selection_system_mut();
                    let _ = selection.sync_selection_component(update_ctx);
                }
                let (selected, highlighted) = {
                    let selection = app.selection_system();
                    (selection.selected(), selection.highlighted())
                };
                self.update_history_selection(update_ctx.scene, selected, highlighted);
            }
        }) else {
            return;
        };
    }

    fn perform_redo(&mut self, ctx: &mut EditorContext) {
        self.clear_gizmo_drag();

        let Some(()) = ctx.with_update_app(|app, update_ctx| {
            if let Some(selection) = self.history.redo(update_ctx.scene) {
                self.refresh_next_editor_entity_id(update_ctx.scene);
                self.apply_history_selection(app, update_ctx.scene, selection);
                update_ctx.scene.propagate_transforms();
                {
                    let selection = app.selection_system_mut();
                    let _ = selection.sync_selection_component(update_ctx);
                }
                let (selected, highlighted) = {
                    let selection = app.selection_system();
                    (selection.selected(), selection.highlighted())
                };
                self.update_history_selection(update_ctx.scene, selected, highlighted);
            }
        }) else {
            return;
        };
    }

    fn update_gizmo_drag(&mut self, ctx: &mut EditorContext) {
        let mut record_history = false;
        let Some(()) = ctx.with_update_app(|app, update_ctx| {
            if !matches!(update_ctx.runtime, RuntimeMode::Editor) {
                self.transform_tool.gizmo_drag = None;
                app.selection_system_mut().set_pointer_press_uv(None);
                return;
            }

            let (press_uv, pointer_down, pointer_uv, selected_entity) = {
                let selection = app.selection_system_mut();
                let press_uv = selection.take_pointer_press_uv();
                let pointer_down = selection.pointer_primary_down();
                let pointer_uv = selection.pointer_scene_uv();
                let selected = selection.selected();
                (press_uv, pointer_down, pointer_uv, selected)
            };

            if let (Some(press_uv), Some(entity)) = (press_uv, selected_entity) {
                if self.try_begin_gizmo_drag(app.scene_viewport(), entity, update_ctx, press_uv) {
                    app.selection_system_mut().set_selection_press_uv(None);
                }
            }

            let mut transforms_dirty = false;
            let mut end_drag = false;

            if let Some(drag) = self.transform_tool.gizmo_drag.as_mut() {
                if !pointer_down {
                    end_drag = true;
                } else if let Some(region) = app.scene_viewport().region() {
                    let width = region.width().max(1) as f32;
                    let height = region.height().max(1) as f32;
                    if width > 0.0 && height > 0.0 {
                        let aspect = width / height;
                        let camera = update_ctx.scene.camera();
                        let uv = pointer_uv
                            .filter(|uv| uv.is_finite())
                            .unwrap_or(drag.last_pointer_uv);
                        let (origin, direction) =
                            EditorApplication::ray_from_uv(camera, uv, aspect);
                        match Self::apply_gizmo_drag(update_ctx, drag, origin, direction) {
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
                update_ctx.scene.propagate_transforms();
            }

            if end_drag {
                if let Some(drag) = self.transform_tool.gizmo_drag.take() {
                    record_history = drag.any_change;
                }
            }
        }) else {
            return;
        };

        if record_history {
            ctx.command_queue()
                .push_back(EditorCommand::HistoryCommitTransforms);
        }
    }

    fn try_begin_gizmo_drag(
        &mut self,
        scene_viewport: &ViewportState,
        entity: Entity,
        ctx: &mut UpdateContext,
        press_uv: Vec2,
    ) -> bool {
        let Some(region) = scene_viewport.region() else {
            return false;
        };
        let width = region.width().max(1) as f32;
        let height = region.height().max(1) as f32;
        if width <= 0.0 || height <= 0.0 {
            return false;
        }

        let aspect = width / height;

        let camera = *ctx.scene.camera();
        let (origin, direction) = EditorApplication::ray_from_uv(&camera, press_uv, aspect);
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
        let gizmo_rotation = match self.transform_tool.gizmo_space {
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

        self.transform_tool.gizmo_drag = Some(GizmoDragState {
            entity,
            handle,
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

    pub(crate) fn safe_normalize(vec: Vec3, fallback: Vec3) -> Vec3 {
        if vec.length_squared() < 1e-6 {
            fallback
        } else {
            vec.normalize()
        }
    }

    pub(crate) fn basis_from_up_forward(up_hint: Vec3, forward: Vec3) -> (Vec3, Vec3) {
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
        let half_fov = (camera.fov_y_radians() * 0.5).max(1e-4);
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
}

impl Default for HistorySystem {
    fn default() -> Self {
        Self::new(EditorHistory::new(), TransformToolSystem::default())
    }
}

impl EditorSystem for HistorySystem {
    fn update<'app, 'ctx, 'scene>(&mut self, ctx: &mut EditorContext<'app, 'ctx, 'scene>) {
        if ctx.update_context_mut().is_none() {
            return;
        }

        self.process_history_commands(ctx);

        let _ = ctx.with_update_app(|app, update_ctx| {
            self.ensure_editor_entity_ids(update_ctx.scene);
            update_ctx
                .scene
                .set_transform_gizmo_mode(self.transform_tool.gizmo_mode);
            update_ctx
                .scene
                .set_transform_gizmo_space(self.transform_tool.gizmo_space);
            let (selected, highlighted) = {
                let selection = app.selection_system();
                (selection.selected(), selection.highlighted())
            };
            self.update_history_selection(update_ctx.scene, selected, highlighted);
        });

        self.update_gizmo_drag(ctx);

        self.process_history_commands(ctx);
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

pub(crate) struct TransformToolSystem {
    pub(crate) gizmo_mode: TransformGizmoMode,
    pub(crate) gizmo_space: TransformGizmoSpace,
    pub(crate) gizmo_drag: Option<GizmoDragState>,
}

impl Default for TransformToolSystem {
    fn default() -> Self {
        Self {
            gizmo_mode: TransformGizmoMode::Translate,
            gizmo_space: TransformGizmoSpace::Local,
            gizmo_drag: None,
        }
    }
}

pub(crate) struct GizmoDragState {
    pub(crate) entity: Entity,
    pub(crate) handle: TransformGizmoHandle,
    pub(crate) parent_world: Transform,
    pub(crate) initial_world: Transform,
    pub(crate) last_pointer_uv: Vec2,
    pub(crate) any_change: bool,
    pub(crate) kind: GizmoDragKind,
}

pub(crate) enum GizmoDragKind {
    TranslateAxis {
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
