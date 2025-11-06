use hecs::Entity;
use wgpu_cube::scene::{
    components::Billboard, ParticleBehaviorConfig, ParticleBehaviorPreset,
    ParticleEmitterComponent, ParticleSystemComponent,
};

use super::{ActionContext, ActionResult};

/// Handle UpdateParticleSystem action
pub fn handle_update_particle_system(
    ctx: &mut ActionContext,
    entity: Entity,
    component: ParticleSystemComponent,
) -> ActionResult {
    let new_spawn_rate = component.spawn_rate;
    let mut spawn_rate_changed = false;
    let mut updated = false;

    let world = ctx.scene.main_world_mut();
    match world.get::<&mut ParticleSystemComponent>(entity) {
        Ok(mut existing) => {
            spawn_rate_changed = (existing.spawn_rate - new_spawn_rate).abs() > f32::EPSILON;
            if *existing != component {
                *existing = component;
                updated = true;
            }
        }
        Err(err) => {
            log::warn!("Failed to update particle system for {:?}: {}", entity, err);
        }
    }

    if spawn_rate_changed {
        match world.get::<&mut ParticleEmitterComponent>(entity) {
            Ok(mut emitter) => {
                if (emitter.spawn_rate - new_spawn_rate).abs() > f32::EPSILON {
                    emitter.spawn_rate = new_spawn_rate;
                    updated = true;
                }
            }
            Err(err) => {
                log::warn!(
                    "Failed to update particle emitter spawn rate for {:?}: {}",
                    entity,
                    err
                );
            }
        }
    }

    if updated {
        ctx.app.record_scene_change(ctx.scene);
        ActionResult::scene_changed()
    } else {
        ActionResult::no_change()
    }
}

/// Handle UpdateParticleEmitter action
pub fn handle_update_particle_emitter(
    ctx: &mut ActionContext,
    entity: Entity,
    component: ParticleEmitterComponent,
) -> ActionResult {
    let updated = {
        let world = ctx.scene.main_world_mut();
        match world.get::<&mut ParticleEmitterComponent>(entity) {
            Ok(mut existing) => {
                if *existing != component {
                    *existing = component;
                    true
                } else {
                    false
                }
            }
            Err(err) => {
                log::warn!("Failed to update particle emitter for {:?}: {}", entity, err);
                false
            }
        }
    };

    if updated {
        ctx.app.record_scene_change(ctx.scene);
        ActionResult::scene_changed()
    } else {
        ActionResult::no_change()
    }
}

/// Handle UpdateParticleBehavior action
pub fn handle_update_particle_behavior(
    ctx: &mut ActionContext,
    entity: Entity,
    behavior: ParticleBehaviorPreset,
    config: ParticleBehaviorConfig,
) -> ActionResult {
    let mut updated = false;
    {
        let world = ctx.scene.main_world_mut();
        match world.get::<&mut ParticleSystemComponent>(entity) {
            Ok(mut existing) => {
                let config = config.ensure_variant(behavior);
                if existing.behavior != behavior || existing.behavior_config != config {
                    existing.behavior = behavior;
                    existing.behavior_config = config;
                    updated = true;
                }
            }
            Err(err) => {
                log::warn!("Failed to update particle behavior for {:?}: {}", entity, err);
            }
        }
    }

    if updated {
        ctx.app.record_scene_change(ctx.scene);
        ActionResult::scene_changed()
    } else {
        ActionResult::no_change()
    }
}

/// Handle SetBillboard action
pub fn handle_set_billboard(
    ctx: &mut ActionContext,
    entity: Entity,
    billboard: Option<Billboard>,
) -> ActionResult {
    let mut updated = false;
    let world = ctx.scene.main_world_mut();

    match billboard {
        Some(component) => {
            let mut needs_insert = false;
            match world.get::<&mut Billboard>(entity) {
                Ok(mut existing) => {
                    *existing = component;
                    updated = true;
                }
                Err(err) => {
                    log::debug!("Billboard missing for {:?} while enabling: {}", entity, err);
                    needs_insert = true;
                }
            }

            if needs_insert {
                match world.insert(entity, (component,)) {
                    Ok(_) => {
                        updated = true;
                    }
                    Err(insert_err) => {
                        log::warn!("Failed to add Billboard to {:?}: {}", entity, insert_err);
                    }
                }
            }
        }
        None => match world.remove_one::<Billboard>(entity) {
            Ok(_) => {
                updated = true;
            }
            Err(err) => {
                log::debug!("Billboard missing for {:?} while disabling: {}", entity, err);
            }
        },
    }

    if updated {
        ctx.app.record_scene_change(ctx.scene);
        ActionResult::scene_changed()
    } else {
        ActionResult::no_change()
    }
}
