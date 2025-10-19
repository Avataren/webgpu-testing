use super::lights::safe_normalize;
use super::rendering::CameraVectors;
use crate::asset::{Assets, Handle, Mesh};
use crate::renderer::batch::{CullMode, InstanceSource, RenderObject, RenderPass};
use crate::renderer::primitives::{cone_mesh, cube_mesh, cylinder_mesh, torus_mesh};
use crate::renderer::{Material, Renderer};
use crate::scene::components::DepthState;
use crate::scene::transform::Transform;
use crate::scene::{TransformGizmoAxis, TransformGizmoHandle, TransformGizmoMode};
use glam::{Mat3, Quat, Vec3};

#[derive(Clone, Copy)]
pub(crate) struct TransformGizmoResources {
    pub axis_cylinder: Handle<Mesh>,
    pub axis_cone: Handle<Mesh>,
    pub scale_cube: Handle<Mesh>,
    pub rotation_ring: Handle<Mesh>,
    pub rotation_ring_highlight: Handle<Mesh>,
}

const AXES: [TransformGizmoAxis; 3] = [
    TransformGizmoAxis::X,
    TransformGizmoAxis::Y,
    TransformGizmoAxis::Z,
];

const TRANSLATE_PLANES: [(TransformGizmoAxis, TransformGizmoAxis); 3] = [
    (TransformGizmoAxis::X, TransformGizmoAxis::Y),
    (TransformGizmoAxis::Y, TransformGizmoAxis::Z),
    (TransformGizmoAxis::Z, TransformGizmoAxis::X),
];

const AXIS_X_COLOR: [u8; 4] = [250, 80, 90, 230];
const AXIS_Y_COLOR: [u8; 4] = [80, 220, 120, 230];
const AXIS_Z_COLOR: [u8; 4] = [90, 140, 250, 230];
const AXIS_CENTER_COLOR: [u8; 4] = [240, 240, 240, 200];
const SCALE_HANDLE_COLOR: [u8; 4] = [240, 240, 240, 220];
const RING_BASE_COLOR: [u8; 4] = [220, 220, 220, 180];

const TRANSLATE_SHAFT_LENGTH: f32 = 0.9;
const TRANSLATE_SHAFT_RADIUS: f32 = 0.035;
const TRANSLATE_CONE_LENGTH: f32 = 0.25;
const TRANSLATE_CONE_RADIUS: f32 = 0.12;
const TRANSLATE_PLANE_SIZE: f32 = 0.32;
const TRANSLATE_PLANE_OFFSET: f32 = 0.16;
const TRANSLATE_PLANE_THICKNESS: f32 = 0.04;
const TRANSLATE_PLANE_PICK_THICKNESS: f32 = 0.1;

const SCALE_HANDLE_SIZE: f32 = 0.18;
const SCALE_HANDLE_OFFSET: f32 = 0.9;

const ROTATION_RING_RADIUS: f32 = 1.0;
const ROTATION_RING_THICKNESS: f32 = 0.04;

const TRANSLATE_AXIS_PICK_RADIUS: f32 = 0.32;
const TRANSLATE_CENTER_PICK_RADIUS: f32 = 0.28;
const TRANSLATE_PLANE_PICK_PADDING: f32 = 0.06;
const SCALE_AXIS_PICK_RADIUS: f32 = 0.18;
const SCALE_UNIFORM_PICK_RADIUS: f32 = 0.2;
const ROTATION_RING_PICK_THICKNESS: f32 = 0.12;

const GIZMO_DEPTH: DepthState = DepthState::new(false, false);

pub(crate) fn create_resources(
    renderer: &mut Renderer,
    assets: &mut Assets,
) -> TransformGizmoResources {
    let (cyl_vertices, cyl_indices) = cylinder_mesh(32);
    let axis_cylinder = assets
        .meshes
        .insert(renderer.create_mesh(&cyl_vertices, &cyl_indices));

    let (cone_vertices, cone_indices) = cone_mesh(32);
    let axis_cone = assets
        .meshes
        .insert(renderer.create_mesh(&cone_vertices, &cone_indices));

    let (cube_vertices, cube_indices) = cube_mesh();
    let scale_cube = assets
        .meshes
        .insert(renderer.create_mesh(&cube_vertices, &cube_indices));

    let (ring_vertices, ring_indices) = torus_mesh(64, 16, 0.5, ROTATION_RING_THICKNESS * 0.5);
    let rotation_ring = assets
        .meshes
        .insert(renderer.create_mesh(&ring_vertices, &ring_indices));

    let (ring_high_vertices, ring_high_indices) =
        torus_mesh(64, 12, 0.5, ROTATION_RING_THICKNESS * 0.25);
    let rotation_ring_highlight = assets
        .meshes
        .insert(renderer.create_mesh(&ring_high_vertices, &ring_high_indices));

    TransformGizmoResources {
        axis_cylinder,
        axis_cone,
        scale_cube,
        rotation_ring,
        rotation_ring_highlight,
    }
}

pub(crate) fn build_transform_gizmos(
    camera: CameraVectors,
    gizmo_transform: Transform,
    mode: TransformGizmoMode,
    resources: TransformGizmoResources,
    hovered: Option<TransformGizmoHandle>,
) -> Vec<RenderObject> {
    match mode {
        TransformGizmoMode::Translate => {
            build_translate_gizmo(camera, gizmo_transform, resources, hovered)
        }
        TransformGizmoMode::Rotate => {
            build_rotate_gizmo(camera, gizmo_transform, resources, hovered)
        }
        TransformGizmoMode::Scale => build_scale_gizmo(camera, gizmo_transform, resources, hovered),
    }
}

pub(crate) fn hit_test(
    camera: CameraVectors,
    gizmo_transform: Transform,
    mode: TransformGizmoMode,
    ray_origin: Vec3,
    ray_dir: Vec3,
) -> Option<TransformGizmoHandle> {
    let ray_dir = safe_normalize(ray_dir, Vec3::ZERO);
    if ray_dir.length_squared() <= 1e-6 {
        return None;
    }

    let origin = gizmo_transform.translation;
    let base_scale = gizmo_screen_scale(camera, origin);

    let mut best: Option<(TransformGizmoHandle, f32)> = None;
    let mut consider = |handle: TransformGizmoHandle, distance: f32| {
        if let Some((_, best_dist)) = best {
            if distance >= best_dist {
                return;
            }
        }
        best = Some((handle, distance));
    };

    match mode {
        TransformGizmoMode::Translate => {
            let shaft_length = TRANSLATE_SHAFT_LENGTH * base_scale;
            let cone_length = TRANSLATE_CONE_LENGTH * base_scale;
            let total_length = shaft_length + cone_length * 1.2;
            let pick_radius = TRANSLATE_AXIS_PICK_RADIUS * base_scale;
            for axis in AXES {
                let axis_dir = axis_direction(gizmo_transform.rotation, axis);
                if let Some(distance) =
                    distance_to_axis_segment(ray_origin, ray_dir, origin, axis_dir, total_length)
                {
                    if distance <= pick_radius {
                        consider(TransformGizmoHandle::TranslateAxis(axis), distance);
                    }
                }
            }
            if let Some(distance) = distance_point_to_ray(ray_origin, ray_dir, origin) {
                if distance <= TRANSLATE_CENTER_PICK_RADIUS * base_scale {
                    consider(TransformGizmoHandle::TranslateCenter, distance);
                }
            }
            let plane_offset = TRANSLATE_PLANE_OFFSET * base_scale;
            let plane_size = TRANSLATE_PLANE_SIZE * base_scale;
            let plane_half = plane_size * 0.5 + TRANSLATE_PLANE_PICK_PADDING * base_scale;
            let plane_thickness = TRANSLATE_PLANE_PICK_THICKNESS * base_scale;
            for &(axis_a, axis_b) in &TRANSLATE_PLANES {
                let dir_a = axis_direction(gizmo_transform.rotation, axis_a);
                let dir_b = axis_direction(gizmo_transform.rotation, axis_b);
                let mut normal = dir_a.cross(dir_b);
                if normal.length_squared() < 1e-6 {
                    continue;
                }
                normal = normal.normalize();
                let denom = ray_dir.dot(normal);
                if denom.abs() < 1e-6 {
                    continue;
                }
                let t = (origin - ray_origin).dot(normal) / denom;
                if !t.is_finite() || t < 0.0 {
                    continue;
                }
                let hit = ray_origin + ray_dir * t;
                let local = hit - origin;
                let coord_a = local.dot(dir_a);
                let coord_b = local.dot(dir_b);
                let coord_n = local.dot(normal).abs();
                if coord_a < plane_offset - plane_half
                    || coord_a > plane_offset + plane_half
                    || coord_b < plane_offset - plane_half
                    || coord_b > plane_offset + plane_half
                    || coord_n > plane_thickness
                {
                    continue;
                }
                let plane_center = origin + dir_a * plane_offset + dir_b * plane_offset;
                let distance = (hit - plane_center).length();
                consider(
                    TransformGizmoHandle::TranslatePlane(axis_a, axis_b),
                    distance,
                );
            }
        }
        TransformGizmoMode::Rotate => {
            let ring_radius = ROTATION_RING_RADIUS * base_scale;
            let tolerance = (ROTATION_RING_PICK_THICKNESS * 1.5).max(0.06) * base_scale;
            for axis in AXES {
                let normal = axis_direction(gizmo_transform.rotation, axis);
                if let Some(distance) =
                    ring_hit(ray_origin, ray_dir, origin, normal, ring_radius, tolerance)
                {
                    consider(TransformGizmoHandle::RotateAxis(axis), distance);
                }
            }
            let camera_forward = safe_normalize(camera.target - camera.position, Vec3::NEG_Z);
            let screen_ring_radius = ring_radius * 1.2;
            if let Some(distance) = ring_hit(
                ray_origin,
                ray_dir,
                origin,
                -camera_forward,
                screen_ring_radius,
                tolerance,
            ) {
                consider(TransformGizmoHandle::RotateScreen, distance);
            }
        }
        TransformGizmoMode::Scale => {
            let axis_radius = SCALE_AXIS_PICK_RADIUS * base_scale;
            for axis in AXES {
                let axis_dir = axis_direction(gizmo_transform.rotation, axis);
                let handle_center = origin + axis_dir * SCALE_HANDLE_OFFSET * base_scale;
                if let Some(distance) = distance_point_to_ray(ray_origin, ray_dir, handle_center) {
                    if distance <= axis_radius {
                        consider(TransformGizmoHandle::ScaleAxis(axis), distance);
                    }
                }
            }
            if let Some(distance) = distance_point_to_ray(ray_origin, ray_dir, origin) {
                if distance <= SCALE_UNIFORM_PICK_RADIUS * base_scale {
                    consider(TransformGizmoHandle::ScaleUniform, distance);
                }
            }
        }
    }

    best.map(|(handle, _)| handle)
}

fn build_translate_gizmo(
    camera: CameraVectors,
    gizmo_transform: Transform,
    resources: TransformGizmoResources,
    hovered: Option<TransformGizmoHandle>,
) -> Vec<RenderObject> {
    let mut gizmos = Vec::new();
    let base_scale = gizmo_screen_scale(camera, gizmo_transform.translation);
    let shaft_length = TRANSLATE_SHAFT_LENGTH * base_scale;
    let shaft_radius = TRANSLATE_SHAFT_RADIUS * base_scale;
    let cone_length = TRANSLATE_CONE_LENGTH * base_scale;
    let cone_radius = TRANSLATE_CONE_RADIUS * base_scale;

    let plane_offset = TRANSLATE_PLANE_OFFSET * base_scale;
    let plane_size = TRANSLATE_PLANE_SIZE * base_scale;
    let plane_thickness = TRANSLATE_PLANE_THICKNESS * base_scale;

    for axis in AXES {
        let axis_vec = axis_direction(gizmo_transform.rotation, axis);
        let color = highlight_color(
            axis_color(axis),
            hovered == Some(TransformGizmoHandle::TranslateAxis(axis)),
        );

        let rotation = align_vector(Vec3::Z, axis_vec);
        let shaft_translation = gizmo_transform.translation + axis_vec * (shaft_length * 0.5);
        gizmos.push(RenderObject {
            mesh: resources.axis_cylinder,
            material: solid_color_material(color),
            transform: Transform::from_trs(
                shaft_translation,
                rotation,
                Vec3::new(shaft_radius, shaft_radius, shaft_length),
            ),
            depth_state: GIZMO_DEPTH,
            force_overlay: false,
            render_pass: Some(RenderPass::GizmoSolid),
            instance_source: InstanceSource::Cpu,
            gpu_index: None,
            cull_mode: CullMode::None,
        });

        let cone_translation =
            gizmo_transform.translation + axis_vec * (shaft_length + cone_length);
        let cone_rotation = align_vector(Vec3::NEG_Z, axis_vec);
        gizmos.push(RenderObject {
            mesh: resources.axis_cone,
            material: solid_color_material(color),
            transform: Transform::from_trs(
                cone_translation,
                cone_rotation,
                Vec3::new(cone_radius, cone_radius, cone_length),
            ),
            depth_state: GIZMO_DEPTH,
            force_overlay: false,
            render_pass: Some(RenderPass::GizmoSolid),
            instance_source: InstanceSource::Cpu,
            gpu_index: None,
            cull_mode: CullMode::Back,
        });
    }

    for &(axis_a, axis_b) in &TRANSLATE_PLANES {
        let axis_a_vec = axis_direction(gizmo_transform.rotation, axis_a);
        let axis_b_vec = axis_direction(gizmo_transform.rotation, axis_b);
        let mut normal = axis_a_vec.cross(axis_b_vec);
        if normal.length_squared() < 1e-6 {
            continue;
        }
        normal = normal.normalize();
        let plane_center =
            gizmo_transform.translation + axis_a_vec * plane_offset + axis_b_vec * plane_offset;
        let orientation = Quat::from_mat3(&Mat3::from_cols(axis_a_vec, axis_b_vec, normal));
        let color = highlight_color(
            plane_color(axis_a, axis_b),
            hovered == Some(TransformGizmoHandle::TranslatePlane(axis_a, axis_b)),
        );
        gizmos.push(RenderObject {
            mesh: resources.scale_cube,
            material: solid_color_material(color),
            transform: Transform::from_trs(
                plane_center,
                orientation,
                Vec3::new(plane_size, plane_size, plane_thickness),
            ),
            depth_state: GIZMO_DEPTH,
            force_overlay: false,
            render_pass: Some(RenderPass::GizmoSolid),
            instance_source: InstanceSource::Cpu,
            gpu_index: None,
            cull_mode: CullMode::Back,
        });
    }

    let center_size = SCALE_HANDLE_SIZE * base_scale * 0.6;
    let center_color = highlight_color(
        AXIS_CENTER_COLOR,
        hovered == Some(TransformGizmoHandle::TranslateCenter),
    );
    gizmos.push(RenderObject {
        mesh: resources.scale_cube,
        material: solid_color_material(center_color),
        transform: Transform::from_trs(
            gizmo_transform.translation,
            Quat::IDENTITY,
            Vec3::splat(center_size),
        ),
        depth_state: GIZMO_DEPTH,
        force_overlay: false,
        render_pass: Some(RenderPass::GizmoSolid),
        instance_source: InstanceSource::Cpu,
        gpu_index: None,
        cull_mode: CullMode::Back,
    });

    gizmos
}

fn build_scale_gizmo(
    camera: CameraVectors,
    gizmo_transform: Transform,
    resources: TransformGizmoResources,
    hovered: Option<TransformGizmoHandle>,
) -> Vec<RenderObject> {
    let mut gizmos = Vec::new();
    let base_scale = gizmo_screen_scale(camera, gizmo_transform.translation);

    for axis in AXES {
        let axis_vec = axis_direction(gizmo_transform.rotation, axis);
        let offset = axis_vec * SCALE_HANDLE_OFFSET * base_scale;
        let color = highlight_color(
            axis_color(axis),
            hovered == Some(TransformGizmoHandle::ScaleAxis(axis)),
        );
        gizmos.push(RenderObject {
            mesh: resources.scale_cube,
            material: solid_color_material(color),
            transform: Transform::from_trs(
                gizmo_transform.translation + offset,
                Quat::IDENTITY,
                Vec3::splat(SCALE_HANDLE_SIZE * base_scale),
            ),
            depth_state: GIZMO_DEPTH,
            force_overlay: false,
            render_pass: Some(RenderPass::GizmoSolid),
            instance_source: InstanceSource::Cpu,
            gpu_index: None,
            cull_mode: CullMode::Back,
        });
    }

    let center_color = highlight_color(
        SCALE_HANDLE_COLOR,
        hovered == Some(TransformGizmoHandle::ScaleUniform),
    );
    gizmos.push(RenderObject {
        mesh: resources.scale_cube,
        material: solid_color_material(center_color),
        transform: Transform::from_trs(
            gizmo_transform.translation,
            Quat::IDENTITY,
            Vec3::splat(SCALE_HANDLE_SIZE * base_scale * 0.8),
        ),
        depth_state: GIZMO_DEPTH,
        force_overlay: false,
        render_pass: Some(RenderPass::GizmoSolid),
        instance_source: InstanceSource::Cpu,
        gpu_index: None,
        cull_mode: CullMode::Back,
    });

    gizmos
}

fn build_rotate_gizmo(
    camera: CameraVectors,
    gizmo_transform: Transform,
    resources: TransformGizmoResources,
    hovered: Option<TransformGizmoHandle>,
) -> Vec<RenderObject> {
    let mut gizmos = Vec::new();
    let base_scale = gizmo_screen_scale(camera, gizmo_transform.translation);
    let ring_radius = ROTATION_RING_RADIUS * base_scale;
    let ring_scale = Vec3::splat(ring_radius * 2.0);

    for axis in AXES {
        let axis_vec = axis_direction(gizmo_transform.rotation, axis);
        let color = highlight_color(
            axis_color(axis),
            hovered == Some(TransformGizmoHandle::RotateAxis(axis)),
        );
        let rotation = align_ring(axis_vec);
        gizmos.push(RenderObject {
            mesh: resources.rotation_ring,
            material: solid_color_material(color),
            transform: Transform::from_trs(gizmo_transform.translation, rotation, ring_scale),
            depth_state: GIZMO_DEPTH,
            force_overlay: false,
            render_pass: Some(RenderPass::Gizmo),
            instance_source: InstanceSource::Cpu,
            gpu_index: None,
            cull_mode: CullMode::None,
        });
    }

    let camera_forward = safe_normalize(camera.target - camera.position, Vec3::NEG_Z);
    let screen_hovered = hovered == Some(TransformGizmoHandle::RotateScreen);
    let screen_color = highlight_color(RING_BASE_COLOR, screen_hovered);
    gizmos.push(RenderObject {
        mesh: resources.rotation_ring_highlight,
        material: solid_color_material(screen_color),
        transform: Transform::from_trs(
            gizmo_transform.translation,
            align_ring(-camera_forward),
            Vec3::splat(ring_radius * 2.4),
        ),
        depth_state: GIZMO_DEPTH,
        force_overlay: false,
        render_pass: Some(RenderPass::Gizmo),
        instance_source: InstanceSource::Cpu,
        gpu_index: None,
        cull_mode: CullMode::None,
    });

    gizmos
}

fn solid_color_material(color: [u8; 4]) -> Material {
    Material::new(color).with_unlit().with_alpha()
}

pub(crate) fn gizmo_screen_scale(camera: CameraVectors, position: Vec3) -> f32 {
    let distance = (camera.position - position).length().max(0.1);
    let half_fov = (camera.fov_y * 0.5).max(1e-4);
    if half_fov <= 1e-3 {
        1.0
    } else {
        2.0 * distance * half_fov.tan() * 0.16
    }
}

fn align_vector(from: Vec3, direction: Vec3) -> Quat {
    if from.length_squared() <= 0.0 || direction.length_squared() <= 0.0 {
        Quat::IDENTITY
    } else {
        Quat::from_rotation_arc(from.normalize(), direction.normalize())
    }
}

fn axis_base(axis: TransformGizmoAxis) -> Vec3 {
    match axis {
        TransformGizmoAxis::X => Vec3::X,
        TransformGizmoAxis::Y => Vec3::Y,
        TransformGizmoAxis::Z => Vec3::Z,
    }
}

fn axis_direction(rotation: Quat, axis: TransformGizmoAxis) -> Vec3 {
    let dir = rotation * axis_base(axis);
    if dir.length_squared() <= 0.0 {
        axis_base(axis)
    } else {
        dir.normalize()
    }
}

fn axis_color(axis: TransformGizmoAxis) -> [u8; 4] {
    match axis {
        TransformGizmoAxis::X => AXIS_X_COLOR,
        TransformGizmoAxis::Y => AXIS_Y_COLOR,
        TransformGizmoAxis::Z => AXIS_Z_COLOR,
    }
}

fn plane_color(axis_a: TransformGizmoAxis, axis_b: TransformGizmoAxis) -> [u8; 4] {
    let color_a = axis_color(axis_a);
    let color_b = axis_color(axis_b);
    let mut result = [0u8; 4];
    for i in 0..3 {
        result[i] = ((color_a[i] as u16 + color_b[i] as u16) / 2) as u8;
    }
    result[3] = ((color_a[3] as u16 + color_b[3] as u16) / 2).min(255) as u8;
    result
}

fn align_ring(axis: Vec3) -> Quat {
    align_vector(Vec3::Z, axis)
}

fn highlight_color(mut color: [u8; 4], hovered: bool) -> [u8; 4] {
    if hovered {
        for channel in &mut color[0..3] {
            let boosted = (*channel as u16 + 60).min(255) as u8;
            *channel = boosted;
        }
        color[3] = color[3].max(200);
    }
    color
}

fn distance_point_to_ray(origin: Vec3, direction: Vec3, point: Vec3) -> Option<f32> {
    let diff = point - origin;
    let t = diff.dot(direction);
    if t < 0.0 {
        return None;
    }
    let closest = origin + direction * t;
    Some((closest - point).length())
}

fn distance_to_axis_segment(
    ray_origin: Vec3,
    ray_dir: Vec3,
    axis_origin: Vec3,
    axis_dir: Vec3,
    axis_length: f32,
) -> Option<f32> {
    if axis_dir.length_squared() < 1e-6 {
        return None;
    }

    let axis_dir = axis_dir.normalize();
    let mut w0 = ray_origin - axis_origin;
    let a = ray_dir.dot(ray_dir);
    let b = ray_dir.dot(axis_dir);
    let c = axis_dir.dot(axis_dir);
    let d = ray_dir.dot(w0);
    let e = axis_dir.dot(w0);
    let denom = a * c - b * b;

    let (mut s, mut t) = if denom.abs() > 1e-6 {
        let s = (b * e - c * d) / denom;
        let t = (a * e - b * d) / denom;
        (s, t)
    } else {
        // Nearly parallel: project ray origin onto axis
        (0.0, e / c)
    };

    if s < 0.0 {
        s = 0.0;
        w0 = ray_origin - axis_origin;
        t = axis_dir.dot(w0);
    }

    let t_clamped = t.clamp(0.0, axis_length.max(0.0));
    let point_axis = axis_origin + axis_dir * t_clamped;
    let point_ray = ray_origin + ray_dir * s;
    Some((point_axis - point_ray).length())
}

fn ring_hit(
    ray_origin: Vec3,
    ray_dir: Vec3,
    origin: Vec3,
    normal: Vec3,
    radius: f32,
    tolerance: f32,
) -> Option<f32> {
    let denom = ray_dir.dot(normal);
    if denom.abs() < 1e-5 {
        return None;
    }
    let t = (origin - ray_origin).dot(normal) / denom;
    if t < 0.0 {
        return None;
    }
    let hit = ray_origin + ray_dir * t;
    let distance = (hit - origin).length();
    let delta = (distance - radius).abs();
    if delta <= tolerance {
        Some(delta)
    } else {
        None
    }
}
