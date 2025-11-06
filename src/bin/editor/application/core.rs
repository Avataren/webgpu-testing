use std::collections::VecDeque;
use std::path::PathBuf;
#[cfg(not(target_arch = "wasm32"))]
use std::{collections::HashSet, fs, path::Path};

use egui_tiles::{Tile, TileId, Tree};
use glam::{Vec2, Vec3};
use hecs::Entity;
use wgpu_cube::app::{GpuUpdateContext, RuntimeMode, RuntimeStateHandle, UpdateContext};
use wgpu_cube::asset::{Handle, MaterialAsset, Mesh};
use wgpu_cube::renderer::RenderRegion;
use wgpu_cube::scene::{
    MeshBounds, Scene, SceneHandle, SceneStateSnapshot, SceneWorkspaceSceneMut,
};
use wgpu_cube::{
    SceneHierarchyHandle, SceneHierarchyRegistryHandle, SceneTabDescriptor, SceneTabsHandle,
};

use super::asset_browser_system::AssetBrowserSystem;
use super::camera_system::CameraSystem;
use super::history_system::{HistorySystem, TransformToolSystem};
use super::particle_system::EditorParticleSystem;
use super::project_system::ProjectSystem;
use super::scene_creation_system::SceneCreationSystem;
use super::scene_tabs_panel::SceneTabsPanel;
use super::script_editor_system::ScriptEditorSystem;
use super::shader_editor_system::ShaderEditorSystem;
use super::selection_system::SelectionSystem;
#[cfg(not(target_arch = "wasm32"))]
use super::shader_watcher::ShaderWatcher;
use super::system::EditorSystem;
use super::{EditorCommand, EditorContext, EditorEvent, EditorSystemsAccess};
use wgpu_cube::DefaultUI;

use crate::asset_browser::AssetBrowserState;
use crate::camera::EditorCameraController;
use crate::history::EditorHistory;
use crate::layout::{create_editor_layout, EditorPane, ViewportState};
use crate::postprocess::ViewportGrid;
use crate::project;
use crate::windows::WindowToggles;
#[cfg(not(target_arch = "wasm32"))]
use wgpu_cube::project::normalize_absolute_path;

pub(super) struct RuntimeModeTransition {
    pub(super) from: RuntimeMode,
    pub(super) to: RuntimeMode,
}

pub struct EditorSharedState {
    pub(super) dock_tree: Tree<EditorPane>,
    pub(super) viewports: ViewportSystem,
    pub(super) windows: WindowToggles,
    pub(super) active_camera_entity: Option<Entity>,
    pub(super) active_scene_handle: Option<SceneHandle>,
    pub(super) runtime_state: RuntimeStateHandle,
    pub(super) last_runtime_mode: RuntimeMode,
    pub(super) pending_mode_transition: Option<RuntimeModeTransition>,
    pub(super) editor_scene_snapshot: Option<SceneStateSnapshot>,
    pub(super) scene_hierarchy_registry: Option<SceneHierarchyRegistryHandle>,
    pub(super) scene_tabs: Option<SceneTabsHandle>,
    pub(super) commands: VecDeque<EditorCommand>,
    #[allow(dead_code)]
    pub(super) events: Vec<EditorEvent>,
    pub(super) particle_mesh: Option<Handle<Mesh>>,
    pub(super) particle_mesh_bounds: Option<MeshBounds>,
    pub(super) pending_new_scenes: Vec<NewSceneRequest>,
    pub(super) next_untitled_scene_index: u32,
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) shader_watcher: Option<ShaderWatcher>,
    /// UI commands from scripts collected during gpu_update
    pub(super) script_ui_commands: std::collections::HashMap<Entity, Vec<wgpu_cube::scripting::rune::api::ui::UiCommand>>,
}

impl EditorSharedState {
    pub(super) fn set_active_scene_handle(&mut self, handle: SceneHandle) {
        self.active_scene_handle = Some(handle);
    }

    pub(super) fn set_scene_hierarchy_registry(&mut self, registry: SceneHierarchyRegistryHandle) {
        if self.scene_hierarchy_registry.is_none() {
            self.scene_hierarchy_registry = Some(registry);
        }
    }

    #[allow(dead_code)]
    pub(super) fn clear_active_scene_handle(&mut self) {
        self.active_scene_handle = None;
    }

    #[allow(dead_code)]
    pub(super) fn active_scene_handle(&self) -> Option<SceneHandle> {
        self.active_scene_handle
    }

    pub(super) fn scene_hierarchy_handle_for_scene(
        &self,
        handle: SceneHandle,
    ) -> Option<SceneHierarchyHandle> {
        let registry = self.scene_hierarchy_registry.as_ref()?;
        let Ok(registry) = registry.lock() else {
            return None;
        };
        registry.get_handle(handle)
    }

    pub(super) fn mark_scene_hierarchy_dirty(&self, handle: SceneHandle) {
        let Some(registry) = &self.scene_hierarchy_registry else {
            return;
        };

        if let Ok(mut registry) = registry.lock() {
            // Ensure entry exists (creates it if missing with dirty=true)
            registry.ensure_entry(handle);
            // Mark as dirty (in case entry already existed)
            registry.mark_dirty(handle);
        }
    }

    pub(super) fn set_scene_tabs_handle(&mut self, handle: SceneTabsHandle) {
        if self.scene_tabs.is_none() {
            self.scene_tabs = Some(handle);
        }
    }

    pub(super) fn scene_tabs(&self) -> Option<Vec<SceneTabDescriptor>> {
        let handle = self.scene_tabs.as_ref()?;
        let Ok(tabs) = handle.lock() else {
            return None;
        };
        Some(tabs.tabs().to_vec())
    }
}

#[derive(Default)]
pub(super) struct NewSceneRequest;

pub struct EditorApplication {
    pub(super) shared: EditorSharedState,
    pub(super) systems: Vec<Box<dyn EditorSystem>>,
    pub(super) camera_system_index: usize,
    pub(super) selection_system_index: usize,
    pub(super) history_system_index: usize,
    pub(super) project_system_index: usize,
    pub(super) script_editor_system_index: usize,
    pub(super) shader_editor_system_index: usize,
    pub(super) asset_browser_system_index: usize,
    pub(super) particle_system_index: usize,
    pub(super) scene_tabs_panel: SceneTabsPanel,
}

#[derive(Clone, Copy)]
pub(super) struct EditorSystemIndices {
    pub(super) camera: usize,
    pub(super) selection: usize,
    pub(super) history: usize,
    pub(super) project: usize,
    pub(super) script_editor: usize,
    pub(super) shader_editor: usize,
    pub(super) asset_browser: usize,
}

#[derive(Default)]
pub struct ViewportSystem {
    pub(super) scene_viewport: ViewportState,
    pub(super) game_viewport: ViewportState,
    pub(super) game_view_display: GameViewDisplayMode,
    pub(super) grid_postprocess: Option<ViewportGrid>,
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
        let history_system_index = {
            let history = self.history.unwrap_or_else(EditorHistory::new);
            let transform_tool = self.transform_tool.unwrap_or_default();
            let system = HistorySystem::new(history, transform_tool);
            let index = systems.len();
            systems.push(Box::new(system));
            index
        };
        let project_system_index = {
            let controller = self.project.unwrap_or_else(project::ProjectController::new);
            let system = ProjectSystem::new(controller);
            let index = systems.len();
            systems.push(Box::new(system));
            index
        };
        let script_editor_system_index = {
            let system = ScriptEditorSystem::default();
            let index = systems.len();
            systems.push(Box::new(system));
            index
        };
        let shader_editor_system_index = {
            let system = ShaderEditorSystem::default();
            let index = systems.len();
            systems.push(Box::new(system));
            index
        };
        let asset_browser_system_index = {
            let state = self.asset_browser.unwrap_or_default();
            let system = AssetBrowserSystem::new(state);
            let index = systems.len();
            systems.push(Box::new(system));
            index
        };
        systems.push(Box::new(SceneCreationSystem));
        let particle_system_index = {
            let system = EditorParticleSystem::default();
            let index = systems.len();
            systems.push(Box::new(system));
            index
        };

        let mut shared = EditorSharedState {
            dock_tree: self.dock_tree.unwrap_or_else(create_editor_layout),
            viewports,
            windows: self.windows.unwrap_or_else(WindowToggles::new),
            active_camera_entity: None,
            active_scene_handle: None,
            runtime_state: RuntimeStateHandle::new(),
            last_runtime_mode: RuntimeMode::Editor,
            pending_mode_transition: None,
            editor_scene_snapshot: None,
            scene_hierarchy_registry: None,
            scene_tabs: None,
            commands: VecDeque::new(),
            events: Vec::new(),
            particle_mesh: None,
            particle_mesh_bounds: None,
            pending_new_scenes: Vec::new(),
            next_untitled_scene_index: 1,
            #[cfg(not(target_arch = "wasm32"))]
            shader_watcher: None,
            script_ui_commands: std::collections::HashMap::new(),
        };

        #[cfg(not(target_arch = "wasm32"))]
        {
            shared.shader_watcher = match ShaderWatcher::new() {
                Ok(watcher) => Some(watcher),
                Err(err) => {
                    log::warn!("Failed to initialize shader file watcher: {err}");
                    None
                }
            };
        }

        EditorApplication {
            shared,
            systems,
            camera_system_index,
            selection_system_index,
            history_system_index,
            project_system_index,
            script_editor_system_index,
            shader_editor_system_index,
            asset_browser_system_index,
            particle_system_index,
            scene_tabs_panel: SceneTabsPanel,
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
        self.shared.runtime_state = handle;
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

    pub(super) fn history_system(&self) -> &HistorySystem {
        self.systems[self.history_system_index]
            .as_any()
            .downcast_ref::<HistorySystem>()
            .expect("history system registered")
    }

    pub(super) fn history_system_mut(&mut self) -> &mut HistorySystem {
        self.systems[self.history_system_index]
            .as_any_mut()
            .downcast_mut::<HistorySystem>()
            .expect("history system registered")
    }

    pub(super) fn project_system(&self) -> &ProjectSystem {
        self.systems[self.project_system_index]
            .as_any()
            .downcast_ref::<ProjectSystem>()
            .expect("project system registered")
    }

    pub(super) fn project_system_mut(&mut self) -> &mut ProjectSystem {
        self.systems[self.project_system_index]
            .as_any_mut()
            .downcast_mut::<ProjectSystem>()
            .expect("project system registered")
    }

    pub(super) fn script_editor_system_mut(&mut self) -> &mut ScriptEditorSystem {
        self.systems[self.script_editor_system_index]
            .as_any_mut()
            .downcast_mut::<ScriptEditorSystem>()
            .expect("script editor system registered")
    }

    pub(super) fn shader_editor_system_mut(&mut self) -> &mut ShaderEditorSystem {
        self.systems[self.shader_editor_system_index]
            .as_any_mut()
            .downcast_mut::<ShaderEditorSystem>()
            .expect("shader editor system registered")
    }

    pub(super) fn asset_browser_system_mut(&mut self) -> &mut AssetBrowserSystem {
        self.systems[self.asset_browser_system_index]
            .as_any_mut()
            .downcast_mut::<AssetBrowserSystem>()
            .expect("asset browser system registered")
    }

    pub(super) fn asset_browser_state_mut(&mut self) -> &mut AssetBrowserState {
        self.asset_browser_system_mut().state_mut()
    }

    pub(super) fn scene_tabs(&self) -> Vec<SceneTabDescriptor> {
        self.shared.scene_tabs().unwrap_or_default()
    }

    pub(super) fn particle_system(&self) -> &EditorParticleSystem {
        self.systems[self.particle_system_index]
            .as_any()
            .downcast_ref::<EditorParticleSystem>()
            .expect("particle system registered")
    }

    pub(super) fn particle_system_mut(&mut self) -> &mut EditorParticleSystem {
        self.systems[self.particle_system_index]
            .as_any_mut()
            .downcast_mut::<EditorParticleSystem>()
            .expect("particle system registered")
    }

    pub(super) fn history(&self) -> &EditorHistory {
        self.history_system().history()
    }

    fn system_indices(&self) -> EditorSystemIndices {
        EditorSystemIndices {
            camera: self.camera_system_index,
            selection: self.selection_system_index,
            history: self.history_system_index,
            project: self.project_system_index,
            script_editor: self.script_editor_system_index,
            shader_editor: self.shader_editor_system_index,
            asset_browser: self.asset_browser_system_index,
        }
    }

    pub(super) fn selection_entities(&self) -> (Option<Entity>, Option<Entity>) {
        let selection = self.selection_system();
        (selection.selected(), selection.highlighted())
    }

    pub(super) fn initialize_history_state(&mut self, scene: &mut Scene) {
        let (selected, highlighted) = self.selection_entities();
        self.history_system_mut()
            .initialize_state(scene, selected, highlighted);
    }

    pub(super) fn record_scene_change(&mut self, scene: &mut SceneWorkspaceSceneMut<'_>) {
        let (selected, highlighted) = self.selection_entities();
        scene.mark_dirty();
        if let Some(handle) = self.shared.active_scene_handle {
            self.shared.mark_scene_hierarchy_dirty(handle);
        }
        self.history_system_mut()
            .record_scene_change(scene, selected, highlighted);
    }

    pub(super) fn update_history_selection(&mut self, scene: &Scene) {
        let (selected, highlighted) = self.selection_entities();
        self.history_system_mut()
            .update_history_selection(scene, selected, highlighted);
    }

    pub(super) fn run_system_updates(&mut self, ctx: &mut UpdateContext) {
        let indices = self.system_indices();
        let len = self.systems.len();
        for current_index in 0..len {
            let (before, rest) = self.systems.split_at_mut(current_index);
            let (current, after) = rest.split_first_mut().expect("system index within bounds");
            let systems = EditorSystemsAccess::new(before, after, indices, current_index);
            let mut editor_ctx = EditorContext::for_update(&mut self.shared, systems, ctx);
            current.update(&mut editor_ctx);
        }
    }

    pub(super) fn run_system_gpu_updates(&mut self, ctx: &mut GpuUpdateContext) {
        let indices = self.system_indices();
        let len = self.systems.len();
        for current_index in 0..len {
            let (before, rest) = self.systems.split_at_mut(current_index);
            let (current, after) = rest.split_first_mut().expect("system index within bounds");
            let systems = EditorSystemsAccess::new(before, after, indices, current_index);
            let mut editor_ctx = EditorContext::for_gpu(&mut self.shared, systems, ctx);
            current.gpu_update(&mut editor_ctx);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn process_shader_file_changes(&mut self, ctx: &mut GpuUpdateContext) {
        let Some(content_root) = self.project_system().content_root() else {
            if let Some(watcher) = self.shared.shader_watcher.as_mut() {
                watcher.clear();
            }
            return;
        };

        let Some(watcher) = self.shared.shader_watcher.as_mut() else {
            return;
        };

        if let Err(err) = watcher.watch_root(&content_root) {
            log::warn!(
                "Failed to watch project shader directory {:?}: {}",
                content_root,
                err
            );
            return;
        }

        let changed_paths = watcher.poll();
        if changed_paths.is_empty() {
            return;
        }

        let mut processed = HashSet::new();
        for path in changed_paths {
            if let Some(canonical) = Self::canonicalize_shader_path(&path) {
                if processed.insert(canonical.clone()) {
                    self.reload_shader_materials_from_path(ctx, &canonical);
                }
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn canonicalize_shader_path(path: &Path) -> Option<PathBuf> {
        let canonical = match path.canonicalize() {
            Ok(path) => path,
            Err(err) => {
                log::warn!("Failed to canonicalize shader path {:?}: {}", path, err);
                return None;
            }
        };

        Some(normalize_absolute_path(canonical))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn reload_shader_materials_from_path(
        &mut self,
        ctx: &mut GpuUpdateContext,
        canonical_path: &Path,
    ) {
        let source = match fs::read_to_string(canonical_path) {
            Ok(contents) => contents,
            Err(err) => {
                log::warn!("Failed to reload shader {:?}: {}", canonical_path, err);
                return;
            }
        };

        let normalized_path = normalize_absolute_path(canonical_path.to_path_buf());
        let material_count = ctx.scene.assets.materials.len();
        let mut matching_handles = Vec::new();

        for index in 0..material_count {
            let handle = Handle::new(index);
            let Some(asset) = ctx.scene.assets.material(handle) else {
                continue;
            };

            let Some(metadata) = asset.shader_metadata() else {
                continue;
            };

            let Some(source_path) = metadata.source_path() else {
                continue;
            };

            let normalized_source = normalize_absolute_path(source_path.to_path_buf());
            if normalized_source == normalized_path {
                matching_handles.push(handle);
            }
        }

        if matching_handles.is_empty() {
            return;
        }

        let mut updated_handles = Vec::new();
        for handle in &matching_handles {
            if let Some(asset) = ctx.scene.assets.material_mut(*handle) {
                if let Some(metadata) = asset.shader_metadata_mut() {
                    metadata.set_wgsl_source(source.clone());
                    updated_handles.push(*handle);
                }
            }
        }

        if updated_handles.is_empty() {
            return;
        }

        for handle in &updated_handles {
            ctx.renderer
                .invalidate_material_shader_modules(*handle, None);
        }

        log::info!(
            "Hot-reloaded WGSL shader {:?} for {} material(s)",
            canonical_path,
            updated_handles.len()
        );

        self.record_scene_change(&mut ctx.scene);
    }

    pub(super) fn run_system_ui(&mut self, ctx: &egui::Context, default_ui: &mut DefaultUI) {
        let indices = self.system_indices();
        let len = self.systems.len();
        for current_index in 0..len {
            let (before, rest) = self.systems.split_at_mut(current_index);
            let (current, after) = rest.split_first_mut().expect("system index within bounds");
            let systems = EditorSystemsAccess::new(before, after, indices, current_index);
            let mut editor_ctx = EditorContext::for_ui(&mut self.shared, systems, ctx, default_ui);
            current.ui(&mut editor_ctx);
        }
    }

    pub(super) fn enqueue_command(&mut self, command: EditorCommand) {
        self.shared.commands.push_back(command);
    }

    pub(super) fn drain_update_commands(&mut self, ctx: &mut UpdateContext) {
        use EditorCommand::*;

        let mut queue = std::mem::take(&mut self.shared.commands);
        let mut remaining = VecDeque::new();
        let mut pending_imports = Vec::new();
        let mut pending_deletions = Vec::new();
        let mut pending_inspector = Vec::new();

        while let Some(command) = queue.pop_front() {
            match command {
                ImportPath(path) => pending_imports.push(path),
                DeleteEntity(entity) => pending_deletions.push(entity),
                Inspector(action) => pending_inspector.push(action),
                ActivateScene(document_id) => ctx.request_active_scene(document_id),
                CloseScene(document_id) => ctx.request_close_scene(document_id),
                NewScene => self.shared.pending_new_scenes.push(NewSceneRequest),
                Script(action) => remaining.push_back(Script(action)),
                Shader(action) => remaining.push_back(Shader(action)),
                CreateScene(action) => remaining.push_back(CreateScene(action)),
                HistoryUndo => remaining.push_back(HistoryUndo),
                HistoryRedo => remaining.push_back(HistoryRedo),
                HistoryCommitTransforms => remaining.push_back(HistoryCommitTransforms),
            }
        }

        if !pending_inspector.is_empty() {
            self.apply_pending_inspector_actions(ctx, pending_inspector);
        }

        if !pending_imports.is_empty() {
            self.process_pending_imports(ctx, pending_imports);
        }

        if !pending_deletions.is_empty() {
            let active_camera = self.shared.active_camera_entity;
            let gizmo_drag_entity = self.history_system().gizmo_drag().map(|drag| drag.entity);

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
                    self.shared.active_camera_entity = ctx.scene.active_camera_entity();
                }

                if outcome.clear_gizmo_drag {
                    self.history_system_mut().clear_gizmo_drag();
                }

                if outcome.selection_changed {
                    self.update_history_selection(&ctx.scene);
                }

                self.record_scene_change(&mut ctx.scene);
            }
        }

        remaining.append(&mut self.shared.commands);
        self.shared.commands = remaining;
    }

    pub(super) fn process_pending_imports(
        &mut self,
        ctx: &mut UpdateContext,
        pending: Vec<PathBuf>,
    ) {
        if pending.is_empty() {
            return;
        }

        let indices = self.system_indices();
        let project_index = self.project_system_index;
        let (before, rest) = self.systems.split_at_mut(project_index);
        let (system, after) = rest
            .split_first_mut()
            .expect("project system index within bounds");
        let systems = EditorSystemsAccess::new(before, after, indices, project_index);
        let mut editor_ctx = EditorContext::for_update(&mut self.shared, systems, ctx);
        system
            .as_any_mut()
            .downcast_mut::<ProjectSystem>()
            .expect("project system registered")
            .process_pending_imports(&mut editor_ctx, pending);
    }

    pub(super) fn find_pane_tile(&self, pane: EditorPane) -> Option<TileId> {
        self.shared
            .dock_tree
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
            let _ = self.shared.dock_tree.make_active(|id, _| id == tile_id);
        }
    }

    pub(super) fn render_region_for_mode(&self, mode: RuntimeMode) -> Option<RenderRegion> {
        match mode {
            RuntimeMode::Editor => self.shared.viewports.scene_viewport.region(),
            RuntimeMode::Playing => self.shared.viewports.game_viewport.region(),
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

pub(super) enum PendingShaderAction {
    Save {
        handle: Handle<MaterialAsset>,
        contents: String,
        message: String,
    },
}

pub(super) struct ViewportPick {
    pub(super) uv: Vec2,
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
