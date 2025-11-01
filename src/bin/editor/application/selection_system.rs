use std::collections::HashMap;

use glam::Vec2;
use hecs::Entity;
use log::warn;
use wgpu_cube::app::{GpuUpdateContext, RuntimeMode, UpdateContext};
use wgpu_cube::renderer::RenderRegion;
use wgpu_cube::scene::components::{EditorEntityId, SelectedInEditor};
use wgpu_cube::scene::{entity_for_pick_value, Children, Parent, Scene, SceneHandle};

use super::core::{EditorApplication, ViewportPick};
use super::system::{EditorAppAccess, EditorContext, EditorSystem};

#[derive(Default)]
pub(crate) struct SelectionSystem {
    state: SelectionState,
    scene_states: HashMap<SceneHandle, SelectionState>,
    active_scene: Option<SceneHandle>,
    pointer: PointerState,
    pending_pick: Option<ViewportPick>,
    gpu_pick: GpuPickState,
    active_scene: Option<SceneHandle>,
    scene_states: HashMap<SceneHandle, SelectionState>,
}

pub(crate) struct SelectionDeletionResult {
    pub active_camera_removed: bool,
    pub clear_gizmo_drag: bool,
    pub selection_changed: bool,
}

impl SelectionSystem {
    pub(crate) fn selected(&self) -> Option<Entity> {
        self.state.selected
    }

    pub(crate) fn set_selected(&mut self, entity: Option<Entity>) {
        self.state.set_selected(entity);
    }

    pub(crate) fn highlighted(&self) -> Option<Entity> {
        self.state.highlighted
    }

    pub(crate) fn set_highlighted(&mut self, entity: Option<Entity>) {
        self.state.set_highlighted(entity);
    }

    pub(crate) fn request_override(&mut self, entity: Option<Entity>) {
        self.state.request_override(entity);
    }

    pub(crate) fn take_override(&mut self) -> Option<Option<Entity>> {
        self.state.take_override()
    }

    pub(crate) fn set_active_scene(&mut self, handle: SceneHandle) {
        if self.active_scene == Some(handle) {
            return;
        }

        if let Some(previous) = self.active_scene {
            self.scene_states.insert(previous, self.state.clone());
        }

        self.state = self.scene_states.remove(&handle).unwrap_or_default();
        self.clear_pending_pick();
        self.active_scene = Some(handle);
    }

    pub(crate) fn reset_workspace(&mut self) {
        self.clear_pending_pick();
        self.state = SelectionState::default();
        self.pointer = PointerState::default();
        self.active_scene = None;
        self.scene_states.clear();
    }

    pub(crate) fn clear_pending_pick(&mut self) {
        self.pending_pick = None;
        self.gpu_pick.mark_discard();
    }

    pub(super) fn set_pending_pick(&mut self, pick: ViewportPick) {
        self.pending_pick = Some(pick);
    }

    pub(super) fn take_pending_pick(&mut self) -> Option<ViewportPick> {
        self.pending_pick.take()
    }

    pub(crate) fn set_active_scene(&mut self, handle: SceneHandle) {
        if self.active_scene == Some(handle) {
            return;
        }

        if let Some(previous) = self.active_scene.replace(handle) {
            let previous_state = std::mem::take(&mut self.state);
            self.scene_states.insert(previous, previous_state);
        }

        if let Some(saved) = self.scene_states.remove(&handle) {
            self.state = saved;
        } else {
            self.state = SelectionState::default();
        }
    }

    pub(crate) fn pointer_scene_uv(&self) -> Option<Vec2> {
        self.pointer.scene_uv
    }

    pub(crate) fn set_pointer_scene_uv(&mut self, uv: Option<Vec2>) {
        self.pointer.set_scene_uv(uv);
    }

    pub(crate) fn pointer_primary_down(&self) -> bool {
        self.pointer.primary_down
    }

    pub(crate) fn set_pointer_primary_down(&mut self, down: bool) {
        self.pointer.primary_down = down;
    }

    pub(crate) fn reset_pointer_press(&mut self) {
        self.pointer.reset_press();
    }

    pub(crate) fn take_pointer_press_uv(&mut self) -> Option<Vec2> {
        self.pointer.press_uv.take()
    }

    pub(crate) fn set_pointer_press_uv(&mut self, uv: Option<Vec2>) {
        self.pointer.press_uv = uv;
    }

    pub(crate) fn set_selection_press_uv(&mut self, uv: Option<Vec2>) {
        self.pointer.selection_press_uv = uv;
    }

    pub(crate) fn take_selection_press_uv(&mut self) -> Option<Vec2> {
        self.pointer.selection_press_uv.take()
    }

    pub(crate) fn process_pending_entity_deletions(
        &mut self,
        ctx: &mut UpdateContext,
        pending: Vec<Entity>,
        active_camera: Option<Entity>,
        gizmo_drag_entity: Option<Entity>,
    ) -> Option<SelectionDeletionResult> {
        if pending.is_empty() {
            return None;
        }

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
            return None;
        }

        let mut active_camera_removed = false;
        let mut clear_gizmo_drag = false;
        let mut selection_changed = false;

        if let Some(active) = active_camera {
            active_camera_removed = removed_entities.contains(&active);
            if active_camera_removed {
                ctx.scene.set_active_camera_entity(None);
            }
        }

        if let Some(drag_entity) = gizmo_drag_entity {
            clear_gizmo_drag = removed_entities.contains(&drag_entity);
        }

        if self
            .selected()
            .is_some_and(|entity| removed_entities.contains(&entity))
        {
            self.set_selected(None);
            selection_changed = true;
        }

        if self
            .highlighted()
            .is_some_and(|entity| removed_entities.contains(&entity))
        {
            self.set_highlighted(None);
            selection_changed = true;
        }

        let current_selected = self.selected();
        self.request_override(current_selected);
        ctx.scene.propagate_transforms();

        Some(SelectionDeletionResult {
            active_camera_removed,
            clear_gizmo_drag,
            selection_changed,
        })
    }

    pub(crate) fn clear_selection(&mut self) {
        self.clear_pending_pick();
        self.reset_pointer_press();
        self.request_override(None);
        self.set_selected(None);
        self.set_highlighted(None);
        self.gpu_pick.mark_discard();
    }

    fn push_history_selection(&self, app: &mut EditorAppAccess<'_>, scene: &Scene) {
        app.history_system_mut().update_history_selection(
            scene,
            self.selected(),
            self.highlighted(),
        );
    }

    fn process_viewport_pick(
        &mut self,
        app: &mut EditorAppAccess<'_>,
        ctx: &mut UpdateContext<'_>,
    ) {
        if !matches!(ctx.runtime, RuntimeMode::Editor) {
            self.clear_pending_pick();
            return;
        };

        let Some(result) = self.gpu_pick.take_result() else {
            return;
        };

        match result {
            PickCompletion::Gpu(entity) => {
                self.set_selected(entity);
                self.request_override(entity);
                self.push_history_selection(app, ctx.scene);
            }
            PickCompletion::CpuFallback(request) => {
                let entity = app.pick_entity(ctx, request.uv, request.region);
                let entity = entity.and_then(|candidate| {
                    let world = ctx.scene.main_world();
                    match world.get::<&EditorEntityId>(candidate) {
                        Ok(editor_id) => {
                            let pick_value = editor_id.pick_identifier();
                            (pick_value == 0).then_some(candidate)
                        }
                        Err(_) => Some(candidate),
                    }
                });
                self.set_selected(entity);
                self.request_override(entity);
                self.push_history_selection(app, ctx.scene);
            }
        }
    }

    fn try_enqueue_pick(
        &mut self,
        app: &EditorAppAccess<'_>,
        gpu_ctx: &mut GpuUpdateContext<'_>,
        request: ViewportPick,
    ) -> bool {
        let Some(region) = app.scene_viewport().region() else {
            self.gpu_pick.complete(PickCompletion::Gpu(None));
            return false;
        };

        let Some(coords) = Self::framebuffer_coords_from_uv(request.uv, region) else {
            self.gpu_pick.complete(PickCompletion::Gpu(None));
            return false;
        };

        if gpu_ctx.renderer.request_pick(coords) {
            self.gpu_pick.mark_in_flight();
            self.gpu_pick.record_request(CpuPickRequest {
                uv: request.uv,
                region,
            });
            true
        } else {
            self.pending_pick = Some(request);
            false
        }
    }

    fn framebuffer_coords_from_uv(uv: Vec2, region: RenderRegion) -> Option<[u32; 2]> {
        let width = region.width();
        let height = region.height();

        if width == 0 || height == 0 {
            return None;
        }

        let clamp = |value: f32| value.clamp(0.0, 1.0);
        let scaled_x = clamp(uv.x) * (width as f32 - 1.0);
        let scaled_y = clamp(uv.y) * (height as f32 - 1.0);

        let x = region.x() + scaled_x.round() as u32;
        let y = region.y() + scaled_y.round() as u32;

        Some([x, y])
    }

    pub(crate) fn sync_selection_component(&mut self, ctx: &mut UpdateContext) -> bool {
        let previous_selected = self.selected();
        let previous_highlighted = self.highlighted();
        let previously_synced = self.state.synced_highlighted();
        let mut state_changed = false;
        let desired_selection = self.selected();

        if desired_selection != previously_synced {
            let mut new_highlight = None;
            {
                let world = ctx.scene.main_world_mut();

                if let Some(previous) = previously_synced {
                    let _ = world.remove_one::<SelectedInEditor>(previous);
                    state_changed = true;
                }

                if let Some(entity) = desired_selection {
                    match world.insert_one(entity, SelectedInEditor) {
                        Ok(()) => {
                            new_highlight = Some(entity);
                            state_changed = true;
                        }
                        Err(err) => {
                            warn!("failed to mark entity {:?} as selected: {err}", entity);
                            self.set_selected(None);
                        }
                    }
                }
            }

            if self.selected().is_none() {
                new_highlight = None;
            }

            self.set_highlighted(new_highlight);
            self.state.set_synced_highlighted(new_highlight);
        } else if let Some(entity) = desired_selection {
            let missing_marker = ctx
                .scene
                .main_world()
                .get::<&SelectedInEditor>(entity)
                .is_err();

            if missing_marker {
                match ctx
                    .scene
                    .main_world_mut()
                    .insert_one(entity, SelectedInEditor)
                {
                    Ok(()) => {
                        state_changed = true;
                        self.set_highlighted(Some(entity));
                        self.state.set_synced_highlighted(Some(entity));
                    }
                    Err(err) => {
                        warn!(
                            "failed to reapply editor selection marker to {:?}: {err}",
                            entity
                        );
                        self.set_selected(None);
                        self.set_highlighted(None);
                        self.state.set_synced_highlighted(None);
                        state_changed = true;
                    }
                }
            } else if self.highlighted() != Some(entity) {
                self.set_highlighted(Some(entity));
                self.state.set_synced_highlighted(Some(entity));
                state_changed = true;
            }
        } else {
            if let Some(previous) = previously_synced {
                let _ = ctx
                    .scene
                    .main_world_mut()
                    .remove_one::<SelectedInEditor>(previous);
                state_changed = true;
            }

            if self.highlighted().is_some() {
                self.set_highlighted(None);
                state_changed = true;
            }

            self.state.set_synced_highlighted(None);
        }

        state_changed
            || self.selected() != previous_selected
            || self.highlighted() != previous_highlighted
    }

    fn update_pointer_hover(&mut self, app: &EditorAppAccess<'_>, ui_ctx: &egui::Context) {
        let runtime_state = app.runtime_state();
        let is_playing = matches!(runtime_state.active_mode(), RuntimeMode::Playing);
        if is_playing || app.camera_system().is_looking() {
            self.set_pointer_scene_uv(None);
            return;
        }

        let Some(rect) = app.scene_viewport().rect() else {
            self.set_pointer_scene_uv(None);
            return;
        };

        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            self.set_pointer_scene_uv(None);
            return;
        }

        let pointer_uv = ui_ctx
            .input(|input| input.pointer.hover_pos())
            .and_then(|pos| rect.contains(pos).then_some(pos))
            .map(|pos| EditorApplication::viewport_uv(rect, pos));

        self.set_pointer_scene_uv(pointer_uv);
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
}

impl EditorSystem for SelectionSystem {
    fn update<'app, 'ctx, 'scene>(&mut self, ctx: &mut EditorContext<'app, 'ctx, 'scene>) {
        let Some(()) = ctx.with_update_app(|app, update_ctx| {
            self.set_active_scene(update_ctx.scene_handle);
            self.process_viewport_pick(app, update_ctx);
            let history_changed = self.sync_selection_component(update_ctx);
            if history_changed {
                self.push_history_selection(app, update_ctx.scene);
            }
        }) else {
            return;
        };
    }

    fn gpu_update<'app, 'ctx, 'scene>(&mut self, ctx: &mut EditorContext<'app, 'ctx, 'scene>) {
        let Some(()) = ctx.with_gpu_app(|app, gpu_ctx| {
            self.set_active_scene(gpu_ctx.scene_handle);
            let runtime_state = app.runtime_state();
            if !matches!(runtime_state.active_mode(), RuntimeMode::Editor) {
                self.clear_pending_pick();
                gpu_ctx.renderer.set_pick_active(false);
                return;
            }

            if let Some(value) = gpu_ctx.renderer.poll_pick_result() {
                let scene_ref: &Scene = &*gpu_ctx.scene;
                if value == 0 {
                    if let Some(request) = self.gpu_pick.take_last_request() {
                        self.gpu_pick.complete(PickCompletion::CpuFallback(request));
                    } else {
                        self.gpu_pick.complete(PickCompletion::Gpu(None));
                    }
                } else {
                    self.gpu_pick.take_last_request();
                    let picked = entity_for_pick_value(scene_ref, value);
                    self.gpu_pick.complete(PickCompletion::Gpu(picked));
                }
            }

            let mut issue_pick_pass = false;

            if !self.gpu_pick.in_flight() {
                if let Some(request) = self.take_pending_pick() {
                    issue_pick_pass = self.try_enqueue_pick(app, gpu_ctx, request);
                }
            }

            gpu_ctx.renderer.set_pick_active(issue_pick_pass);
        }) else {
            return;
        };
    }

    fn ui<'app, 'ctx>(&mut self, ctx: &mut EditorContext<'app, 'ctx, 'ctx>) {
        let _ = ctx.with_ui_app(|app, ui_ctx| {
            let runtime_state = app.runtime_state();
            let is_playing = matches!(runtime_state.active_mode(), RuntimeMode::Playing);
            if is_playing {
                self.clear_pending_pick();
                self.reset_pointer_press();
                self.set_pointer_scene_uv(None);
                return;
            }

            self.update_pointer_hover(app, ui_ctx.egui());
        });
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[derive(Clone, Default)]
struct SelectionState {
    selected: Option<Entity>,
    highlighted: Option<Entity>,
    synced_highlighted: Option<Entity>,
    override_request: Option<Option<Entity>>,
}

impl SelectionState {
    fn set_selected(&mut self, entity: Option<Entity>) {
        self.selected = entity;
    }

    fn set_highlighted(&mut self, entity: Option<Entity>) {
        self.highlighted = entity;
    }

    fn synced_highlighted(&self) -> Option<Entity> {
        self.synced_highlighted
    }

    fn set_synced_highlighted(&mut self, entity: Option<Entity>) {
        self.synced_highlighted = entity;
    }

    fn request_override(&mut self, entity: Option<Entity>) {
        self.override_request = Some(entity);
    }

    fn take_override(&mut self) -> Option<Option<Entity>> {
        self.override_request.take()
    }
}

#[derive(Default)]
struct GpuPickState {
    in_flight: bool,
    pending_result: Option<PickCompletion>,
    discard_next_result: bool,
    last_request: Option<CpuPickRequest>,
}

impl GpuPickState {
    fn mark_in_flight(&mut self) {
        self.in_flight = true;
        self.discard_next_result = false;
    }

    fn in_flight(&self) -> bool {
        self.in_flight
    }

    fn complete(&mut self, result: PickCompletion) {
        self.in_flight = false;
        self.last_request = None;
        if self.discard_next_result {
            self.discard_next_result = false;
            return;
        }
        self.pending_result = Some(result);
    }

    fn take_result(&mut self) -> Option<PickCompletion> {
        self.pending_result.take()
    }

    fn mark_discard(&mut self) {
        self.in_flight = false;
        self.pending_result = None;
        self.discard_next_result = true;
        self.last_request = None;
    }

    fn record_request(&mut self, request: CpuPickRequest) {
        self.last_request = Some(request);
    }

    fn take_last_request(&mut self) -> Option<CpuPickRequest> {
        self.last_request.take()
    }
}

#[derive(Clone, Copy)]
struct CpuPickRequest {
    uv: Vec2,
    region: RenderRegion,
}

#[derive(Clone, Copy)]
enum PickCompletion {
    Gpu(Option<Entity>),
    CpuFallback(CpuPickRequest),
}

#[derive(Default)]
struct PointerState {
    scene_uv: Option<Vec2>,
    primary_down: bool,
    press_uv: Option<Vec2>,
    selection_press_uv: Option<Vec2>,
}

impl PointerState {
    fn reset_press(&mut self) {
        self.primary_down = false;
        self.press_uv = None;
        self.selection_press_uv = None;
    }

    fn set_scene_uv(&mut self, uv: Option<Vec2>) {
        self.scene_uv = uv;
    }
}
