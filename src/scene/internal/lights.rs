use super::rendering::CameraVectors;
use crate::renderer::{
    DirectionalShadowData, LightsData, PointShadowData, SpotLightDescriptor, SpotShadowData,
};
use crate::scene::components::{
    CanCastShadow, DirectionalLight, PointLight, SpotLight, TransformComponent, WorldTransform,
};
use crate::scene::transform::Transform;
use glam::{Mat4, Quat, Vec3};
use hecs::World;

pub(crate) fn collect_lights(world: &World, camera: CameraVectors) -> LightsData {
    let mut lights = LightsData::default();

    collect_directional_lights(world, camera, &mut lights);
    collect_point_lights(world, &mut lights);
    collect_spot_lights(world, &mut lights);

    lights
}

fn collect_directional_lights(world: &World, camera: CameraVectors, lights: &mut LightsData) {
    for (_entity, (light, world_transform, local_transform, shadow_flag)) in world
        .query::<(
            &DirectionalLight,
            Option<&WorldTransform>,
            Option<&TransformComponent>,
            Option<&CanCastShadow>,
        )>()
        .iter()
    {
        let transform = resolve_light_transform(world_transform, local_transform);
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

fn collect_point_lights(world: &World, lights: &mut LightsData) {
    for (_entity, (light, world_transform, local_transform, shadow_flag)) in world
        .query::<(
            &PointLight,
            Option<&WorldTransform>,
            Option<&TransformComponent>,
            Option<&CanCastShadow>,
        )>()
        .iter()
    {
        let transform = resolve_light_transform(world_transform, local_transform);

        let shadow = if shadow_enabled(shadow_flag) {
            Some(build_point_shadow(transform.translation, light.range))
        } else {
            None
        };

        lights.add_point(
            transform.translation,
            light.color,
            light.intensity,
            light.range,
            shadow,
        );
    }
}

fn collect_spot_lights(world: &World, lights: &mut LightsData) {
    for (_entity, (light, world_transform, local_transform, shadow_flag)) in world
        .query::<(
            &SpotLight,
            Option<&WorldTransform>,
            Option<&TransformComponent>,
            Option<&CanCastShadow>,
        )>()
        .iter()
    {
        let transform = resolve_light_transform(world_transform, local_transform);
        let direction = safe_normalize(transform.rotation * Vec3::NEG_Z, Vec3::new(0.0, -1.0, 0.0));

        let shadow = if shadow_enabled(shadow_flag) {
            Some(build_spot_shadow(transform, light))
        } else {
            None
        };

        lights.add_spot(SpotLightDescriptor {
            position: transform.translation,
            direction,
            color: light.color,
            intensity: light.intensity,
            range: light.range,
            inner_angle: light.inner_angle,
            outer_angle: light.outer_angle,
            shadow,
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

pub(crate) fn build_directional_shadow(
    camera_pos: Vec3,
    camera_target: Vec3,
    light_transform: Transform,
    shadow_size: f32,
) -> DirectionalShadowData {
    let shadow_distance = DirectionalLight::DEFAULT_SHADOW_DISTANCE;

    let raw_dir = light_transform.rotation * Vec3::NEG_Z;
    let direction = safe_normalize(raw_dir, Vec3::new(0.0, -1.0, 0.0));

    let focus = if (camera_target - camera_pos).length_squared() > 1e-4 {
        camera_target
    } else {
        camera_pos
    };
    let light_pos = focus - direction * shadow_distance;

    let mut up = light_transform.rotation * Vec3::Y;
    if up.length_squared() > 0.0 {
        up = up.normalize();
    }
    if up.length_squared() <= 0.0 || up.abs().dot(direction).abs() > 0.999 {
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

pub(crate) fn build_point_shadow(position: Vec3, range: f32) -> PointShadowData {
    use std::f32::consts::FRAC_PI_2;

    let near = 0.1f32;
    let far = range.max(near + 0.1);
    let projection = Mat4::perspective_rh(FRAC_PI_2, 1.0, near, far);

    let dirs = [
        Vec3::X,
        Vec3::NEG_X,
        Vec3::Y,
        Vec3::NEG_Y,
        Vec3::Z,
        Vec3::NEG_Z,
    ];
    let ups = [Vec3::Y, Vec3::Y, Vec3::Z, Vec3::NEG_Z, Vec3::Y, Vec3::Y];

    let mut matrices = [Mat4::IDENTITY; 6];
    for ((matrix, dir), up) in matrices.iter_mut().zip(dirs.iter()).zip(ups.iter()) {
        let view = Mat4::look_at_rh(position, position + *dir, *up);
        *matrix = projection * view;
    }

    PointShadowData {
        view_proj: matrices,
        near,
        far,
    }
}

pub(crate) fn build_spot_shadow(transform: Transform, light: &SpotLight) -> SpotShadowData {
    let near = 0.1f32;
    let far = light.range.max(near + 0.1);
    let fov = (light.outer_angle * 2.0).clamp(0.1, std::f32::consts::PI - 0.1);

    let position = transform.translation;
    let forward = safe_normalize(transform.rotation * Vec3::NEG_Z, Vec3::NEG_Z);
    let mut up = safe_normalize(transform.rotation * Vec3::Y, Vec3::Y);

    let mut right = forward.cross(up);
    if right.length_squared() < 1e-8 {
        let fallback = if forward.dot(Vec3::X).abs() < 0.9 {
            Vec3::X
        } else {
            Vec3::Y
        };
        right = forward.cross(fallback);
    }
    right = right.normalize();
    up = right.cross(forward).normalize();

    let view = Mat4::look_at_rh(position, position + forward, up);
    let projection = Mat4::perspective_rh(fov, 1.0, near, far);

    SpotShadowData {
        view_proj: projection * view,
        far,
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

    let sun1_direction = Vec3::new(0.3, -1.0, -1.1).normalize();
    let sun1_rotation = Quat::from_rotation_arc(Vec3::NEG_Z, sun1_direction);

    world.spawn((
        crate::scene::components::Name::new("Default Sky Light"),
        TransformComponent(Transform::from_trs(Vec3::ZERO, sun1_rotation, Vec3::ONE)),
        DirectionalLight::new(Vec3::new(0.49, 0.95, 0.85), 2.5),
        CanCastShadow(true),
    ));
    created += 1;

    let sun2_direction = Vec3::new(-1.4, -1.0, 1.25).normalize();
    let sun2_rotation = Quat::from_rotation_arc(Vec3::NEG_Z, sun2_direction);

    world.spawn((
        crate::scene::components::Name::new("Default Sky Light"),
        TransformComponent(Transform::from_trs(Vec3::ZERO, sun2_rotation, Vec3::ONE)),
        DirectionalLight::new(Vec3::new(0.9, 0.95, 0.5), 2.5),
        CanCastShadow(true),
    ));
    created += 1;

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

    #[test]
    fn spot_shadow_view_matrix_uses_transform_basis() {
        let rotation = Quat::from_euler(EulerRot::YXZ, 0.45, -0.35, 0.2);
        let transform = Transform::from_trs(Vec3::new(2.0, 5.0, -1.0), rotation, Vec3::ONE);
        let light = SpotLight {
            color: Vec3::splat(1.0),
            intensity: 10.0,
            inner_angle: 0.3,
            outer_angle: 0.6,
            range: 25.0,
        };

        let shadow = build_spot_shadow(transform, &light);

        let near = 0.1;
        let far = light.range.max(near + 0.1);
        let fov = (light.outer_angle * 2.0).clamp(0.1, std::f32::consts::PI - 0.1);
        let expected_view = Mat4::look_at_rh(
            transform.translation,
            transform.translation + transform.rotation * Vec3::NEG_Z,
            transform.rotation * Vec3::Y,
        );
        let projection = Mat4::perspective_rh(fov, 1.0, near, far);
        let expected_view_proj = projection * expected_view;

        assert!(shadow.view_proj.abs_diff_eq(expected_view_proj, EPS));

        let actual_view = projection.inverse() * shadow.view_proj;
        assert!(actual_view.abs_diff_eq(expected_view, EPS));

        let forward = transform.rotation * Vec3::NEG_Z;
        let up = transform.rotation * Vec3::Y;
        let forward_in_view = (actual_view * forward.extend(0.0)).truncate().normalize();
        let up_in_view = (actual_view * up.extend(0.0)).truncate().normalize();
        assert!(forward_in_view.abs_diff_eq(Vec3::new(0.0, 0.0, -1.0), EPS));
        assert!(up_in_view.abs_diff_eq(Vec3::Y, EPS));
    }

    #[test]
    fn spot_shadow_depth_maps_into_wgpu_range() {
        let rotation = Quat::from_euler(EulerRot::YXZ, -0.35, 0.5, 0.1);
        let transform = Transform::from_trs(Vec3::new(-4.0, 3.0, 6.0), rotation, Vec3::ONE);
        let light = SpotLight {
            color: Vec3::splat(1.0),
            intensity: 5.0,
            inner_angle: 0.4,
            outer_angle: 0.7,
            range: 30.0,
        };

        let shadow = build_spot_shadow(transform, &light);

        let near = 0.1;
        let far = light.range.max(near + 0.1);
        let forward = (transform.rotation * Vec3::NEG_Z).normalize();
        let position = transform.translation;

        let near_world = position + forward * near;
        let far_world = position + forward * far;

        let clip_near = shadow.view_proj * near_world.extend(1.0);
        let clip_far = shadow.view_proj * far_world.extend(1.0);
        assert!(clip_near.w > 0.0 && clip_far.w > 0.0);

        let ndc_near = clip_near.truncate() / clip_near.w;
        let ndc_far = clip_far.truncate() / clip_far.w;

        assert!(ndc_near.z >= -EPS && ndc_near.z <= 1.0 + EPS);
        assert!(ndc_far.z >= -EPS && ndc_far.z <= 1.0 + EPS);

        assert!((ndc_near.z - 0.0).abs() < 1e-4, "near depth {}", ndc_near.z);
        assert!((ndc_far.z - 1.0).abs() < 1e-4, "far depth {}", ndc_far.z);
    }

    #[test]
    fn point_shadow_view_matrices_cover_all_cubemap_faces() {
        let position = Vec3::new(-3.0, 4.5, 1.0);
        let range = 12.0;
        let shadow = build_point_shadow(position, range);

        let near = 0.1;
        let far = range.max(near + 0.1);
        let projection = Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, near, far);

        let dirs = [
            Vec3::X,
            Vec3::NEG_X,
            Vec3::Y,
            Vec3::NEG_Y,
            Vec3::Z,
            Vec3::NEG_Z,
        ];
        let ups = [Vec3::Y, Vec3::Y, Vec3::Z, Vec3::NEG_Z, Vec3::Y, Vec3::Y];

        for (((matrix, dir), up), face) in shadow
            .view_proj
            .iter()
            .zip(dirs.iter())
            .zip(ups.iter())
            .zip(0usize..)
        {
            let expected_view = Mat4::look_at_rh(position, position + *dir, *up);
            let expected_view_proj = projection * expected_view;
            assert!(
                matrix.abs_diff_eq(expected_view_proj, EPS),
                "face {} mismatch",
                face
            );

            let actual_view = projection.inverse() * *matrix;
            assert!(actual_view.abs_diff_eq(expected_view, EPS));

            let dir_in_view = (actual_view * dir.extend(0.0)).truncate().normalize();
            assert!(dir_in_view.abs_diff_eq(Vec3::new(0.0, 0.0, -1.0), EPS));
        }
    }

    #[test]
    fn point_shadow_depth_maps_into_wgpu_range() {
        let position = Vec3::new(2.5, -1.5, 7.0);
        let range = 18.0;
        let shadow = build_point_shadow(position, range);

        for (matrix, dir) in shadow.view_proj.iter().zip([
            Vec3::X,
            Vec3::NEG_X,
            Vec3::Y,
            Vec3::NEG_Y,
            Vec3::Z,
            Vec3::NEG_Z,
        ]) {
            let forward = dir.normalize();
            let near_world = position + forward * shadow.near;
            let far_world = position + forward * shadow.far;

            let clip_near = *matrix * near_world.extend(1.0);
            let clip_far = *matrix * far_world.extend(1.0);
            assert!(clip_near.w > 0.0 && clip_far.w > 0.0);

            let ndc_near = clip_near.truncate() / clip_near.w;
            let ndc_far = clip_far.truncate() / clip_far.w;

            assert!(ndc_near.z >= -EPS && ndc_near.z <= 1.0 + EPS);
            assert!(ndc_far.z >= -EPS && ndc_far.z <= 1.0 + EPS);

            assert!(
                (ndc_near.z - 0.0).abs() < 1e-4,
                "face dir {:?} near {}",
                dir,
                ndc_near.z
            );
            assert!(
                (ndc_far.z - 1.0).abs() < 1e-4,
                "face dir {:?} far {}",
                dir,
                ndc_far.z
            );
        }
    }
}
