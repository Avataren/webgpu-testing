/// Action dispatcher - routes InspectorActions to their handlers
///
/// This replaces the massive 946-line match statement in apply_pending_inspector_actions()
/// with a cleaner dispatch mechanism that calls focused handler functions.

use crate::inspector::InspectorAction;
use super::{ActionContext, ActionResult};
use super::transform::handle_update_transform;
use super::camera::handle_update_camera;
use super::lights::{
    handle_update_point_light, handle_update_directional_light, handle_update_spot_light,
};
use super::misc::{handle_set_can_cast_shadow, handle_rename_entity};
use super::components::{
    handle_add_camera, handle_add_mesh, handle_add_point_light, handle_add_directional_light,
    handle_add_spot_light, handle_add_environment, handle_add_particle_system,
};
use super::materials::{
    handle_update_material, handle_set_material_kind, handle_assign_shader_source,
    handle_create_shader_source, handle_create_shader_material,
};
use super::scripts::{
    handle_add_script, handle_change_script_source, handle_edit_script,
};

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

        // Light actions
        InspectorAction::UpdatePointLight { entity, light } => {
            handle_update_point_light(ctx, entity, light)
        }
        InspectorAction::UpdateDirectionalLight { entity, light } => {
            handle_update_directional_light(ctx, entity, light)
        }
        InspectorAction::UpdateSpotLight { entity, light } => {
            handle_update_spot_light(ctx, entity, light)
        }

        // Misc actions
        InspectorAction::SetCanCastShadow { entity, casts_shadow } => {
            handle_set_can_cast_shadow(ctx, entity, casts_shadow)
        }
        InspectorAction::RenameEntity { entity, new_name } => {
            handle_rename_entity(ctx, entity, new_name)
        }

        // Add component actions
        InspectorAction::AddCamera { entity } => {
            handle_add_camera(ctx, entity)
        }
        InspectorAction::AddMesh { entity } => {
            handle_add_mesh(ctx, entity)
        }
        InspectorAction::AddPointLight { entity } => {
            handle_add_point_light(ctx, entity)
        }
        InspectorAction::AddDirectionalLight { entity } => {
            handle_add_directional_light(ctx, entity)
        }
        InspectorAction::AddSpotLight { entity } => {
            handle_add_spot_light(ctx, entity)
        }
        InspectorAction::AddEnvironment { entity } => {
            handle_add_environment(ctx, entity)
        }
        InspectorAction::AddParticleSystem { entity } => {
            handle_add_particle_system(ctx, entity)
        }

        // Material actions
        InspectorAction::UpdateMaterial { entity, handle, material } => {
            handle_update_material(ctx, entity, handle, material)
        }
        InspectorAction::SetMaterialKind { entity, handle, kind } => {
            handle_set_material_kind(ctx, entity, handle, kind)
        }
        InspectorAction::AssignShaderSource { entity, handle, shader_path } => {
            handle_assign_shader_source(ctx, entity, handle, shader_path)
        }
        InspectorAction::CreateShaderSource { entity, handle, suggested_stem } => {
            handle_create_shader_source(ctx, entity, handle, suggested_stem)
        }
        InspectorAction::CreateShaderMaterial { entity, source } => {
            handle_create_shader_material(ctx, entity, source)
        }

        // Script actions
        InspectorAction::AddScript { entity } => {
            handle_add_script(ctx, entity)
        }
        InspectorAction::ChangeScriptSource { entity, script_path } => {
            handle_change_script_source(ctx, entity, script_path)
        }
        InspectorAction::EditScript { entity, component } => {
            handle_edit_script(ctx, entity, component)
        }

        // TODO: Add remaining action handlers:
        // - Environment: UpdateEnvironment
        // - Particles: UpdateParticleSystem, UpdateParticleEmitter, UpdateParticleBehavior, SetBillboard
        // - Shader: EditShader
        //
        // For now, fall back to unhandled action (will be removed as handlers are added)
        _ => {
            log::warn!("Unhandled inspector action: {:?}", std::mem::discriminant(&action));
            ActionResult::no_change()
        }
    }
}
