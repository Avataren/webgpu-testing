use std::path::PathBuf;

use egui_tiles::{Tile, TileId, Tree};
use glam::{Vec2, Vec3};
use hecs::Entity;
use wgpu_cube::app::{RuntimeMode, RuntimeStateHandle};
use wgpu_cube::renderer::RenderRegion;
use wgpu_cube::scene::{
    Transform, TransformGizmoAxis, TransformGizmoHandle, TransformGizmoMode, TransformGizmoSpace,
};

use crate::camera::EditorCameraController;
use crate::history::EditorHistory;
use crate::layout::{create_editor_layout, EditorPane, ViewportState};
use crate::postprocess::ViewportGrid;
use crate::project;
use crate::script_editor::ScriptEditorState;
use crate::windows::WindowToggles;

pub struct EditorApplication {
    pub(super) dock_tree: Tree<EditorPane>,
    pub(super) scene_viewport: ViewportState,
    pub(super) game_viewport: ViewportState,
    pub(super) game_view_display: GameViewDisplayMode,
    pub(super) camera_controller: EditorCameraController,
    pub(super) grid_postprocess: Option<ViewportGrid>,
    pub(super) pending_imports: Vec<PathBuf>,
    pub(super) pending_entity_deletions: Vec<Entity>,
    pub(super) windows: WindowToggles,
    pub(super) selection: SelectionState,
    pub(super) pointer: PointerState,
    pub(super) runtime_state: RuntimeStateHandle,
    pub(super) last_runtime_mode: RuntimeMode,
    pub(super) transform_gizmo_mode: TransformGizmoMode,
    pub(super) transform_gizmo_space: TransformGizmoSpace,
    pub(super) gizmo_drag: Option<GizmoDragState>,
    pub(super) history: EditorHistory,
    pub(super) next_editor_entity_id: u128,
    pub(super) undo_redo: UndoRedoState,
    pub(super) project: project::ProjectController,
    pub(super) script_editor: Option<ScriptEditorState>,
    pub(super) pending_script_actions: Vec<PendingScriptAction>,
}

#[derive(Default)]
pub struct EditorApplicationBuilder {
    dock_tree: Option<Tree<EditorPane>>,
    scene_viewport: Option<ViewportState>,
    game_viewport: Option<ViewportState>,
    game_view_display: Option<GameViewDisplayMode>,
    camera_controller: Option<EditorCameraController>,
    grid_postprocess: Option<Option<ViewportGrid>>,
    windows: Option<WindowToggles>,
    project: Option<project::ProjectController>,
    history: Option<EditorHistory>,
}

impl EditorApplicationBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_dock_tree(mut self, dock_tree: Tree<EditorPane>) -> Self {
        self.dock_tree = Some(dock_tree);
        self
    }

    pub fn with_scene_viewport(mut self, viewport: ViewportState) -> Self {
        self.scene_viewport = Some(viewport);
        self
    }

    pub fn with_game_viewport(mut self, viewport: ViewportState) -> Self {
        self.game_viewport = Some(viewport);
        self
    }

    pub fn with_game_view_display(mut self, display: GameViewDisplayMode) -> Self {
        self.game_view_display = Some(display);
        self
    }

    pub fn with_camera_controller(mut self, controller: EditorCameraController) -> Self {
        self.camera_controller = Some(controller);
        self
    }

    pub fn with_grid_postprocess(mut self, grid: Option<ViewportGrid>) -> Self {
        self.grid_postprocess = Some(grid);
        self
    }

    pub fn with_windows(mut self, windows: WindowToggles) -> Self {
        self.windows = Some(windows);
        self
    }

    pub fn with_project(mut self, project: project::ProjectController) -> Self {
        self.project = Some(project);
        self
    }

    pub fn with_history(mut self, history: EditorHistory) -> Self {
        self.history = Some(history);
        self
    }

    pub fn build(self) -> EditorApplication {
        EditorApplication {
            dock_tree: self.dock_tree.unwrap_or_else(create_editor_layout),
            scene_viewport: self.scene_viewport.unwrap_or_default(),
            game_viewport: self.game_viewport.unwrap_or_default(),
            game_view_display: self.game_view_display.unwrap_or_default(),
            camera_controller: self.camera_controller.unwrap_or_default(),
            grid_postprocess: self.grid_postprocess.unwrap_or_default(),
            pending_imports: Vec::new(),
            pending_entity_deletions: Vec::new(),
            windows: self.windows.unwrap_or_else(WindowToggles::new),
            selection: SelectionState::default(),
            pointer: PointerState::default(),
            runtime_state: RuntimeStateHandle::new(),
            last_runtime_mode: RuntimeMode::Editor,
            transform_gizmo_mode: TransformGizmoMode::Translate,
            transform_gizmo_space: TransformGizmoSpace::Local,
            gizmo_drag: None,
            history: self.history.unwrap_or_else(EditorHistory::new),
            next_editor_entity_id: 1,
            undo_redo: UndoRedoState::default(),
            project: self.project.unwrap_or_else(project::ProjectController::new),
            script_editor: None,
            pending_script_actions: Vec::new(),
        }
    }
}

impl EditorApplication {
    pub fn new() -> Self {
        Self::builder()
            .with_dock_tree(create_editor_layout())
            .with_scene_viewport(ViewportState::default())
            .with_game_viewport(ViewportState::default())
            .with_game_view_display(GameViewDisplayMode::default())
            .with_camera_controller(EditorCameraController::default())
            .with_grid_postprocess(None)
            .with_windows(WindowToggles::new())
            .with_project(project::ProjectController::new())
            .with_history(EditorHistory::new())
            .build()
    }

    pub fn builder() -> EditorApplicationBuilder {
        EditorApplicationBuilder::new()
    }

    pub fn set_runtime_state_handle(&mut self, handle: RuntimeStateHandle) {
        self.runtime_state = handle;
    }

    pub(super) fn find_pane_tile(&self, pane: EditorPane) -> Option<TileId> {
        self.dock_tree
            .tiles
            .iter()
            .find_map(|(id, tile)| match tile {
                Tile::Pane(current) if *current == pane => Some(*id),
                _ => None,
            })
    }

    pub(super) fn ensure_viewport_tab_for_mode(&mut self, mode: RuntimeMode) {
        let target = match mode {
            RuntimeMode::Editor => EditorPane::SceneViewport,
            RuntimeMode::Playing => EditorPane::GameViewport,
        };

        if let Some(tile_id) = self.find_pane_tile(target) {
            let _ = self.dock_tree.make_active(|id, _| id == tile_id);
        }
    }

    pub(super) fn render_region_for_mode(&self, mode: RuntimeMode) -> Option<RenderRegion> {
        match mode {
            RuntimeMode::Editor => self.scene_viewport.region(),
            RuntimeMode::Playing => self.game_viewport.region(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum GameViewDisplayMode {
    #[default]
    Viewport,
    Fullscreen,
}

pub(super) enum PendingScriptAction {
    SaveInline {
        entity: Entity,
        name: String,
        contents: String,
        message: String,
    },
    ReloadRuntime {
        entity: Entity,
        message: String,
    },
}

pub(super) struct ViewportPick {
    pub(super) uv: Vec2,
}

pub(super) struct GizmoDragState {
    pub(super) entity: Entity,
    pub(super) handle: TransformGizmoHandle,
    pub(super) parent_world: Transform,
    pub(super) initial_world: Transform,
    pub(super) last_pointer_uv: Vec2,
    pub(super) any_change: bool,
    pub(super) kind: GizmoDragKind,
}

pub(super) enum GizmoDragKind {
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

#[derive(Default)]
pub(super) struct SelectionState {
    pub(super) selected: Option<Entity>,
    pub(super) highlighted: Option<Entity>,
    pub(super) pending_pick: Option<ViewportPick>,
    pub(super) override_request: Option<Option<Entity>>,
}

impl SelectionState {
    pub(super) fn set_selected(&mut self, entity: Option<Entity>) {
        self.selected = entity;
    }

    pub(super) fn set_highlighted(&mut self, entity: Option<Entity>) {
        self.highlighted = entity;
    }

    pub(super) fn request_override(&mut self, entity: Option<Entity>) {
        self.override_request = Some(entity);
    }

    pub(super) fn take_override(&mut self) -> Option<Option<Entity>> {
        self.override_request.take()
    }

    pub(super) fn clear_pending_pick(&mut self) {
        self.pending_pick = None;
    }
}

#[derive(Default)]
pub(super) struct PointerState {
    pub(super) scene_uv: Option<Vec2>,
    pub(super) primary_down: bool,
    pub(super) press_uv: Option<Vec2>,
    pub(super) selection_press_uv: Option<Vec2>,
}

impl PointerState {
    pub(super) fn reset_press(&mut self) {
        self.primary_down = false;
        self.press_uv = None;
        self.selection_press_uv = None;
    }

    pub(super) fn set_scene_uv(&mut self, uv: Option<Vec2>) {
        self.scene_uv = uv;
    }
}

#[derive(Default)]
pub(super) struct UndoRedoState {
    pending_undo: bool,
    pending_redo: bool,
}

impl UndoRedoState {
    pub(super) fn request_undo(&mut self) {
        self.pending_undo = true;
    }

    pub(super) fn request_redo(&mut self) {
        self.pending_redo = true;
    }

    pub(super) fn take_undo(&mut self) -> bool {
        std::mem::take(&mut self.pending_undo)
    }

    pub(super) fn take_redo(&mut self) -> bool {
        std::mem::take(&mut self.pending_redo)
    }

    pub(super) fn clear_redo(&mut self) {
        self.pending_redo = false;
    }
}

#[derive(Clone, Copy)]
pub(super) struct CameraView {
    pub(super) eye: Vec3,
    pub(super) up: Vec3,
    pub(super) fov_y: f32,
}

impl CameraView {
    pub(super) fn new(eye: Vec3, up: Vec3, fov_y: f32) -> Self {
        Self { eye, up, fov_y }
    }
}

#[derive(Clone, Copy)]
pub(super) struct SceneRay {
    pub(super) origin: Vec3,
    pub(super) direction: Vec3,
}

impl SceneRay {
    pub(super) fn new(origin: Vec3, direction: Vec3) -> Self {
        Self { origin, direction }
    }
}

impl Default for EditorApplication {
    fn default() -> Self {
        EditorApplication::new()
    }
}
