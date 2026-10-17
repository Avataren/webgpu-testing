use wgpu_cube::app::RuntimeMode;

use super::system::{EditorContext, EditorSystem};
use crate::inspector::AiPanelStatus;

/// Refreshes what the AI section of the inspector shows: the debug view, the brains
/// defined and, in play, the selected agent's mind.
#[derive(Default)]
pub(crate) struct AiPanelSystem;

impl EditorSystem for AiPanelSystem {
    fn gpu_update<'app, 'ctx, 'scene>(&mut self, ctx: &mut EditorContext<'app, 'ctx, 'scene>) {
        ctx.with_gpu_app(|app, gpu| {
            let playing = app.runtime_state().active_mode() == RuntimeMode::Playing;
            let ai = gpu.scene.ai();
            let selected = app
                .selection_system()
                .selected()
                .filter(|_| playing)
                .and_then(|entity| ai.inspect(gpu.scene.main_world(), entity));
            let brains = ai.shared().library.borrow().names();
            *app.ai_status_mut() = AiPanelStatus {
                playing,
                debug: ai.debug_draw(),
                brains,
                selected,
            };
        });
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
