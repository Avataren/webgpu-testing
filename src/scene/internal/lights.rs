use super::rendering::CameraVectors;
use crate::renderer::{DirectionalShadowData, LightsData, LocalLight, SpotCone};
use crate::scene::components::{
    CanCastShadow, DirectionalLight, PointLight, SpotLight, TransformComponent, WorldTransform,
};
use crate::scene::transform::Transform;
use glam::{Mat4, Quat, Vec3};
use hecs::World;

/// The lights of the scene node `world` (identified by `scene_key`, as for skinning)
/// placed by `root_transform`.
pub(crate) fn collect_lights(
    world: &World,
    camera: CameraVectors,
    root_transform: Transform,
    scene_key: u64,
) -> LightsData {
    let mut lights = LightsData::default();

    collect_directional_lights(world, camera, &mut lights, root_transform);
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

fn collect_directional_lights(
    world: &World,
    camera: CameraVectors,
    lights: &mut LightsData,
    root_transform: Transform,
) {
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

        let shadow = if shadow_enabled(shadow_flag) {
            Some(build_directional_shadow(
                camera.position,
                camera.target,
                transform,
                light.shadow_size,
            ))
        } else {
            None
        };

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

/// Where the directional shadow area is centered. The editor camera orbits a target in the
/// scene, so that is used. A game camera's target is only a step ahead of its eye, which
/// would center the shadows in the air in front of a high camera (and leave the ground it
/// looks at without any), so for it the shadows center where its view meets the ground
/// plane (y = 0), or half the shadow area ahead when it does not look down.
fn shadow_focus(camera_pos: Vec3, camera_target: Vec3, shadow_size: f32) -> Vec3 {
    const ORBIT_TARGET_MIN_DISTANCE: f32 = 2.0;
    let offset = camera_target - camera_pos;
    let length = offset.length();
    if length <= 1e-2 {
        return camera_pos;
    }
    if length >= ORBIT_TARGET_MIN_DISTANCE {
        return camera_target;
    }
    let forward = offset / length;
    let extent = shadow_size.max(0.1);
    if forward.y < -0.05 && camera_pos.y > 0.0 {
        let to_ground = (camera_pos.y / -forward.y).min(extent * 4.0);
        return camera_pos + forward * to_ground;
    }
    camera_pos + forward * extent
}

pub(crate) fn build_directional_shadow(
    camera_pos: Vec3,
    camera_target: Vec3,
    light_transform: Transform,
    shadow_size: f32,
) -> DirectionalShadowData {
    let shadow_distance = DirectionalLight::DEFAULT_SHADOW_DISTANCE;

    let raw_dir = light_transform.rotation * Vec3::NEG_Z;
    let direction = safe_normalize(raw_dir, Vec3::new(0.0, -1.0, 0.0));

    let focus = shadow_focus(camera_pos, camera_target, shadow_size);
    let light_pos = focus - direction * shadow_distance;

    let mut up = light_transform.rotation * Vec3::Y;
    if up.length_squared() > 0.0 {
        up = up.normalize();
    }
    if up.length_squared() <= 0.0 || up.dot(direction).abs() > 0.999 {
        up = shadow_up(direction);
    }

    let view = Mat4::look_at_rh(light_pos, focus, up);

    let extent = shadow_size.max(0.1);
    let left = -extent;
    let right = extent;
    let bottom = -extent;
    let top = extent;
    let near = 0.1;
    let far = shadow_distance * 2.0;

    let projection = Mat4::from_cols(
        glam::Vec4::new(2.0 / (right - left), 0.0, 0.0, 0.0),
        glam::Vec4::new(0.0, 2.0 / (top - bottom), 0.0, 0.0),
        glam::Vec4::new(0.0, 0.0, -1.0 / (far - near), 0.0),
        glam::Vec4::new(
            -(right + left) / (right - left),
            -(top + bottom) / (top - bottom),
            -near / (far - near),
            1.0,
        ),
    );

    DirectionalShadowData {
        view_proj: projection * view,
    }
}

fn shadow_up(direction: Vec3) -> Vec3 {
    let up = Vec3::Y;
    if direction.abs().dot(up) > 0.95 {
        Vec3::Z
    } else {
        up
    }
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
    use crate::scene::transform::Transform;
    use glam::{EulerRot, Vec2, Vec4};

    const EPS: f32 = 1e-5;

    fn build_directional_projection() -> Mat4 {
        let extent = DirectionalLight::DEFAULT_SHADOW_SIZE;
        let left = -extent;
        let right = extent;
        let bottom = -extent;
        let top = extent;
        let near = 0.1;
        let far = 60.0;

        Mat4::from_cols(
            Vec4::new(2.0 / (right - left), 0.0, 0.0, 0.0),
            Vec4::new(0.0, 2.0 / (top - bottom), 0.0, 0.0),
            Vec4::new(0.0, 0.0, -1.0 / (far - near), 0.0),
            Vec4::new(
                -(right + left) / (right - left),
                -(top + bottom) / (top - bottom),
                -near / (far - near),
                1.0,
            ),
        )
    }

    #[test]
    fn directional_shadow_view_matrix_matches_expected_orientation() {
        let camera_pos = Vec3::new(8.0, 6.0, -4.0);
        let camera_target = Vec3::new(2.5, 1.0, -3.0);
        let rotation = Quat::from_euler(EulerRot::YXZ, 0.35, -0.6, 0.5)
            * Quat::from_euler(EulerRot::ZXY, 0.2, 0.0, 0.1);
        let transform = Transform::from_trs(Vec3::new(1.5, 3.0, -2.0), rotation, Vec3::ONE);

        let shadow = build_directional_shadow(
            camera_pos,
            camera_target,
            transform,
            DirectionalLight::DEFAULT_SHADOW_SIZE,
        );

        let direction = (rotation * Vec3::NEG_Z).normalize();
        let up = (rotation * Vec3::Y).normalize();
        let focus = camera_target;
        let light_pos = focus - direction * 30.0;
        let expected_view = Mat4::look_at_rh(light_pos, focus, up);
        let projection = build_directional_projection();
        let expected_view_proj = projection * expected_view;

        assert!(
            shadow.view_proj.abs_diff_eq(expected_view_proj, EPS),
            "view projection mismatch: {:?} vs {:?}",
            shadow.view_proj,
            expected_view_proj
        );

        let actual_view = projection.inverse() * shadow.view_proj;
        assert!(actual_view.abs_diff_eq(expected_view, EPS));

        let dir_in_view = (actual_view * direction.extend(0.0)).truncate().normalize();
        assert!(dir_in_view.abs_diff_eq(Vec3::new(0.0, 0.0, -1.0), EPS));
    }

    #[test]
    fn directional_shadow_of_a_game_camera_centers_where_it_looks_at_the_ground() {
        // A game camera high above the arena: its target is a step ahead of the eye.
        let eye = Vec3::new(0.0, 40.0, 20.0);
        let forward = (Vec3::new(0.0, 0.0, -3.0) - eye).normalize();
        let focus = shadow_focus(eye, eye + forward, 24.0);
        assert!(
            focus.abs_diff_eq(Vec3::new(0.0, 0.0, -3.0), 1e-3),
            "{focus:?}"
        );

        // An orbit target in the scene is used as it is.
        let target = Vec3::new(1.0, 0.5, -2.0);
        assert_eq!(shadow_focus(eye, target, 24.0), target);

        // Looking at the horizon: half the shadow area ahead.
        let level = shadow_focus(Vec3::new(0.0, 2.0, 0.0), Vec3::new(0.0, 2.0, -1.0), 10.0);
        assert!(
            level.abs_diff_eq(Vec3::new(0.0, 2.0, -10.0), 1e-4),
            "{level:?}"
        );
    }

    #[test]
    fn scripted_orbit_camera_keeps_the_sculpture_inside_the_shadow_volume() {
        use crate::scene::camera_control::SceneCamera;
        use crate::scene::transform::look_rotation;
        use crate::scripting::lua::commands::ScriptCommands;
        use crate::scripting::lua::entity_registry::EntityHandleRegistry;
        use std::cell::RefCell;
        use std::rc::Rc;

        let target = Vec3::new(0.0, 3.7, 0.0);
        let light_eye = Vec3::new(-8.0, 12.0, 8.0);
        let light = Transform::from_trs(
            light_eye,
            look_rotation(light_eye, Vec3::new(0.0, 2.0, 0.0)).unwrap(),
            Vec3::ONE,
        );
        let mut commands =
            ScriptCommands::new(Rc::new(RefCell::new(EntityHandleRegistry::default())));
        let mut world = World::new();
        let camera_entity = commands.spawn_entity(None);
        commands
            .set_translation(camera_entity, Vec3::new(0.0, 4.5, 13.5))
            .unwrap();
        commands.look_at(camera_entity, target).unwrap();
        commands.apply(&mut world).unwrap();
        let entity =
            hecs::Entity::from_bits(commands.resolve_entity_bits(camera_entity).unwrap()).unwrap();
        let mut camera = SceneCamera::new();
        camera.apply_from_entity(&world, entity);
        assert!(
            camera.camera().target.abs_diff_eq(target, EPS),
            "{:?}",
            camera.camera().target
        );

        let expected = build_directional_shadow(camera.camera().eye, target, light, 28.0);
        for radius in [13.5, 20.2, 30.0] {
            for height in [4.5, 5.21, 8.4, 15.0] {
                for step in 0..12 {
                    let angle = step as f32 * std::f32::consts::TAU / 12.0;
                    let eye = Vec3::new(angle.sin() * radius, height, angle.cos() * radius);
                    commands.set_translation(camera_entity, eye).unwrap();
                    commands.look_at(camera_entity, target).unwrap();
                    commands.apply(&mut world).unwrap();
                    camera.apply_from_entity(&world, entity);
                    assert!(camera.camera().target.abs_diff_eq(target, EPS));
                    let shadow = build_directional_shadow(eye, camera.camera().target, light, 28.0);
                    assert!(shadow.view_proj.abs_diff_eq(expected.view_proj, EPS));
                    for point in [
                        Vec3::new(-14.0, -0.15, -14.0),
                        Vec3::new(14.0, -0.15, -14.0),
                        Vec3::new(0.0, 9.0, 0.0),
                    ] {
                        let clip = shadow.view_proj * point.extend(1.0);
                        let ndc = clip.truncate() / clip.w;
                        assert!(
                            ndc.x.abs() < 1.0 && ndc.y.abs() < 1.0 && (0.0..=1.0).contains(&ndc.z),
                            "radius {radius}, height {height}, step {step}: {ndc}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn directional_shadow_centers_camera_target() {
        let camera_pos = Vec3::new(4.0, 6.0, 12.0);
        let camera_target = Vec3::new(1.0, 0.5, -2.0);
        let rotation = Quat::from_euler(EulerRot::YXZ, -0.2, -0.9, 0.3);
        let transform = Transform::from_trs(Vec3::ZERO, rotation, Vec3::ONE);

        let shadow = build_directional_shadow(
            camera_pos,
            camera_target,
            transform,
            DirectionalLight::DEFAULT_SHADOW_SIZE,
        );

        let clip = shadow.view_proj * camera_target.extend(1.0);
        assert!(clip.w > 0.0);
        let ndc = clip.truncate() / clip.w;
        let uv = Vec2::new(ndc.x * 0.5 + 0.5, -ndc.y * 0.5 + 0.5);

        assert!(
            uv.abs_diff_eq(Vec2::splat(0.5), 1e-4),
            "camera target projected to {:?}",
            uv
        );
    }

    #[test]
    fn directional_shadow_respects_light_roll() {
        let camera_pos = Vec3::new(-6.0, 5.0, 2.0);
        let camera_target = Vec3::new(0.5, 1.5, -3.0);
        let rotation = Quat::from_euler(EulerRot::ZXY, 0.3, -0.5, 0.9);
        let transform = Transform::from_trs(Vec3::new(-1.0, 2.0, 0.5), rotation, Vec3::splat(1.0));

        let shadow = build_directional_shadow(
            camera_pos,
            camera_target,
            transform,
            DirectionalLight::DEFAULT_SHADOW_SIZE,
        );

        let projection = build_directional_projection();
        let actual_view = projection.inverse() * shadow.view_proj;

        let forward = (rotation * Vec3::NEG_Z).normalize();
        let up = (rotation * Vec3::Y).normalize();
        let right = (rotation * Vec3::X).normalize();

        let forward_in_view = (actual_view * forward.extend(0.0)).truncate().normalize();
        let up_in_view = (actual_view * up.extend(0.0)).truncate().normalize();
        let right_in_view = (actual_view * right.extend(0.0)).truncate().normalize();

        assert!(forward_in_view.abs_diff_eq(Vec3::new(0.0, 0.0, -1.0), EPS));
        assert!(up_in_view.abs_diff_eq(Vec3::Y, EPS));
        assert!(right_in_view.abs_diff_eq(Vec3::X, EPS));
    }

    #[test]
    fn directional_shadow_scales_with_per_light_extent() {
        let camera_pos = Vec3::new(0.0, 4.0, 12.0);
        let camera_target = Vec3::ZERO;
        let transform = Transform::from_trs(Vec3::ZERO, Quat::IDENTITY, Vec3::ONE);
        let default_extent = DirectionalLight::DEFAULT_SHADOW_SIZE;
        let world_point = Vec3::new(default_extent * 1.2, 0.0, 0.0);

        let small = build_directional_shadow(camera_pos, camera_target, transform, default_extent);
        let large =
            build_directional_shadow(camera_pos, camera_target, transform, default_extent * 3.0);

        let project_to_uv = |matrix: Mat4| {
            let clip = matrix * world_point.extend(1.0);
            assert!(clip.w > 0.0);
            let ndc = clip.truncate() / clip.w;
            Vec2::new(ndc.x * 0.5 + 0.5, -ndc.y * 0.5 + 0.5)
        };

        let small_uv = project_to_uv(small.view_proj);
        let large_uv = project_to_uv(large.view_proj);

        assert!(
            small_uv.x > 1.0 + 1e-4,
            "expected point outside default shadow extent, uv={small_uv:?}"
        );
        assert!(
            large_uv.x <= 1.0 + 1e-4,
            "expected point within larger shadow extent, uv={large_uv:?}"
        );
    }
}
