/// Action dispatcher - routes InspectorActions to their handlers
///
/// This replaces the massive 946-line match statement in apply_pending_inspector_actions()
/// with a cleaner dispatch mechanism that calls focused handler functions.

use crate::inspector::InspectorAction;
use super::{ActionContext, ActionResult};
use super::transform::handle_update_transform;
use super::camera::handle_update_camera;

/// Dispatch a single InspectorAction to its handler
///
/// This function routes each action to its specific handler function.
/// Handlers are organized by concern (transform, camera, lights, etc.)
/// and return ActionResult indicating what changed.
pub fn dispatch_action(
    ctx: &mut ActionContext,
    action: InspectorAction,
) -> ActionResult {
    match action {
        // Transform actions
        InspectorAction::UpdateTransform { entity, transform } => {
            handle_update_transform(ctx, entity, transform)
        }

        // Camera actions
        InspectorAction::UpdateCamera { entity, component } => {
            handle_update_camera(ctx, entity, component)
        }

        // TODO: Add remaining action handlers:
        // - UpdateMaterial
        // - UpdatePointLight / UpdateDirectionalLight / UpdateSpotLight
        // - UpdateEnvironment
        // - UpdateParticleSystem / UpdateParticleEmitter / UpdateParticleBehavior
        // - SetBillboard
        // - EditScript / ChangeScriptSource / AddScript
        // - EditShader / CreateShaderMaterial / SetMaterialKind / etc.
        // - Add* actions (AddCamera, AddMesh, AddPointLight, etc.)
        // - RenameEntity
        // - SetCanCastShadow
        //
        // For now, fall back to unhandled action (will be removed as handlers are added)
        _ => {
            log::warn!("Unhandled inspector action: {:?}", std::mem::discriminant(&action));
            ActionResult::no_change()
        }
    }
}
