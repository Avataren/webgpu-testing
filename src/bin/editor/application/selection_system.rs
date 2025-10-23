use glam::Vec2;
use hecs::Entity;
use log::warn;
use wgpu_cube::app::{RuntimeMode, UpdateContext};
use wgpu_cube::scene::components::SelectedInEditor;
use wgpu_cube::scene::{Children, Parent};

use super::core::{EditorApplication, ViewportPick};
use super::system::{EditorContext, EditorSystem};

#[derive(Default)]
pub(crate) struct SelectionSystem {
    state: SelectionState,
    pointer: PointerState,
    pending_pick: Option<ViewportPick>,
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

    pub(crate) fn take_highlighted(&mut self) -> Option<Entity> {
        self.state.take_highlighted()
    }

    pub(crate) fn request_override(&mut self, entity: Option<Entity>) {
        self.state.request_override(entity);
    }

    pub(crate) fn take_override(&mut self) -> Option<Option<Entity>> {
        self.state.take_override()
    }

    pub(crate) fn clear_pending_pick(&mut self) {
        self.pending_pick = None;
    }

    pub(super) fn set_pending_pick(&mut self, pick: ViewportPick) {
        self.pending_pick = Some(pick);
    }

    pub(super) fn take_pending_pick(&mut self) -> Option<ViewportPick> {
        self.pending_pick.take()
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
    }

    fn process_viewport_pick(&mut self, app: &mut EditorApplication, ctx: &mut UpdateContext) {
        if !matches!(ctx.runtime, RuntimeMode::Editor) {
            self.clear_pending_pick();
            return;
        };

        let Some(request) = self.take_pending_pick() else {
            return;
        };

        let Some(region) = app.viewports.scene_viewport.region() else {
            self.set_selected(None);
            self.request_override(None);
            app.update_history_selection(ctx.scene);
            return;
        };

        let picked = app.pick_entity(ctx, request.uv, region);
        self.set_selected(picked);
        self.request_override(picked);
        app.update_history_selection(ctx.scene);
    }

    pub(crate) fn sync_selection_component(&mut self, ctx: &mut UpdateContext) -> bool {
        let previous_selected = self.selected();
        let previous_highlighted = self.highlighted();

        if previous_selected == previous_highlighted {
            if let Some(entity) = previous_selected {
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
                        self.set_selected(None);
                        self.set_highlighted(None);
                    }
                }
            }

            return self.selected() != previous_selected
                || self.highlighted() != previous_highlighted;
        }

        let mut new_highlight = None;
        let mut highlight_changed = false;

        {
            let world = ctx.scene.main_world_mut();

            if let Some(previous) = self.take_highlighted() {
                let _ = world.remove_one::<SelectedInEditor>(previous);
                highlight_changed = true;
            }

            if let Some(entity) = self.selected() {
                match world.insert_one(entity, SelectedInEditor) {
                    Ok(()) => {
                        new_highlight = Some(entity);
                        highlight_changed = true;
                    }
                    Err(err) => {
                        warn!("failed to mark entity {:?} as selected: {err}", entity);
                        self.set_selected(None);
                    }
                }
            }
        }

        self.set_highlighted(new_highlight);

        let selection_changed = self.selected() != previous_selected;
        let highlight_changed = highlight_changed || self.highlighted() != previous_highlighted;

        selection_changed || highlight_changed
    }

    fn update_pointer_hover(&mut self, app: &EditorApplication, ui_ctx: &egui::Context) {
        let is_playing = matches!(app.runtime_state.active_mode(), RuntimeMode::Playing);
        if is_playing || app.camera_system().is_looking() {
            self.set_pointer_scene_uv(None);
            return;
        }

        let Some(rect) = app.viewports.scene_viewport.rect() else {
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
        let Some(()) = ctx.with_update(|app, update_ctx| {
            self.process_viewport_pick(app, update_ctx);
            let history_changed = self.sync_selection_component(update_ctx);
            if history_changed {
                app.update_history_selection(update_ctx.scene);
            }
        }) else {
            return;
        };
    }

    fn ui<'app, 'ctx>(&mut self, ctx: &mut EditorContext<'app, 'ctx, 'ctx>) {
        let _ = ctx.with_ui(|app, ui_ctx| {
            let is_playing = matches!(app.runtime_state.active_mode(), RuntimeMode::Playing);
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

#[derive(Default)]
struct SelectionState {
    selected: Option<Entity>,
    highlighted: Option<Entity>,
    override_request: Option<Option<Entity>>,
}

impl SelectionState {
    fn set_selected(&mut self, entity: Option<Entity>) {
        self.selected = entity;
    }

    fn set_highlighted(&mut self, entity: Option<Entity>) {
        self.highlighted = entity;
    }

    fn take_highlighted(&mut self) -> Option<Entity> {
        let current = self.highlighted;
        self.highlighted = None;
        current
    }

    fn request_override(&mut self, entity: Option<Entity>) {
        self.override_request = Some(entity);
    }

    fn take_override(&mut self) -> Option<Option<Entity>> {
        self.override_request.take()
    }
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
