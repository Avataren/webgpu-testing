//! Handlers for the AI section's requests: the debug view, restarting and pausing.

use super::{ActionContext, ActionResult};
use crate::inspector::AiRequest;

pub fn handle_ai_request(ctx: &mut ActionContext, request: AiRequest) -> ActionResult {
    let ai = ctx.scene.ai_mut();
    match request {
        AiRequest::SetDebug(debug) => ai.set_debug_draw(debug),
        AiRequest::Restart(entity) => ai.shared().queue(wgpu_cube::ai::AiCommand::Restart(entity)),
        AiRequest::Pause(entity, paused) => {
            if let Some(agent) = ai.shared().agents.borrow_mut().get_mut(&entity) {
                agent.paused = paused;
            }
        }
    }
    ActionResult::no_change()
}
