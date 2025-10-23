use std::collections::VecDeque;

use egui_tiles::{Tile, TileId, Tree};
use glam::{Vec2, Vec3};
use hecs::Entity;
use wgpu_cube::app::{GpuUpdateContext, RuntimeMode, RuntimeStateHandle, UpdateContext};
use wgpu_cube::renderer::RenderRegion;
use wgpu_cube::scene::{
    Transform, TransformGizmoAxis, TransformGizmoHandle, TransformGizmoMode, TransformGizmoSpace,
};
use wgpu_cube::SceneHierarchyHandle;

use super::camera_system::CameraSystem;
use super::selection_system::SelectionSystem;
use super::system::EditorSystem;
use super::{EditorCommand, EditorContext, EditorEvent};
use egui::Context as EguiContext;
use wgpu_cube::DefaultUI;

use crate::asset_browser::AssetBrowserState;
use crate::camera::EditorCameraController;
use crate::history::EditorHistory;
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
    pub(super) windows: WindowToggles,
    pub(super) systems: Vec<Box<dyn EditorSystem>>,
    pub(super) camera_system_index: usize,
    pub(super) selection_system_index: usize,
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
    pub(super) pending_mode_transition: Option<RuntimeModeTransition>,
    pub(super) editor_scene_snapshot: Option<wgpu_cube::scene::SceneStateSnapshot>,
    pub(super) scene_hierarchy_handle: Option<SceneHierarchyHandle>,
    pub(super) commands: VecDeque<EditorCommand>,
    #[allow(dead_code)]
    pub(super) events: Vec<EditorEvent>,
}

#[derive(Default)]
pub struct ViewportSystem {
    pub(super) scene_viewport: ViewportState,
    pub(super) game_viewport: ViewportState,
    pub(super) game_view_display: GameViewDisplayMode,
    pub(super) grid_postprocess: Option<ViewportGrid>,
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
    camera_system: Option<CameraSystem>,
    windows: Option<WindowToggles>,
    project: Option<project::ProjectController>,
    history: Option<EditorHistory>,
    viewports: Option<ViewportSystem>,
    selection_system: Option<SelectionSystem>,
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
        self.camera_system = Some(CameraSystem::new(controller));
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
        self.selection_system = Some(selection);
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

        let mut systems: Vec<Box<dyn EditorSystem>> = Vec::new();
        let selection_system_index = {
            let system = self.selection_system.unwrap_or_default();
            let index = systems.len();
            systems.push(Box::new(system));
            index
        };
        let camera_system_index = {
            let system = self.camera_system.unwrap_or_default();
            let index = systems.len();
            systems.push(Box::new(system));
            index
        };

        EditorApplication {
            dock_tree: self.dock_tree.unwrap_or_else(create_editor_layout),
            viewports,
            windows: self.windows.unwrap_or_else(WindowToggles::new),
            systems,
            camera_system_index,
            selection_system_index,
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
            pending_mode_transition: None,
            editor_scene_snapshot: None,
            scene_hierarchy_handle: None,
            commands: VecDeque::new(),
            events: Vec::new(),
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

    pub(super) fn selection_system(&self) -> &SelectionSystem {
        self.systems[self.selection_system_index]
            .as_any()
            .downcast_ref::<SelectionSystem>()
            .expect("selection system registered")
    }

    pub(super) fn selection_system_mut(&mut self) -> &mut SelectionSystem {
        self.systems[self.selection_system_index]
            .as_any_mut()
            .downcast_mut::<SelectionSystem>()
            .expect("selection system registered")
    }

    pub(super) fn camera_system(&self) -> &CameraSystem {
        self.systems[self.camera_system_index]
            .as_any()
            .downcast_ref::<CameraSystem>()
            .expect("camera system registered")
    }

    pub(super) fn make_update_context<'app, 'ctx, 'scene>(
        &'app mut self,
        ctx: &'ctx mut UpdateContext<'scene>,
    ) -> EditorContext<'app, 'ctx, 'scene> {
        EditorContext::for_update(self, ctx)
    }

    pub(super) fn run_system_updates(&mut self, ctx: &mut UpdateContext) {
        let len = self.systems.len();
        let systems_ptr = self.systems.as_mut_ptr();
        for index in 0..len {
            let mut editor_ctx = self.make_update_context(ctx);
            // SAFETY: `systems_ptr` points into `self.systems`, which is not reallocated or
            // mutated within this loop body. Each iteration exclusively reborrows the element
            // at `index` to invoke the system while `editor_ctx` holds a raw pointer back to
            // the parent application.
            unsafe {
                (&mut *systems_ptr.add(index)).update(&mut editor_ctx);
            }
        }
    }

    pub(super) fn make_gpu_update_context<'app, 'ctx, 'scene>(
        &'app mut self,
        ctx: &'ctx mut GpuUpdateContext<'scene>,
    ) -> EditorContext<'app, 'ctx, 'scene> {
        EditorContext::for_gpu(self, ctx)
    }

    pub(super) fn run_system_ui(&mut self, ctx: &egui::Context, default_ui: &mut DefaultUI) {
        let len = self.systems.len();
        let systems_ptr = self.systems.as_mut_ptr();
        for index in 0..len {
            let mut editor_ctx = self.make_ui_context(ctx, default_ui);
            // SAFETY: See comment above in `run_system_updates`; the systems vector is left
            // untouched while we temporarily reborrow each element to run its UI pass.
            unsafe {
                (&mut *systems_ptr.add(index)).ui(&mut editor_ctx);
            }
        }
    }

    pub(super) fn enqueue_command(&mut self, command: EditorCommand) {
        self.commands.push_back(command);
    }

    pub(super) fn drain_update_commands(&mut self, ctx: &mut UpdateContext) {
        use EditorCommand::*;

        let mut queue = std::mem::take(&mut self.commands);
        let mut remaining = VecDeque::new();
        let mut pending_imports = Vec::new();
        let mut pending_deletions = Vec::new();
        let mut pending_inspector = Vec::new();
        let mut pending_scripts = Vec::new();

        while let Some(command) = queue.pop_front() {
            match command {
                ImportPath(path) => pending_imports.push(path),
                DeleteEntity(entity) => pending_deletions.push(entity),
                Inspector(action) => pending_inspector.push(action),
                Script(action) => pending_scripts.push(action),
                CreateScene(action) => remaining.push_back(CreateScene(action)),
            }
        }

        if !pending_scripts.is_empty() {
            self.apply_pending_script_actions(ctx, pending_scripts);
        }

        if !pending_inspector.is_empty() {
            self.apply_pending_inspector_actions(ctx, pending_inspector);
        }

        if !pending_imports.is_empty() {
            self.process_pending_imports(ctx, pending_imports);
        }

        if !pending_deletions.is_empty() {
            let active_camera = self.active_camera_entity;
            let gizmo_drag_entity = self
                .transform_tool
                .gizmo_drag
                .as_ref()
                .map(|drag| drag.entity);

            let result = {
                let selection = self.selection_system_mut();
                selection.process_pending_entity_deletions(
                    ctx,
                    pending_deletions,
                    active_camera,
                    gizmo_drag_entity,
                )
            };

            if let Some(outcome) = result {
                if outcome.active_camera_removed {
                    self.active_camera_entity = ctx.scene.active_camera_entity();
                }

                if outcome.clear_gizmo_drag {
                    self.transform_tool.gizmo_drag = None;
                }

                if outcome.selection_changed {
                    self.update_history_selection(ctx.scene);
                }

                self.record_scene_change(ctx.scene);
            }
        }

        remaining.append(&mut self.commands);
        self.commands = remaining;
    }

    pub(super) fn drain_gpu_commands(&mut self, ctx: &mut GpuUpdateContext) {
        let mut queue = std::mem::take(&mut self.commands);
        let mut remaining = VecDeque::new();
        let mut creations = Vec::new();

        while let Some(command) = queue.pop_front() {
            match command {
                EditorCommand::CreateScene(action) => creations.push(action),
                other => remaining.push_back(other),
            }
        }

        if !creations.is_empty() {
            self.apply_pending_scene_creations(ctx, creations);
        }

        remaining.append(&mut self.commands);
        self.commands = remaining;
    }

    pub(super) fn make_ui_context<'app, 'ctx>(
        &'app mut self,
        ctx: &'ctx EguiContext,
        default_ui: &'ctx mut DefaultUI,
    ) -> EditorContext<'app, 'ctx, 'ctx> {
        EditorContext::for_ui(self, ctx, default_ui)
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
