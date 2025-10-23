use wgpu_cube::app::RuntimeMode;

use super::core::EditorApplication;
use super::system::{EditorContext, EditorSystem};
use crate::camera::EditorCameraController;

#[derive(Default)]
pub(crate) struct CameraSystem {
    controller: EditorCameraController,
}

impl CameraSystem {
    pub(crate) fn new(controller: EditorCameraController) -> Self {
        Self { controller }
    }

    pub(crate) fn is_looking(&self) -> bool {
        self.controller.is_looking()
    }
}

impl EditorSystem for CameraSystem {
    fn update<'app, 'ctx, 'scene>(&mut self, ctx: &mut EditorContext<'app, 'ctx, 'scene>) {
        let Some(()) = ctx.with_update(|app, update_ctx| {
            if matches!(update_ctx.runtime, RuntimeMode::Editor) {
                self.controller.update_camera(update_ctx);
            }

            let hovered_handle = if let Some(drag) = app.history_system().gizmo_drag() {
                Some(drag.handle)
            } else if matches!(update_ctx.runtime, RuntimeMode::Editor) {
                let selection = app.selection_system();
                if let (Some(uv), Some(region)) = (
                    selection.pointer_scene_uv(),
                    app.viewports.scene_viewport.region(),
                ) {
                    let width = region.width().max(1) as f32;
                    let height = region.height().max(1) as f32;
                    let aspect = width / height;
                    let camera = update_ctx.scene.camera();
                    let (origin, direction) = EditorApplication::ray_from_uv(camera, uv, aspect);
                    update_ctx.scene.transform_gizmo_hit(origin, direction)
                } else {
                    None
                }
            } else {
                None
            };

            update_ctx.scene.set_transform_gizmo_hover(hovered_handle);
        }) else {
            return;
        };
    }

    fn ui<'app, 'ctx>(&mut self, ctx: &mut EditorContext<'app, 'ctx, 'ctx>) {
        let _ = ctx.with_ui(|app, ui_ctx| {
            let is_playing = matches!(app.runtime_state.active_mode(), RuntimeMode::Playing);
            if is_playing {
                self.controller.set_viewport_rect(None);
            } else {
                self.controller
                    .set_viewport_rect(app.viewports.scene_viewport.rect());
            }

            self.controller.capture_input(ui_ctx.egui());
        });
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
