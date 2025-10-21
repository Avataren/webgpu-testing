use hecs::Entity;
use log::warn;
use wgpu_cube::app::UpdateContext;
use wgpu_cube::scene::components::SelectedInEditor;
use wgpu_cube::scene::{Children, Parent};

use super::core::EditorApplication;

impl EditorApplication {
    pub(super) fn process_pending_entity_deletions(&mut self, ctx: &mut UpdateContext) {
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

        if let Some(selected) = self.selection.selected() {
            if removed_entities.contains(&selected) {
                self.selection.set_selected(None);
            }
        }

        if let Some(highlighted) = self.selection.highlighted() {
            if removed_entities.contains(&highlighted) {
                self.selection.set_highlighted(None);
            }
        }

        if self
            .transform_tool
            .gizmo_drag
            .as_ref()
            .is_some_and(|drag| removed_entities.contains(&drag.entity))
        {
            self.transform_tool.gizmo_drag = None;
        }

        let current_selected = self.selection.selected();
        self.selection.request_override(current_selected);
        ctx.scene.propagate_transforms();
        self.record_scene_change(ctx.scene);
    }

    pub(super) fn remove_entity_subtree(
        world: &mut hecs::World,
        root: Entity,
    ) -> Option<Vec<Entity>> {
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

    pub(super) fn sync_selection_component(&mut self, ctx: &mut UpdateContext) {
        let current_selected = self.selection.selected();
        let current_highlighted = self.selection.highlighted();
        if current_selected == current_highlighted {
            if let Some(entity) = current_selected {
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
                        self.selection.set_selected(None);
                        self.selection.set_highlighted(None);
                    }
                }
            }
            return;
        }

        let mut new_highlight = None;
        {
            let world = ctx.scene.main_world_mut();

            if let Some(previous) = self.selection.take_highlighted() {
                let _ = world.remove_one::<SelectedInEditor>(previous);
            }

            if let Some(entity) = self.selection.selected() {
                match world.insert_one(entity, SelectedInEditor) {
                    Ok(()) => new_highlight = Some(entity),
                    Err(err) => {
                        warn!("failed to mark entity {:?} as selected: {err}", entity);
                        self.selection.set_selected(None);
                    }
                }
            }
        }

        self.selection.set_highlighted(new_highlight);
        self.update_history_selection(ctx.scene);
    }

    pub(super) fn clear_selection(&mut self) {
        self.selection.clear_pending_pick();
        self.selection.pointer.reset_press();
        self.transform_tool.gizmo_drag = None;
        self.selection.request_override(None);
        self.selection.set_selected(None);
    }
}
