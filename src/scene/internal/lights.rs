use crate::renderer::{DirectionalShadowData, LightsData, LocalLight, SpotCone};
use crate::scene::components::{
    CanCastShadow, DirectionalLight, PointLight, SpotLight, TransformComponent, WorldTransform,
};
use crate::scene::transform::Transform;
use glam::{Quat, Vec3};
use hecs::World;

/// The lights of the scene node `world` (identified by `scene_key`, as for skinning)
/// placed by `root_transform`.
pub(crate) fn collect_lights(
    world: &World,
    root_transform: Transform,
    scene_key: u64,
) -> LightsData {
    let mut lights = LightsData::default();

    collect_directional_lights(world, &mut lights, root_transform);
    collect_point_lights(world, &mut lights, root_transform, scene_key);
    collect_spot_lights(world, &mut lights, root_transform, scene_key);

    lights
}

/// A stable identity for one point or spot light in one scene node, for the renderer's
/// shadow maps (see `LocalLight::key`).
pub(crate) fn light_key(scene_key: u64, entity: hecs::Entity) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    ("light", scene_key, entity.to_bits()).hash(&mut hasher);
    hasher.finish()
}

fn collect_directional_lights(world: &World, lights: &mut LightsData, root_transform: Transform) {
    for (_entity, (light, world_transform, local_transform, shadow_flag)) in world
        .query::<(
            &DirectionalLight,
            Option<&WorldTransform>,
            Option<&TransformComponent>,
            Option<&CanCastShadow>,
        )>()
        .iter()
    {
        let mut transform = resolve_light_transform(world_transform, local_transform);
        transform = root_transform.mul_transform(&transform);
        let direction = safe_normalize(transform.rotation * Vec3::NEG_Z, Vec3::new(0.0, -1.0, 0.0));

        // The renderer fits the shadow cascades to the camera each frame.
        let shadow = shadow_enabled(shadow_flag).then_some(DirectionalShadowData {
            distance: light.shadow_distance.max(0.0),
        });

        lights.add_directional(direction, light.color, light.intensity, shadow);
    }
}

fn collect_point_lights(
    world: &World,
    lights: &mut LightsData,
    root_transform: Transform,
    scene_key: u64,
) {
    for (entity, (light, world_transform, local_transform, shadow_flag)) in world
        .query::<(
            &PointLight,
            Option<&WorldTransform>,
            Option<&TransformComponent>,
            Option<&CanCastShadow>,
        )>()
        .iter()
    {
        let mut transform = resolve_light_transform(world_transform, local_transform);
        transform = root_transform.mul_transform(&transform);

        lights.add_local(LocalLight {
            key: light_key(scene_key, entity),
            position: transform.translation,
            color: light.color,
            intensity: light.intensity,
            range: light.range,
            cone: None,
            casts_shadow: shadow_enabled(shadow_flag),
            shadow_priority: light.shadow_priority,
        });
    }
}

fn collect_spot_lights(
    world: &World,
    lights: &mut LightsData,
    root_transform: Transform,
    scene_key: u64,
) {
    for (entity, (light, world_transform, local_transform, shadow_flag)) in world
        .query::<(
            &SpotLight,
            Option<&WorldTransform>,
            Option<&TransformComponent>,
            Option<&CanCastShadow>,
        )>()
        .iter()
    {
        let mut transform = resolve_light_transform(world_transform, local_transform);
        transform = root_transform.mul_transform(&transform);
        let direction = safe_normalize(transform.rotation * Vec3::NEG_Z, Vec3::new(0.0, -1.0, 0.0));

        lights.add_local(LocalLight {
            key: light_key(scene_key, entity),
            position: transform.translation,
            color: light.color,
            intensity: light.intensity,
            range: light.range,
            cone: Some(SpotCone {
                direction,
                inner_angle: light.inner_angle,
                outer_angle: light.outer_angle,
            }),
            casts_shadow: shadow_enabled(shadow_flag),
            shadow_priority: light.shadow_priority,
        });
    }
}

pub(crate) fn resolve_light_transform(
    world_transform: Option<&WorldTransform>,
    local_transform: Option<&TransformComponent>,
) -> Transform {
    world_transform
        .map(|t| t.0)
        .or_else(|| local_transform.map(|t| t.0))
        .unwrap_or(Transform::IDENTITY)
}

fn shadow_enabled(flag: Option<&CanCastShadow>) -> bool {
    flag.map(|flag| flag.0).unwrap_or(false)
}

pub(crate) fn has_any_lights(world: &World) -> bool {
    if world.query::<&DirectionalLight>().iter().next().is_some() {
        return true;
    }

    if world.query::<&PointLight>().iter().next().is_some() {
        return true;
    }

    if world.query::<&SpotLight>().iter().next().is_some() {
        return true;
    }

    false
}

pub(crate) fn add_default_lighting(world: &mut World) -> usize {
    if has_any_lights(world) {
        return 0;
    }

    log::info!("No lights found in scene - adding default lighting setup");

    let mut created = 0usize;

    let directional_direction = Vec3::new(0.35, -1.0, -0.85).normalize();
    let directional_rotation = Quat::from_rotation_arc(Vec3::NEG_Z, directional_direction);
    let directional_position = Vec3::new(0.0, 6.0, 0.0);

    world.spawn((
        crate::scene::components::Name::new("Default Directional Light"),
        TransformComponent(Transform::from_trs(
            directional_position,
            directional_rotation,
            Vec3::ONE,
        )),
        DirectionalLight::new(Vec3::new(0.8, 0.9, 1.0), 2.75),
        CanCastShadow(true),
    ));
    created += 1;

    // let point_position = Vec3::new(3.5, 3.0, 2.5);
    // world.spawn((
    //     crate::scene::components::Name::new("Default Point Light"),
    //     TransformComponent(Transform::from_trs(
    //         point_position,
    //         Quat::IDENTITY,
    //         Vec3::ONE,
    //     )),
    //     PointLight {
    //         color: Vec3::new(1.0, 0.96, 0.85),
    //         intensity: 100.0,
    //         range: 5.0,
    //     },
    //     CanCastShadow(true),
    // ));
    // created += 1;

    // let spot_position = Vec3::new(-4.0, 5.0, -3.0);
    // let spot_target = Vec3::new(0.0, 1.0, 0.0);
    // let spot_direction = (spot_target - spot_position).normalize();
    // let spot_rotation = Quat::from_rotation_arc(Vec3::NEG_Z, spot_direction);

    // world.spawn((
    //     crate::scene::components::Name::new("Default Spot Light"),
    //     TransformComponent(Transform::from_trs(spot_position, spot_rotation, Vec3::ONE)),
    //     SpotLight {
    //         color: Vec3::new(0.8, 0.9, 1.0),
    //         intensity: 10.0,
    //         inner_angle: 0.35,
    //         outer_angle: 0.6,
    //         range: 10.0,
    //     },
    //     CanCastShadow(true),
    // ));
    // created += 1;

    created
}

pub(crate) fn safe_normalize(vec: Vec3, fallback: Vec3) -> Vec3 {
    if vec.length_squared() > 1e-6 {
        vec.normalize()
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directional_lights_pass_their_shadow_distance_when_they_cast() {
        let mut world = World::new();
        world.spawn((
            DirectionalLight::new(Vec3::ONE, 2.0).with_shadow_distance(80.0),
            CanCastShadow(true),
        ));
        world.spawn((DirectionalLight::new(Vec3::ONE, 1.0),));
        let lights = collect_lights(&world, Transform::IDENTITY, 0);
        let mut shadows = lights.directional_shadows().to_vec();
        shadows.sort_by_key(Option::is_none);
        assert_eq!(
            shadows,
            vec![Some(DirectionalShadowData { distance: 80.0 }), None]
        );
    }
}
