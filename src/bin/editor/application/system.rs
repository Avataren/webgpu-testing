use std::any::Any;
use std::collections::VecDeque;
use std::path::PathBuf;

use super::asset_browser_system::AssetBrowserSystem;
use super::camera_system::CameraSystem;
use super::core::{EditorApplication, EditorSharedState, EditorSystemIndices, PendingScriptAction};
use super::history_system::HistorySystem;
use super::project_system::ProjectSystem;
use super::script_editor_system::ScriptEditorSystem;
use super::selection_system::SelectionSystem;
use crate::asset_browser::AssetBrowserState;
use crate::history::EditorHistory;
use crate::inspector::InspectorAction;
use crate::layout::ViewportState;
use egui::Context as EguiContext;
use glam::Vec2;
use hecs::Entity;
use wgpu_cube::app::{GpuUpdateContext, RuntimeMode, RuntimeStateHandle, UpdateContext};
use wgpu_cube::renderer::{RenderRegion, Renderer};
use wgpu_cube::scene::{ParticleBehaviorPreset, Scene};
use wgpu_cube::{DefaultUI, SceneCreationAction, SceneHierarchyHandle, ScenePrimitivePreset};

pub(super) enum EditorCommand {
    ImportPath(PathBuf),
    DeleteEntity(Entity),
    Inspector(InspectorAction),
    CreateScene(SceneCreationAction),
    Script(PendingScriptAction),
    HistoryUndo,
    HistoryRedo,
    HistoryCommitTransforms,
}

#[derive(Debug)]
pub(super) enum EditorEvent {}

pub struct EditorContext<'app, 'ctx, 'scene> {
    shared: Option<&'app mut EditorSharedState>,
    systems: Option<EditorSystemsAccess<'app>>,
    update: Option<&'ctx mut UpdateContext<'scene>>,
    gpu: Option<&'ctx mut GpuUpdateContext<'scene>>,
    ui: Option<EditorUiContext<'ctx>>,
}

pub struct EditorSystemsAccess<'app> {
    before: &'app mut [Box<dyn EditorSystem>],
    after: &'app mut [Box<dyn EditorSystem>],
    indices: EditorSystemIndices,
    current_index: usize,
}

struct EditorUiContext<'ctx> {
    egui: &'ctx EguiContext,
    default_ui: &'ctx mut DefaultUI,
}

pub struct EditorUiContextMut<'a> {
    egui: &'a EguiContext,
    _default_ui: &'a mut DefaultUI,
}

impl<'a> EditorUiContextMut<'a> {
    pub fn egui(&self) -> &'a EguiContext {
        self.egui
    }

    #[allow(dead_code)]
    pub fn default_ui(&mut self) -> &mut DefaultUI {
        self._default_ui
    }
}

pub struct EditorAppAccess<'shared> {
    shared: &'shared mut EditorSharedState,
    systems: EditorSystemsAccess<'shared>,
}

impl<'shared> EditorAppAccess<'shared> {
    fn new(shared: &'shared mut EditorSharedState, systems: EditorSystemsAccess<'shared>) -> Self {
        Self { shared, systems }
    }

    fn into_inner(self) -> (&'shared mut EditorSharedState, EditorSystemsAccess<'shared>) {
        (self.shared, self.systems)
    }

    pub fn runtime_state(&self) -> RuntimeStateHandle {
        self.shared.runtime_state.clone()
    }

    pub fn scene_viewport(&self) -> &ViewportState {
        &self.shared.viewports.scene_viewport
    }

    pub fn camera_system(&self) -> &CameraSystem {
        self.systems.camera_system()
    }

    pub fn history_system(&self) -> &HistorySystem {
        self.systems.history_system()
    }

    pub fn history_system_mut(&mut self) -> &mut HistorySystem {
        self.systems.history_system_mut()
    }

    pub fn selection_system(&self) -> &SelectionSystem {
        self.systems.selection_system()
    }

    pub fn selection_system_mut(&mut self) -> &mut SelectionSystem {
        self.systems.selection_system_mut()
    }

    pub fn asset_browser_state_mut(&mut self) -> &mut AssetBrowserState {
        self.systems.asset_browser_system_mut().state_mut()
    }

    pub fn record_scene_change(&mut self, scene: &mut Scene) {
        let (selected, highlighted) = self.selection_entities();
        self.history_system_mut()
            .record_scene_change(scene, selected, highlighted);
    }

    pub fn ensure_editor_scene_basics(&mut self, scene: &mut Scene, renderer: &mut Renderer) {
        EditorApplication::ensure_editor_scene_basics(scene, renderer);
    }

    pub fn initialize_history_state(&mut self, scene: &mut Scene) {
        let (selected, highlighted) = self.selection_entities();
        self.history_system_mut()
            .initialize_state(scene, selected, highlighted);
    }

    pub(super) fn command_queue_mut(&mut self) -> &mut VecDeque<EditorCommand> {
        &mut self.shared.commands
    }

    pub(super) fn scene_hierarchy_handle(&self) -> Option<&SceneHierarchyHandle> {
        self.shared.scene_hierarchy_handle.as_ref()
    }

    pub fn request_runtime_mode(&mut self, mode: RuntimeMode) {
        self.shared.runtime_state.request_mode(mode);
    }

    pub fn create_primitive(
        &mut self,
        ctx: &mut GpuUpdateContext<'_>,
        preset: ScenePrimitivePreset,
    ) -> Option<Entity> {
        EditorApplication::create_primitive(ctx, preset)
    }

    pub fn create_particle_system(
        &mut self,
        ctx: &mut GpuUpdateContext<'_>,
        preset: ParticleBehaviorPreset,
    ) -> Option<Entity> {
        EditorApplication::create_particle_system(self.shared, ctx, preset)
    }

    pub fn create_point_light(&mut self, ctx: &mut GpuUpdateContext<'_>) -> Option<Entity> {
        EditorApplication::create_point_light(ctx)
    }

    pub fn create_directional_light(&mut self, ctx: &mut GpuUpdateContext<'_>) -> Option<Entity> {
        EditorApplication::create_directional_light(ctx)
    }

    pub fn create_spot_light(&mut self, ctx: &mut GpuUpdateContext<'_>) -> Option<Entity> {
        EditorApplication::create_spot_light(ctx)
    }

    pub fn create_camera(&mut self, ctx: &mut GpuUpdateContext<'_>) -> Option<Entity> {
        EditorApplication::create_camera(self.shared, ctx)
    }

    pub fn create_environment(&mut self, ctx: &mut GpuUpdateContext<'_>) -> Option<Entity> {
        EditorApplication::create_environment(ctx)
    }

    pub fn pick_entity(
        &self,
        ctx: &UpdateContext<'_>,
        uv: Vec2,
        region: RenderRegion,
    ) -> Option<Entity> {
        EditorApplication::pick_entity(ctx, uv, region)
    }

    fn selection_entities(&self) -> (Option<Entity>, Option<Entity>) {
        let selection = self.selection_system();
        (selection.selected(), selection.highlighted())
    }
}

impl<'app> EditorSystemsAccess<'app> {
    pub(super) fn new(
        before: &'app mut [Box<dyn EditorSystem>],
        after: &'app mut [Box<dyn EditorSystem>],
        indices: EditorSystemIndices,
        current_index: usize,
    ) -> Self {
        Self {
            before,
            after,
            indices,
            current_index,
        }
    }

    fn resolve_index(&self, index: usize) -> usize {
        if index < self.current_index {
            index
        } else {
            index - self.current_index - 1
        }
    }

    fn system_ref<T: 'static>(&self, index: usize) -> &T {
        assert!(
            index != self.current_index,
            "system attempted to access itself"
        );
        if index < self.current_index {
            let slice: &[Box<dyn EditorSystem>] = &*self.before;
            slice[index]
                .as_any()
                .downcast_ref::<T>()
                .expect("system registered")
        } else {
            let offset = self.resolve_index(index);
            let slice: &[Box<dyn EditorSystem>] = &*self.after;
            slice[offset]
                .as_any()
                .downcast_ref::<T>()
                .expect("system registered")
        }
    }

    fn system_mut<T: 'static>(&mut self, index: usize) -> &mut T {
        assert!(
            index != self.current_index,
            "system attempted to access itself mutably"
        );
        if index < self.current_index {
            self.before[index]
                .as_any_mut()
                .downcast_mut::<T>()
                .expect("system registered")
        } else {
            let offset = self.resolve_index(index);
            self.after[offset]
                .as_any_mut()
                .downcast_mut::<T>()
                .expect("system registered")
        }
    }

    pub fn camera_system(&self) -> &CameraSystem {
        self.system_ref(self.indices.camera)
    }

    pub fn history_system(&self) -> &HistorySystem {
        self.system_ref(self.indices.history)
    }

    pub fn history_system_mut(&mut self) -> &mut HistorySystem {
        self.system_mut(self.indices.history)
    }

    pub fn selection_system(&self) -> &SelectionSystem {
        self.system_ref(self.indices.selection)
    }

    pub fn selection_system_mut(&mut self) -> &mut SelectionSystem {
        self.system_mut(self.indices.selection)
    }

    pub(super) fn project_system(&self) -> &ProjectSystem {
        self.system_ref(self.indices.project)
    }

    pub(super) fn project_system_mut(&mut self) -> &mut ProjectSystem {
        self.system_mut(self.indices.project)
    }

    pub fn script_editor_system(&self) -> &ScriptEditorSystem {
        self.system_ref(self.indices.script_editor)
    }

    pub fn script_editor_system_mut(&mut self) -> &mut ScriptEditorSystem {
        self.system_mut(self.indices.script_editor)
    }

    pub fn asset_browser_system(&self) -> &AssetBrowserSystem {
        self.system_ref(self.indices.asset_browser)
    }

    pub fn asset_browser_system_mut(&mut self) -> &mut AssetBrowserSystem {
        self.system_mut(self.indices.asset_browser)
    }
}

#[allow(dead_code)]
impl<'app, 'ctx, 'scene> EditorContext<'app, 'ctx, 'scene> {
    pub(crate) fn for_update(
        shared: &'app mut EditorSharedState,
        systems: EditorSystemsAccess<'app>,
        ctx: &'ctx mut UpdateContext<'scene>,
    ) -> EditorContext<'app, 'ctx, 'scene> {
        Self {
            shared: Some(shared),
            systems: Some(systems),
            update: Some(ctx),
            gpu: None,
            ui: None,
        }
    }

    pub(crate) fn for_gpu(
        shared: &'app mut EditorSharedState,
        systems: EditorSystemsAccess<'app>,
        ctx: &'ctx mut GpuUpdateContext<'scene>,
    ) -> EditorContext<'app, 'ctx, 'scene> {
        Self {
            shared: Some(shared),
            systems: Some(systems),
            update: None,
            gpu: Some(ctx),
            ui: None,
        }
    }

    fn shared(&self) -> &EditorSharedState {
        self.shared
            .as_deref()
            .expect("editor shared state available")
    }

    fn shared_mut(&mut self) -> &mut EditorSharedState {
        self.shared
            .as_deref_mut()
            .expect("editor shared state available")
    }

    fn systems(&self) -> &EditorSystemsAccess<'app> {
        self.systems
            .as_ref()
            .expect("editor systems access available")
    }

    fn systems_mut(&mut self) -> &mut EditorSystemsAccess<'app> {
        self.systems
            .as_mut()
            .expect("editor systems access available")
    }

    pub fn update_context_mut(&mut self) -> Option<&mut UpdateContext<'scene>> {
        self.update.as_deref_mut()
    }

    pub fn gpu_context_mut(&mut self) -> Option<&mut GpuUpdateContext<'scene>> {
        self.gpu.as_deref_mut()
    }

    pub fn with_update_app<R, F>(&mut self, f: F) -> Option<R>
    where
        F: FnOnce(&mut EditorAppAccess<'app>, &mut UpdateContext<'scene>) -> R,
    {
        let update = self.update.as_deref_mut()?;
        let shared = self
            .shared
            .take()
            .expect("editor shared state temporarily taken");
        let systems = self
            .systems
            .take()
            .expect("editor systems temporarily taken");
        let mut app = EditorAppAccess::new(shared, systems);
        let result = f(&mut app, update);
        let (shared, systems) = app.into_inner();
        self.shared = Some(shared);
        self.systems = Some(systems);
        Some(result)
    }

    pub fn with_gpu_app<R, F>(&mut self, f: F) -> Option<R>
    where
        F: FnOnce(&mut EditorAppAccess<'app>, &mut GpuUpdateContext<'scene>) -> R,
    {
        let gpu = self.gpu.as_deref_mut()?;
        let shared = self
            .shared
            .take()
            .expect("editor shared state temporarily taken");
        let systems = self
            .systems
            .take()
            .expect("editor systems temporarily taken");
        let mut app = EditorAppAccess::new(shared, systems);
        let result = f(&mut app, gpu);
        let (shared, systems) = app.into_inner();
        self.shared = Some(shared);
        self.systems = Some(systems);
        Some(result)
    }

    pub fn runtime_handle(&self) -> RuntimeStateHandle {
        self.shared().runtime_state.clone()
    }

    pub fn scene(&mut self) -> Option<&mut Scene> {
        if let Some(update) = self.update.as_deref_mut() {
            Some(update.scene)
        } else if let Some(gpu) = self.gpu.as_deref_mut() {
            Some(gpu.scene)
        } else {
            None
        }
    }

    pub fn asset_browser(&mut self) -> &mut AssetBrowserState {
        self.systems_mut().asset_browser_system_mut().state_mut()
    }

    pub fn asset_browser_system(&self) -> &AssetBrowserSystem {
        self.systems().asset_browser_system()
    }

    pub fn asset_browser_system_mut(&mut self) -> &mut AssetBrowserSystem {
        self.systems_mut().asset_browser_system_mut()
    }

    pub(super) fn project_system(&self) -> &ProjectSystem {
        self.systems().project_system()
    }

    pub(super) fn project_system_mut(&mut self) -> &mut ProjectSystem {
        self.systems_mut().project_system_mut()
    }

    pub fn script_editor_system(&self) -> &ScriptEditorSystem {
        self.systems().script_editor_system()
    }

    pub fn script_editor_system_mut(&mut self) -> &mut ScriptEditorSystem {
        self.systems_mut().script_editor_system_mut()
    }

    pub fn history(&mut self) -> &mut EditorHistory {
        self.systems_mut().history_system_mut().history_mut()
    }

    pub(super) fn command_queue(&mut self) -> &mut VecDeque<EditorCommand> {
        &mut self.shared_mut().commands
    }

    pub(super) fn events(&mut self) -> &mut Vec<EditorEvent> {
        &mut self.shared_mut().events
    }
}

#[allow(dead_code)]
impl<'app, 'ctx> EditorContext<'app, 'ctx, 'ctx> {
    pub(crate) fn for_ui(
        shared: &'app mut EditorSharedState,
        systems: EditorSystemsAccess<'app>,
        ctx: &'ctx EguiContext,
        default_ui: &'ctx mut DefaultUI,
    ) -> EditorContext<'app, 'ctx, 'ctx> {
        Self {
            shared: Some(shared),
            systems: Some(systems),
            update: None,
            gpu: None,
            ui: Some(EditorUiContext {
                egui: ctx,
                default_ui,
            }),
        }
    }

    pub fn ui_context(&mut self) -> Option<EditorUiContextMut<'_>> {
        self.ui.as_mut().map(|ui| EditorUiContextMut {
            egui: ui.egui,
            _default_ui: &mut *ui.default_ui,
        })
    }

    pub fn with_ui_app<R, F>(&mut self, f: F) -> Option<R>
    where
        F: FnOnce(&mut EditorAppAccess<'app>, EditorUiContextMut<'_>) -> R,
    {
        let ui = self.ui.as_mut()?;
        let shared = self
            .shared
            .take()
            .expect("editor shared state temporarily taken");
        let systems = self
            .systems
            .take()
            .expect("editor systems temporarily taken");
        let mut app = EditorAppAccess::new(shared, systems);
        let ui_ctx = EditorUiContextMut {
            egui: ui.egui,
            _default_ui: &mut *ui.default_ui,
        };
        let result = f(&mut app, ui_ctx);
        let (shared, systems) = app.into_inner();
        self.shared = Some(shared);
        self.systems = Some(systems);
        Some(result)
    }
}

#[allow(dead_code)]
pub trait EditorSystem: Any {
    fn update<'app, 'ctx, 'scene>(&mut self, _ctx: &mut EditorContext<'app, 'ctx, 'scene>) {}

    fn gpu_update<'app, 'ctx, 'scene>(&mut self, _ctx: &mut EditorContext<'app, 'ctx, 'scene>) {}

    fn ui<'app, 'ctx>(&mut self, _ctx: &mut EditorContext<'app, 'ctx, 'ctx>) {}

    fn as_any(&self) -> &dyn Any;

    fn as_any_mut(&mut self) -> &mut dyn Any;
}
