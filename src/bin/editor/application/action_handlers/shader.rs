use hecs::Entity;
use wgpu_cube::asset::MaterialAsset;

use super::{ActionContext, ActionResult};

/// Handle EditShader action
/// Note: Shader edits are handled immediately in the UI stage via shader editor system
/// This handler is a no-op as the action is processed synchronously
pub fn handle_edit_shader(
    _ctx: &mut ActionContext,
    _entity: Entity,
    _handle: slotmap::DefaultKey,
    _material_asset: MaterialAsset,
) -> ActionResult {
    // Shader edits are handled immediately in the UI stage
    ActionResult::no_change()
}
