use hecs::Entity;
use std::path::PathBuf;
use wgpu_cube::asset::MaterialAsset;
use wgpu_cube::renderer::primitives::PrimitiveMeshDescriptor;
use wgpu_cube::renderer::Material;
use wgpu_cube::scene::components::PrimitiveMeshComponent;
use wgpu_cube::scene::{
    blend_post_process_volumes, CameraComponent, DirectionalLight, EnvironmentComponent,
    LightProbeVolume, MaterialComponent, MeshBounds, MeshComponent, ParticleEmitterComponent,
    ParticleSystemComponent, PointLight, PostProcessVolume, SpotLight,
};
use wgpu_cube::scripting::LuaScriptComponent;
use wgpu_cube::ScenePrimitivePreset;

use crate::inspector::RemovableComponent;

use super::{ActionContext, ActionResult};

/// Handle AddCamera action
pub fn handle_add_camera(ctx: &mut ActionContext, entity: Entity) -> ActionResult {
    let world = ctx.scene.main_world_mut();
    if world.get::<&CameraComponent>(entity).is_ok() {
        log::warn!("Entity {:?} already has a camera component", entity);
        ActionResult::no_change()
    } else {
        let camera = CameraComponent::default();
        match world.insert(entity, (camera,)) {
            Ok(_) => {
                log::info!("Added camera component to entity {:?}", entity);
                ActionResult::scene_changed()
            }
            Err(err) => {
                log::warn!("Failed to add camera component to {:?}: {}", entity, err);
                ActionResult::no_change()
            }
        }
    }
}

/// Handle AddMesh action
pub fn handle_add_mesh(ctx: &mut ActionContext, entity: Entity) -> ActionResult {
    // Check if entity already has a mesh component
    let world = ctx.scene.main_world_mut();
    if world.get::<&MeshComponent>(entity).is_ok() {
        log::warn!("Entity {:?} already has a mesh component", entity);
        return ActionResult::no_change();
    }

    // Try to use existing cube mesh (created during setup or previous usage)
    let descriptor = PrimitiveMeshDescriptor::from(ScenePrimitivePreset::Cube);
    if let Some(mesh_handle) = ctx.scene.assets.primitive_mesh_handle(descriptor) {
        let material_handle = ctx
            .scene
            .assets
            .materials
            .insert(MaterialAsset::from_material(
                Material::pbr(),
                PathBuf::new(),
            ));

        let world = ctx.scene.main_world_mut();
        match world.insert(
            entity,
            (
                MeshComponent(mesh_handle),
                MaterialComponent(material_handle),
            ),
        ) {
            Ok(_) => {
                log::info!("Added mesh component to entity {:?}", entity);
                ActionResult::scene_changed()
            }
            Err(err) => {
                log::warn!("Failed to add mesh component to {:?}: {}", entity, err);
                ActionResult::no_change()
            }
        }
    } else {
        log::warn!(
            "Cannot add mesh component: cube primitive not yet created. Try creating a cube primitive first."
        );
        ActionResult::no_change()
    }
}

/// Handle AddPointLight action
pub fn handle_add_point_light(ctx: &mut ActionContext, entity: Entity) -> ActionResult {
    let world = ctx.scene.main_world_mut();
    if world.get::<&PointLight>(entity).is_ok() {
        log::warn!("Entity {:?} already has a point light component", entity);
        ActionResult::no_change()
    } else {
        let light = PointLight::default();
        match world.insert(entity, (light,)) {
            Ok(_) => {
                log::info!("Added point light component to entity {:?}", entity);
                ActionResult::scene_changed()
            }
            Err(err) => {
                log::warn!(
                    "Failed to add point light component to {:?}: {}",
                    entity,
                    err
                );
                ActionResult::no_change()
            }
        }
    }
}

/// Handle AddDirectionalLight action
pub fn handle_add_directional_light(ctx: &mut ActionContext, entity: Entity) -> ActionResult {
    let world = ctx.scene.main_world_mut();
    if world.get::<&DirectionalLight>(entity).is_ok() {
        log::warn!(
            "Entity {:?} already has a directional light component",
            entity
        );
        ActionResult::no_change()
    } else {
        let light = DirectionalLight::default();
        match world.insert(entity, (light,)) {
            Ok(_) => {
                log::info!("Added directional light component to entity {:?}", entity);
                ActionResult::scene_changed()
            }
            Err(err) => {
                log::warn!(
                    "Failed to add directional light component to {:?}: {}",
                    entity,
                    err
                );
                ActionResult::no_change()
            }
        }
    }
}

/// Handle AddSpotLight action
pub fn handle_add_spot_light(ctx: &mut ActionContext, entity: Entity) -> ActionResult {
    let world = ctx.scene.main_world_mut();
    if world.get::<&SpotLight>(entity).is_ok() {
        log::warn!("Entity {:?} already has a spot light component", entity);
        ActionResult::no_change()
    } else {
        let light = SpotLight::default();
        match world.insert(entity, (light,)) {
            Ok(_) => {
                log::info!("Added spot light component to entity {:?}", entity);
                ActionResult::scene_changed()
            }
            Err(err) => {
                log::warn!(
                    "Failed to add spot light component to {:?}: {}",
                    entity,
                    err
                );
                ActionResult::no_change()
            }
        }
    }
}

/// Handle AddEnvironment action
pub fn handle_add_environment(ctx: &mut ActionContext, entity: Entity) -> ActionResult {
    // Check if entity already has an environment component
    let world = ctx.scene.main_world_mut();
    if world.get::<&EnvironmentComponent>(entity).is_ok() {
        log::warn!("Entity {:?} already has an environment component", entity);
        return ActionResult::no_change();
    }

    let component = EnvironmentComponent::from_environment(ctx.scene.environment());
    let world = ctx.scene.main_world_mut();
    match world.insert(entity, (component.clone(),)) {
        Ok(_) => {
            log::info!("Added environment component to entity {:?}", entity);
        }
        Err(err) => {
            log::warn!(
                "Failed to add environment component to {:?}: {}",
                entity,
                err
            );
            return ActionResult::no_change();
        }
    }

    ctx.scene.set_environment(component.to_environment());
    ActionResult::scene_changed()
}

/// Handle AddLightProbeVolume action: a volume of the default size around the entity.
/// Handle SetPhysics: add, replace or (with null) remove a physics or navigation
/// component.
pub fn handle_set_physics(
    ctx: &mut ActionContext,
    entity: Entity,
    component: &'static str,
    value: serde_json::Value,
) -> ActionResult {
    let world = ctx.scene.main_world_mut();
    #[cfg(feature = "navigation")]
    if wgpu_cube::navigation::edit::canonical_name(component).is_some() {
        let before = wgpu_cube::navigation::edit::component_json(world, entity, component);
        if before.as_ref() == Some(&value) || (before.is_none() && value.is_null()) {
            return ActionResult::no_change();
        }
        return match wgpu_cube::navigation::edit::patch_component(world, entity, component, &value)
        {
            Ok(()) => ActionResult::scene_changed(),
            Err(err) => {
                log::warn!("Cannot change {component} of {entity:?}: {err}");
                ActionResult::no_change()
            }
        };
    }
    let before = wgpu_cube::physics::edit::component_json(world, entity, component);
    if before.as_ref() == Some(&value) || (before.is_none() && value.is_null()) {
        return ActionResult::no_change();
    }
    match wgpu_cube::physics::edit::patch_component(world, entity, component, &value) {
        Ok(()) => ActionResult::scene_changed(),
        Err(err) => {
            log::warn!("Cannot change {component} of {entity:?}: {err}");
            ActionResult::no_change()
        }
    }
}

pub fn handle_add_light_probe_volume(ctx: &mut ActionContext, entity: Entity) -> ActionResult {
    let world = ctx.scene.main_world_mut();
    if world.get::<&LightProbeVolume>(entity).is_ok() {
        log::warn!("Entity {entity:?} already has a light probe volume");
        return ActionResult::no_change();
    }
    match world.insert_one(entity, LightProbeVolume::default()) {
        Ok(()) => ActionResult::scene_changed(),
        Err(err) => {
            log::warn!("Failed to add a light probe volume to {entity:?}: {err}");
            ActionResult::no_change()
        }
    }
}

/// Handle AddPostProcessVolume action: an infinite volume holding the settings in effect
/// now, so the picture does not change until it is edited.
pub fn handle_add_post_process_volume(ctx: &mut ActionContext, entity: Entity) -> ActionResult {
    let camera = ctx.scene.camera().eye;
    let settings = blend_post_process_volumes(ctx.scene.main_world(), camera).unwrap_or_default();
    let world = ctx.scene.main_world_mut();
    if world.get::<&PostProcessVolume>(entity).is_ok() {
        log::warn!("Entity {entity:?} already has a post-process volume");
        return ActionResult::no_change();
    }
    let first = world.query::<&PostProcessVolume>().iter().next().is_none();
    let volume = if first {
        PostProcessVolume::infinite(settings)
    } else {
        PostProcessVolume::additional(settings)
    };
    match world.insert_one(entity, volume) {
        Ok(()) => ActionResult::scene_changed(),
        Err(err) => {
            log::warn!("Failed to add a post-process volume to {entity:?}: {err}");
            ActionResult::no_change()
        }
    }
}

/// Handle AddParticleSystem action
pub fn handle_add_particle_system(ctx: &mut ActionContext, entity: Entity) -> ActionResult {
    let world = ctx.scene.main_world_mut();
    if world.get::<&ParticleSystemComponent>(entity).is_ok() {
        log::warn!(
            "Entity {:?} already has a particle system component",
            entity
        );
        ActionResult::no_change()
    } else {
        let component = ParticleSystemComponent::default();
        let emitter = ParticleEmitterComponent::default();
        match world.insert(entity, (component, emitter)) {
            Ok(_) => {
                log::info!("Added particle system component to entity {:?}", entity);
                ActionResult::scene_changed()
            }
            Err(err) => {
                log::warn!(
                    "Failed to add particle system component to {:?}: {}",
                    entity,
                    err
                );
                ActionResult::no_change()
            }
        }
    }
}

/// Handle RemoveComponent action
pub fn handle_remove_component(
    ctx: &mut ActionContext,
    entity: Entity,
    component: RemovableComponent,
) -> ActionResult {
    fn remove<T: hecs::Component>(world: &mut hecs::World, entity: Entity) -> bool {
        world.remove_one::<T>(entity).is_ok()
    }

    let world = ctx.scene.main_world_mut();
    let removed = match component {
        RemovableComponent::Camera => remove::<CameraComponent>(world, entity),
        RemovableComponent::Mesh => {
            // The inspector adds meshes together with a material, so remove both.
            let removed = remove::<MeshComponent>(world, entity);
            remove::<PrimitiveMeshComponent>(world, entity);
            remove::<MeshBounds>(world, entity);
            remove::<MaterialComponent>(world, entity);
            removed
        }
        RemovableComponent::PointLight => remove::<PointLight>(world, entity),
        RemovableComponent::DirectionalLight => remove::<DirectionalLight>(world, entity),
        RemovableComponent::SpotLight => remove::<SpotLight>(world, entity),
        RemovableComponent::Environment => remove::<EnvironmentComponent>(world, entity),
        RemovableComponent::PostProcessVolume => remove::<PostProcessVolume>(world, entity),
        RemovableComponent::LightProbeVolume => remove::<LightProbeVolume>(world, entity),
        RemovableComponent::ParticleSystem => {
            let removed = remove::<ParticleSystemComponent>(world, entity);
            remove::<ParticleEmitterComponent>(world, entity);
            removed
        }
        RemovableComponent::Script => remove::<LuaScriptComponent>(world, entity),
    };

    if !removed {
        log::warn!("Entity {entity:?} has no {} component", component.label());
        return ActionResult::no_change();
    }
    if component == RemovableComponent::Camera && ctx.scene.active_camera_entity() == Some(entity) {
        ctx.scene.set_active_camera_entity(None);
    }
    log::info!("Removed {} from entity {entity:?}", component.label());
    ActionResult::scene_changed()
}
