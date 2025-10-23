use std::any::Any;
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::ptr::NonNull;

use super::core::{EditorApplication, PendingScriptAction};
use crate::asset_browser::AssetBrowserState;
use crate::history::EditorHistory;
use crate::inspector::InspectorAction;
use egui::Context as EguiContext;
use hecs::Entity;
use wgpu_cube::app::{GpuUpdateContext, RuntimeStateHandle, UpdateContext};
use wgpu_cube::scene::Scene;
use wgpu_cube::DefaultUI;
use wgpu_cube::SceneCreationAction;

pub(super) enum EditorCommand {
    ImportPath(PathBuf),
    DeleteEntity(Entity),
    Inspector(InspectorAction),
    CreateScene(SceneCreationAction),
    Script(PendingScriptAction),
}

#[derive(Debug)]
pub(super) enum EditorEvent {}

pub struct EditorContext<'app, 'ctx, 'scene> {
    application: NonNull<EditorApplication>,
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

#[allow(dead_code)]
impl<'app, 'ctx, 'scene> EditorContext<'app, 'ctx, 'scene> {
    pub(crate) fn for_update(
        application: &'app mut EditorApplication,
        ctx: &'ctx mut UpdateContext<'scene>,
    ) -> EditorContext<'app, 'ctx, 'scene> {
        Self {
            application: NonNull::from(application),
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
            application: NonNull::from(application),
            update: None,
            gpu: Some(ctx),
            ui: None,
            _marker: PhantomData,
        }
    }

    pub fn application_mut(&mut self) -> &mut EditorApplication {
        unsafe { self.application.as_mut() }
    }

    pub fn update_context_mut(&mut self) -> Option<&mut UpdateContext<'scene>> {
        self.update.as_deref_mut()
    }

    pub fn gpu_context_mut(&mut self) -> Option<&mut GpuUpdateContext<'scene>> {
        self.gpu.as_deref_mut()
    }

    pub fn runtime_handle(&self) -> RuntimeStateHandle {
        unsafe { self.application.as_ref().runtime_state.clone() }
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
        unsafe { &mut self.application.as_mut().asset_browser }
    }

    pub fn history(&mut self) -> &mut EditorHistory {
        unsafe { self.application.as_mut().history_mut() }
    }

    pub(super) fn command_queue(&mut self) -> &mut VecDeque<EditorCommand> {
        unsafe { &mut self.application.as_mut().commands }
    }

    pub(super) fn events(&mut self) -> &mut Vec<EditorEvent> {
        unsafe { &mut self.application.as_mut().events }
    }

    pub fn with_update<R, F>(&mut self, f: F) -> Option<R>
    where
        F: FnOnce(&mut EditorApplication, &mut UpdateContext<'scene>) -> R,
    {
        let update = self.update.as_deref_mut()?;
        let app = unsafe { self.application.as_mut() };
        Some(f(app, update))
    }

    pub fn with_gpu<R, F>(&mut self, f: F) -> Option<R>
    where
        F: FnOnce(&mut EditorApplication, &mut GpuUpdateContext<'scene>) -> R,
    {
        let gpu = self.gpu.as_deref_mut()?;
        let app = unsafe { self.application.as_mut() };
        Some(f(app, gpu))
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
            application: NonNull::from(application),
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

    pub fn with_ui<R>(
        &mut self,
        f: impl FnOnce(&mut EditorApplication, EditorUiContextMut<'_>) -> R,
    ) -> Option<R> {
        let ui = self.ui.as_mut()?;
        let ui_ctx = EditorUiContextMut {
            egui: ui.egui,
            default_ui: &mut *ui.default_ui,
        };
        let app = unsafe { self.application.as_mut() };
        Some(f(app, ui_ctx))
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
