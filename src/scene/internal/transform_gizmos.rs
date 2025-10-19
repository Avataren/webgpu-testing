use super::lights::safe_normalize;
use super::rendering::CameraVectors;
use crate::asset::{Assets, Handle, Mesh};
use crate::renderer::batch::{CullMode, InstanceSource, RenderObject, RenderPass};
use crate::renderer::primitives::{cone_mesh, cube_mesh, cylinder_mesh, torus_mesh};
use crate::renderer::{Material, Renderer};
use crate::scene::components::DepthState;
use crate::scene::transform::Transform;
use crate::scene::TransformGizmoMode;
use glam::{Quat, Vec3};

#[derive(Clone, Copy)]
pub(crate) struct TransformGizmoResources {
    pub axis_cylinder: Handle<Mesh>,
    pub axis_cone: Handle<Mesh>,
    pub scale_cube: Handle<Mesh>,
    pub rotation_ring: Handle<Mesh>,
    pub rotation_ring_highlight: Handle<Mesh>,
}

const AXIS_X_COLOR: [u8; 4] = [250, 80, 90, 230];
const AXIS_Y_COLOR: [u8; 4] = [80, 220, 120, 230];
const AXIS_Z_COLOR: [u8; 4] = [90, 140, 250, 230];
const AXIS_CENTER_COLOR: [u8; 4] = [240, 240, 240, 200];

const RING_OPACITY: u8 = 180;
const SCALE_HANDLE_COLOR: [u8; 4] = [240, 240, 240, 220];

const TRANSLATE_SHAFT_LENGTH: f32 = 0.9;
const TRANSLATE_SHAFT_RADIUS: f32 = 0.035;
const TRANSLATE_CONE_LENGTH: f32 = 0.25;
const TRANSLATE_CONE_RADIUS: f32 = 0.12;

const SCALE_HANDLE_SIZE: f32 = 0.18;
const SCALE_HANDLE_OFFSET: f32 = 0.9;

const ROTATION_RING_RADIUS: f32 = 1.0;
const ROTATION_RING_THICKNESS: f32 = 0.04;

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

    let (ring_vertices, ring_indices) = torus_mesh(64, 12, 0.5, ROTATION_RING_THICKNESS * 0.5);
    let rotation_ring = assets
        .meshes
        .insert(renderer.create_mesh(&ring_vertices, &ring_indices));

    // Thin highlight ring
    let (ring_high_vertices, ring_high_indices) =
        torus_mesh(64, 6, 0.5, ROTATION_RING_THICKNESS * 0.25);
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
) -> Vec<RenderObject> {
    match mode {
        TransformGizmoMode::Translate => build_translate_gizmo(camera, gizmo_transform, resources),
        TransformGizmoMode::Rotate => build_rotate_gizmo(camera, gizmo_transform, resources),
        TransformGizmoMode::Scale => build_scale_gizmo(camera, gizmo_transform, resources),
    }
}

fn build_translate_gizmo(
    camera: CameraVectors,
    gizmo_transform: Transform,
    resources: TransformGizmoResources,
) -> Vec<RenderObject> {
    let mut gizmos = Vec::new();
    let base_scale = gizmo_screen_scale(camera, gizmo_transform.translation);
    let shaft_length = TRANSLATE_SHAFT_LENGTH * base_scale;
    let shaft_radius = TRANSLATE_SHAFT_RADIUS * base_scale;
    let cone_length = TRANSLATE_CONE_LENGTH * base_scale;
    let cone_radius = TRANSLATE_CONE_RADIUS * base_scale;

    let axes = [
        (Vec3::X, AXIS_X_COLOR),
        (Vec3::Y, AXIS_Y_COLOR),
        (Vec3::Z, AXIS_Z_COLOR),
    ];

    for (axis, color) in axes {
        let rotation = align_vector(Vec3::Z, axis);
        let shaft_translation = gizmo_transform.translation + axis * (shaft_length * 0.5);
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

        let cone_translation = gizmo_transform.translation + axis * (shaft_length + cone_length);
        let cone_rotation = align_vector(Vec3::NEG_Z, axis);
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

    // Center cube handle
    let center_size = SCALE_HANDLE_SIZE * base_scale * 0.6;
    gizmos.push(RenderObject {
        mesh: resources.scale_cube,
        material: solid_color_material(AXIS_CENTER_COLOR),
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
) -> Vec<RenderObject> {
    let mut gizmos = Vec::new();
    let base_scale = gizmo_screen_scale(camera, gizmo_transform.translation);

    let axes = [
        (Vec3::X, AXIS_X_COLOR),
        (Vec3::Y, AXIS_Y_COLOR),
        (Vec3::Z, AXIS_Z_COLOR),
    ];

    for (axis, color) in axes {
        let offset = axis * SCALE_HANDLE_OFFSET * base_scale;
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

    // Center scaling handle
    gizmos.push(RenderObject {
        mesh: resources.scale_cube,
        material: solid_color_material(SCALE_HANDLE_COLOR),
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
) -> Vec<RenderObject> {
    let mut gizmos = Vec::new();
    let base_scale = gizmo_screen_scale(camera, gizmo_transform.translation);
    let ring_radius = ROTATION_RING_RADIUS * base_scale;
    let ring_scale = Vec3::new(ring_radius, ring_radius, ring_radius);

    let axes = [
        (Vec3::X, AXIS_X_COLOR),
        (Vec3::Y, AXIS_Y_COLOR),
        (Vec3::Z, AXIS_Z_COLOR),
    ];

    for (axis, color) in axes {
        let ring_rotation = align_ring(axis);
        gizmos.push(RenderObject {
            mesh: resources.rotation_ring,
            material: solid_color_material([color[0], color[1], color[2], RING_OPACITY]),
            transform: Transform::from_trs(gizmo_transform.translation, ring_rotation, ring_scale),
            depth_state: GIZMO_DEPTH,
            force_overlay: false,
            render_pass: Some(RenderPass::Gizmo),
            instance_source: InstanceSource::Cpu,
            gpu_index: None,
            cull_mode: CullMode::None,
        });
    }

    // Screen aligned free-rotate ring
    gizmos.push(RenderObject {
        mesh: resources.rotation_ring_highlight,
        material: solid_color_material([200, 200, 200, RING_OPACITY]),
        transform: Transform::from_trs(
            gizmo_transform.translation,
            camera_aligned_rotation(camera),
            Vec3::new(ring_radius * 1.2, ring_radius * 1.2, ring_radius * 1.2),
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

fn gizmo_screen_scale(camera: CameraVectors, position: Vec3) -> f32 {
    let distance = (camera.position - position).length().max(0.1);
    let half_fov = (camera.fov_y * 0.5).max(1e-4);
    if half_fov <= 1e-3 {
        1.0
    } else {
        2.0 * distance * half_fov.tan() * 0.16
    }
}

fn align_vector(from: Vec3, direction: Vec3) -> Quat {
    if direction.length_squared() <= 0.0 || from.length_squared() <= 0.0 {
        Quat::IDENTITY
    } else {
        Quat::from_rotation_arc(from.normalize(), direction.normalize())
    }
}

fn align_ring(axis: Vec3) -> Quat {
    if axis.length_squared() <= 0.0 {
        Quat::IDENTITY
    } else {
        // ring mesh lies in XY plane; align its normal (Z) with desired axis
        align_vector(Vec3::Z, axis)
    }
}

fn camera_aligned_rotation(camera: CameraVectors) -> Quat {
    let forward = safe_normalize(camera.target - camera.position, Vec3::NEG_Z);
    align_ring(-forward)
}
