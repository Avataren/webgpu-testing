use hecs::Entity;
use wgpu_cube::app::UpdateContext;
use wgpu_cube::scene::{EditorEntityId, Scene};

use super::core::EditorApplication;
use super::EditorCommand;
use crate::history::HistorySelection;

impl EditorApplication {
    pub(super) fn ensure_editor_entity_ids(&mut self, scene: &mut Scene) {
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

    pub(super) fn allocate_editor_entity_id(&mut self) -> u128 {
        let id = self.next_editor_entity_id.max(1);
        self.next_editor_entity_id = id.saturating_add(1);
        id
    }

    pub(super) fn refresh_next_editor_entity_id(&mut self, scene: &Scene) {
        let world = scene.main_world();
        let mut max_seen = 0u128;
        for (_, editor_id) in world.query::<&EditorEntityId>().iter() {
            max_seen = max_seen.max(editor_id.0);
        }
        self.next_editor_entity_id = max_seen.saturating_add(1).max(1);
    }

    pub(super) fn editor_id_for_entity(scene: &Scene, entity: Entity) -> Option<EditorEntityId> {
        let world = scene.main_world();
        if !world.contains(entity) {
            return None;
        }
        world.get::<&EditorEntityId>(entity).ok().map(|id| *id)
    }

    pub(super) fn entity_by_editor_id(scene: &Scene, target: EditorEntityId) -> Option<Entity> {
        scene
            .main_world()
            .query::<&EditorEntityId>()
            .iter()
            .find_map(|(entity, id)| (id.0 == target.0).then_some(entity))
    }

    pub(super) fn current_selection_ids(
        &self,
        scene: &Scene,
    ) -> (Option<EditorEntityId>, Option<EditorEntityId>) {
        let selection = self.selection_system();
        let selected = selection
            .selected()
            .and_then(|entity| Self::editor_id_for_entity(scene, entity));
        let highlighted = selection
            .highlighted()
            .and_then(|entity| Self::editor_id_for_entity(scene, entity));
        (selected, highlighted)
    }

    pub(super) fn initialize_history_state(&mut self, scene: &mut Scene) {
        self.ensure_editor_entity_ids(scene);
        self.refresh_next_editor_entity_id(scene);
        let (selected, highlighted) = self.current_selection_ids(scene);
        self.history.initialize(scene, selected, highlighted);
    }

    pub(super) fn record_scene_change(&mut self, scene: &mut Scene) {
        self.ensure_editor_entity_ids(scene);
        let (selected, highlighted) = self.current_selection_ids(scene);
        self.history.record_change(scene, selected, highlighted);
    }

    pub(super) fn update_history_selection(&mut self, scene: &Scene) {
        if !self.history.is_initialized() {
            return;
        }
        let (selected, highlighted) = self.current_selection_ids(scene);
        self.history.update_selection(selected, highlighted);
    }

    pub(super) fn apply_history_selection(&mut self, scene: &Scene, selection: HistorySelection) {
        let selected = selection
            .selected
            .and_then(|id| Self::entity_by_editor_id(scene, id));
        let highlighted = selection
            .highlighted
            .and_then(|id| Self::entity_by_editor_id(scene, id))
            .or(selected);
        let selection_system = self.selection_system_mut();
        selection_system.set_selected(selected);
        selection_system.set_highlighted(highlighted);
        selection_system.request_override(selected);
    }

    pub(super) fn perform_undo(&mut self, ctx: &mut UpdateContext) {
        self.transform_tool.gizmo_drag = None;
        self.commands
            .retain(|command| !matches!(command, EditorCommand::DeleteEntity(_)));
        if let Some(selection) = self.history.undo(ctx.scene) {
            self.refresh_next_editor_entity_id(ctx.scene);
            self.apply_history_selection(ctx.scene, selection);
            ctx.scene.propagate_transforms();
            {
                let selection = self.selection_system_mut();
                let _ = selection.sync_selection_component(ctx);
            }
            self.update_history_selection(ctx.scene);
        }
    }

    pub(super) fn perform_redo(&mut self, ctx: &mut UpdateContext) {
        self.transform_tool.gizmo_drag = None;
        self.commands
            .retain(|command| !matches!(command, EditorCommand::DeleteEntity(_)));
        if let Some(selection) = self.history.redo(ctx.scene) {
            self.refresh_next_editor_entity_id(ctx.scene);
            self.apply_history_selection(ctx.scene, selection);
            ctx.scene.propagate_transforms();
            {
                let selection = self.selection_system_mut();
                let _ = selection.sync_selection_component(ctx);
            }
            self.update_history_selection(ctx.scene);
        }
    }
}
