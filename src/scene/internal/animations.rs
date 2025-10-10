use crate::scene::animation::{AnimationClip, AnimationState, MaterialUpdate, TransformUpdate};
use crate::scene::components::{
    GltfMaterial, MaterialComponent, OrbitAnimation, RotateAnimation, TransformComponent,
};
use glam::{Quat, Vec3};
use hecs::World;
use rayon::prelude::*;
use std::collections::HashMap;

pub(crate) fn advance_animations(
    world: &mut World,
    animations: &[AnimationClip],
    animation_states: &mut Vec<AnimationState>,
    dt: f64,
) {
    if animation_states.is_empty() || animations.is_empty() {
        return;
    }

    let dt = dt as f32;

    let mut transform_updates: HashMap<hecs::Entity, TransformUpdate> = HashMap::new();
    let mut material_updates: HashMap<usize, MaterialUpdate> = HashMap::new();

    for state in animation_states.iter_mut() {
        if state.clip_index >= animations.len() {
            continue;
        }

        let clip = &animations[state.clip_index];
        let sample_time = state.advance(dt, clip.duration);
        clip.sample(sample_time, &mut transform_updates, &mut material_updates);
    }

    for (entity, update) in transform_updates {
        apply_transform_update(world, entity, update);
    }

    apply_material_updates(world, material_updates);
}

pub(crate) fn update_rotate_animations(world: &mut World, dt: f64) {
    let entities: Vec<_> = world
        .query::<(&TransformComponent, &RotateAnimation)>()
        .iter()
        .map(|(entity, (transform, anim))| (entity, transform.0, *anim))
        .collect();

    let updates: Vec<_> = entities
        .par_iter()
        .map(|(entity, transform, anim)| {
            let rotation = Quat::from_axis_angle(anim.axis, anim.speed * dt as f32);
            let new_rotation = rotation * transform.rotation;
            (*entity, new_rotation)
        })
        .collect();

    for (entity, new_rotation) in updates {
        if let Ok(mut transform) = world.get::<&mut TransformComponent>(entity) {
            transform.0.rotation = new_rotation;
        }
    }
}

pub(crate) fn update_orbit_animations(world: &mut World, time: f64) {
    let time = time as f32;

    let entities: Vec<_> = world
        .query::<(&TransformComponent, &OrbitAnimation)>()
        .iter()
        .map(|(entity, (_, orbit))| (entity, *orbit))
        .collect();

    let updates: Vec<_> = entities
        .par_iter()
        .map(|(entity, orbit)| {
            let angle = time * orbit.speed + orbit.offset;
            let new_translation = orbit.center
                + Vec3::new(
                    angle.cos() * orbit.radius,
                    (time + orbit.offset).sin() * 0.5,
                    angle.sin() * orbit.radius,
                );
            (*entity, new_translation)
        })
        .collect();

    for (entity, new_translation) in updates {
        if let Ok(mut transform) = world.get::<&mut TransformComponent>(entity) {
            transform.0.translation = new_translation;
        }
    }
}

fn apply_transform_update(world: &mut World, entity: hecs::Entity, update: TransformUpdate) {
    if let Ok(mut transform) = world.get::<&mut TransformComponent>(entity) {
        if let Some(translation) = update.translation {
            transform.0.translation = translation;
        }

        if let Some(rotation) = update.rotation {
            transform.0.rotation = rotation;
        }

        if let Some(scale) = update.scale {
            transform.0.scale = scale;
        }
    }
}

fn apply_material_updates(world: &mut World, material_updates: HashMap<usize, MaterialUpdate>) {
    if material_updates.is_empty() {
        return;
    }

    let mut material_entities: Vec<hecs::Entity> = Vec::new();

    for (material_index, update) in material_updates {
        let Some(color) = update.base_color else {
            continue;
        };

        material_entities.clear();
        {
            let mut query = world.query::<&GltfMaterial>();
            for (entity, gltf_material) in query.iter() {
                if gltf_material.0 == material_index {
                    material_entities.push(entity);
                }
            }
        }

        if material_entities.is_empty() {
            continue;
        }

        let to_u8 = |value: f32| -> u8 { (value.clamp(0.0, 1.0) * 255.0).round() as u8 };

        for entity in &material_entities {
            if let Ok(mut material) = world.get::<&mut MaterialComponent>(*entity) {
                material.0.base_color = [
                    to_u8(color.x),
                    to_u8(color.y),
                    to_u8(color.z),
                    to_u8(color.w),
                ];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::components::TransformComponent;
    use crate::scene::transform::Transform;

    #[test]
    fn transform_updates_modify_world() {
        let mut world = World::new();
        let entity = world.spawn((TransformComponent(Transform::IDENTITY),));

        apply_transform_update(
            &mut world,
            entity,
            TransformUpdate {
                translation: Some(glam::Vec3::new(1.0, 2.0, 3.0)),
                rotation: None,
                scale: None,
            },
        );

        let transform = world.get::<&TransformComponent>(entity).unwrap();
        assert_eq!(transform.0.translation, glam::Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn material_updates_apply_base_color() {
        let mut world = World::new();
        let entity = world.spawn((
            GltfMaterial(3),
            MaterialComponent(crate::renderer::Material::default()),
        ));

        let mut updates = HashMap::new();
        updates.insert(
            3usize,
            MaterialUpdate {
                base_color: Some(glam::Vec4::new(0.5, 0.25, 0.75, 1.0)),
            },
        );

        apply_material_updates(&mut world, updates);

        let material = world.get::<&MaterialComponent>(entity).unwrap();
        assert_eq!(material.0.base_color, [128, 64, 191, 255]);
    }

    #[test]
    fn orbit_animation_moves_entities() {
        let mut world = World::new();
        let entity = world.spawn((
            TransformComponent(Transform::IDENTITY),
            OrbitAnimation {
                center: Vec3::ZERO,
                radius: 2.0,
                speed: 1.0,
                offset: 0.0,
            },
        ));

        update_orbit_animations(&mut world, std::f64::consts::PI as f64);

        let transform = world.get::<&TransformComponent>(entity).unwrap();
        assert!((transform.0.translation.length() - 2.0).abs() < 1e-3);
    }
}
