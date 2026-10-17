//! Handlers for the navigation sections' requests: bake, debug view and path test.

use super::{ActionContext, ActionResult};
use crate::inspector::NavigationRequest;

pub fn handle_navigation_request(
    ctx: &mut ActionContext,
    request: NavigationRequest,
) -> ActionResult {
    match request {
        NavigationRequest::Bake { entity } => {
            if let Err(error) = ctx.scene.start_navmesh_bake(Some(entity)) {
                log::warn!("Navigation: cannot bake: {error}");
            }
        }
        NavigationRequest::SetDebug(debug) => ctx.scene.navigation_mut().set_debug_draw(debug),
        NavigationRequest::PathTool(active) => {
            ctx.app.shared.nav_path_tool.active = active;
        }
        NavigationRequest::ClearPath => {
            let tool = &mut ctx.app.shared.nav_path_tool;
            tool.from = None;
            tool.to = None;
            ctx.scene.navigation_mut().set_test_path(None);
        }
    }
    ActionResult::no_change()
}
