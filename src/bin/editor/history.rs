use std::collections::VecDeque;

use wgpu_cube::scene::{EditorEntityId, Scene, SceneSnapshot};

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
        let snapshot = SceneSnapshot::capture(scene);
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

        let snapshot = SceneSnapshot::capture(scene);
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
    snapshot: Option<SceneSnapshot>,
    selected: Option<EditorEntityId>,
    highlighted: Option<EditorEntityId>,
}

impl HistoryEntry {
    fn new(
        snapshot: SceneSnapshot,
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
        let snapshot = self
            .snapshot
            .take()
            .expect("history entry snapshot must be present");
        *scene = snapshot.into_scene();
        let selection = self.selection();
        self.snapshot = Some(SceneSnapshot::capture(scene));
        selection
    }

    fn selection(&self) -> HistorySelection {
        HistorySelection {
            selected: self.selected,
            highlighted: self.highlighted,
        }
    }
}
