use super::lights::{resolve_light_transform, safe_normalize};
use super::rendering::{apply_billboard_transform, CameraVectors};
use crate::asset::{Assets, Handle, Mesh};
use crate::renderer::batch::{CullMode, InstanceSource, RenderObject, RenderPass};
use crate::renderer::primitives::{cone_mesh, cube_mesh, quad_mesh, sphere_mesh};
use crate::renderer::{Material, Renderer};
use crate::scene::components::{
    Billboard, BillboardOrientation, DepthState, DirectionalLight, PointLight, SpotLight,
    TransformComponent, WorldTransform,
};
use crate::scene::transform::Transform;
use glam::{Quat, Vec3};
use hecs::World;

#[derive(Clone, Copy)]
pub(crate) struct GizmoResources {
    pub quad: Handle<Mesh>,
    pub sphere: Handle<Mesh>,
    pub cone: Handle<Mesh>,
    pub cube: Handle<Mesh>,
}

const POINT_SPRITE_COLOR: [u8; 4] = [255, 226, 120, 255];
const SPOT_SPRITE_COLOR: [u8; 4] = [120, 220, 255, 255];
const DIRECTIONAL_SPRITE_COLOR: [u8; 4] = [140, 180, 255, 255];
const POINT_VOLUME_COLOR: [u8; 4] = [255, 226, 120, 72];
const SPOT_VOLUME_COLOR: [u8; 4] = [120, 220, 255, 72];
const DIRECTIONAL_SHAFT_COLOR: [u8; 4] = [140, 180, 255, 160];
const DIRECTIONAL_HEAD_COLOR: [u8; 4] = [140, 180, 255, 200];

const BILLBOARD_SCALE: f32 = 0.65;
const DIRECTIONAL_SHAFT_LENGTH: f32 = 2.25;
const DIRECTIONAL_SHAFT_THICKNESS: f32 = 0.08;
const DIRECTIONAL_HEAD_LENGTH: f32 = 0.45;
const DIRECTIONAL_HEAD_RADIUS: f32 = 0.28;

pub(crate) fn create_resources(renderer: &Renderer, assets: &mut Assets) -> GizmoResources {
    let (quad_vertices, quad_indices) = quad_mesh();
    let quad_mesh = renderer.create_mesh(&quad_vertices, &quad_indices);
    let quad = assets.meshes.insert(quad_mesh);

    let (sphere_vertices, sphere_indices) = sphere_mesh(16, 16);
    let sphere_mesh = renderer.create_mesh(&sphere_vertices, &sphere_indices);
    let sphere = assets.meshes.insert(sphere_mesh);

    let (cone_vertices, cone_indices) = cone_mesh(24);
    let cone_mesh = renderer.create_mesh(&cone_vertices, &cone_indices);
    let cone = assets.meshes.insert(cone_mesh);

    let (cube_vertices, cube_indices) = cube_mesh();
    let cube_mesh = renderer.create_mesh(&cube_vertices, &cube_indices);
    let cube = assets.meshes.insert(cube_mesh);

    GizmoResources {
        quad,
        sphere,
        cone,
        cube,
    }
}

pub(crate) fn build_light_gizmos(
    world: &World,
    camera: CameraVectors,
    root_transform: Transform,
    resources: GizmoResources,
) -> Vec<RenderObject> {
    let mut gizmos = Vec::new();

    build_point_light_gizmos(world, camera, root_transform, resources, &mut gizmos);
    build_spot_light_gizmos(world, camera, root_transform, resources, &mut gizmos);
    build_directional_light_gizmos(world, camera, root_transform, resources, &mut gizmos);

    gizmos
}

fn build_point_light_gizmos(
    world: &World,
    camera: CameraVectors,
    root_transform: Transform,
    resources: GizmoResources,
    output: &mut Vec<RenderObject>,
) {
    for (_entity, (light, world_transform, local_transform)) in world
        .query::<(
            &PointLight,
            Option<&WorldTransform>,
            Option<&TransformComponent>,
        )>()
        .iter()
    {
        let mut transform = resolve_light_transform(world_transform, local_transform);
        transform = root_transform.mul_transform(&transform);

        let billboard = Billboard::new(BillboardOrientation::FaceCamera);
        let icon_transform = apply_billboard_transform(
            Transform::from_trs(
                transform.translation,
                Quat::IDENTITY,
                Vec3::splat(BILLBOARD_SCALE),
            ),
            billboard,
            camera.position,
            camera.target,
            camera.up,
        );

        output.push(RenderObject {
            mesh: resources.quad,
            material: sprite_material(POINT_SPRITE_COLOR),
            transform: icon_transform,
            depth_state: DepthState::new(false, false),
            force_overlay: false,
            render_pass: Some(RenderPass::Gizmo),
            instance_source: InstanceSource::Cpu,
            gpu_index: None,
            cull_mode: CullMode::Back,
        });

        let radius = light.range.max(0.01);
        output.push(RenderObject {
            mesh: resources.sphere,
            material: volume_material(POINT_VOLUME_COLOR),
            transform: Transform::from_trs(
                transform.translation,
                Quat::IDENTITY,
                Vec3::splat(radius),
            ),
            depth_state: DepthState::new(false, false),
            force_overlay: false,
            render_pass: Some(RenderPass::Gizmo),
            instance_source: InstanceSource::Cpu,
            gpu_index: None,
            cull_mode: CullMode::Front,
        });
    }
}

fn build_spot_light_gizmos(
    world: &World,
    camera: CameraVectors,
    root_transform: Transform,
    resources: GizmoResources,
    output: &mut Vec<RenderObject>,
) {
    for (_entity, (light, world_transform, local_transform)) in world
        .query::<(
            &SpotLight,
            Option<&WorldTransform>,
            Option<&TransformComponent>,
        )>()
        .iter()
    {
        let mut transform = resolve_light_transform(world_transform, local_transform);
        transform = root_transform.mul_transform(&transform);

        let billboard = Billboard::new(BillboardOrientation::FaceCamera);
        let icon_transform = apply_billboard_transform(
            Transform::from_trs(
                transform.translation,
                Quat::IDENTITY,
                Vec3::splat(BILLBOARD_SCALE),
            ),
            billboard,
            camera.position,
            camera.target,
            camera.up,
        );

        output.push(RenderObject {
            mesh: resources.quad,
            material: sprite_material(SPOT_SPRITE_COLOR),
            transform: icon_transform,
            depth_state: DepthState::new(false, false),
            force_overlay: false,
            render_pass: Some(RenderPass::Gizmo),
            instance_source: InstanceSource::Cpu,
            gpu_index: None,
            cull_mode: CullMode::Back,
        });

        let range = light.range.max(0.01);
        let outer_angle = light.outer_angle.max(0.01);
        let radius = (outer_angle.tan() * range).max(0.05);
        let rotation = transform.rotation;

        output.push(RenderObject {
            mesh: resources.cone,
            material: volume_material(SPOT_VOLUME_COLOR),
            transform: Transform::from_trs(
                transform.translation,
                rotation,
                Vec3::new(radius, radius, range),
            ),
            depth_state: DepthState::new(false, false),
            force_overlay: false,
            render_pass: Some(RenderPass::Gizmo),
            instance_source: InstanceSource::Cpu,
            gpu_index: None,
            cull_mode: CullMode::Front,
        });
    }
}

fn build_directional_light_gizmos(
    world: &World,
    camera: CameraVectors,
    root_transform: Transform,
    resources: GizmoResources,
    output: &mut Vec<RenderObject>,
) {
    for (_entity, (_light, world_transform, local_transform)) in world
        .query::<(
            &DirectionalLight,
            Option<&WorldTransform>,
            Option<&TransformComponent>,
        )>()
        .iter()
    {
        let mut transform = resolve_light_transform(world_transform, local_transform);
        transform = root_transform.mul_transform(&transform);
        let position = transform.translation;
        let direction = safe_normalize(transform.rotation * Vec3::NEG_Z, Vec3::NEG_Z);

        let billboard = Billboard::new(BillboardOrientation::FaceCamera);
        let icon_transform = apply_billboard_transform(
            Transform::from_trs(position, Quat::IDENTITY, Vec3::splat(BILLBOARD_SCALE)),
            billboard,
            camera.position,
            camera.target,
            camera.up,
        );

        output.push(RenderObject {
            mesh: resources.quad,
            material: sprite_material(DIRECTIONAL_SPRITE_COLOR),
            transform: icon_transform,
            depth_state: DepthState::new(false, false),
            force_overlay: false,
            render_pass: Some(RenderPass::Gizmo),
            instance_source: InstanceSource::Cpu,
            gpu_index: None,
            cull_mode: CullMode::Back,
        });

        let shaft_rotation = align_vector(Vec3::Z, direction);
        let shaft_translation = position + direction * (DIRECTIONAL_SHAFT_LENGTH * 0.5);
        output.push(RenderObject {
            mesh: resources.cube,
            material: volume_material(DIRECTIONAL_SHAFT_COLOR),
            transform: Transform::from_trs(
                shaft_translation,
                shaft_rotation,
                Vec3::new(
                    DIRECTIONAL_SHAFT_THICKNESS,
                    DIRECTIONAL_SHAFT_THICKNESS,
                    DIRECTIONAL_SHAFT_LENGTH,
                ),
            ),
            depth_state: DepthState::new(false, false),
            force_overlay: false,
            render_pass: Some(RenderPass::Gizmo),
            instance_source: InstanceSource::Cpu,
            gpu_index: None,
            cull_mode: CullMode::Back,
        });

        let head_rotation = align_vector(Vec3::NEG_Z, direction);
        let head_apex = position + direction * (DIRECTIONAL_SHAFT_LENGTH + DIRECTIONAL_HEAD_LENGTH);
        output.push(RenderObject {
            mesh: resources.cone,
            material: volume_material(DIRECTIONAL_HEAD_COLOR),
            transform: Transform::from_trs(
                head_apex,
                head_rotation,
                Vec3::new(
                    DIRECTIONAL_HEAD_RADIUS,
                    DIRECTIONAL_HEAD_RADIUS,
                    DIRECTIONAL_HEAD_LENGTH,
                ),
            ),
            depth_state: DepthState::new(false, false),
            force_overlay: false,
            render_pass: Some(RenderPass::Gizmo),
            instance_source: InstanceSource::Cpu,
            gpu_index: None,
            cull_mode: CullMode::Back,
        });
    }
}

fn sprite_material(color: [u8; 4]) -> Material {
    let mut material = Material::checker().with_unlit().with_alpha();
    material.base_color = color;
    material
}

fn volume_material(color: [u8; 4]) -> Material {
    let mut material = Material::new(color).with_unlit().with_alpha();
    material.base_color = color;
    material
}

fn align_vector(from: Vec3, direction: Vec3) -> Quat {
    if direction.length_squared() <= 0.0 {
        Quat::IDENTITY
    } else {
        Quat::from_rotation_arc(from, direction.normalize())
    }
}
