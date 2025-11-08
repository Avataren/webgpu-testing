use hecs::Entity;
use std::path::PathBuf;
use wgpu_cube::asset::MaterialAsset;
use wgpu_cube::renderer::primitives::PrimitiveMeshDescriptor;
use wgpu_cube::renderer::Material;
use wgpu_cube::scene::{
    CameraComponent, DirectionalLight, EnvironmentComponent, MaterialComponent, MeshComponent,
    ParticleEmitterComponent, ParticleSystemComponent, PointLight, SpotLight,
};
use wgpu_cube::ScenePrimitivePreset;

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
                ctx.app.record_scene_change(ctx.scene);
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
                ctx.app.record_scene_change(ctx.scene);
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
                ctx.app.record_scene_change(ctx.scene);
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
                ctx.app.record_scene_change(ctx.scene);
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
                ctx.app.record_scene_change(ctx.scene);
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
    ctx.app.record_scene_change(ctx.scene);
    ActionResult::scene_changed()
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
                ctx.app.record_scene_change(ctx.scene);
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
