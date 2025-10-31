use std::any::Any;
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::path::PathBuf;
// Use a direct mutable borrow instead of a raw pointer for safety.

use super::asset_browser_system::AssetBrowserSystem;
use super::camera_system::CameraSystem;
use super::core::{EditorApplication, PendingScriptAction};
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
use wgpu_cube::app::{GpuUpdateContext, RuntimeStateHandle, UpdateContext};
use wgpu_cube::renderer::RenderRegion;
use wgpu_cube::scene::Scene;
use wgpu_cube::DefaultUI;
use wgpu_cube::SceneCreationAction;

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
    application: &'app mut EditorApplication,
    update: Option<&'ctx mut UpdateContext<'scene>>,
    gpu: Option<&'ctx mut GpuUpdateContext<'scene>>,
    ui: Option<EditorUiContext<'ctx>>,
    _marker: PhantomData<&'app mut EditorApplication>,
}

struct EditorUiContext<'ctx> {
    egui: &'ctx EguiContext,
    default_ui: &'ctx mut DefaultUI,
}

pub struct EditorUiContextMut<'a> {
    egui: &'a EguiContext,
    default_ui: &'a mut DefaultUI,
}

impl<'a> EditorUiContextMut<'a> {
    pub fn egui(&self) -> &'a EguiContext {
        self.egui
    }

    pub fn default_ui(&mut self) -> &mut DefaultUI {
        self.default_ui
    }
}

pub struct EditorAppAccess<'app> {
    application: &'app mut EditorApplication,
}

impl<'app> EditorAppAccess<'app> {
    fn new(application: &'app mut EditorApplication) -> Self {
        Self { application }
    }

    pub fn runtime_state(&self) -> RuntimeStateHandle {
        self.application.shared.runtime_state.clone()
    }

    pub fn scene_viewport(&self) -> &ViewportState {
        &self.application.shared.viewports.scene_viewport
    }

    pub fn camera_system(&self) -> &CameraSystem {
        self.application.camera_system()
    }

    pub fn selection_system_mut(&mut self) -> &mut SelectionSystem {
        self.application.selection_system_mut()
    }

    pub fn update_history_selection(&mut self, scene: &Scene) {
        self.application.update_history_selection(scene);
    }

    pub fn pick_entity(
        &self,
        ctx: &UpdateContext<'_>,
        uv: Vec2,
        region: RenderRegion,
    ) -> Option<Entity> {
        self.application.pick_entity(ctx, uv, region)
    }

    pub fn application_mut(&mut self) -> &mut EditorApplication {
        self.application
    }
}

#[allow(dead_code)]
impl<'app, 'ctx, 'scene> EditorContext<'app, 'ctx, 'scene> {
    pub(crate) fn for_update(
        application: &'app mut EditorApplication,
        ctx: &'ctx mut UpdateContext<'scene>,
    ) -> EditorContext<'app, 'ctx, 'scene> {
        Self {
            application,
            update: Some(ctx),
            gpu: None,
            ui: None,
            _marker: PhantomData,
        }
    }

    pub(crate) fn for_gpu(
        application: &'app mut EditorApplication,
        ctx: &'ctx mut GpuUpdateContext<'scene>,
    ) -> EditorContext<'app, 'ctx, 'scene> {
        Self {
            application,
            update: None,
            gpu: Some(ctx),
            ui: None,
            _marker: PhantomData,
        }
    }

    pub fn application_mut(&mut self) -> &mut EditorApplication {
        &mut *self.application
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
        let mut app = EditorAppAccess::new(&mut *self.application);
        let result = f(&mut app, update);
        Some(result)
    }

    pub fn with_gpu_app<R, F>(&mut self, f: F) -> Option<R>
    where
        F: FnOnce(&mut EditorAppAccess<'app>, &mut GpuUpdateContext<'scene>) -> R,
    {
        let gpu = self.gpu.as_deref_mut()?;
        let mut app = EditorAppAccess::new(&mut *self.application);
        let result = f(&mut app, gpu);
        Some(result)
    }

    pub fn runtime_handle(&self) -> RuntimeStateHandle {
        (&*self.application).runtime_state.clone()
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
        self.application.asset_browser_state_mut()
    }

    pub fn asset_browser_system(&self) -> &AssetBrowserSystem {
        (&*self.application).asset_browser_system()
    }

    pub fn asset_browser_system_mut(&mut self) -> &mut AssetBrowserSystem {
        self.application.asset_browser_system_mut()
    }

    pub(super) fn project_system(&self) -> &ProjectSystem {
        (&*self.application).project_system()
    }

    pub(super) fn project_system_mut(&mut self) -> &mut ProjectSystem {
        self.application.project_system_mut()
    }

    pub fn script_editor_system(&self) -> &ScriptEditorSystem {
        (&*self.application).script_editor_system()
    }

    pub fn script_editor_system_mut(&mut self) -> &mut ScriptEditorSystem {
        self.application.script_editor_system_mut()
    }

    pub fn history(&mut self) -> &mut EditorHistory {
        self.application.history_mut()
    }

    pub(super) fn command_queue(&mut self) -> &mut VecDeque<EditorCommand> {
        &mut self.application.commands
    }

    pub(super) fn events(&mut self) -> &mut Vec<EditorEvent> {
        &mut self.application.events
    }

    pub fn with_update<R, F>(&mut self, f: F) -> Option<R>
    where
        F: FnOnce(&mut EditorApplication, &mut UpdateContext<'scene>) -> R,
    {
        self.with_update_app(|app, update| f(app.application_mut(), update))
    }

    pub fn with_gpu<R, F>(&mut self, f: F) -> Option<R>
    where
        F: FnOnce(&mut EditorApplication, &mut GpuUpdateContext<'scene>) -> R,
    {
        self.with_gpu_app(|app, gpu| f(app.application_mut(), gpu))
    }
}

#[allow(dead_code)]
impl<'app, 'ctx> EditorContext<'app, 'ctx, 'ctx> {
    pub(crate) fn for_ui(
        application: &'app mut EditorApplication,
        ctx: &'ctx EguiContext,
        default_ui: &'ctx mut DefaultUI,
    ) -> EditorContext<'app, 'ctx, 'ctx> {
        Self {
            application,
            update: None,
            gpu: None,
            ui: Some(EditorUiContext {
                egui: ctx,
                default_ui,
            }),
            _marker: PhantomData,
        }
    }

    pub fn ui_context(&mut self) -> Option<EditorUiContextMut<'_>> {
        self.ui.as_mut().map(|ui| EditorUiContextMut {
            egui: ui.egui,
            default_ui: &mut *ui.default_ui,
        })
    }

    pub fn with_ui_app<R, F>(&mut self, f: F) -> Option<R>
    where
        F: FnOnce(&mut EditorAppAccess<'app>, EditorUiContextMut<'_>) -> R,
    {
        let ui = self.ui.as_mut()?;
        let ui_ctx = EditorUiContextMut {
            egui: ui.egui,
            default_ui: &mut *ui.default_ui,
        };
        let mut app = EditorAppAccess::new(&mut *self.application);
        let result = f(&mut app, ui_ctx);
        Some(result)
    }

    pub fn with_ui<R>(
        &mut self,
        f: impl FnOnce(&mut EditorApplication, EditorUiContextMut<'_>) -> R,
    ) -> Option<R> {
        self.with_ui_app(|app, ui_ctx| f(app.application_mut(), ui_ctx))
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
