use std::path::PathBuf;

use egui_tiles::{Tile, TileId, Tree};
use glam::{Vec2, Vec3};
use hecs::Entity;
use wgpu_cube::app::{RuntimeMode, RuntimeStateHandle};
use wgpu_cube::renderer::RenderRegion;
use wgpu_cube::scene::{
    Transform, TransformGizmoAxis, TransformGizmoHandle, TransformGizmoMode, TransformGizmoSpace,
};
use wgpu_cube::{SceneCreationAction, SceneHierarchyHandle};

use crate::asset_browser::AssetBrowserState;
use crate::camera::EditorCameraController;
use crate::history::EditorHistory;
use crate::inspector::InspectorAction;
use crate::layout::{create_editor_layout, EditorPane, ViewportState};
use crate::postprocess::ViewportGrid;
use crate::project;
use crate::script_editor::ScriptEditorState;
use crate::windows::WindowToggles;

pub(super) struct RuntimeModeTransition {
    pub(super) from: RuntimeMode,
    pub(super) to: RuntimeMode,
}

pub struct EditorApplication {
    pub(super) dock_tree: Tree<EditorPane>,
    pub(super) viewports: ViewportSystem,
    pub(super) camera_controller: EditorCameraController,
    pub(super) pending_imports: Vec<PathBuf>,
    pub(super) pending_entity_deletions: Vec<Entity>,
    pub(super) pending_inspector_actions: Vec<InspectorAction>,
    pub(super) pending_scene_creations: Vec<SceneCreationAction>,
    pub(super) windows: WindowToggles,
    pub(super) selection: SelectionSystem,
    pub(super) active_camera_entity: Option<Entity>,
    pub(super) runtime_state: RuntimeStateHandle,
    pub(super) last_runtime_mode: RuntimeMode,
    pub(super) transform_tool: TransformToolSystem,
    pub(super) history: EditorHistory,
    pub(super) next_editor_entity_id: u128,
    pub(super) undo_redo: UndoRedoState,
    pub(super) project: project::ProjectController,
    pub(super) asset_browser: AssetBrowserState,
    pub(super) script_editor: Option<ScriptEditorState>,
    pub(super) pending_script_actions: Vec<PendingScriptAction>,
    pub(super) pending_mode_transition: Option<RuntimeModeTransition>,
    pub(super) editor_scene_snapshot: Option<wgpu_cube::scene::SceneStateSnapshot>,
    pub(super) scene_hierarchy_handle: Option<SceneHierarchyHandle>,
}

#[derive(Default)]
pub struct ViewportSystem {
    pub(super) scene_viewport: ViewportState,
    pub(super) game_viewport: ViewportState,
    pub(super) game_view_display: GameViewDisplayMode,
    pub(super) grid_postprocess: Option<ViewportGrid>,
}

#[derive(Default)]
pub struct SelectionSystem {
    state: SelectionState,
    pub(super) pointer: PointerState,
    pending_pick: Option<ViewportPick>,
}

impl SelectionSystem {
    pub(super) fn selected(&self) -> Option<Entity> {
        self.state.selected
    }

    pub(super) fn set_selected(&mut self, entity: Option<Entity>) {
        self.state.set_selected(entity);
    }

    pub(super) fn highlighted(&self) -> Option<Entity> {
        self.state.highlighted
    }

    pub(super) fn set_highlighted(&mut self, entity: Option<Entity>) {
        self.state.set_highlighted(entity);
    }

    pub(super) fn take_highlighted(&mut self) -> Option<Entity> {
        let current = self.state.highlighted;
        self.state.highlighted = None;
        current
    }

    pub(super) fn request_override(&mut self, entity: Option<Entity>) {
        self.state.request_override(entity);
    }

    pub(super) fn take_override(&mut self) -> Option<Option<Entity>> {
        self.state.take_override()
    }

    pub(super) fn clear_pending_pick(&mut self) {
        self.pending_pick = None;
    }

    pub(super) fn set_pending_pick(&mut self, pick: ViewportPick) {
        self.pending_pick = Some(pick);
    }

    pub(super) fn take_pending_pick(&mut self) -> Option<ViewportPick> {
        self.pending_pick.take()
    }
}

pub struct TransformToolSystem {
    pub(super) gizmo_mode: TransformGizmoMode,
    pub(super) gizmo_space: TransformGizmoSpace,
    pub(super) gizmo_drag: Option<GizmoDragState>,
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

#[derive(Default)]
pub struct EditorApplicationBuilder {
    dock_tree: Option<Tree<EditorPane>>,
    camera_controller: Option<EditorCameraController>,
    windows: Option<WindowToggles>,
    project: Option<project::ProjectController>,
    history: Option<EditorHistory>,
    viewports: Option<ViewportSystem>,
    selection: Option<SelectionSystem>,
    transform_tool: Option<TransformToolSystem>,
    asset_browser: Option<AssetBrowserState>,
}

impl EditorApplicationBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_dock_tree(mut self, dock_tree: Tree<EditorPane>) -> Self {
        self.dock_tree = Some(dock_tree);
        self
    }

    pub fn with_camera_controller(mut self, controller: EditorCameraController) -> Self {
        self.camera_controller = Some(controller);
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

    pub fn with_viewports(mut self, viewports: ViewportSystem) -> Self {
        self.viewports = Some(viewports);
        self
    }

    pub fn with_selection(mut self, selection: SelectionSystem) -> Self {
        self.selection = Some(selection);
        self
    }

    pub fn with_transform_tool(mut self, transform_tool: TransformToolSystem) -> Self {
        self.transform_tool = Some(transform_tool);
        self
    }

    pub fn with_asset_browser(mut self, asset_browser: AssetBrowserState) -> Self {
        self.asset_browser = Some(asset_browser);
        self
    }

    pub fn build(self) -> EditorApplication {
        let viewports = self.viewports.unwrap_or_default();

        EditorApplication {
            dock_tree: self.dock_tree.unwrap_or_else(create_editor_layout),
            viewports,
            camera_controller: self.camera_controller.unwrap_or_default(),
            pending_imports: Vec::new(),
            pending_entity_deletions: Vec::new(),
            pending_inspector_actions: Vec::new(),
            pending_scene_creations: Vec::new(),
            windows: self.windows.unwrap_or_else(WindowToggles::new),
            selection: self.selection.unwrap_or_default(),
            active_camera_entity: None,
            runtime_state: RuntimeStateHandle::new(),
            last_runtime_mode: RuntimeMode::Editor,
            transform_tool: self.transform_tool.unwrap_or_default(),
            history: self.history.unwrap_or_else(EditorHistory::new),
            next_editor_entity_id: 1,
            undo_redo: UndoRedoState::default(),
            project: self.project.unwrap_or_else(project::ProjectController::new),
            asset_browser: self.asset_browser.unwrap_or_default(),
            script_editor: None,
            pending_script_actions: Vec::new(),
            pending_mode_transition: None,
            editor_scene_snapshot: None,
            scene_hierarchy_handle: None,
        }
    }
}

impl EditorApplication {
    pub fn new() -> Self {
        Self::builder()
            .with_dock_tree(create_editor_layout())
            .with_viewports(ViewportSystem::default())
            .with_camera_controller(EditorCameraController::default())
            .with_windows(WindowToggles::new())
            .with_project(project::ProjectController::new())
            .with_history(EditorHistory::new())
            .with_selection(SelectionSystem::default())
            .with_transform_tool(TransformToolSystem::default())
            .with_asset_browser(AssetBrowserState::default())
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
            RuntimeMode::Editor => self.viewports.scene_viewport.region(),
            RuntimeMode::Playing => self.viewports.game_viewport.region(),
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
