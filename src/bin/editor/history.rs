use std::collections::VecDeque;

use wgpu_cube::scene::{EditorEntityId, Scene, SceneStateSnapshot};

const DEFAULT_HISTORY_CAPACITY: usize = 64;

pub struct EditorHistory {
    current: Option<HistoryEntry>,
    undo_stack: VecDeque<HistoryEntry>,
    redo_stack: VecDeque<HistoryEntry>,
    capacity: usize,
}

impl EditorHistory {
    pub fn new() -> Self {
        Self {
            current: None,
            undo_stack: VecDeque::new(),
            redo_stack: VecDeque::new(),
            capacity: DEFAULT_HISTORY_CAPACITY,
        }
    }

    pub fn is_initialized(&self) -> bool {
        self.current.is_some()
    }

    pub fn initialize(
        &mut self,
        scene: &Scene,
        selected: Option<EditorEntityId>,
        highlighted: Option<EditorEntityId>,
    ) {
        let snapshot = SceneStateSnapshot::capture(scene);
        self.current = Some(HistoryEntry::new(snapshot, selected, highlighted));
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    pub fn update_selection(
        &mut self,
        selected: Option<EditorEntityId>,
        highlighted: Option<EditorEntityId>,
    ) {
        if let Some(current) = self.current.as_mut() {
            current.set_selection(selected, highlighted);
        }
    }

    pub fn record_change(
        &mut self,
        scene: &Scene,
        selected: Option<EditorEntityId>,
        highlighted: Option<EditorEntityId>,
    ) {
        if !self.is_initialized() {
            self.initialize(scene, selected, highlighted);
            return;
        }

        if let Some(current) = self.current.take() {
            self.push_undo_entry(current);
        }

        let snapshot = SceneStateSnapshot::capture(scene);
        self.current = Some(HistoryEntry::new(snapshot, selected, highlighted));
        self.redo_stack.clear();
    }

    pub fn undo(&mut self, scene: &mut Scene) -> Option<HistorySelection> {
        let current = self.current.take()?;
        let mut entry = self.undo_stack.pop_back()?;
        self.push_redo_entry(current);
        let selection = entry.restore(scene);
        self.current = Some(entry);
        Some(selection)
    }

    pub fn redo(&mut self, scene: &mut Scene) -> Option<HistorySelection> {
        let current = self.current.take()?;
        let mut entry = self.redo_stack.pop_back()?;
        self.push_undo_entry(current);
        let selection = entry.restore(scene);
        self.current = Some(entry);
        Some(selection)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    fn push_undo_entry(&mut self, entry: HistoryEntry) {
        if self.undo_stack.len() == self.capacity {
            self.undo_stack.pop_front();
        }
        self.undo_stack.push_back(entry);
    }

    fn push_redo_entry(&mut self, entry: HistoryEntry) {
        if self.redo_stack.len() == self.capacity {
            self.redo_stack.pop_front();
        }
        self.redo_stack.push_back(entry);
    }
}

#[derive(Clone, Copy, Debug)]
pub struct HistorySelection {
    pub selected: Option<EditorEntityId>,
    pub highlighted: Option<EditorEntityId>,
}

struct HistoryEntry {
    snapshot: Option<SceneStateSnapshot>,
    selected: Option<EditorEntityId>,
    highlighted: Option<EditorEntityId>,
}

impl HistoryEntry {
    fn new(
        snapshot: SceneStateSnapshot,
        selected: Option<EditorEntityId>,
        highlighted: Option<EditorEntityId>,
    ) -> Self {
        Self {
            snapshot: Some(snapshot),
            selected,
            highlighted,
        }
    }

    fn set_selection(
        &mut self,
        selected: Option<EditorEntityId>,
        highlighted: Option<EditorEntityId>,
    ) {
        self.selected = selected;
        self.highlighted = highlighted;
    }

    fn restore(&mut self, scene: &mut Scene) -> HistorySelection {
        let current_camera = *scene.camera();
        let snapshot = self
            .snapshot
            .take()
            .expect("history entry snapshot must be present");
        *scene = snapshot.into_scene();
        scene.set_camera(current_camera);
        let selection = self.selection();
        self.snapshot = Some(SceneStateSnapshot::capture(scene));
        selection
    }

    fn selection(&self) -> HistorySelection {
        HistorySelection {
            selected: self.selected,
            highlighted: self.highlighted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use wgpu::Color;

    #[test]
    fn undo_restores_scene_while_preserving_camera() {
        let mut scene = Scene::new();
        let mut history = EditorHistory::new();
        history.initialize(&scene, None, None);

        let initial_clear_color = scene.environment().clear_color();

        {
            let camera = scene.camera_mut();
            camera.eye = Vec3::new(1.0, 2.0, 3.0);
            camera.target = Vec3::new(-4.0, 0.5, 0.25);
        }
        let moved_camera = *scene.camera();

        scene.environment_mut().set_clear_color(Color {
            r: 0.2,
            g: 0.4,
            b: 0.6,
            a: 1.0,
        });
        history.record_change(&scene, None, None);

        assert_ne!(scene.environment().clear_color(), initial_clear_color);

        history
            .undo(&mut scene)
            .expect("undo should restore previous scene state");

        let restored_camera = scene.camera();
        assert_eq!(restored_camera.eye, moved_camera.eye);
        assert_eq!(restored_camera.target, moved_camera.target);
        assert_eq!(restored_camera.up, moved_camera.up);
        assert_eq!(restored_camera.projection(), moved_camera.projection());
        assert_eq!(scene.environment().clear_color(), initial_clear_color);
    }
}
