use std::collections::VecDeque;

use super::core::EditorApplication;
use crate::asset_browser::AssetBrowserState;
use crate::history::EditorHistory;
use egui::Context as EguiContext;
use wgpu_cube::app::{GpuUpdateContext, RuntimeStateHandle, UpdateContext};
use wgpu_cube::scene::Scene;
use wgpu_cube::DefaultUI;

#[derive(Debug)]
pub enum EditorCommand {}

#[derive(Debug)]
pub enum EditorEvent {}

pub struct EditorContext<'a> {
    application: &'a mut EditorApplication,
    update: Option<&'a mut UpdateContext<'a>>,
    gpu: Option<&'a mut GpuUpdateContext<'a>>,
    ui: Option<EditorUiContext<'a>>,
}

struct EditorUiContext<'a> {
    egui: &'a EguiContext,
    default_ui: &'a mut DefaultUI,
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

impl<'a> EditorContext<'a> {
    pub(crate) fn for_update(
        application: &'a mut EditorApplication,
        ctx: &'a mut UpdateContext<'a>,
    ) -> Self {
        Self {
            application,
            update: Some(ctx),
            gpu: None,
            ui: None,
        }
    }

    pub(crate) fn for_gpu(
        application: &'a mut EditorApplication,
        ctx: &'a mut GpuUpdateContext<'a>,
    ) -> Self {
        Self {
            application,
            update: None,
            gpu: Some(ctx),
            ui: None,
        }
    }

    pub(crate) fn for_ui(
        application: &'a mut EditorApplication,
        ctx: &'a EguiContext,
        default_ui: &'a mut DefaultUI,
    ) -> Self {
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

    pub fn application_mut(&mut self) -> &mut EditorApplication {
        self.application
    }

    pub fn update_context_mut(&mut self) -> Option<&mut UpdateContext<'a>> {
        self.update.as_mut()
    }

    pub fn gpu_context_mut(&mut self) -> Option<&mut GpuUpdateContext<'a>> {
        self.gpu.as_mut()
    }

    pub fn ui_context(&mut self) -> Option<EditorUiContextMut<'_>> {
        self.ui.as_mut().map(|ui| EditorUiContextMut {
            egui: ui.egui,
            default_ui: ui.default_ui,
        })
    }

    pub fn runtime_handle(&self) -> RuntimeStateHandle {
        self.application.runtime_state.clone()
    }

    pub fn scene(&mut self) -> Option<&mut Scene> {
        if let Some(update) = self.update.as_mut() {
            Some(update.scene)
        } else if let Some(gpu) = self.gpu.as_mut() {
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

    pub fn command_queue(&mut self) -> &mut VecDeque<EditorCommand> {
        &mut self.application.commands
    }

    pub fn events(&mut self) -> &mut Vec<EditorEvent> {
        &mut self.application.events
    }
}

pub trait EditorSystem {
    fn update(&mut self, _ctx: &mut EditorContext<'_>) {}

    fn gpu_update(&mut self, _ctx: &mut EditorContext<'_>) {}

    fn ui(&mut self, _ctx: &mut EditorContext<'_>) {}
}
