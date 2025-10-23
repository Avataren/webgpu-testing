use std::collections::VecDeque;
use std::path::PathBuf;

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
    application: &'app mut EditorApplication,
    update: Option<&'ctx mut UpdateContext<'scene>>,
    gpu: Option<&'ctx mut GpuUpdateContext<'scene>>,
    ui: Option<EditorUiContext<'ctx>>,
}

struct EditorUiContext<'ctx> {
    egui: &'ctx EguiContext,
    default_ui: &'ctx mut DefaultUI,
}

pub struct EditorUiContextMut<'ctx> {
    egui: &'ctx EguiContext,
    default_ui: &'ctx mut DefaultUI,
}

impl<'ctx> EditorUiContextMut<'ctx> {
    pub fn egui(&self) -> &'ctx EguiContext {
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
            application,
            update: Some(ctx),
            gpu: None,
            ui: None,
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
        }
    }

    pub fn application_mut(&mut self) -> &mut EditorApplication {
        self.application
    }

    pub fn update_context_mut(&mut self) -> Option<&mut UpdateContext<'scene>> {
        self.update.as_deref_mut()
    }

    pub fn gpu_context_mut(&mut self) -> Option<&mut GpuUpdateContext<'scene>> {
        self.gpu.as_deref_mut()
    }

    pub fn runtime_handle(&self) -> RuntimeStateHandle {
        self.application.runtime_state.clone()
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
        &mut self.application.asset_browser
    }

    pub fn history(&mut self) -> &mut EditorHistory {
        &mut self.application.history
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
        let update = self.update.as_deref_mut()?;
        let app = &mut *self.application;
        Some(f(app, update))
    }

    pub fn with_gpu<R, F>(&mut self, f: F) -> Option<R>
    where
        F: FnOnce(&mut EditorApplication, &mut GpuUpdateContext<'scene>) -> R,
    {
        let gpu = self.gpu.as_deref_mut()?;
        let app = &mut *self.application;
        Some(f(app, gpu))
    }
}

#[allow(dead_code)]
impl<'app, 'ctx> EditorContext<'app, 'ctx, 'ctx>
where
    'app: 'ctx,
{
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
        }
    }

    pub fn ui_context(&'ctx mut self) -> Option<EditorUiContextMut<'ctx>> {
        self.ui.as_mut().map(move |ui| EditorUiContextMut {
            egui: ui.egui,
            default_ui: ui.default_ui,
        })
    }

    pub fn with_ui<R, F>(&'ctx mut self, f: F) -> Option<R>
    where
        F: FnOnce(&mut EditorApplication, EditorUiContextMut<'ctx>) -> R,
    {
        let ui = self.ui.as_mut()?;
        let ui_ctx = EditorUiContextMut {
            egui: ui.egui,
            default_ui: ui.default_ui,
        };
        let result = {
            let app = &mut *self.application;
            f(app, ui_ctx)
        };
        Some(result)
    }
}

#[allow(dead_code)]
pub trait EditorSystem {
    fn update(&mut self, _ctx: &mut EditorContext<'_, '_, '_>) {}

    fn gpu_update(&mut self, _ctx: &mut EditorContext<'_, '_, '_>) {}

    fn ui(&mut self, _ctx: &mut EditorContext<'_, '_, '_>) {}
}
