use super::animation::{
    handle_add_animation_library, handle_generate_animation_tree, handle_import_animations,
    handle_make_character, handle_set_animation_tree, handle_set_animator,
};
use super::camera::handle_update_camera;
use super::components::{
    handle_add_camera, handle_add_directional_light, handle_add_environment,
    handle_add_light_probe_volume, handle_add_mesh, handle_add_particle_system,
    handle_add_point_light, handle_add_post_process_volume, handle_add_spot_light,
    handle_remove_component, handle_set_physics,
};
use super::environment::{
    handle_light_probe_request, handle_sky_request, handle_update_environment,
    handle_update_light_probe_volume, handle_update_post_process_volume,
};
use super::lights::{
    handle_update_directional_light, handle_update_point_light, handle_update_spot_light,
};
use super::materials::{
    handle_assign_shader_source, handle_create_shader_material, handle_create_shader_source,
    handle_set_material_kind, handle_update_material,
};
use super::misc::{handle_rename_entity, handle_set_can_cast_shadow};
use super::particles::{
    handle_set_billboard, handle_update_particle_behavior, handle_update_particle_emitter,
    handle_update_particle_system,
};
use super::scripts::{handle_add_script, handle_change_script_source, handle_edit_script};
use super::shader::handle_edit_shader;
use super::transform::handle_update_transform;
use super::{ActionContext, ActionResult};
use crate::application::EditorApplication;
/// Action dispatcher - routes InspectorActions to their handlers
///
/// This replaces the massive 946-line match statement in apply_pending_inspector_actions()
/// with a cleaner dispatch mechanism that calls focused handler functions.
use crate::inspector::InspectorAction;
use wgpu_cube::scene::SceneWorkspaceSceneMut;

/// Dispatch a single InspectorAction to its handler
///
/// This function routes each action to its specific handler function.
/// Handlers are organized by concern (transform, camera, lights, etc.)
/// and return ActionResult indicating what changed.
pub fn dispatch_action(
    scene: &mut SceneWorkspaceSceneMut<'_>,
    app: &mut EditorApplication,
    action: InspectorAction,
) -> ActionResult {
    let mut ctx = ActionContext { scene, app };
    match action {
        // Transform actions
        InspectorAction::UpdateTransform { entity, transform } => {
            handle_update_transform(&mut ctx, entity, transform)
        }

        // Camera actions
        InspectorAction::UpdateCamera { entity, component } => {
            handle_update_camera(&mut ctx, entity, component)
        }

        // Light actions
        InspectorAction::UpdatePointLight { entity, light } => {
            handle_update_point_light(&mut ctx, entity, light)
        }
        InspectorAction::UpdateDirectionalLight { entity, light } => {
            handle_update_directional_light(&mut ctx, entity, light)
        }
        InspectorAction::UpdateSpotLight { entity, light } => {
            handle_update_spot_light(&mut ctx, entity, light)
        }

        // Misc actions
        InspectorAction::SetCanCastShadow {
            entity,
            casts_shadow,
        } => handle_set_can_cast_shadow(&mut ctx, entity, casts_shadow),
        InspectorAction::RenameEntity { entity, new_name } => {
            handle_rename_entity(&mut ctx, entity, new_name)
        }

        // Add component actions
        InspectorAction::AddCamera { entity } => handle_add_camera(&mut ctx, entity),
        InspectorAction::AddMesh { entity } => handle_add_mesh(&mut ctx, entity),
        InspectorAction::AddPointLight { entity } => handle_add_point_light(&mut ctx, entity),
        InspectorAction::AddDirectionalLight { entity } => {
            handle_add_directional_light(&mut ctx, entity)
        }
        InspectorAction::AddSpotLight { entity } => handle_add_spot_light(&mut ctx, entity),
        InspectorAction::AddEnvironment { entity } => handle_add_environment(&mut ctx, entity),
        InspectorAction::AddPostProcessVolume { entity } => {
            handle_add_post_process_volume(&mut ctx, entity)
        }
        InspectorAction::AddLightProbeVolume { entity } => {
            handle_add_light_probe_volume(&mut ctx, entity)
        }
        InspectorAction::AddParticleSystem { entity } => {
            handle_add_particle_system(&mut ctx, entity)
        }
        InspectorAction::SetPhysics {
            entity,
            component,
            value,
        } => handle_set_physics(&mut ctx, entity, component, value),

        // Animation actions
        InspectorAction::SetAnimator { entity, animator } => {
            handle_set_animator(&mut ctx, entity, animator)
        }
        InspectorAction::AddAnimationLibrary { entity, path } => {
            handle_add_animation_library(&mut ctx, entity, path)
        }
        InspectorAction::ImportAnimations { entity, path } => {
            handle_import_animations(&mut ctx, entity, path)
        }
        InspectorAction::SetAnimationTree { entity, tree } => {
            handle_set_animation_tree(&mut ctx, entity, tree)
        }
        InspectorAction::GenerateAnimationTree { entity } => {
            handle_generate_animation_tree(&mut ctx, entity)
        }
        InspectorAction::MakeCharacter { entity, script } => {
            handle_make_character(&mut ctx, entity, script)
        }

        // Material actions
        InspectorAction::UpdateMaterial {
            entity,
            handle,
            material,
        } => handle_update_material(&mut ctx, entity, handle, material),
        InspectorAction::SetMaterialKind {
            entity,
            handle,
            kind,
        } => handle_set_material_kind(&mut ctx, entity, handle, kind),
        InspectorAction::AssignShaderSource {
            entity,
            handle,
            shader_path,
        } => handle_assign_shader_source(&mut ctx, entity, handle, shader_path),
        InspectorAction::CreateShaderSource {
            entity,
            handle,
            suggested_stem,
        } => handle_create_shader_source(&mut ctx, entity, handle, suggested_stem),
        InspectorAction::CreateShaderMaterial { entity, source } => {
            handle_create_shader_material(&mut ctx, entity, source)
        }

        // Script actions
        InspectorAction::AddScript { entity } => handle_add_script(&mut ctx, entity),
        InspectorAction::ChangeScriptSource {
            entity,
            script_path,
        } => handle_change_script_source(&mut ctx, entity, script_path),
        InspectorAction::EditScript { entity, component } => {
            handle_edit_script(&mut ctx, entity, component)
        }
        InspectorAction::RemoveComponent { entity, component } => {
            handle_remove_component(&mut ctx, entity, component)
        }

        // Particle actions
        InspectorAction::UpdateParticleSystem { entity, component } => {
            handle_update_particle_system(&mut ctx, entity, component)
        }
        InspectorAction::UpdateParticleEmitter { entity, component } => {
            handle_update_particle_emitter(&mut ctx, entity, component)
        }
        InspectorAction::UpdateParticleBehavior {
            entity,
            behavior,
            config,
        } => handle_update_particle_behavior(&mut ctx, entity, behavior, config),
        InspectorAction::SetBillboard { entity, billboard } => {
            handle_set_billboard(&mut ctx, entity, billboard)
        }

        // Environment actions
        InspectorAction::UpdateEnvironment { entity, component } => {
            handle_update_environment(&mut ctx, entity, component)
        }
        InspectorAction::UpdatePostProcessVolume { entity, component } => {
            handle_update_post_process_volume(&mut ctx, entity, *component)
        }
        InspectorAction::UpdateLightProbeVolume { entity, component } => {
            handle_update_light_probe_volume(&mut ctx, entity, component)
        }
        InspectorAction::LightProbes(request) => handle_light_probe_request(&mut ctx, request),
        InspectorAction::Sky(request) => handle_sky_request(&mut ctx, request),
        #[cfg(feature = "navigation")]
        InspectorAction::Navigation(request) => {
            super::navigation::handle_navigation_request(&mut ctx, request)
        }
        InspectorAction::Ai(request) => super::ai::handle_ai_request(&mut ctx, request),

        // Shader actions
        InspectorAction::EditShader {
            entity,
            handle,
            metadata,
        } => handle_edit_shader(&mut ctx, entity, handle, metadata),
    }
}
